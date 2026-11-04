//! Loading an issue's comments and state history with the real client
//! against a local fake HTTP server.

// clippy's allow-unwrap-in-tests does not cover helpers outside #[test] functions.
#![allow(clippy::unwrap_used)]

use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use jiff::Timestamp;
use linear_rat::auth::ApiKey;
use linear_rat::linear::model::{Project, StateType};
use linear_rat::linear::{ClientConfig, ClientError, HttpClient, LinearClient, Timer, Urgency};
use serde_json::{Value, json};
use wiremock::matchers::{body_partial_json, body_string_contains, method};
use wiremock::{Mock, MockServer, ResponseTemplate};

const KEY: &str = "lin_api_fictional0000000000000000000000000000";

/// 2026-11-03T09:00:00Z.
const START: Timestamp = Timestamp::constant(1_793_696_400, 0);

/// A clock that only moves when the client sleeps, and remembers each sleep.
#[derive(Debug)]
struct FakeTimer {
    now: Mutex<Timestamp>,
    slept: Mutex<Vec<Duration>>,
}

impl FakeTimer {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            now: Mutex::new(START),
            slept: Mutex::new(Vec::new()),
        })
    }

    fn slept(&self) -> Vec<Duration> {
        self.slept.lock().unwrap().clone()
    }
}

impl Timer for FakeTimer {
    fn now(&self) -> Timestamp {
        *self.now.lock().unwrap()
    }

    fn sleep(&self, duration: Duration) -> Pin<Box<dyn Future<Output = ()> + Send>> {
        self.slept.lock().unwrap().push(duration);
        let mut now = self.now.lock().unwrap();
        *now = now.checked_add(duration).unwrap();
        Box::pin(std::future::ready(()))
    }
}

fn client_for(server: &MockServer) -> HttpClient {
    client_with_timer(server, FakeTimer::new())
}

fn client_with_timer(server: &MockServer, timer: Arc<FakeTimer>) -> HttpClient {
    let config = ClientConfig {
        endpoint: format!("{}/graphql", server.uri()),
        ..ClientConfig::default()
    };
    HttpClient::with_config(&ApiKey::new(KEY).unwrap(), config)
        .unwrap()
        .with_timer(timer)
}

fn comment(id: &str, at: &str, user: Value) -> Value {
    json!({ "id": id, "body": format!("Comment {id}"), "createdAt": at, "user": user })
}

fn emile() -> Value {
    json!({ "id": "member-emile", "name": "Emile Brie", "displayName": "emile" })
}

fn comments_page(nodes: Vec<Value>, end_cursor: Option<&str>) -> Value {
    json!({
        "data": {
            "issue": {
                "id": "issue-1",
                "comments": {
                    "nodes": nodes,
                    "pageInfo": { "hasNextPage": end_cursor.is_some(), "endCursor": end_cursor }
                }
            }
        }
    })
}

fn state(id: &str, name: &str, kind: &str) -> Value {
    json!({ "id": id, "name": name, "type": kind, "position": 1, "team": { "id": "team-eng" } })
}

fn history_page(nodes: Vec<Value>) -> Value {
    json!({
        "data": {
            "issue": {
                "id": "issue-1",
                "project": { "id": "project-rind", "name": "Rind photos" },
                "stateHistory": {
                    "nodes": nodes,
                    "pageInfo": { "hasNextPage": false, "endCursor": null }
                }
            }
        }
    })
}

