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
use linear_rat::linear::model::{Attachment, Project, ProjectMilestone, RelationType, StateType};
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
    history_page_with(nodes, Vec::new())
}

fn history_page_with(nodes: Vec<Value>, attachments: Vec<Value>) -> Value {
    json!({
        "data": {
            "issue": {
                "id": "issue-1",
                "project": { "id": "project-rind", "name": "Rind photos" },
                "projectMilestone": {
                    "id": "milestone-beta",
                    "name": "Beta",
                    "project": { "id": "project-rind" }
                },
                "attachments": { "nodes": attachments },
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

fn relations_page(issue: Value) -> Value {
    json!({ "data": { "issue": issue } })
}

fn no_relations() -> Value {
    relations_page(json!({
        "id": "issue-1",
        "parent": null,
        "children": { "nodes": [] },
        "relations": { "nodes": [] },
        "inverseRelations": { "nodes": [] }
    }))
}

async fn mount_relations(server: &MockServer, body: Value) {
    Mock::given(method("POST"))
        .and(body_string_contains("query IssueRelations"))
        .and(body_partial_json(
            json!({ "variables": { "id": "issue-1" } }),
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(server)
        .await;
}

/// An issue node as the relations query returns it, in state `state_type`.
fn related(id: &str, identifier: &str, state_type: &str, parent: Option<&str>) -> Value {
    json!({
        "id": id,
        "identifier": identifier,
        "title": format!("Issue {identifier}"),
        "description": null,
        "url": format!("https://linear.app/squeakworks/issue/{identifier}"),
        "branchName": identifier.to_lowercase(),
        "priority": 0,
        "estimate": null,
        "labelIds": [],
        "createdAt": "2026-10-01T09:00:00.000Z",
        "updatedAt": "2026-10-02T09:00:00.000Z",
        "startedAt": null,
        "completedAt": null,
        "canceledAt": null,
        "triagedAt": null,
        "team": { "id": "team-eng", "key": "ENG", "name": "Engineering", "issueEstimationType": "notUsed" },
        "state": { "id": format!("state-{state_type}"), "name": state_type, "type": state_type, "position": 1 },
        "assignee": null,
        "creator": { "id": "member-ada", "name": "Ada Gouda", "displayName": "ada" },
        "cycle": null,
        "project": null,
        "projectMilestone": null,
        "parent": parent.map(|id| json!({ "id": id })),
        "dueDate": null
    })
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
    mount_relations(&server, no_relations()).await;

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
    assert_eq!(
        detail.milestone,
        Some(ProjectMilestone {
            id: "milestone-beta".to_owned(),
            name: "Beta".to_owned(),
            project_id: "project-rind".to_owned()
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
    mount_relations(&server, no_relations()).await;

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
    assert_eq!(queries.len(), 3);
    assert!(queries.iter().any(|q| q.contains("comments(first: 100")));
    assert!(queries.iter().any(|q| q.contains("children(first: 50")));
    assert!(queries.iter().any(|q| q.contains("relations(first: 25")));
    assert!(
        queries
            .iter()
            .any(|q| q.contains("inverseRelations(first: 25"))
    );
    assert!(queries.iter().any(|q| q.contains("stateHistory(first: 50")));
    assert!(queries.iter().any(|q| q.contains("attachments(first: 50")));
}

#[tokio::test]
async fn loads_the_attachments_with_a_pull_requests_status() {
    let server = MockServer::start().await;
    mount(
        &server,
        "IssueComments",
        None,
        comments_page(Vec::new(), None),
    )
    .await;
    mount(
        &server,
        "IssueStateHistory",
        None,
        history_page_with(
            Vec::new(),
            vec![
                json!({
                    "id": "attachment-pr", "url": "https://github.com/acme/rind/pull/7",
                    "title": "Rotate rind photos", "subtitle": "#7", "sourceType": "github",
                    "metadata": { "status": "merged", "number": 7 }
                }),
                json!({
                    "id": "attachment-slack", "url": "https://acme.slack.com/archives/C1/p1",
                    "title": "Thread in #cellar", "subtitle": null, "sourceType": null,
                    "metadata": {}
                }),
            ],
        ),
    )
    .await;
    mount_relations(&server, no_relations()).await;

    let detail = client_for(&server)
        .issue_detail("issue-1", Urgency::Urgent)
        .await
        .unwrap();

    assert_eq!(
        detail.attachments,
        [
            Attachment {
                id: "attachment-pr".to_owned(),
                issue_id: "issue-1".to_owned(),
                url: "https://github.com/acme/rind/pull/7".to_owned(),
                title: "Rotate rind photos".to_owned(),
                subtitle: Some("#7".to_owned()),
                source_type: Some("github".to_owned()),
                status: Some("merged".to_owned()),
            },
            Attachment {
                id: "attachment-slack".to_owned(),
                issue_id: "issue-1".to_owned(),
                url: "https://acme.slack.com/archives/C1/p1".to_owned(),
                title: "Thread in #cellar".to_owned(),
                subtitle: None,
                source_type: None,
                status: None,
            },
        ]
    );
    assert!(detail.attachments[0].is_pull_request());
    assert!(!detail.attachments[1].is_pull_request());
}

#[tokio::test]
async fn loads_the_issues_own_reactions_oldest_first() {
    let server = MockServer::start().await;
    mount(
        &server,
        "IssueComments",
        None,
        comments_page(Vec::new(), None),
    )
    .await;
    let mut history = history_page(Vec::new());
    history["data"]["issue"]["reactions"] = json!([
        { "id": "reaction-2", "emoji": "🎉", "createdAt": "2026-10-02T00:00:00.000Z", "user": { "id": "member-ana" } },
        { "id": "reaction-1", "emoji": "+1", "createdAt": "2026-10-01T00:00:00.000Z", "user": null },
    ]);
    mount(&server, "IssueStateHistory", None, history).await;
    mount_relations(&server, no_relations()).await;

    let detail = client_for(&server)
        .issue_detail("issue-1", Urgency::Urgent)
        .await
        .unwrap();

    let reactions: Vec<(&str, &str, Option<&str>)> = detail
        .reactions
        .iter()
        .map(|r| (r.id.as_str(), r.emoji.as_str(), r.user_id.as_deref()))
        .collect();
    assert_eq!(
        reactions,
        [
            ("reaction-1", "+1", None),
            ("reaction-2", "🎉", Some("member-ana")),
        ]
    );
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
        ("IssueRelations", no_relations()),
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

#[tokio::test]
async fn loads_the_parent_the_sub_issues_and_the_relations_each_way() {
    let server = MockServer::start().await;
    mount(
        &server,
        "IssueComments",
        None,
        comments_page(Vec::new(), None),
    )
    .await;
    mount(&server, "IssueStateHistory", None, history_page(Vec::new())).await;
    mount_relations(
        &server,
        relations_page(json!({
            "id": "issue-1",
            "parent": related("issue-top", "ENG-10", "started", None),
            "children": { "nodes": [
                related("issue-child", "ENG-11", "completed", Some("issue-1")),
            ] },
            "relations": { "nodes": [
                { "id": "rel-blocks", "type": "blocks",
                  "relatedIssue": related("issue-later", "ENG-12", "unstarted", None) },
                { "id": "rel-similar", "type": "similar",
                  "relatedIssue": related("issue-alike", "ENG-13", "backlog", None) },
            ] },
            "inverseRelations": { "nodes": [
                { "id": "rel-blocker", "type": "blocks",
                  "issue": related("issue-first", "ENG-9", "started", None) },
            ] }
        })),
    )
    .await;

    let detail = client_for(&server)
        .issue_detail("issue-1", Urgency::Urgent)
        .await
        .unwrap();

    let relations: Vec<(&str, RelationType, &str, &str)> = detail
        .relations
        .iter()
        .map(|relation| {
            (
                relation.id.as_str(),
                relation.kind,
                relation.issue_id.as_str(),
                relation.related_issue_id.as_str(),
            )
        })
        .collect();
    assert_eq!(
        relations,
        [
            ("rel-blocks", RelationType::Blocks, "issue-1", "issue-later"),
            (
                "rel-similar",
                RelationType::Similar,
                "issue-1",
                "issue-alike"
            ),
            (
                "rel-blocker",
                RelationType::Blocks,
                "issue-first",
                "issue-1"
            ),
        ]
    );
    let mut keys: Vec<&str> = detail
        .related_issues
        .iter()
        .map(|issue| issue.identifier.as_str())
        .collect();
    keys.sort_unstable();
    assert_eq!(keys, ["ENG-10", "ENG-11", "ENG-12", "ENG-13", "ENG-9"]);
    let child = detail
        .related_issues
        .iter()
        .find(|issue| issue.id == "issue-child")
        .unwrap();
    assert_eq!(child.parent_id.as_deref(), Some("issue-1"));
    assert!(
        detail
            .members
            .iter()
            .any(|member| member.name == "Ada Gouda")
    );
    assert!(detail.problems.is_empty());
}
