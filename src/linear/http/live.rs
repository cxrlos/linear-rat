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
use jiff::Timestamp;
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
use super::assigned::{Decoded, IssueNode, PersonNode, timestamp};
use crate::linear::model::Comment;
use crate::linear::{LiveEvent, PostedComment, TeamChanges};

/// Linear's websocket endpoint.
pub const LIVE_ENDPOINT: &str = "wss://api.linear.app/graphql";

const PROTOCOL: &str = "graphql-transport-ws";

const ISSUE_CREATED: &str = include_str!("../graphql/issue_created_live.graphql");
const ISSUE_UPDATED: &str = include_str!("../graphql/issue_updated_live.graphql");
const COMMENT_CREATED: &str = include_str!("../graphql/comment_created_live.graphql");

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

/// The live events for the issues of `team_ids` and every new comment.
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
/// team, then new comments.
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
    subscribe.push_back(json!({
        "id": "commentCreated",
        "type": "subscribe",
        "payload": { "query": COMMENT_CREATED, "variables": {} },
    }));
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
    if let Some(node) = data.issue_created.or(data.issue_updated) {
        let team_id = node.issue.team.id.clone();
        // The subscriptions filter by team already; this guards the store.
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
        return Some(LiveEvent::Issue(TeamChanges {
            team_id,
            issues: decoded.issues,
            trashed,
            cycles: decoded.cycles,
            labels: Vec::new(),
            members: decoded.members,
            teams: decoded.teams,
            newest: None,
            as_of: client.timer.now(),
        }));
    }
    let comment = data.comment_created?;
    Some(LiveEvent::Comment(PostedComment {
        comment: Comment {
            id: comment.id,
            issue_id: comment.issue_id?,
            author_id: comment.user.as_ref().map(|user| user.id.clone()),
            body: comment.body,
            created_at: comment.created_at,
        },
        author: comment.user.map(PersonNode::into_member),
    }))
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
    comment_created: Option<LiveCommentNode>,
}

#[derive(Deserialize)]
struct LiveIssueNode {
    #[serde(flatten)]
    issue: IssueNode,
    trashed: Option<bool>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct LiveCommentNode {
    id: String,
    body: String,
    #[serde(deserialize_with = "timestamp")]
    created_at: Timestamp,
    issue_id: Option<String>,
    user: Option<PersonNode>,
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
    fn each_team_gets_new_and_updated_issues_and_comments_come_once() {
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
                "commentCreated"
            ]
            .map(Value::from)
        );
    }
}
