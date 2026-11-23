//! Live updates: Linear's websocket, spoken in `graphql-transport-ws` with a
//! small hand-written client over tokio-tungstenite (ADR 0003). The key goes
//! in the `Authorization` header of the upgrade request. Every disconnect is
//! followed by a reconnect after a jittered, growing wait, so the stream
//! never ends; polling stays in charge of correctness.

use std::collections::VecDeque;
use std::collections::hash_map::RandomState;
use std::fmt;
use std::future::Future;
use std::hash::{BuildHasher, Hasher};
use std::io;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, anyhow, bail};
use futures_util::{SinkExt, Stream, StreamExt};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::TcpStream;
use tokio::time::{Instant, sleep, sleep_until, timeout_at};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::header::{
    AUTHORIZATION, HeaderValue, SEC_WEBSOCKET_PROTOCOL, USER_AGENT,
};
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, client_async_tls};

use super::HttpClient;
use super::assigned::{Decoded, IssueNode};
use super::comment::CommentNode;
use super::notifications::{self, NotificationNode};
use crate::linear::{LiveEvent, PostedComment, TeamChanges};

/// Linear's websocket endpoint.
pub const LIVE_ENDPOINT: &str = "wss://api.linear.app/graphql";

const PROTOCOL: &str = "graphql-transport-ws";

const ISSUE_CREATED: &str = include_str!("../graphql/issue_created_live.graphql");
const ISSUE_UPDATED: &str = include_str!("../graphql/issue_updated_live.graphql");
const COMMENT_CREATED: &str = include_str!("../graphql/comment_created_live.graphql");
const ISSUE_ARCHIVED: &str = include_str!("../graphql/issue_archived_live.graphql");
const ISSUE_UNARCHIVED: &str = include_str!("../graphql/issue_unarchived_live.graphql");
const COMMENT_UPDATED: &str = include_str!("../graphql/comment_updated_live.graphql");
const COMMENT_DELETED: &str = include_str!("../graphql/comment_deleted_live.graphql");
const NOTIFICATION_CREATED: &str = include_str!("../graphql/notification_created_live.graphql");
const NOTIFICATION_UPDATED: &str = include_str!("../graphql/notification_updated_live.graphql");
const NOTIFICATION_ARCHIVED: &str = include_str!("../graphql/notification_archived_live.graphql");

/// The subscriptions with no team filter, after the per-team ones, by the
/// id each goes out under.
const WORKSPACE_SUBSCRIPTIONS: [(&str, &str); 8] = [
    ("commentCreated", COMMENT_CREATED),
    ("issueArchived", ISSUE_ARCHIVED),
    ("issueUnarchived", ISSUE_UNARCHIVED),
    ("commentUpdated", COMMENT_UPDATED),
    ("commentDeleted", COMMENT_DELETED),
    ("notificationCreated", NOTIFICATION_CREATED),
    ("notificationUpdated", NOTIFICATION_UPDATED),
    ("notificationArchived", NOTIFICATION_ARCHIVED),
];

/// A connection that stayed up this long starts the backoff over.
const STABLE_AFTER: Duration = Duration::from_secs(60);

/// A byte stream the websocket runs over: a TCP connection, which TLS
/// wraps for `wss`, or an in-memory pipe in tests.
pub trait Pipe: AsyncRead + AsyncWrite + Send + Unpin {}

impl<T: AsyncRead + AsyncWrite + Send + Unpin> Pipe for T {}

/// Opens a [`Pipe`] to a host and port.
pub type Dial = Arc<
    dyn Fn(String, u16) -> Pin<Box<dyn Future<Output = io::Result<Box<dyn Pipe>>> + Send>>
        + Send
        + Sync,
>;

/// Where live updates connect and how patient they are.
#[derive(Clone)]
pub struct LiveConfig {
    pub endpoint: String,
    /// How the connection to the endpoint's host is opened; TCP by default.
    pub dial: Dial,
    /// Time allowed to connect, and then for `connection_ack` to come.
    pub ack_timeout: Duration,
    /// Time between two subscriptions, so Linear does not close the socket
    /// for subscribing too fast.
    pub subscribe_gap: Duration,
    /// Silence from Linear this long sends a ping; as long again without an
    /// answer drops the connection.
    pub keep_alive: Duration,
    /// The wait before the first reconnect; it doubles up to `max_backoff`,
    /// and each wait is jittered down by up to half.
    pub first_backoff: Duration,
    pub max_backoff: Duration,
}

