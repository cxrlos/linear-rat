//! Loading team analytics and the teams the viewer administers with the real
//! client against a local fake HTTP server: the window in the request, the
//! recent cycles, open and in-progress issues when the window reaches now,
//! and the page bound.

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
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

const KEY: &str = "lin_api_fictional0000000000000000000000000000";

/// 2026-11-03T09:00:00Z.
const START: Timestamp = Timestamp::constant(1_793_696_400, 0);

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

fn issue(n: u32, state: &str, assignee: &str) -> Value {
    json!({
        "id": format!("issue-{n}"),
        "identifier": format!("ENG-{n}"),
        "title": format!("Issue {n}"),
        "description": null,
        "url": format!("https://linear.app/cheddar-works/issue/ENG-{n}"),
        "branchName": format!("eng-{n}"),
        "priority": 2,
        "estimate": 3,
        "labelIds": [],
        "createdAt": "2026-09-01T10:00:00.000Z",
        "updatedAt": "2026-10-02T10:00:00.000Z",
        "startedAt": null,
        "completedAt": null,
        "canceledAt": null,
        "triagedAt": null,
        "team": { "id": "team-eng", "key": "ENG", "name": "Engineering", "issueEstimationType": "fibonacci" },
        "state": { "id": format!("state-{state}"), "name": state, "type": state, "position": 1 },
        "assignee": { "id": assignee, "name": assignee, "displayName": assignee },
        "creator": null,
        "cycle": null,
        "project": null
    })
}

fn issues_page(nodes: Vec<Value>, end_cursor: Option<&str>) -> Value {
    json!({ "data": { "team": { "issues": {
        "nodes": nodes,
        "pageInfo": { "hasNextPage": end_cursor.is_some(), "endCursor": end_cursor }
    } } } })
}

fn settings() -> Value {
    json!({ "data": { "team": {
        "id": "team-eng",
        "key": "ENG",
        "name": "Engineering",
        "issueEstimationType": "fibonacci",
        "cycleDuration": 2.0,
        "cycleCooldownTime": 0.0
    } } })
}

fn cycle(number: u32) -> Value {
    json!({
        "id": format!("cycle-{number}"),
        "number": f64::from(number),
        "name": null,
        "startsAt": "2026-10-26T00:00:00.000Z",
        "endsAt": "2026-11-09T00:00:00.000Z",
        "completedAt": null,
        "scopeHistory": [10.0],
        "completedScopeHistory": [0.0],
        "issueCountHistory": [4.0],
        "completedIssueCountHistory": [0.0]
    })
}

async fn mount(server: &MockServer, query: &str, body: Value) {
    Mock::given(method("POST"))
        .and(body_string_contains(query))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(server)
        .await;
}

#[tokio::test]
async fn administrable_teams_are_the_ids_on_every_page() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(body_string_contains("query AdministrableTeams"))
        .and(body_partial_json(json!({ "variables": { "after": null } })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "data": {
            "administrableTeams": {
                "nodes": [{ "id": "team-eng" }],
                "pageInfo": { "hasNextPage": true, "endCursor": "page-2" }
            }
        } })))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(body_string_contains("query AdministrableTeams"))
        .and(body_partial_json(
            json!({ "variables": { "after": "page-2" } }),
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "data": {
            "administrableTeams": {
                "nodes": [{ "id": "team-ops" }],
                "pageInfo": { "hasNextPage": false, "endCursor": null }
            }
        } })))
        .mount(&server)
        .await;

    let ids = client_for(&server).administrable_teams().await.unwrap();

    assert_eq!(ids, ["team-eng", "team-ops"]);
}

