//! Loading analytics with the real client against a local fake HTTP server:
//! the window in the request, pagination, state history only for in-progress
//! issues, cycles with their history and the label and project names.

// clippy's allow-unwrap-in-tests does not cover helpers outside #[test] functions.
#![allow(clippy::unwrap_used)]

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use jiff::Timestamp;
use linear_rat::auth::ApiKey;
use linear_rat::linear::model::{Estimation, StateType};
use linear_rat::linear::{AnalyticsWindow, ClientConfig, HttpClient, LinearClient, Timer};
use serde_json::{Value, json};
use wiremock::matchers::{body_partial_json, body_string_contains, method};
use wiremock::{Mock, MockServer, ResponseTemplate};

const KEY: &str = "lin_api_fictional0000000000000000000000000000";

/// 2026-11-03T09:00:00Z.
const START: Timestamp = Timestamp::constant(1_793_696_400, 0);

/// A clock that stands still; the budget never runs low here, so nothing
/// sleeps.
#[derive(Debug)]
struct StillTimer;

impl Timer for StillTimer {
    fn now(&self) -> Timestamp {
        START
    }

    fn sleep(&self, _: Duration) -> Pin<Box<dyn Future<Output = ()> + Send>> {
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
        .with_timer(Arc::new(StillTimer))
}

fn at(text: &str) -> Timestamp {
    text.parse().unwrap()
}

fn issue(n: u32, state: &str, completed_at: Option<&str>) -> Value {
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
        "createdAt": "2026-09-01T10:00:00.000Z",
        "updatedAt": "2026-10-02T10:00:00.000Z",
        "startedAt": "2026-09-02T10:00:00.000Z",
        "completedAt": completed_at,
        "canceledAt": null,
        "triagedAt": null,
        "team": { "id": "team-eng", "key": "ENG", "name": "Engineering", "issueEstimationType": "fibonacci" },
        "state": { "id": format!("state-{state}"), "name": state, "type": state, "position": 1 },
        "assignee": { "id": "viewer", "name": "Remy Fromage", "displayName": "remy" },
        "creator": { "id": "member-emile", "name": "Emile Brie", "displayName": "emile" },
        "cycle": null,
        "project": { "id": "project-brie" }
    })
}

fn issues_page(nodes: Vec<Value>, end_cursor: Option<&str>) -> Value {
    json!({
        "data": {
            "viewer": {
                "id": "viewer",
                "name": "Remy Fromage",
                "assignedIssues": {
                    "nodes": nodes,
                    "pageInfo": { "hasNextPage": end_cursor.is_some(), "endCursor": end_cursor }
                }
            }
        }
    })
}

fn teams_page() -> Value {
    json!({
        "data": {
            "viewer": {
                "id": "viewer",
                "name": "Remy Fromage",
                "teams": {
                    "nodes": [{
                        "id": "team-eng",
                        "key": "ENG",
                        "name": "Engineering",
                        "issueEstimationType": "fibonacci",
                        "cycles": { "nodes": [{
                            "id": "cycle-7",
                            "number": 7.0,
                            "name": null,
                            "startsAt": "2026-10-26T00:00:00.000Z",
                            "endsAt": "2026-11-09T00:00:00.000Z",
                            "completedAt": null,
                            "scopeHistory": [10.0, 12.0],
                            "completedScopeHistory": [0.0, 3.0],
                            "issueCountHistory": [4.0, 5.0],
                            "completedIssueCountHistory": [0.0, 1.0]
                        }] }
                    }],
                    "pageInfo": { "hasNextPage": false, "endCursor": null }
                }
            }
        }
    })
}

fn names(field: &str, nodes: Value) -> Value {
    json!({
        "data": {
            field: { "nodes": nodes, "pageInfo": { "hasNextPage": false, "endCursor": null } }
        }
    })
}

async fn mount(server: &MockServer, query: &str, body: Value) {
    Mock::given(method("POST"))
        .and(body_string_contains(query))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(server)
        .await;
}

async fn mount_names(server: &MockServer) {
    mount(
        server,
        "query Labels",
        names(
            "issueLabels",
            json!([{ "id": "label-bug", "name": "Bug", "team": null }]),
        ),
    )
    .await;
    mount(
        server,
        "query Projects",
        names(
            "projects",
            json!([{ "id": "project-brie", "name": "Brie" }]),
        ),
    )
    .await;
}