impl fmt::Debug for LiveConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LiveConfig")
            .field("endpoint", &self.endpoint)
            .field("ack_timeout", &self.ack_timeout)
            .field("subscribe_gap", &self.subscribe_gap)
            .field("keep_alive", &self.keep_alive)
            .field("first_backoff", &self.first_backoff)
            .field("max_backoff", &self.max_backoff)
            .finish_non_exhaustive()
    }
}

impl Default for LiveConfig {
    fn default() -> Self {
        Self {
            endpoint: LIVE_ENDPOINT.to_owned(),
            dial: Arc::new(|host, port| {
                Box::pin(async move {
                    let stream = TcpStream::connect((host.as_str(), port)).await?;
                    Ok(Box::new(stream) as Box<dyn Pipe>)
                })
            }),
            ack_timeout: Duration::from_secs(10),
            subscribe_gap: Duration::from_millis(1500),
            keep_alive: Duration::from_secs(30),
            first_backoff: Duration::from_secs(1),
            max_backoff: Duration::from_secs(60),
        }
    }
}

/// The live events for the issues of `team_ids`, the comments and issue
/// archives of the workspace, and notifications.
pub(super) fn events(
    client: &HttpClient,
    team_ids: Vec<String>,
) -> impl Stream<Item = LiveEvent> + '_ {
    let live = Live {
        client,
        backoff: Backoff::new(client.live.first_backoff, client.live.max_backoff),
        wait: Duration::ZERO,
        session: None,
        team_ids,
    };
    futures_util::stream::unfold(live, |mut live| async move {
        let event = live.next().await;
        Some((event, live))
    })
}

struct Live<'a> {
    client: &'a HttpClient,
    team_ids: Vec<String>,
    backoff: Backoff,
    /// The wait before the next connection attempt.
    wait: Duration,
    session: Option<Session>,
}

impl Live<'_> {
    async fn next(&mut self) -> LiveEvent {
        loop {
            let Some(session) = &mut self.session else {
                sleep(self.wait).await;
                match self.connect().await {
                    Ok(session) => {
                        tracing::info!("live updates connected");
                        self.session = Some(session);
                        return LiveEvent::Connected;
                    }
                    Err(error) => {
                        self.wait = self.backoff.next(jitter());
                        tracing::warn!(
                            seconds = self.wait.as_secs_f64(),
                            "could not open live updates, trying again: {error:#}"
                        );
                        continue;
                    }
                }
            };
            match session.next(self.client, &self.team_ids).await {
                Ok(event) => return event,
                Err(error) => {
                    if session.opened.elapsed() >= STABLE_AFTER {
                        self.backoff.reset();
                    }
                    self.session = None;
                    self.wait = self.backoff.next(jitter());
                    tracing::warn!(
                        seconds = self.wait.as_secs_f64(),
                        "live updates disconnected, reconnecting: {error:#}"
                    );
                }
            }
        }
    }

    /// Opens the socket, sends `connection_init` at once and waits for the
    /// ack, all within the ack timeout.
    async fn connect(&self) -> anyhow::Result<Session> {
        let config = &self.client.live;
        let deadline = Instant::now() + config.ack_timeout;
        let mut request = config
            .endpoint
            .as_str()
            .into_client_request()
            .context("the live endpoint is not a websocket URL")?;
        let headers = request.headers_mut();
        headers.insert(AUTHORIZATION, self.client.authorization.clone());
        headers.insert(SEC_WEBSOCKET_PROTOCOL, HeaderValue::from_static(PROTOCOL));
        headers.insert(
            USER_AGENT,
            HeaderValue::from_static(concat!("lrat/", env!("CARGO_PKG_VERSION"))),
        );
        let uri = request.uri();
        let host = uri
            .host()
            .context("the live endpoint has no host")?
            .to_owned();
        let port = uri.port_u16().unwrap_or(if uri.scheme_str() == Some("ws") {
            80
        } else {
            443
        });
        let opened = async {
            let pipe = (config.dial)(host, port)
                .await
                .context("could not reach the live endpoint")?;
            client_async_tls(request, pipe)
                .await
                .context("the websocket upgrade failed")
        };
        let (mut socket, _) = timeout_at(deadline, opened)
            .await
            .map_err(|_| anyhow!("the connection timed out"))??;
        send(
            &mut socket,
            json!({ "type": "connection_init", "payload": {} }),
        )
        .await?;
        loop {
            let message = timeout_at(deadline, socket.next())
                .await
                .map_err(|_| anyhow!("Linear did not acknowledge the connection in time"))?
                .context("the socket closed before the ack")?
                .context("reading the ack")?;
            let Message::Text(text) = message else {
                continue;
            };
            match serde_json::from_str::<ServerMessage>(&text)?.kind.as_str() {
                "connection_ack" => break,
                "ping" => send(&mut socket, json!({ "type": "pong" })).await?,
                other => bail!("expected connection_ack, got {other}"),
            }
        }
        let now = Instant::now();
        Ok(Session {
            socket,
            to_subscribe: subscriptions(&self.team_ids),
            next_subscribe: now,
            last_heard: now,
            pinged: false,
            opened: now,
        })
    }
}

