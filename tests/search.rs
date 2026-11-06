//! Linear's server-side issue search with the real client against a local
//! fake HTTP server: the request, decoding and search's own budget.

// clippy's allow-unwrap-in-tests does not cover helpers outside #[test] functions.
#![allow(clippy::unwrap_used)]

use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use jiff::Timestamp;
use linear_rat::auth::ApiKey;
use linear_rat::linear::model::StateType;
use linear_rat::linear::{ClientConfig, FakeClient, HttpClient, LinearClient, Timer};
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

fn client_for(server: &MockServer, timer: Arc<FakeTimer>) -> HttpClient {
    let config = ClientConfig {
        endpoint: format!("{}/graphql", server.uri()),
        ..ClientConfig::default()
    };
    HttpClient::with_config(&ApiKey::new(KEY).unwrap(), config)
        .unwrap()
        .with_timer(timer)
}

fn result(n: u32, state_type: &str) -> Value {
    json!({
        "id": format!("issue-{n}"),
        "identifier": format!("OPS-{n}"),
        "title": format!("Rotate the cheese vault keys, part {n}"),
        "description": "Every key, every vault.",
        "url": format!("https://linear.app/cheddar-works/issue/OPS-{n}"),
        "branchName": format!("emile/ops-{n}"),
        "priority": 3,
        "estimate": null,
        "labelIds": ["label-chore"],
        "createdAt": "2026-09-01T10:00:00.000Z",
        "updatedAt": "2026-09-02T10:00:00.000Z",
        "startedAt": null,
        "completedAt": null,
        "canceledAt": null,
        "triagedAt": null,
        "team": { "id": "team-ops", "key": "OPS", "name": "Operations", "issueEstimationType": "notUsed" },
        "state": { "id": format!("state-{state_type}"), "name": "Done", "type": state_type, "position": 3 },
        "assignee": { "id": "member-emile", "name": "Emile Brie", "displayName": "emile" },
        "creator": null,
        "cycle": null,
        "project": null,
        "labels": { "nodes": [{ "id": "label-chore", "name": "Chore", "team": null }] }
    })
}

fn results(nodes: Vec<Value>) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(json!({
        "data": { "searchIssues": { "nodes": nodes } }
    }))
}

#[tokio::test]
async fn sends_the_term_and_keeps_the_results_in_linears_order() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(body_string_contains("query SearchIssues"))
        .and(body_partial_json(
            json!({ "variables": { "term": "vault keys" } }),
        ))
        .respond_with(results(vec![result(7, "completed"), result(3, "started")]))
        .expect(1)
        .mount(&server)
        .await;

    let found = client_for(&server, FakeTimer::new())
        .search_issues("vault keys")
        .await
        .unwrap();

    let identifiers: Vec<&str> = found.issues.iter().map(|i| i.identifier.as_str()).collect();
    assert_eq!(identifiers, ["OPS-7", "OPS-3"]);
    assert_eq!(found.issues[0].state.state_type, StateType::Completed);
    assert_eq!(found.labels[0].name, "Chore");
    assert_eq!(found.teams[0].key, "OPS");
    assert_eq!(found.members[0].name, "Emile Brie");
    assert!(found.problems.is_empty());
}

#[tokio::test]
async fn a_burst_beyond_thirty_searches_a_minute_is_queued_not_failed() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(results(vec![result(1, "started")]))
        .mount(&server)
        .await;
    let timer = FakeTimer::new();
    let client = client_for(&server, timer.clone());

    for n in 0..32 {
        let found = client.search_issues(&format!("cheese {n}")).await.unwrap();
        assert_eq!(found.issues.len(), 1);
    }

    assert_eq!(server.received_requests().await.unwrap().len(), 32);
    // The 31st waited for the next token; by then the clock had moved, so
    // the 32nd waited one more refill.
    assert_eq!(
        timer.slept(),
        [Duration::from_secs(2), Duration::from_secs(2)]
    );
}

#[tokio::test]
async fn the_fake_client_searches_the_whole_demo_workspace() {
    let found = FakeClient.search_issues("cheese").await.unwrap();

    assert!(!found.issues.is_empty());
    assert!(found.issues.iter().all(|issue| {
        let text = format!(
            "{} {}",
            issue.title,
            issue.description.as_deref().unwrap_or_default()
        );
        text.to_lowercase().contains("cheese")
    }));
    assert!(
        found
            .issues
            .iter()
            .any(|issue| !issue.state.state_type.is_open()),
        "closed issues are found too"
    );
}
