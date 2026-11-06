//! Live updates with the real client against a local fake websocket server:
//! the handshake, the subscriptions, deliveries, keep-alive and reconnects.
//! The clock is tokio's paused test clock, so no test sleeps for real, and
//! the client dials an in-memory pipe, since the paused clock jumps ahead
//! while real sockets are in flight.

// clippy's allow-unwrap-in-tests does not cover helpers outside #[test] functions.
#![allow(clippy::unwrap_used)]

use std::io;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures_util::{SinkExt, Stream, StreamExt};
use linear_rat::app::{App, State};
use linear_rat::auth::ApiKey;
use linear_rat::linear::{ClientConfig, HttpClient, LinearClient, LiveConfig, LiveEvent, Pipe};
use linear_rat::{logging, runtime};
use ratatui::backend::TestBackend;
use serde_json::{Value, json};
use tokio::io::DuplexStream;
use tokio::sync::mpsc::{UnboundedReceiver, unbounded_channel};
use tokio::task::JoinHandle;
use tokio::time::Instant;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::handshake::server::{Request, Response};
use tokio_tungstenite::tungstenite::http::HeaderValue;
use tokio_tungstenite::{WebSocketStream, accept_hdr_async};
use tracing_subscriber::EnvFilter;
use tracing_subscriber::util::SubscriberInitExt;

const KEY: &str = "lin_api_fictional0000000000000000000000000000";

const PROTOCOL: &str = "graphql-transport-ws";

/// One connection the fake server accepted, and the upgrade headers it saw.
struct Connection {
    socket: WebSocketStream<DuplexStream>,
    authorization: Option<String>,
    protocol: Option<String>,
    accepted_at: Instant,
}

impl Connection {
    // tungstenite's handshake callback returns its large error type.
    #[allow(clippy::result_large_err)]
    async fn accept(listener: &mut Listener) -> Self {
        let (stream, host, port) = listener.recv().await.unwrap();
        assert_eq!((host.as_str(), port), ("api.example", 443));
        let accepted_at = Instant::now();
        let mut authorization = None;
        let mut protocol = None;
        let socket = accept_hdr_async(stream, |request: &Request, mut response: Response| {
            let header = |name: &str| {
                request
                    .headers()
                    .get(name)
                    .map(|value| value.to_str().unwrap().to_owned())
            };
            authorization = header("authorization");
            protocol = header("sec-websocket-protocol");
            response
                .headers_mut()
                .insert("sec-websocket-protocol", HeaderValue::from_static(PROTOCOL));
            Ok(response)
        })
        .await
        .unwrap();
        Self {
            socket,
            authorization,
            protocol,
            accepted_at,
        }
    }

    /// The next protocol message, or `None` once the client has gone.
    async fn receive(&mut self) -> Option<Value> {
        loop {
            match self.socket.next().await? {
                Ok(Message::Text(text)) => return Some(serde_json::from_str(&text).unwrap()),
                Ok(Message::Close(_)) | Err(_) => return None,
                Ok(_) => {}
            }
        }
    }

    async fn send(&mut self, message: Value) {
        self.socket
            .send(Message::text(message.to_string()))
            .await
            .unwrap();
    }

    /// Takes `connection_init` and acknowledges it.
    async fn handshake(&mut self) {
        let init = self.receive().await.unwrap();
        assert_eq!(init["type"], "connection_init", "{init}");
        self.send(json!({ "type": "connection_ack" })).await;
    }

    /// The `subscribe` messages the client sends after the ack, `count` of
    /// them, with when each arrived.
    async fn subscriptions(&mut self, count: usize) -> Vec<(Value, Instant)> {
        let mut subscribed = Vec::new();
        while subscribed.len() < count {
            let message = self.receive().await.unwrap();
            assert_eq!(message["type"], "subscribe", "{message}");
            subscribed.push((message, Instant::now()));
        }
        subscribed
    }
}

/// The server ends of the pipes the client dialed, with the host and port
/// it dialed.
type Listener = UnboundedReceiver<(DuplexStream, String, u16)>;

/// A client whose live updates dial pipes that `Listener` hands out.
fn client() -> (HttpClient, Listener) {
    let (dialed, listener) = unbounded_channel();
    let live = LiveConfig {
        // `ws` with an explicit port: the pipe stands in for TCP, and TLS
        // is not what these tests are about.
        endpoint: "ws://api.example:443/graphql".to_owned(),
        dial: Arc::new(move |host, port| {
            let (client_end, server_end) = tokio::io::duplex(64 * 1024);
            let sent = dialed.send((server_end, host, port));
            Box::pin(async move {
                sent.map_err(|_| std::io::Error::other("the fake server is gone"))?;
                Ok(Box::new(client_end) as Box<dyn Pipe>)
            })
        }),
        ..LiveConfig::default()
    };
    // Requests go to a port nothing listens on; these tests only need live
    // updates.
    let config = ClientConfig {
        endpoint: "http://127.0.0.1:9/graphql".to_owned(),
        ..ClientConfig::default()
    };
    let client = HttpClient::with_config(&ApiKey::new(KEY).unwrap(), config)
        .unwrap()
        .with_live(live);
    (client, listener)
}