type Socket = WebSocketStream<MaybeTlsStream<Box<dyn Pipe>>>;

/// One acknowledged connection.
struct Session {
    socket: Socket,
    /// Subscriptions not sent yet, as `subscribe` messages.
    to_subscribe: VecDeque<Value>,
    next_subscribe: Instant,
    last_heard: Instant,
    /// A ping went out and nothing has come back since.
    pinged: bool,
    opened: Instant,
}

impl Session {
    /// The next event this connection delivers, subscribing and keeping
    /// the connection alive meanwhile. An error means it is gone.
    async fn next(
        &mut self,
        client: &HttpClient,
        team_ids: &[String],
    ) -> anyhow::Result<LiveEvent> {
        let config = &client.live;
        loop {
            let subscribing = !self.to_subscribe.is_empty();
            tokio::select! {
                message = self.socket.next() => {
                    let message = message
                        .context("the socket closed")?
                        .context("reading from the socket")?;
                    self.last_heard = Instant::now();
                    self.pinged = false;
                    match message {
                        Message::Text(text) => {
                            if let Some(event) = self.receive(&text, client, team_ids).await? {
                                return Ok(event);
                            }
                        }
                        Message::Close(frame) => bail!("Linear closed the connection: {frame:?}"),
                        _ => {}
                    }
                }
                () = sleep_until(self.next_subscribe), if subscribing => {
                    if let Some(subscribe) = self.to_subscribe.pop_front() {
                        send(&mut self.socket, subscribe).await?;
                    }
                    self.next_subscribe = Instant::now() + config.subscribe_gap;
                }
                () = sleep_until(self.last_heard + config.keep_alive) => {
                    if self.pinged {
                        bail!("Linear did not answer a ping");
                    }
                    send(&mut self.socket, json!({ "type": "ping" })).await?;
                    self.pinged = true;
                    self.last_heard = Instant::now();
                }
            }
        }
    }

    /// Acts on one protocol message, and returns the event it carries.
    async fn receive(
        &mut self,
        text: &str,
        client: &HttpClient,
        team_ids: &[String],
    ) -> anyhow::Result<Option<LiveEvent>> {
        let message: ServerMessage =
            serde_json::from_str(text).context("decoding a protocol message")?;
        match message.kind.as_str() {
            "ping" => send(&mut self.socket, json!({ "type": "pong" })).await?,
            "next" => return Ok(decode(message.payload, client, team_ids)),
            "error" => tracing::warn!(
                id = ?message.id,
                errors = %message.payload,
                "Linear refused a live subscription"
            ),
            "complete" => tracing::warn!(id = ?message.id, "Linear ended a live subscription"),
            _ => {}
        }
        Ok(None)
    }
}

