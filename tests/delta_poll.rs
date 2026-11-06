//! The delta poll and the reconciliation with the real client against a local
//! fake HTTP server.

// clippy's allow-unwrap-in-tests does not cover helpers outside #[test] functions.
#![allow(clippy::unwrap_used)]

use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use jiff::Timestamp;
use linear_rat::auth::ApiKey;
use linear_rat::linear::{ClientConfig, HttpClient, LinearClient, Timer};
use serde_json::{Value, json};
use wiremock::matchers::{body_partial_json, body_string_contains, method};
use wiremock::{Mock, MockServer, ResponseTemplate};

const KEY: &str = "lin_api_fictional0000000000000000000000000000";

/// 2026-11-03T09:00:00Z.
const START: Timestamp = Timestamp::constant(1_793_696_400, 0);

/// A clock that only moves when the client sleeps.
#[derive(Debug)]
struct FakeTimer(Mutex<Timestamp>);

impl Timer for FakeTimer {
    fn now(&self) -> Timestamp {
        *self.0.lock().unwrap()
    }

    fn sleep(&self, duration: Duration) -> Pin<Box<dyn Future<Output = ()> + Send>> {
        let mut now = self.0.lock().unwrap();
        *now = now.checked_add(duration).unwrap();
        Box::pin(std::future::ready(()))
    }
}

fn client_for(server: &MockServer) -> HttpClient {
    let config = ClientConfig {
        endpoint: format!("{}/graphql", server.uri()),
        ..ClientConfig::default()
    };
    HttpClient::with_config(&ApiKey::new(KEY).unwrap(), config)
        .unwrap()
        .with_timer(Arc::new(FakeTimer(Mutex::new(START))))
}

fn issue(n: u32, updated_at: &str, trashed: Option<bool>) -> Value {
    json!({
        "id": format!("issue-{n}"),
        "identifier": format!("ENG-{n}"),
        "title": format!("Issue {n}"),
        "description": null,
        "url": format!("https://linear.app/cheddar-works/issue/ENG-{n}"),
        "branchName": format!("remy/eng-{n}"),
        "priority": 2,
        "estimate": 3,
        "labelIds": ["label-bug"],
        "createdAt": "2026-10-01T10:00:00.000Z",
        "updatedAt": updated_at,
        "startedAt": null,
        "completedAt": null,
        "canceledAt": null,
        "triagedAt": null,
        "trashed": trashed,
        "team": { "id": "team-eng", "key": "ENG", "name": "Engineering", "issueEstimationType": "fibonacci" },
        "state": { "id": "state-todo", "name": "Todo", "type": "unstarted", "position": 1 },
        "assignee": { "id": "member-emile", "name": "Emile Brie", "displayName": "emile" },
        "creator": { "id": "viewer", "name": "Remy Fromage", "displayName": "remy" },
        "cycle": null,
        "project": null
    })
}

fn page(nodes: Vec<Value>, end_cursor: Option<&str>) -> Value {
    json!({
        "data": {
            "issues": {
                "nodes": nodes,
                "pageInfo": { "hasNextPage": end_cursor.is_some(), "endCursor": end_cursor }
            }
        }
    })
}

async fn mount(server: &MockServer, query: &str, variables: Value, body: Value) {
    Mock::given(method("POST"))
        .and(body_string_contains(format!("query {query}")))
        .and(body_partial_json(json!({ "variables": variables })))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(server)
        .await;
}

async fn mount_labels(server: &MockServer) {
    let labels = json!({
        "data": {
            "issueLabels": {
                "nodes": [{ "id": "label-bug", "name": "Bug", "team": null }],
                "pageInfo": { "hasNextPage": false, "endCursor": null }
            }
        }
    });
    mount(server, "Labels", json!({}), labels).await;
}

async fn bodies(server: &MockServer) -> Vec<Value> {
    server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .map(|request| request.body_json().unwrap())
        .collect()
}