fn issue(updated_at: &str) -> Value {
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
        "updatedAt": updated_at,
        "startedAt": null,
        "completedAt": null,
        "canceledAt": null,
        "triagedAt": null,
        "trashed": null,
        "team": { "id": "team-eng", "key": "ENG", "name": "Engineering", "issueEstimationType": "notUsed" },
        "state": { "id": "state-todo", "name": "Todo", "type": "unstarted", "position": 1 },
        "assignee": { "id": "viewer", "name": "Remy Fromage", "displayName": "remy" },
        "creator": null,
        "cycle": null,
        "project": null
    })
}

/// The id the client gave the subscription to `field`.
fn id_of(subscriptions: &[(Value, Instant)], field: &str) -> String {
    subscriptions
        .iter()
        .find(|(message, _)| {
            message["payload"]["query"]
                .as_str()
                .unwrap()
                .contains(field)
        })
        .map(|(message, _)| message["id"].as_str().unwrap().to_owned())
        .unwrap()
}

/// What `server` returns, while the client's stream keeps running; any event
/// from it fails the test.
async fn finish<T>(server: JoinHandle<T>, mut events: impl Stream<Item = LiveEvent> + Unpin) -> T {
    tokio::select! {
        output = server => output.unwrap(),
        event = events.next() => panic!("an unexpected event: {event:?}"),
    }
}

fn serve<F>(listener: Listener, script: impl FnOnce(Listener) -> F) -> JoinHandle<F::Output>
where
    F: Future + Send + 'static,
    F::Output: Send + 'static,
{
    tokio::spawn(script(listener))
}

#[tokio::test(start_paused = true)]
async fn the_upgrade_carries_the_key_and_the_subprotocol_and_init_comes_first() {
    let (client, listener) = client();
    let server = serve(listener, |mut listener| async move {
        let mut connection = Connection::accept(&mut listener).await;
        let first = connection.receive().await.unwrap();
        // Nothing more comes before the ack.
        let early = tokio::time::timeout(Duration::from_secs(5), connection.receive()).await;
        connection.send(json!({ "type": "connection_ack" })).await;
        let subscriptions = connection.subscriptions(3).await;
        (
            connection.authorization,
            connection.protocol,
            first,
            early.is_err(),
            subscriptions,
        )
    });
    let mut events = std::pin::pin!(client.live_events(vec!["team-eng".to_owned()]));

    assert_eq!(events.next().await, Some(LiveEvent::Connected));
    let (authorization, protocol, first, quiet_before_ack, subscriptions) =
        finish(server, events.as_mut()).await;

    assert_eq!(
        authorization.as_deref(),
        Some(KEY),
        "the bare key, no Bearer"
    );
    assert_eq!(protocol.as_deref(), Some(PROTOCOL));
    assert_eq!(first, json!({ "type": "connection_init", "payload": {} }));
    assert!(quiet_before_ack, "the client subscribed before the ack");
    let queries: Vec<&str> = subscriptions
        .iter()
        .map(|(message, _)| message["payload"]["query"].as_str().unwrap())
        .collect();
    assert!(queries[0].contains("issueCreated"), "{}", queries[0]);
    assert!(queries[1].contains("issueUpdated"), "{}", queries[1]);
    assert!(queries[2].contains("commentCreated"), "{}", queries[2]);
    for (message, _) in &subscriptions[..2] {
        assert_eq!(
            message["payload"]["variables"],
            json!({ "teamId": "team-eng" })
        );
    }
    for pair in subscriptions.windows(2) {
        let gap = pair[1].1 - pair[0].1;
        assert!(gap >= Duration::from_millis(1500), "{gap:?}");
    }
}