/// The `subscribe` messages for `team_ids`: new and updated issues of each
/// team, then those of the whole workspace: new, edited and deleted
/// comments, archived and unarchived issues, and notifications.
fn subscriptions(team_ids: &[String]) -> VecDeque<Value> {
    let mut subscribe = VecDeque::new();
    for team_id in team_ids {
        for (name, query) in [
            ("issueCreated", ISSUE_CREATED),
            ("issueUpdated", ISSUE_UPDATED),
        ] {
            subscribe.push_back(json!({
                "id": format!("{name}:{team_id}"),
                "type": "subscribe",
                "payload": { "query": query, "variables": { "teamId": team_id } },
            }));
        }
    }
    for (id, query) in WORKSPACE_SUBSCRIPTIONS {
        subscribe.push_back(json!({
            "id": id,
            "type": "subscribe",
            "payload": { "query": query, "variables": {} },
        }));
    }
    subscribe
}

/// The event in a `next` payload, or `None` for one lrat does not show or
/// cannot read, which is logged and skipped.
fn decode(payload: Value, client: &HttpClient, team_ids: &[String]) -> Option<LiveEvent> {
    let payload: NextPayload = match serde_json::from_value(payload) {
        Ok(payload) => payload,
        Err(error) => {
            tracing::warn!(%error, "could not read a live event");
            return None;
        }
    };
    for error in &payload.errors {
        tracing::warn!(%error, "a live event also carried an error");
    }
    let data = payload.data?;
    let as_of = client.timer.now();
    if let Some(node) = data.issue_created.or(data.issue_updated) {
        return team_changes(node, team_ids, as_of).map(LiveEvent::Issue);
    }
    // Archives are not filtered by team, so this guard is the only one.
    if let Some(node) = data.issue_archived {
        return team_changes(node, team_ids, as_of).map(LiveEvent::Archived);
    }
    if let Some(node) = data.issue_unarchived {
        return team_changes(node, team_ids, as_of).map(LiveEvent::Unarchived);
    }
    if let Some(comment) = data.comment_created {
        return posted(comment).map(LiveEvent::Comment);
    }
    if let Some(comment) = data.comment_updated {
        return posted(comment).map(LiveEvent::CommentUpdated);
    }
    if let Some(deleted) = data.comment_deleted {
        return Some(LiveEvent::CommentDeleted {
            issue_id: deleted.issue_id?,
            comment_id: deleted.id,
        });
    }
    let node = data
        .notification_created
        .or(data.notification_updated)
        .or(data.notification_archived)?;
    let recipient_id = node.user.as_ref()?.id.clone();
    let loaded = notifications::decode([node], as_of);
    if loaded.notifications.is_empty() {
        return None;
    }
    Some(LiveEvent::Notification {
        recipient_id,
        loaded,
    })
}

/// The changes one live issue makes to its team, or `None` for a team lrat
/// did not subscribe to.
fn team_changes(
    node: LiveIssueNode,
    team_ids: &[String],
    as_of: jiff::Timestamp,
) -> Option<TeamChanges> {
    let team_id = node.issue.team.id.clone();
    // The issue subscriptions filter by team already; this guards the store.
    if !team_ids.contains(&team_id) {
        return None;
    }
    let mut decoded = Decoded::default();
    let mut trashed = Vec::new();
    if node.trashed == Some(true) {
        trashed.push(node.issue.id);
    } else {
        decoded.add(node.issue);
    }
    Some(TeamChanges {
        team_id,
        issues: decoded.issues,
        trashed,
        cycles: decoded.cycles,
        labels: Vec::new(),
        members: decoded.members,
        teams: decoded.teams,
        newest: None,
        as_of,
    })
}

/// A comment on an issue, or `None` for one on anything else.
fn posted(comment: CommentNode) -> Option<PostedComment> {
    let issue_id = comment.issue_id.clone()?;
    let (comment, author) = comment.into_comment(&issue_id);
    Some(PostedComment { comment, author })
}

async fn send(socket: &mut Socket, message: Value) -> anyhow::Result<()> {
    socket
        .send(Message::text(message.to_string()))
        .await
        .context("writing to the socket")
}

#[derive(Deserialize)]
struct ServerMessage {
    #[serde(rename = "type")]
    kind: String,
    id: Option<String>,
    #[serde(default)]
    payload: Value,
}