#[tokio::test]
async fn a_delta_asks_for_the_team_updated_after_since_archived_included_and_follows_pages() {
    let server = MockServer::start().await;
    let since = "2026-11-03T08:58:00Z";
    mount(
        &server,
        "TeamIssuesChanged",
        json!({ "teamId": "team-eng", "since": since, "after": null }),
        page(vec![issue(2, "2026-11-03T08:59:30.000Z", None)], Some("c1")),
    )
    .await;
    mount(
        &server,
        "TeamIssuesChanged",
        json!({ "after": "c1" }),
        page(
            vec![issue(1, "2026-11-03T08:59:00.000Z", Some(false))],
            None,
        ),
    )
    .await;
    mount_labels(&server).await;

    let changes = client_for(&server)
        .changed_issues("team-eng", Some(since.parse().unwrap()))
        .await
        .unwrap();

    let identifiers: Vec<&str> = changes
        .issues
        .iter()
        .map(|issue| issue.identifier.as_str())
        .collect();
    assert_eq!(identifiers, ["ENG-2", "ENG-1"]);
    assert_eq!(changes.team_id, "team-eng");
    assert_eq!(
        changes.newest,
        Some("2026-11-03T08:59:30Z".parse().unwrap())
    );
    assert_eq!(changes.labels[0].name, "Bug");
    assert_eq!(changes.as_of, START);
    let query = bodies(&server).await[0]["query"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(query.contains("orderBy: updatedAt"), "{query}");
    assert!(query.contains("includeArchived: true"), "{query}");
}

#[tokio::test]
async fn trashed_issues_come_back_as_ids_and_still_move_the_watermark() {
    let server = MockServer::start().await;
    mount(
        &server,
        "TeamIssuesChanged",
        json!({}),
        page(
            vec![
                issue(3, "2026-11-03T08:59:50.000Z", Some(true)),
                issue(4, "2026-11-03T08:59:10.000Z", None),
            ],
            None,
        ),
    )
    .await;
    mount_labels(&server).await;

    let changes = client_for(&server)
        .changed_issues("team-eng", Some(START))
        .await
        .unwrap();

    assert_eq!(changes.trashed, ["issue-3"]);
    assert_eq!(changes.issues.len(), 1);
    assert_eq!(
        changes.newest,
        Some("2026-11-03T08:59:50Z".parse().unwrap())
    );
}

#[tokio::test]
async fn without_a_watermark_only_the_newest_issue_is_asked_for() {
    let server = MockServer::start().await;
    mount(
        &server,
        "TeamIssuesChanged",
        json!({ "first": 1 }),
        page(
            vec![issue(9, "2026-11-03T08:30:00.000Z", None)],
            Some("more"),
        ),
    )
    .await;
    mount_labels(&server).await;

    let changes = client_for(&server)
        .changed_issues("team-eng", None)
        .await
        .unwrap();

    assert_eq!(
        changes.newest,
        Some("2026-11-03T08:30:00Z".parse().unwrap())
    );
    let delta_requests = bodies(&server)
        .await
        .iter()
        .filter(|body| {
            body["query"]
                .as_str()
                .unwrap()
                .contains("query TeamIssuesChanged")
        })
        .count();
    assert_eq!(delta_requests, 1, "the next page is not followed");
}

#[tokio::test]
async fn reconciliation_keeps_the_ids_that_still_exist_and_are_not_trashed() {
    let server = MockServer::start().await;
    let nodes = json!({
        "data": {
            "issues": {
                "nodes": [
                    { "id": "issue-1", "trashed": null },
                    { "id": "issue-3", "trashed": true }
                ],
                "pageInfo": { "hasNextPage": false, "endCursor": null }
            }
        }
    });
    mount(&server, "ExistingIssues", json!({}), nodes).await;
    let ids: Vec<String> = ["issue-1", "issue-2", "issue-3"]
        .map(str::to_owned)
        .to_vec();

    let existing = client_for(&server).existing_issues(&ids).await.unwrap();

    assert_eq!(existing, ["issue-1"]);
}

#[tokio::test]
async fn reconciliation_asks_about_at_most_a_hundred_ids_at_a_time() {
    let server = MockServer::start().await;
    mount(&server, "ExistingIssues", json!({}), page(Vec::new(), None)).await;
    let ids: Vec<String> = (0..250).map(|n| format!("issue-{n}")).collect();

    client_for(&server).existing_issues(&ids).await.unwrap();

    let sizes: Vec<usize> = bodies(&server)
        .await
        .iter()
        .map(|body| body["variables"]["ids"].as_array().unwrap().len())
        .collect();
    assert_eq!(sizes, [100, 100, 50]);
}

#[tokio::test]
async fn a_low_budget_reports_the_client_as_throttled() {
    let server = MockServer::start().await;
    let reset = (START.as_second() + 600) * 1000;
    Mock::given(method("POST"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("x-ratelimit-requests-remaining", "100")
                .insert_header("x-ratelimit-requests-limit", "1500")
                .insert_header("x-ratelimit-requests-reset", reset.to_string().as_str())
                .set_body_json(page(Vec::new(), None)),
        )
        .mount(&server)
        .await;
    let client = client_for(&server);
    assert!(!client.throttled());

    client
        .changed_issues("team-eng", Some(START))
        .await
        .unwrap();

    assert!(client.throttled());
}