#[tokio::test(start_paused = true)]
async fn a_delivered_issue_and_comment_come_out_as_live_events() {
    let (client, listener) = client();
    let server = serve(listener, |mut listener| async move {
        let mut connection = Connection::accept(&mut listener).await;
        connection.handshake().await;
        let subscriptions = connection.subscriptions(3).await;
        let updated = id_of(&subscriptions, "issueUpdated");
        let commented = id_of(&subscriptions, "commentCreated");
        connection
            .send(json!({
                "id": updated,
                "type": "next",
                "payload": { "data": { "issueUpdated": issue("2026-11-03T09:00:00.000Z") } }
            }))
            .await;
        connection
            .send(json!({
                "id": commented,
                "type": "next",
                "payload": { "data": { "commentCreated": {
                    "id": "comment-1",
                    "body": "Looks good",
                    "createdAt": "2026-11-03T09:01:00.000Z",
                    "issueId": "issue-7",
                    "user": { "id": "member-emile", "name": "Emile Brie", "displayName": "emile" }
                } } }
            }))
            .await;
        // A comment on something other than an issue is dropped.
        connection
            .send(json!({
                "id": commented,
                "type": "next",
                "payload": { "data": { "commentCreated": {
                    "id": "comment-2",
                    "body": "On a project update",
                    "createdAt": "2026-11-03T09:02:00.000Z",
                    "issueId": null,
                    "user": null
                } } }
            }))
            .await;
        connection.receive().await
    });
    let mut events = std::pin::pin!(client.live_events(vec!["team-eng".to_owned()]));

    assert_eq!(events.next().await, Some(LiveEvent::Connected));
    let Some(LiveEvent::Issue(changes)) = events.next().await else {
        panic!("no issue event");
    };
    assert_eq!(changes.team_id, "team-eng");
    assert_eq!(changes.issues.len(), 1);
    assert_eq!(changes.issues[0].title, "Seen live");
    assert_eq!(changes.newest, None, "a live event moves no watermark");
    assert_eq!(changes.members[0].id, "viewer");
    let Some(LiveEvent::Comment(posted)) = events.next().await else {
        panic!("no comment event");
    };
    assert_eq!(posted.comment.issue_id, "issue-7");
    assert_eq!(posted.comment.body, "Looks good");
    assert_eq!(posted.author.unwrap().display_name, "emile");
    let next = tokio::time::timeout(Duration::from_secs(5), events.next()).await;
    assert!(
        next.is_err(),
        "the comment without an issue came out: {next:?}"
    );
    server.abort();
}

#[tokio::test(start_paused = true)]
async fn an_issue_from_a_team_not_subscribed_is_dropped() {
    let (client, listener) = client();
    let server = serve(listener, |mut listener| async move {
        let mut connection = Connection::accept(&mut listener).await;
        connection.handshake().await;
        let subscriptions = connection.subscriptions(3).await;
        let updated = id_of(&subscriptions, "issueUpdated");
        let mut foreign = issue("2026-11-03T09:00:00.000Z");
        foreign["title"] = json!("From another team");
        foreign["team"]["id"] = json!("team-ops");
        for issue in [foreign, issue("2026-11-03T09:01:00.000Z")] {
            connection
                .send(json!({
                    "id": updated,
                    "type": "next",
                    "payload": { "data": { "issueUpdated": issue } }
                }))
                .await;
        }
        connection.receive().await
    });
    let mut events = std::pin::pin!(client.live_events(vec!["team-eng".to_owned()]));

    assert_eq!(events.next().await, Some(LiveEvent::Connected));
    let Some(LiveEvent::Issue(changes)) = events.next().await else {
        panic!("no issue event");
    };
    assert_eq!(changes.team_id, "team-eng");
    assert_eq!(changes.issues[0].title, "Seen live");
    server.abort();
}

/// A log file in memory.
#[derive(Clone, Default)]
struct LogBuffer(Arc<Mutex<Vec<u8>>>);