#[tokio::test]
async fn a_window_up_to_now_loads_every_page_and_the_in_progress_history() {
    let server = MockServer::start().await;
    mount(&server, "query AnalyticsTeams", teams_page()).await;
    let since = "2026-08-17T00:00:00Z";
    let filter = json!({ "or": [
        { "createdAt": { "gte": since } },
        { "completedAt": { "gte": since } },
    ] });
    Mock::given(method("POST"))
        .and(body_string_contains("query AnalyticsIssues"))
        .and(body_partial_json(
            json!({ "variables": { "filter": filter, "after": null } }),
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(issues_page(
            vec![
                issue(1, "completed", Some("2026-10-01T10:00:00.000Z")),
                issue(2, "started", None),
            ],
            Some("page-2"),
        )))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(body_string_contains("query AnalyticsIssues"))
        .and(body_partial_json(
            json!({ "variables": { "after": "page-2" } }),
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(issues_page(
            vec![issue(3, "completed", Some("2026-10-05T10:00:00.000Z"))],
            None,
        )))
        .expect(1)
        .mount(&server)
        .await;
    let mut in_progress = issue(2, "started", None);
    // Linear does not promise an order; the later span comes first here.
    in_progress["stateHistory"] = json!({ "nodes": [
        { "stateId": "state-review", "startedAt": "2026-09-20T10:00:00.000Z", "endedAt": null, "state": { "type": "started" } },
        { "stateId": "state-todo", "startedAt": "2026-09-01T10:00:00.000Z", "endedAt": "2026-09-02T10:00:00.000Z", "state": { "type": "unstarted" } },
        { "stateId": "state-gone", "startedAt": "2026-09-02T10:00:00.000Z", "endedAt": "2026-09-20T10:00:00.000Z", "state": null },
    ] });
    mount(
        &server,
        "query InProgressIssues",
        issues_page(vec![in_progress], None),
    )
    .await;
    mount_names(&server).await;

    let loaded = client_for(&server)
        .analytics_issues(AnalyticsWindow {
            since: at(since),
            until: None,
        })
        .await
        .unwrap();

    let mut ids: Vec<&str> = loaded.issues.iter().map(|i| i.id.as_str()).collect();
    ids.sort_unstable();
    assert_eq!(ids, ["issue-1", "issue-2", "issue-3"]);
    let started = loaded.issues.iter().find(|i| i.id == "issue-2").unwrap();
    let history: Vec<(&str, StateType)> = started
        .state_history
        .iter()
        .map(|span| (span.state_id.as_str(), span.state_type))
        .collect();
    assert_eq!(
        history,
        [
            ("state-todo", StateType::Unstarted),
            ("state-review", StateType::Started)
        ]
    );
    let done = loaded.issues.iter().find(|i| i.id == "issue-1").unwrap();
    assert!(done.state_history.is_empty());

    assert_eq!(loaded.viewer.id, "viewer");
    assert_eq!(loaded.teams.len(), 1);
    assert_eq!(loaded.teams[0].estimation, Estimation::Fibonacci);
    assert_eq!(loaded.cycles.len(), 1);
    let cycle = &loaded.cycles[0];
    assert_eq!(cycle.team_id, "team-eng");
    assert_eq!(cycle.scope_history, [10.0, 12.0]);
    assert_eq!(cycle.completed_issue_count_history, [0, 1]);
    assert_eq!(loaded.labels[0].name, "Bug");
    assert_eq!(loaded.projects[0].name, "Brie");
    assert_eq!(loaded.as_of, START);
    assert!(loaded.problems.is_empty(), "{:?}", loaded.problems);
}

#[tokio::test]
async fn an_older_window_asks_only_for_its_weeks_and_no_history() {
    let server = MockServer::start().await;
    let since = "2026-05-11T00:00:00Z";
    let until = "2026-08-17T00:00:00Z";
    let range = json!({ "gte": since, "lt": until });
    Mock::given(method("POST"))
        .and(body_string_contains("query AnalyticsTeams"))
        .and(body_partial_json(json!({ "variables": { "cycles": {
            "endsAt": { "gt": since },
            "startsAt": { "lt": until },
        } } })))
        .respond_with(ResponseTemplate::new(200).set_body_json(teams_page()))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(body_string_contains("query AnalyticsIssues"))
        .and(body_partial_json(
            json!({ "variables": { "filter": { "or": [
            { "createdAt": range },
            { "completedAt": range },
        ] } } }),
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(issues_page(
            vec![issue(4, "completed", Some("2026-06-01T10:00:00.000Z"))],
            None,
        )))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(body_string_contains("query InProgressIssues"))
        .respond_with(ResponseTemplate::new(500))
        .expect(0)
        .mount(&server)
        .await;
    mount_names(&server).await;

    let loaded = client_for(&server)
        .analytics_issues(AnalyticsWindow {
            since: at(since),
            until: Some(at(until)),
        })
        .await
        .unwrap();

    let ids: Vec<&str> = loaded.issues.iter().map(|i| i.id.as_str()).collect();
    assert_eq!(ids, ["issue-4"]);
}

#[tokio::test]
async fn a_failed_name_lookup_still_returns_the_issues_and_says_why() {
    let server = MockServer::start().await;
    mount(&server, "query AnalyticsTeams", teams_page()).await;
    mount(
        &server,
        "query AnalyticsIssues",
        issues_page(
            vec![issue(1, "completed", Some("2026-10-01T10:00:00.000Z"))],
            None,
        ),
    )
    .await;
    Mock::given(method("POST"))
        .and(body_string_contains("query Projects"))
        .respond_with(ResponseTemplate::new(503))
        .mount(&server)
        .await;
    mount(
        &server,
        "query Labels",
        names(
            "issueLabels",
            json!([{ "id": "label-bug", "name": "Bug", "team": null }]),
        ),
    )
    .await;

    let loaded = client_for(&server)
        .analytics_issues(AnalyticsWindow {
            since: at("2026-08-17T00:00:00Z"),
            until: Some(at("2026-10-12T00:00:00Z")),
        })
        .await
        .unwrap();

    assert_eq!(loaded.issues.len(), 1);
    assert!(loaded.projects.is_empty());
    assert_eq!(
        loaded.problems,
        ["project names did not load: Linear answered with HTTP 503"]
    );
}
