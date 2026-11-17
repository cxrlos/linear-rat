//! The Cycles and Projects calls of the real client against a local fake
//! HTTP server: a team's recent cycles, the viewer's projects with their
//! milestones, and the issues of one cycle or project.

// clippy's allow-unwrap-in-tests does not cover helpers outside #[test] functions.
#![allow(clippy::unwrap_used)]

use linear_rat::auth::ApiKey;
use linear_rat::linear::model::ProjectStatusType;
use linear_rat::linear::{ClientConfig, HttpClient, LinearClient};
use serde_json::{Value, json};
use wiremock::matchers::{body_string_contains, method};
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

fn issue(n: u32, milestone: Option<&str>) -> Value {
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
        "state": { "id": "state-todo", "name": "Todo", "type": "unstarted", "position": 0 },
        "assignee": null,
        "creator": null,
        "cycle": null,
        "project": { "id": "project-cave" },
        "projectMilestone": milestone.map(|id| json!({ "id": id })),
        "parent": null,
        "dueDate": null
    })
}

#[tokio::test]
async fn projects_come_with_status_target_lead_progress_and_milestones_in_order() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(body_string_contains("query ViewerProjects"))
        .respond_with(ok(json!({
            "data": { "projects": {
                "nodes": [{
                    "id": "project-cave",
                    "name": "Cave aging",
                    "status": { "name": "In Progress", "type": "started" },
                    "targetDate": "2026-12-15",
                    "progress": 0.4,
                    "lead": { "id": "member-emile", "name": "Emile Brie", "displayName": "emile" },
                    "projectMilestones": {
                        "nodes": [
                            { "id": "m-launch", "name": "Launch", "sortOrder": 2.0 },
                            { "id": "m-beta", "name": "Beta", "sortOrder": 1.0 }
                        ],
                        "pageInfo": { "hasNextPage": false }
                    }
                }],
                "pageInfo": { "hasNextPage": true }
            } }
        })))
        .mount(&server)
        .await;

    let list = client_for(&server).viewer_projects().await.unwrap();

    let project = &list.projects[0];
    assert_eq!(project.project.name, "Cave aging");
    assert_eq!(project.status.name, "In Progress");
    assert_eq!(project.status.status_type, ProjectStatusType::Started);
    assert_eq!(project.target_date, Some(jiff::civil::date(2026, 12, 15)));
    assert_eq!(project.lead.as_ref().unwrap().name, "Emile Brie");
    assert!((project.progress - 0.4).abs() < f64::EPSILON);
    let milestones: Vec<&str> = project.milestones.iter().map(|m| m.name.as_str()).collect();
    assert_eq!(milestones, ["Beta", "Launch"]);
    assert_eq!(project.milestones[0].project_id, "project-cave");
    assert_eq!(list.problems.len(), 1, "{:?}", list.problems);
    let body: Value = server.received_requests().await.unwrap()[0]
        .body_json()
        .unwrap();
    let query = body["query"].as_str().unwrap();
    assert!(query.contains("status {"), "{query}");
    assert!(!query.contains("state"), "{query}");
}

#[tokio::test]
async fn project_issues_follow_pages_and_name_their_milestone() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(body_string_contains("query ProjectIssues"))
        .respond_with(ok(json!({
            "data": { "project": { "issues": {
                "nodes": [issue(1, Some("m-beta")), issue(2, None)],
                "pageInfo": { "hasNextPage": false, "endCursor": null }
            } } }
        })))
        .mount(&server)
        .await;

    let found = client_for(&server)
        .project_issues("project-cave")
        .await
        .unwrap();

    let milestones: Vec<Option<&str>> = found
        .issues
        .iter()
        .map(|issue| issue.project_milestone_id.as_deref())
        .collect();
    assert_eq!(milestones, [Some("m-beta"), None]);
    assert!(found.problems.is_empty(), "{:?}", found.problems);
    let body: Value = server.received_requests().await.unwrap()[0]
        .body_json()
        .unwrap();
    assert_eq!(body["variables"]["projectId"], "project-cave");
}

#[tokio::test]
async fn cycle_issues_ask_for_the_cycle_by_id() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(body_string_contains("query CycleIssues"))
        .respond_with(ok(json!({
            "data": { "cycle": { "issues": {
                "nodes": [issue(3, None)],
                "pageInfo": { "hasNextPage": false, "endCursor": null }
            } } }
        })))
        .mount(&server)
        .await;

    let found = client_for(&server).cycle_issues("cycle-9").await.unwrap();

    assert_eq!(found.issues[0].identifier, "ENG-3");
    let body: Value = server.received_requests().await.unwrap()[0]
        .body_json()
        .unwrap();
    assert_eq!(body["variables"]["cycleId"], "cycle-9");
}

#[tokio::test]
async fn recent_cycles_come_with_their_issue_counts() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(body_string_contains("query RecentCycles"))
        .respond_with(ok(json!({
            "data": { "team": { "cycles": {
                "nodes": [{
                    "id": "cycle-9",
                    "number": 9,
                    "name": null,
                    "startsAt": "2026-11-09T00:00:00.000Z",
                    "endsAt": "2026-11-23T00:00:00.000Z",
                    "completedAt": null,
                    "scopeHistory": [3.0],
                    "completedScopeHistory": [1.0],
                    "issueCountHistory": [4, 5],
                    "completedIssueCountHistory": [1, 2]
                }],
                "pageInfo": { "hasNextPage": false }
            } } }
        })))
        .mount(&server)
        .await;

    let cycles = client_for(&server).recent_cycles("team-eng").await.unwrap();

    assert_eq!(cycles[0].number, 9);
    assert_eq!(cycles[0].team_id, "team-eng");
    assert_eq!(cycles[0].issue_count_history, [4, 5]);
    let body: Value = server.received_requests().await.unwrap()[0]
        .body_json()
        .unwrap();
    assert_eq!(body["variables"]["teamId"], "team-eng");
}