async fn mount(server: &MockServer, query: &str, after: Option<&str>, body: Value) {
    Mock::given(method("POST"))
        .and(body_string_contains(format!("query {query}")))
        .and(body_partial_json(
            json!({ "variables": { "id": "issue-1", "after": after } }),
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(server)
        .await;
}

fn at(text: &str) -> Timestamp {
    text.parse().unwrap()
}

#[tokio::test]
async fn loads_every_comment_page_and_the_state_history_oldest_first() {
    let server = MockServer::start().await;
    mount(
        &server,
        "IssueComments",
        None,
        comments_page(
            vec![
                comment("c3", "2026-10-03T10:00:00.000Z", emile()),
                comment("c1", "2026-10-01T10:00:00.000Z", json!(null)),
            ],
            Some("cursor-1"),
        ),
    )
    .await;
    mount(
        &server,
        "IssueComments",
        Some("cursor-1"),
        comments_page(
            vec![comment("c2", "2026-10-02T10:00:00.000Z", emile())],
            None,
        ),
    )
    .await;
    mount(
        &server,
        "IssueStateHistory",
        None,
        history_page(vec![
            json!({
                "stateId": "state-wip", "startedAt": "2026-10-02T09:00:00.000Z", "endedAt": null,
                "state": state("state-wip", "In Progress", "started")
            }),
            json!({
                "stateId": "state-todo", "startedAt": "2026-10-01T09:00:00.000Z",
                "endedAt": "2026-10-02T09:00:00.000Z",
                "state": state("state-todo", "Todo", "unstarted")
            }),
            json!({
                "stateId": "state-gone", "startedAt": "2026-09-30T09:00:00.000Z",
                "endedAt": "2026-10-01T09:00:00.000Z", "state": null
            }),
        ]),
    )
    .await;

    let detail = client_for(&server)
        .issue_detail("issue-1", Urgency::Urgent)
        .await
        .unwrap();

    let comments: Vec<(&str, Option<&str>)> = detail
        .comments
        .iter()
        .map(|comment| (comment.id.as_str(), comment.author_id.as_deref()))
        .collect();
    assert_eq!(
        comments,
        [
            ("c1", None),
            ("c2", Some("member-emile")),
            ("c3", Some("member-emile"))
        ]
    );
    assert!(detail.comments.iter().all(|c| c.issue_id == "issue-1"));
    assert_eq!(detail.members.len(), 1);
    assert_eq!(detail.members[0].name, "Emile Brie");

    let history: Vec<(&str, StateType, Option<Timestamp>)> = detail
        .state_history
        .iter()
        .map(|span| (span.state_id.as_str(), span.state_type, span.ended_at))
        .collect();
    assert_eq!(
        history,
        [
            (
                "state-todo",
                StateType::Unstarted,
                Some(at("2026-10-02T09:00:00Z"))
            ),
            ("state-wip", StateType::Started, None),
        ],
        "a span without its state is left out"
    );
    let names: Vec<&str> = detail
        .workflow_states
        .iter()
        .map(|state| state.name.as_str())
        .collect();
    assert_eq!(names, ["In Progress", "Todo"]);
    assert_eq!(
        detail.project,
        Some(Project {
            id: "project-rind".to_owned(),
            name: "Rind photos".to_owned()
        })
    );
    assert!(detail.problems.is_empty());
}

#[tokio::test]
async fn bounds_both_connections_with_first() {
    let server = MockServer::start().await;
    mount(
        &server,
        "IssueComments",
        None,
        comments_page(Vec::new(), None),
    )
    .await;
    mount(&server, "IssueStateHistory", None, history_page(Vec::new())).await;

    client_for(&server)
        .issue_detail("issue-1", Urgency::Urgent)
        .await
        .unwrap();

    let queries: Vec<String> = server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .map(|request| {
            let body: Value = request.body_json().unwrap();
            body["query"].as_str().unwrap().to_owned()
        })
        .collect();
    assert_eq!(queries.len(), 2);
    assert!(queries.iter().any(|q| q.contains("comments(first: 100")));
    assert!(queries.iter().any(|q| q.contains("stateHistory(first: 50")));
}

#[tokio::test]
async fn an_issue_linear_cannot_find_is_an_error() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "data": null,
            "errors": [{ "message": "Entity not found: Issue" }]
        })))
        .mount(&server)
        .await;

    let error = client_for(&server)
        .issue_detail("issue-1", Urgency::Urgent)
        .await
        .unwrap_err();

    assert!(matches!(error, ClientError::GraphQl(_)), "{error:?}");
}

#[tokio::test]
async fn a_background_load_waits_for_a_spent_budget_and_an_urgent_one_does_not() {
    let server = MockServer::start().await;
    // 20 000 of 250 000 complexity points left is inside the reserve, which
    // resets 600 s after START.
    let reset_ms = ((START.as_second() + 600) * 1000).to_string();
    for (query, body) in [
        ("IssueComments", comments_page(Vec::new(), None)),
        ("IssueStateHistory", history_page(Vec::new())),
    ] {
        Mock::given(method("POST"))
            .and(body_string_contains(format!("query {query}")))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(body)
                    .insert_header("x-ratelimit-complexity-remaining", "20000")
                    .insert_header("x-ratelimit-complexity-limit", "250000")
                    .insert_header("x-ratelimit-complexity-reset", reset_ms.as_str())
                    .insert_header("x-complexity", "800"),
            )
            .mount(&server)
            .await;
    }
    let timer = FakeTimer::new();
    let client = client_with_timer(&server, timer.clone());

    client
        .issue_detail("issue-1", Urgency::Urgent)
        .await
        .unwrap();
    client
        .issue_detail("issue-1", Urgency::Urgent)
        .await
        .unwrap();
    assert!(timer.slept().is_empty(), "{:?}", timer.slept());

    client
        .issue_detail("issue-1", Urgency::Background)
        .await
        .unwrap();
    assert_eq!(timer.slept().first(), Some(&Duration::from_secs(600)));
}