#[tokio::test]
async fn a_window_up_to_now_loads_recent_cycles_and_open_work() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(body_string_contains("query TeamSettings"))
        .and(body_partial_json(
            json!({ "variables": { "teamId": "team-eng" } }),
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(settings()))
        .expect(1)
        .mount(&server)
        .await;
    // Seven two-week cycles back from 3 November 2026.
    Mock::given(method("POST"))
        .and(body_string_contains("query TeamCycles"))
        .and(body_partial_json(json!({ "variables": { "cycles": {
            "isFuture": { "eq": false },
            "endsAt": { "gt": "2026-07-28T09:00:00Z" },
        } } })))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({ "data": { "team": {
            "cycles": { "nodes": (1..=8).map(cycle).collect::<Vec<_>>() }
        } } })),
        )
        .expect(1)
        .mount(&server)
        .await;
    let since = "2026-08-17T00:00:00Z";
    Mock::given(method("POST"))
        .and(body_string_contains("query TeamRangeIssues"))
        .and(body_partial_json(
            json!({ "variables": { "filter": { "or": [
            { "createdAt": { "gte": since } },
            { "completedAt": { "gte": since } },
            { "canceledAt": { "gte": since } },
        ] } } }),
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(issues_page(
            vec![
                issue(1, "completed", "member-ada"),
                issue(2, "started", "member-jun"),
            ],
            None,
        )))
        .expect(1)
        .mount(&server)
        .await;
    mount(
        &server,
        "query TeamOpenIssues",
        issues_page(vec![issue(3, "triage", "member-ada")], None),
    )
    .await;
    let mut started = issue(2, "started", "member-jun");
    started["stateHistory"] = json!({ "nodes": [
        { "stateId": "state-started", "startedAt": "2026-10-20T10:00:00.000Z", "endedAt": null, "state": { "type": "started" } },
    ] });
    mount(
        &server,
        "query TeamStartedIssues",
        issues_page(vec![started], None),
    )
    .await;

    let loaded = client_for(&server)
        .team_analytics(
            "team-eng",
            AnalyticsWindow {
                since: at(since),
                until: None,
            },
        )
        .await
        .unwrap();

    assert_eq!(loaded.team.key, "ENG");
    assert_eq!(loaded.team.estimation, Estimation::Fibonacci);
    let numbers: Vec<u32> = loaded.cycles.iter().map(|c| c.number).collect();
    assert_eq!(numbers, [8, 7, 6, 5, 4, 3, 2]);
    assert!(loaded.cycles.iter().all(|c| c.team_id == "team-eng"));
    let ids: Vec<&str> = loaded.issues.iter().map(|i| i.id.as_str()).collect();
    assert_eq!(ids, ["issue-1", "issue-2", "issue-3"]);
    let in_progress = &loaded.issues[1];
    assert_eq!(in_progress.state.state_type, StateType::Started);
    assert_eq!(in_progress.state_history.len(), 1);
    let mut members: Vec<&str> = loaded.members.iter().map(|m| m.id.as_str()).collect();
    members.sort_unstable();
    assert_eq!(members, ["member-ada", "member-jun"]);
    assert_eq!(loaded.as_of, START);
    assert!(loaded.problems.is_empty(), "{:?}", loaded.problems);
}

#[tokio::test]
async fn an_older_window_asks_only_for_its_issues() {
    let server = MockServer::start().await;
    mount(&server, "query TeamSettings", settings()).await;
    for query in [
        "query TeamCycles",
        "query TeamOpenIssues",
        "query TeamStartedIssues",
    ] {
        Mock::given(method("POST"))
            .and(body_string_contains(query))
            .respond_with(ResponseTemplate::new(500))
            .expect(0)
            .mount(&server)
            .await;
    }
    let range = json!({ "gte": "2026-05-11T00:00:00Z", "lt": "2026-08-17T00:00:00Z" });
    Mock::given(method("POST"))
        .and(body_string_contains("query TeamRangeIssues"))
        .and(body_partial_json(
            json!({ "variables": { "filter": { "or": [
            { "createdAt": range },
            { "completedAt": range },
            { "canceledAt": range },
        ] } } }),
        ))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(issues_page(vec![issue(4, "completed", "member-ada")], None)),
        )
        .expect(1)
        .mount(&server)
        .await;

    let loaded = client_for(&server)
        .team_analytics(
            "team-eng",
            AnalyticsWindow {
                since: at("2026-05-11T00:00:00Z"),
                until: Some(at("2026-08-17T00:00:00Z")),
            },
        )
        .await
        .unwrap();

    assert!(loaded.cycles.is_empty());
    let ids: Vec<&str> = loaded.issues.iter().map(|i| i.id.as_str()).collect();
    assert_eq!(ids, ["issue-4"]);
}

/// A page of one new issue that always says there is another page.
struct EndlessPages;

impl Respond for EndlessPages {
    fn respond(&self, request: &Request) -> ResponseTemplate {
        let body: Value = serde_json::from_slice(&request.body).unwrap();
        let n = body["variables"]["after"]
            .as_str()
            .map_or(0, |cursor| cursor.parse::<u32>().unwrap());
        let cursor = (n + 1).to_string();
        ResponseTemplate::new(200).set_body_json(issues_page(
            vec![issue(100 + n, "completed", "member-ada")],
            Some(&cursor),
        ))
    }
}

#[tokio::test]
async fn a_team_with_too_many_issues_loads_ten_pages_and_says_so() {
    let server = MockServer::start().await;
    mount(&server, "query TeamSettings", settings()).await;
    Mock::given(method("POST"))
        .and(body_string_contains("query TeamRangeIssues"))
        .respond_with(EndlessPages)
        .expect(10)
        .mount(&server)
        .await;

    let loaded = client_for(&server)
        .team_analytics(
            "team-eng",
            AnalyticsWindow {
                since: at("2026-05-11T00:00:00Z"),
                until: Some(at("2026-08-17T00:00:00Z")),
            },
        )
        .await
        .unwrap();

    assert_eq!(loaded.issues.len(), 10);
    assert_eq!(
        loaded.problems,
        ["ENG has too many issues in range to load; team analytics count only the first 10 pages"]
    );
}