impl io::Write for LogBuffer {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[tokio::test(start_paused = true)]
async fn the_key_stays_out_of_the_log_even_at_trace() {
    let log = LogBuffer::default();
    let writer = log.clone();
    // The global logger, as lrat installs it, so tungstenite's `log` records
    // reach it too.
    logging::subscriber(move || writer.clone(), EnvFilter::new("trace"))
        .try_init()
        .unwrap();
    let (client, listener) = client();
    let server = serve(listener, |mut listener| async move {
        let mut connection = Connection::accept(&mut listener).await;
        connection.handshake().await;
        connection.receive().await
    });
    let mut events = std::pin::pin!(client.live_events(vec!["team-eng".to_owned()]));

    assert_eq!(events.next().await, Some(LiveEvent::Connected));
    server.abort();

    let logged = String::from_utf8(log.0.lock().unwrap().clone()).unwrap();
    assert!(logged.contains("live updates connected"), "{logged}");
    assert!(logged.contains("tungstenite"), "{logged}");
    assert!(!logged.contains(KEY), "{logged}");
}

#[tokio::test(start_paused = true)]
async fn a_ping_gets_a_pong() {
    let (client, listener) = client();
    let server = serve(listener, |mut listener| async move {
        let mut connection = Connection::accept(&mut listener).await;
        connection.handshake().await;
        connection.send(json!({ "type": "ping" })).await;
        loop {
            let message = connection.receive().await.unwrap();
            if message["type"] != "subscribe" {
                return message;
            }
        }
    });
    let mut events = std::pin::pin!(client.live_events(vec!["team-eng".to_owned()]));

    assert_eq!(events.next().await, Some(LiveEvent::Connected));
    let answer = finish(server, events.as_mut()).await;
    assert_eq!(answer, json!({ "type": "pong" }));
}

#[tokio::test(start_paused = true)]
async fn without_an_ack_in_time_the_client_reconnects() {
    let (client, listener) = client();
    let server = serve(listener, |mut listener| async move {
        let mut ignored = Connection::accept(&mut listener).await;
        ignored.receive().await.unwrap();
        let mut second = Connection::accept(&mut listener).await;
        second.handshake().await;
        let gone = ignored.receive().await;
        (second.accepted_at - ignored.accepted_at, gone)
    });
    let mut events = std::pin::pin!(client.live_events(vec!["team-eng".to_owned()]));

    assert_eq!(events.next().await, Some(LiveEvent::Connected));
    let (waited, gone) = server.await.unwrap();

    assert!(waited >= LiveConfig::default().ack_timeout, "{waited:?}");
    assert_eq!(gone, None, "the unacknowledged socket stayed open");
}

#[tokio::test(start_paused = true)]
async fn after_a_disconnect_the_client_reconnects_with_backoff_and_says_so() {
    let (client, listener) = client();
    let server = serve(listener, |mut listener| async move {
        let mut gaps = Vec::new();
        let mut closed_at = None;
        for _ in 0..3 {
            let mut connection = Connection::accept(&mut listener).await;
            if let Some(closed_at) = closed_at {
                gaps.push(connection.accepted_at - closed_at);
            }
            connection.handshake().await;
            connection.socket.close(None).await.unwrap();
            closed_at = Some(Instant::now());
        }
        let mut last = Connection::accept(&mut listener).await;
        if let Some(closed_at) = closed_at {
            gaps.push(last.accepted_at - closed_at);
        }
        last.handshake().await;
        gaps
    });
    let mut events = std::pin::pin!(client.live_events(vec!["team-eng".to_owned()]));

    for _ in 0..4 {
        assert_eq!(events.next().await, Some(LiveEvent::Connected));
    }
    let gaps = server.await.unwrap();

    let config = LiveConfig::default();
    assert!(gaps[0] >= config.first_backoff / 2, "{gaps:?}");
    assert!(
        gaps[0] <= config.first_backoff + Duration::from_millis(100),
        "{gaps:?}"
    );
    assert!(
        gaps[1] >= config.first_backoff,
        "the second wait doubles: {gaps:?}"
    );
    assert!(
        gaps[2] >= config.first_backoff * 2,
        "and doubles again: {gaps:?}"
    );
}

#[tokio::test(start_paused = true)]
async fn a_server_that_goes_quiet_is_pinged_then_dropped() {
    let (client, listener) = client();
    let server = serve(listener, |mut listener| async move {
        let mut quiet = Connection::accept(&mut listener).await;
        quiet.handshake().await;
        let mut ping = None;
        while let Some(message) = quiet.receive().await {
            if message["type"] == "ping" {
                ping = Some(message);
            }
        }
        let mut next = Connection::accept(&mut listener).await;
        next.handshake().await;
        ping
    });
    let mut events = std::pin::pin!(client.live_events(vec!["team-eng".to_owned()]));

    assert_eq!(events.next().await, Some(LiveEvent::Connected));
    assert_eq!(events.next().await, Some(LiveEvent::Connected));
    assert_eq!(server.await.unwrap(), Some(json!({ "type": "ping" })));
}

/// Runs the demo workspace with `sync.live` set for a few seconds, and hands
/// back how many connections its live updates dialed.
async fn dials_with_live(live: bool) -> usize {
    let (client, mut listener) = client();
    let mut state = State::demo();
    state.sync.live = live;
    let mut app = App::new(TestBackend::new(80, 24), state).unwrap();
    let running = runtime::run(
        &mut app,
        Some(&client),
        None,
        futures_util::stream::pending(),
        std::future::pending(),
    );
    let stopped = tokio::time::timeout(Duration::from_secs(3), running).await;
    assert!(stopped.is_err(), "the runtime stopped: {stopped:?}");
    let mut dialed = 0;
    while listener.try_recv().is_ok() {
        dialed += 1;
    }
    dialed
}

#[tokio::test(start_paused = true)]
async fn with_live_off_no_socket_is_opened() {
    assert_eq!(dials_with_live(false).await, 0);
}

#[tokio::test(start_paused = true)]
async fn with_live_on_the_runtime_opens_the_socket() {
    assert!(dials_with_live(true).await >= 1);
}
