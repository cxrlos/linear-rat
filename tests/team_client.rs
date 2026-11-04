//! The Team Issues calls of the real client against a local fake HTTP
//! server: the viewer's teams, a team's filter options, one page of its
//! issues at a time, and counting them.

// clippy's allow-unwrap-in-tests does not cover helpers outside #[test] functions.
#![allow(clippy::unwrap_used)]

use linear_rat::auth::ApiKey;
use linear_rat::linear::model::{Estimation, StateType};
use linear_rat::linear::{ClientConfig, HttpClient, LinearClient, TeamFilter, TeamQuery};
use serde_json::{Value, json};
use wiremock::matchers::{body_partial_json, body_string_contains, method};
use wiremock::{Mock, MockServer, ResponseTemplate};

const KEY: &str = "lin_api_fictional0000000000000000000000000000";

fn client_for(server: &MockServer) -> HttpClient {
    let config = ClientConfig {
        endpoint: format!("{}/graphql", server.uri()),
        ..ClientConfig::default()
    };
    HttpClient::with_config(&ApiKey::new(KEY).unwrap(), config).unwrap()
}

fn ok(body: Value) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(body)
}

fn issue(n: u32) -> Value {
    json!({
        "id": format!("issue-{n}"),
        "identifier": format!("ENG-{n}"),
        "title": format!("Issue {n}"),
        "description": null,
        "url": format!("https://linear.app/cheddar-works/issue/ENG-{n}"),
        "branchName": format!("remy/eng-{n}"),
        "priority": 3,
        "estimate": null,
        "labelIds": [],
        "createdAt": "2026-10-01T10:00:00.000Z",
        "updatedAt": "2026-10-02T10:00:00.000Z",
        "startedAt": null,
        "completedAt": null,
        "canceledAt": null,
        "triagedAt": null,
        "team": { "id": "team-eng", "key": "ENG", "name": "Engineering", "issueEstimationType": "fibonacci" },
        "state": { "id": "state-done", "name": "Done", "type": "completed", "position": 0 },
        "assignee": { "id": "member-emile", "name": "Emile Brie", "displayName": "emile" },
        "creator": null,
        "cycle": null,
        "project": null
    })
}

fn issues_page(nodes: Value, end_cursor: Option<&str>) -> Value {
    json!({
        "data": {
            "team": {
                "issues": {
                    "nodes": nodes,
                    "pageInfo": { "hasNextPage": end_cursor.is_some(), "endCursor": end_cursor }
                }
            }
        }
    })
}

fn bodies(requests: &[wiremock::Request]) -> Vec<Value> {
    requests
        .iter()
        .map(|request| request.body_json().unwrap())
        .collect()
}

fn eng() -> TeamQuery {
    TeamQuery {
        team_id: "team-eng".to_owned(),
        filter: TeamFilter::default(),
    }
}

#[tokio::test]
async fn a_page_of_team_issues_names_the_cursor_of_the_next_one() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(body_string_contains("query TeamIssues("))
        .respond_with(ok(issues_page(json!([issue(1), issue(2)]), Some("c1"))))
        .mount(&server)
        .await;

    let page = client_for(&server).team_issues(&eng(), None).await.unwrap();

    let identifiers: Vec<&str> = page.issues.iter().map(|i| i.identifier.as_str()).collect();
    assert_eq!(identifiers, ["ENG-1", "ENG-2"]);
    assert_eq!(page.issues[0].state.state_type, StateType::Completed);
    assert_eq!(page.members[0].name, "Emile Brie");
    assert_eq!(page.next.as_deref(), Some("c1"));
    let body = &bodies(&server.received_requests().await.unwrap())[0];
    assert_eq!(
        body["variables"],
        json!({ "teamId": "team-eng", "filter": null, "after": null })
    );
    let query = body["query"].as_str().unwrap();
    assert!(query.contains("first: 50"), "{query}");
    assert!(query.contains("orderBy: updatedAt"), "{query}");
}

#[tokio::test]
async fn the_next_page_starts_after_the_cursor_and_carries_the_filter() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(body_partial_json(json!({ "variables": { "after": "c1" } })))
        .respond_with(ok(issues_page(json!([issue(3)]), None)))
        .mount(&server)
        .await;
    let query = TeamQuery {
        filter: TeamFilter {
            assignee: Some("member-emile".to_owned()),
            label: Some("label-bug".to_owned()),
            ..TeamFilter::default()
        },
        ..eng()
    };

    let page = client_for(&server)
        .team_issues(&query, Some("c1"))
        .await
        .unwrap();

    assert_eq!(page.issues.len(), 1);
    assert_eq!(page.next, None);
    let body = &bodies(&server.received_requests().await.unwrap())[0];
    assert_eq!(
        body["variables"]["filter"],
        json!({
            "assignee": { "id": { "eq": "member-emile" } },
            "labels": { "some": { "id": { "eq": "label-bug" } } },
        })
    );
}

#[tokio::test]
async fn a_page_that_hands_back_the_same_cursor_is_an_error() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ok(issues_page(json!([issue(3)]), Some("c1"))))
        .mount(&server)
        .await;

    let error = client_for(&server)
        .team_issues(&eng(), Some("c1"))
        .await
        .unwrap_err();

    assert!(error.to_string().contains("cursor"), "{error}");
}