#[derive(Deserialize)]
struct NextPayload {
    data: Option<NextData>,
    #[serde(default)]
    errors: Vec<Value>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct NextData {
    issue_created: Option<LiveIssueNode>,
    issue_updated: Option<LiveIssueNode>,
    comment_created: Option<CommentNode>,
    issue_archived: Option<LiveIssueNode>,
    issue_unarchived: Option<LiveIssueNode>,
    comment_updated: Option<CommentNode>,
    comment_deleted: Option<DeletedComment>,
    notification_created: Option<NotificationNode>,
    notification_updated: Option<NotificationNode>,
    notification_archived: Option<NotificationNode>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct DeletedComment {
    id: String,
    issue_id: Option<String>,
}

#[derive(Deserialize)]
struct LiveIssueNode {
    #[serde(flatten)]
    issue: IssueNode,
    trashed: Option<bool>,
}

/// Waits that double from the first to the most, each cut by a jitter so
/// clients that dropped together do not come back together.
#[derive(Debug, Clone)]
struct Backoff {
    first: Duration,
    max: Duration,
    next: Duration,
}

impl Backoff {
    fn new(first: Duration, max: Duration) -> Self {
        Self {
            first,
            max,
            next: first,
        }
    }

    /// The next wait, between half and all of the current step, with
    /// `fraction` in `0.0..=1.0` choosing where.
    fn next(&mut self, fraction: f64) -> Duration {
        let step = self.next;
        self.next = (step * 2).min(self.max);
        step.mul_f64(0.5 + fraction.clamp(0.0, 1.0) / 2.0)
    }

    fn reset(&mut self) {
        self.next = self.first;
    }
}

/// A fraction in `0.0..=1.0` that differs from call to call, from the
/// standard library's per-hasher random keys; good enough for jitter.
fn jitter() -> f64 {
    let bits = RandomState::new().build_hasher().finish();
    (bits >> 11) as f64 / (1u64 << 53) as f64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn waits_double_up_to_the_most_and_start_over_on_reset() {
        let mut backoff = Backoff::new(Duration::from_secs(1), Duration::from_secs(5));

        let full: Vec<Duration> = (0..5).map(|_| backoff.next(1.0)).collect();
        assert_eq!(full, [1, 2, 4, 5, 5].map(Duration::from_secs));

        backoff.reset();
        assert_eq!(backoff.next(0.0), Duration::from_millis(500));
    }

    #[test]
    fn jitter_stays_between_zero_and_one() {
        for _ in 0..1000 {
            assert!((0.0..=1.0).contains(&jitter()));
        }
    }

    #[test]
    fn each_team_gets_new_and_updated_issues_and_the_workspace_ones_come_once() {
        let ids: Vec<Value> = subscriptions(&["a".to_owned(), "b".to_owned()])
            .into_iter()
            .map(|message| message["id"].clone())
            .collect();

        assert_eq!(
            ids,
            [
                "issueCreated:a",
                "issueUpdated:a",
                "issueCreated:b",
                "issueUpdated:b",
                "commentCreated",
                "issueArchived",
                "issueUnarchived",
                "commentUpdated",
                "commentDeleted",
                "notificationCreated",
                "notificationUpdated",
                "notificationArchived",
            ]
            .map(Value::from)
        );
    }

    #[test]
    fn each_subscription_asks_for_its_own_field() {
        for message in subscriptions(&["a".to_owned()]) {
            let id = message["id"].as_str().unwrap_or_default();
            let field = id.split(':').next().unwrap_or_default();
            let query = message["payload"]["query"].as_str().unwrap_or_default();
            let selects = query.contains(&format!("\n  {field}("))
                || query.contains(&format!("\n  {field} {{"));
            assert!(selects, "{id}: {query}");
        }
    }

    fn client() -> HttpClient {
        let key = crate::auth::ApiKey::new("lin_api_fictional0000000000000000000000000000");
        let Ok(key) = key else {
            panic!("the fictional key is not a key");
        };
        match HttpClient::with_config(&key, super::super::ClientConfig::default()) {
            Ok(client) => client,
            Err(error) => panic!("{error}"),
        }
    }

    fn issue(team_id: &str, archived_at: Option<&str>) -> Value {
        json!({
            "id": "issue-7",
            "identifier": "ENG-7",
            "title": "Seen live",
            "description": null,
            "url": "https://linear.app/cheddar-works/issue/ENG-7",
            "branchName": "remy/eng-7",
            "priority": 2,
            "estimate": null,
            "labelIds": [],
            "createdAt": "2026-10-01T10:00:00.000Z",
            "updatedAt": "2026-11-03T09:00:00.000Z",
            "startedAt": null,
            "completedAt": null,
            "canceledAt": null,
            "triagedAt": null,
            "trashed": null,
            "team": { "id": team_id, "key": "ENG", "name": "Engineering", "issueEstimationType": "notUsed" },
            "state": { "id": "state-todo", "name": "Todo", "type": "unstarted", "position": 1 },
            "assignee": null,
            "creator": null,
            "cycle": null,
            "project": null,
            "archivedAt": archived_at
        })
    }

    fn decoded(data: Value) -> Option<LiveEvent> {
        decode(json!({ "data": data }), &client(), &["team-eng".to_owned()])
    }

    #[test]
    fn an_archived_or_unarchived_issue_of_a_subscribed_team_decodes() {
        let Some(LiveEvent::Archived(changes)) = decoded(json!({
            "issueArchived": issue("team-eng", Some("2026-11-03T09:00:00.000Z"))
        })) else {
            panic!("no archive event");
        };
        assert!(changes.issues[0].archived_at.is_some());
        assert_eq!(changes.newest, None, "a live event moves no watermark");

        let Some(LiveEvent::Unarchived(changes)) =
            decoded(json!({ "issueUnarchived": issue("team-eng", None) }))
        else {
            panic!("no unarchive event");
        };
        assert!(changes.issues[0].archived_at.is_none());

        let other = decoded(json!({ "issueArchived": issue("team-other", None) }));
        assert_eq!(other, None, "another team's archive is dropped");
    }

    #[test]
    fn an_edited_or_deleted_comment_decodes() {
        let Some(LiveEvent::CommentUpdated(posted)) = decoded(json!({ "commentUpdated": {
            "id": "comment-1",
            "body": "Looks good now",
            "createdAt": "2026-11-03T09:01:00.000Z",
            "editedAt": "2026-11-03T09:05:00.000Z",
            "issueId": "issue-7",
            "user": { "id": "member-emile", "name": "Emile Brie", "displayName": "emile" }
        } })) else {
            panic!("no comment update");
        };
        assert_eq!(posted.comment.body, "Looks good now");
        assert!(posted.comment.edited_at.is_some());

        assert_eq!(
            decoded(json!({ "commentDeleted": { "id": "comment-1", "issueId": "issue-7" } })),
            Some(LiveEvent::CommentDeleted {
                issue_id: "issue-7".to_owned(),
                comment_id: "comment-1".to_owned(),
            })
        );
        assert_eq!(
            decoded(json!({ "commentDeleted": { "id": "comment-2", "issueId": null } })),
            None,
            "a comment on anything but an issue is dropped"
        );
    }

    fn notification(field: &str, typename: &str, recipient: &str) -> Value {
        let mut node = json!({
            "__typename": typename,
            "id": "notification-1",
            "type": "issueNewComment",
            "createdAt": "2026-11-03T09:00:00.000Z",
            "updatedAt": "2026-11-03T09:00:00.000Z",
            "readAt": null,
            "snoozedUntilAt": null,
            "archivedAt": null,
            "actor": { "id": "member-emile", "name": "Emile Brie", "displayName": "emile" },
            "user": { "id": recipient }
        });
        if typename == "IssueNotification" {
            node["commentId"] = json!("comment-1");
            node["issue"] = issue("team-eng", None);
        }
        json!({ field: node })
    }

    #[test]
    fn a_notification_decodes_with_its_recipient() {
        for field in [
            "notificationCreated",
            "notificationUpdated",
            "notificationArchived",
        ] {
            let Some(LiveEvent::Notification {
                recipient_id,
                loaded,
            }) = decoded(notification(field, "IssueNotification", "member-remy"))
            else {
                panic!("no notification from {field}");
            };
            assert_eq!(recipient_id, "member-remy");
            assert_eq!(loaded.notifications[0].issue_id, "issue-7");
            assert_eq!(loaded.issues[0].identifier, "ENG-7");
        }
        assert_eq!(
            decoded(notification(
                "notificationCreated",
                "ProjectNotification",
                "member-remy"
            )),
            None,
            "a notification about anything but an issue is dropped"
        );
    }
}