#[tokio::test]
async fn counting_follows_every_page_of_ids() {
    let server = MockServer::start().await;
    let ids = |range: std::ops::Range<u32>| {
        Value::Array(
            range
                .map(|n| json!({ "id": format!("issue-{n}") }))
                .collect(),
        )
    };
    Mock::given(method("POST"))
        .and(body_string_contains("query TeamIssueIds"))
        .and(body_partial_json(json!({ "variables": { "after": null } })))
        .respond_with(ok(issues_page(ids(0..100), Some("c1"))))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(body_string_contains("query TeamIssueIds"))
        .and(body_partial_json(json!({ "variables": { "after": "c1" } })))
        .respond_with(ok(issues_page(ids(100..103), None)))
        .mount(&server)
        .await;
    let query = TeamQuery {
        filter: TeamFilter {
            state: Some("state-todo".to_owned()),
            ..TeamFilter::default()
        },
        ..eng()
    };

    let count = client_for(&server).count_team_issues(&query).await.unwrap();

    assert_eq!(count, 103);
    for body in bodies(&server.received_requests().await.unwrap()) {
        assert_eq!(
            body["variables"]["filter"],
            json!({ "state": { "id": { "eq": "state-todo" } } })
        );
    }
}

#[tokio::test]
async fn the_viewers_teams_come_from_every_page() {
    let server = MockServer::start().await;
    let teams = |nodes: Value, end_cursor: Option<&str>| {
        json!({ "data": { "viewer": { "teams": {
            "nodes": nodes,
            "pageInfo": { "hasNextPage": end_cursor.is_some(), "endCursor": end_cursor }
        } } } })
    };
    Mock::given(method("POST"))
        .and(body_partial_json(json!({ "variables": { "after": null } })))
        .respond_with(ok(teams(
            json!([{ "id": "team-eng", "key": "ENG", "name": "Engineering", "issueEstimationType": "fibonacci" }]),
            Some("t1"),
        )))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(body_partial_json(json!({ "variables": { "after": "t1" } })))
        .respond_with(ok(teams(
            json!([{ "id": "team-ops", "key": "OPS", "name": "Operations", "issueEstimationType": "notUsed" }]),
            None,
        )))
        .mount(&server)
        .await;

    let teams = client_for(&server).teams().await.unwrap();

    let keys: Vec<&str> = teams.iter().map(|team| team.key.as_str()).collect();
    assert_eq!(keys, ["ENG", "OPS"]);
    assert_eq!(teams[1].estimation, Estimation::NotUsed);
}

#[tokio::test]
async fn team_options_hold_the_teams_and_the_workspaces_labels() {
    let server = MockServer::start().await;
    let one_page = |nodes: Value| json!({ "nodes": nodes, "pageInfo": { "hasNextPage": false } });
    Mock::given(method("POST"))
        .and(body_partial_json(json!({ "variables": { "teamId": "team-eng" } })))
        .respond_with(ok(json!({ "data": {
            "team": {
                "id": "team-eng",
                "states": one_page(json!([{ "id": "state-todo", "name": "Todo", "type": "unstarted", "position": 1 }])),
                "members": one_page(json!([{ "id": "member-emile", "name": "Emile Brie", "displayName": "emile" }])),
                "labels": one_page(json!([{ "id": "label-backend", "name": "backend" }])),
                "cycles": one_page(json!([{
                    "id": "cycle-7", "number": 7, "name": null,
                    "startsAt": "2026-10-26T00:00:00.000Z", "endsAt": "2026-11-09T00:00:00.000Z",
                    "completedAt": null
                }])),
                "projects": one_page(json!([{ "id": "project-1", "name": "Nibble 3.0" }])),
            },
            "issueLabels": one_page(json!([{ "id": "label-bug", "name": "bug" }])),
        } })))
        .mount(&server)
        .await;

    let options = client_for(&server).team_options("team-eng").await.unwrap();

    assert_eq!(options.states[0].team_id, "team-eng");
    assert_eq!(options.members[0].display_name, "emile");
    let labels: Vec<(&str, Option<&str>)> = options
        .labels
        .iter()
        .map(|label| (label.name.as_str(), label.team_id.as_deref()))
        .collect();
    assert_eq!(labels, [("backend", Some("team-eng")), ("bug", None)]);
    assert_eq!(options.cycles[0].number, 7);
    assert_eq!(options.projects[0].name, "Nibble 3.0");
    assert!(options.problems.is_empty(), "{:?}", options.problems);
}

#[tokio::test]
async fn team_options_name_the_lists_linear_cut_short() {
    let server = MockServer::start().await;
    let page = |more: bool| json!({ "nodes": [], "pageInfo": { "hasNextPage": more } });
    Mock::given(method("POST"))
        .and(body_string_contains("query TeamOptions("))
        .respond_with(ok(json!({ "data": {
            "team": {
                "id": "team-eng",
                "states": page(false),
                "members": page(false),
                "labels": page(false),
                "cycles": page(false),
                "projects": page(true),
            },
            "issueLabels": page(true),
        } })))
        .mount(&server)
        .await;

    let options = client_for(&server).team_options("team-eng").await.unwrap();

    assert_eq!(
        options.problems,
        [
            "The filters may be missing some projects and workspace labels: \
             Linear sent only the first page"
        ]
    );
}
