//! Loading the viewer's assigned issues with the real client against a local
//! fake HTTP server: pagination, decoding and the budget governor.

// clippy's allow-unwrap-in-tests does not cover helpers outside #[test] functions.
#![allow(clippy::unwrap_used)]

use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use jiff::Timestamp;
use linear_rat::auth::ApiKey;
use linear_rat::linear::model::{Estimation, Priority, StateType};
use linear_rat::linear::{ClientConfig, ClientError, HttpClient, LinearClient, Timer};
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

fn issue(n: u32) -> Value {
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
        "updatedAt": "2026-10-02T10:00:00.000Z",
        "startedAt": null,
        "completedAt": null,
        "canceledAt": null,
        "triagedAt": null,
        "team": { "id": "team-eng", "key": "ENG", "name": "Engineering", "issueEstimationType": "fibonacci" },
        "state": { "id": "state-todo", "name": "Todo", "type": "unstarted", "position": 1 },
        "assignee": { "id": "viewer", "name": "Remy Fromage", "displayName": "remy" },
        "creator": { "id": "member-emile", "name": "Emile Brie", "displayName": "emile" },
        "cycle": null,
        "project": null
    })
}

fn page(issues: Vec<Value>, end_cursor: Option<&str>) -> Value {
    json!({
        "data": {
            "viewer": {
                "id": "viewer",
                "name": "Remy Fromage",
                "assignedIssues": {
                    "nodes": issues,
                    "pageInfo": { "hasNextPage": end_cursor.is_some(), "endCursor": end_cursor }
                }
            }
        }
    })
}

fn labels(nodes: Value) -> Value {
    json!({
        "data": {
            "issueLabels": {
                "nodes": nodes,
                "pageInfo": { "hasNextPage": false, "endCursor": null }
            }
        }
    })
}

async fn mount_page(server: &MockServer, after: Option<&str>, response: ResponseTemplate) {
    Mock::given(method("POST"))
        .and(body_string_contains("query AssignedIssues"))
        .and(body_partial_json(
            json!({ "variables": { "after": after } }),
        ))
        .respond_with(response)
        .mount(server)
        .await;
}

async fn mount_labels(server: &MockServer) {
    Mock::given(method("POST"))
        .and(body_string_contains("query Labels"))
        .respond_with(ResponseTemplate::new(200).set_body_json(labels(json!([
            { "id": "label-bug", "name": "Bug", "team": null }
        ]))))
        .mount(server)
        .await;
}

async fn query_names(server: &MockServer) -> Vec<String> {
    server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .map(|request| {
            let body: Value = request.body_json().unwrap();
            let query = body["query"].as_str().unwrap();
            let (_, after_keyword) = query.split_once("query ").unwrap();
            after_keyword
                .split(|c: char| c == '(' || c == '{' || c.is_whitespace())
                .next()
                .unwrap()
                .to_owned()
        })
        .collect()
}

#[tokio::test]
async fn follows_every_page_until_linear_says_there_are_no_more() {
    let server = MockServer::start().await;
    let ok = |body| ResponseTemplate::new(200).set_body_json(body);
    mount_page(
        &server,
        None,
        ok(page(vec![issue(1), issue(2)], Some("c1"))),
    )
    .await;
    mount_page(&server, Some("c1"), ok(page(vec![issue(3)], Some("c2")))).await;
    mount_page(&server, Some("c2"), ok(page(vec![issue(4)], None))).await;
    mount_labels(&server).await;

    let loaded = client_for(&server, FakeTimer::new())
        .assigned_issues()
        .await
        .unwrap();

    let identifiers: Vec<&str> = loaded
        .issues
        .iter()
        .map(|issue| issue.identifier.as_str())
        .collect();
    assert_eq!(identifiers, ["ENG-1", "ENG-2", "ENG-3", "ENG-4"]);
    assert_eq!(
        query_names(&server).await,
        [
            "AssignedIssues",
            "AssignedIssues",
            "AssignedIssues",
            "Labels"
        ]
    );
}

#[tokio::test]
async fn asks_linear_only_for_open_issues_in_pages_of_a_hundred() {
    let server = MockServer::start().await;
    mount_page(
        &server,
        None,
        ResponseTemplate::new(200).set_body_json(page(Vec::new(), None)),
    )
    .await;

    let loaded = client_for(&server, FakeTimer::new())
        .assigned_issues()
        .await
        .unwrap();

    assert!(loaded.issues.is_empty());
    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 1, "no labels to look up");
    let body: Value = requests[0].body_json().unwrap();
    let query = body["query"].as_str().unwrap();
    assert!(query.contains("first: 100"), "{query}");
    assert!(
        query.contains(r#"nin: ["completed", "canceled", "duplicate"]"#),
        "{query}"
    );
}

#[tokio::test]
async fn decodes_issues_and_what_they_refer_to_into_the_model() {
    let server = MockServer::start().await;
    let mut in_cycle = issue(7);
    in_cycle["cycle"] = json!({
        "id": "cycle-7", "number": 7, "name": null,
        "startsAt": "2026-10-26T00:00:00.000Z", "endsAt": "2026-11-09T00:00:00.000Z",
        "completedAt": null
    });
    in_cycle["priority"] = json!(1);
    in_cycle["estimate"] = json!(null);
    in_cycle["state"] =
        json!({ "id": "state-wip", "name": "In Progress", "type": "started", "position": 2 });
    in_cycle["startedAt"] = json!("2026-10-27T09:30:00.000Z");
    in_cycle["project"] = json!({ "id": "project-rind" });
    mount_page(
        &server,
        None,
        ResponseTemplate::new(200).set_body_json(page(vec![issue(1), in_cycle], None)),
    )
    .await;
    mount_labels(&server).await;

    let loaded = client_for(&server, FakeTimer::new())
        .assigned_issues()
        .await
        .unwrap();

    assert_eq!(loaded.viewer.id, "viewer");
    assert_eq!(loaded.as_of, START);
    let issue = &loaded.issues[1];
    assert_eq!(issue.identifier, "ENG-7");
    assert_eq!(issue.branch_name, "remy/eng-7");
    assert_eq!(issue.team_id, "team-eng");
    assert_eq!(issue.state.state_type, StateType::Started);
    assert_eq!(issue.state.team_id, "team-eng");
    assert_eq!(issue.priority, Priority::Urgent);
    assert_eq!(issue.estimate, None);
    assert_eq!(issue.assignee_id.as_deref(), Some("viewer"));
    assert_eq!(issue.creator_id.as_deref(), Some("member-emile"));
    assert_eq!(issue.cycle_id.as_deref(), Some("cycle-7"));
    assert_eq!(issue.project_id.as_deref(), Some("project-rind"));
    assert_eq!(issue.label_ids, ["label-bug"]);
    assert_eq!(
        issue.started_at,
        Some("2026-10-27T09:30:00Z".parse().unwrap())
    );
    assert!(issue.state_history.is_empty());
    assert_eq!(loaded.issues[0].priority, Priority::High);
    assert_eq!(loaded.issues[0].estimate, Some(3.0));

    assert_eq!(loaded.teams.len(), 1, "teams are listed once");
    assert_eq!(loaded.teams[0].estimation, Estimation::Fibonacci);
    let members: Vec<&str> = loaded.members.iter().map(|m| m.id.as_str()).collect();
    assert_eq!(members, ["viewer", "member-emile"]);
    assert_eq!(loaded.cycles.len(), 1);
    assert_eq!(loaded.cycles[0].number, 7);
    assert_eq!(loaded.cycles[0].team_id, "team-eng");
    assert_eq!(loaded.labels.len(), 1);
    assert_eq!(loaded.labels[0].name, "Bug");
    assert_eq!(loaded.labels[0].team_id, None);
}

#[tokio::test]
async fn looks_up_only_the_labels_the_issues_use() {
    let server = MockServer::start().await;
    let mut second = issue(2);
    second["labelIds"] = json!(["label-bug", "label-rind"]);
    mount_page(
        &server,
        None,
        ResponseTemplate::new(200).set_body_json(page(vec![issue(1), second], None)),
    )
    .await;
    mount_labels(&server).await;

    client_for(&server, FakeTimer::new())
        .assigned_issues()
        .await
        .unwrap();

    let requests = server.received_requests().await.unwrap();
    let body: Value = requests[1].body_json().unwrap();
    assert_eq!(body["variables"]["ids"], json!(["label-bug", "label-rind"]));
}

#[tokio::test]
async fn a_failed_label_lookup_keeps_the_issues_and_says_what_is_missing() {
    let server = MockServer::start().await;
    mount_page(
        &server,
        None,
        ResponseTemplate::new(200).set_body_json(page(vec![issue(1)], None)),
    )
    .await;
    Mock::given(method("POST"))
        .and(body_string_contains("query Labels"))
        .respond_with(ResponseTemplate::new(503))
        .mount(&server)
        .await;

    let loaded = client_for(&server, FakeTimer::new())
        .assigned_issues()
        .await
        .unwrap();

    assert_eq!(loaded.issues.len(), 1);
    assert!(loaded.labels.is_empty());
    assert_eq!(
        loaded.problems,
        ["label names did not load: Linear answered with HTTP 503"]
    );
}

#[tokio::test]
async fn errors_that_come_with_a_page_are_kept_with_the_issues() {
    let server = MockServer::start().await;
    let mut partial = page(vec![issue(1)], None);
    partial["errors"] = json!([{
        "message": "Cycle could not be loaded",
        "path": ["viewer", "assignedIssues", "nodes", 0, "cycle"],
        "extensions": { "code": "INTERNAL_SERVER_ERROR" }
    }]);
    mount_page(
        &server,
        None,
        ResponseTemplate::new(200).set_body_json(partial),
    )
    .await;
    mount_labels(&server).await;

    let loaded = client_for(&server, FakeTimer::new())
        .assigned_issues()
        .await
        .unwrap();

    assert_eq!(loaded.issues.len(), 1);
    assert_eq!(
        loaded.problems,
        ["Linear left something out: Cycle could not be loaded (INTERNAL_SERVER_ERROR)"]
    );
}

#[tokio::test]
async fn a_page_that_promises_more_without_a_cursor_is_an_error() {
    let server = MockServer::start().await;
    let mut broken = page(vec![issue(1)], None);
    broken["data"]["viewer"]["assignedIssues"]["pageInfo"]["hasNextPage"] = json!(true);
    mount_page(
        &server,
        None,
        ResponseTemplate::new(200).set_body_json(broken),
    )
    .await;

    let error = client_for(&server, FakeTimer::new())
        .assigned_issues()
        .await
        .unwrap_err();

    assert!(matches!(error, ClientError::Decode(_)), "{error:?}");
}

#[tokio::test]
async fn a_cursor_that_comes_back_is_an_error_rather_than_a_loop() {
    let server = MockServer::start().await;
    let ok = |body| ResponseTemplate::new(200).set_body_json(body);
    mount_page(&server, None, ok(page(vec![issue(1)], Some("c1")))).await;
    mount_page(&server, Some("c1"), ok(page(vec![issue(2)], Some("c2")))).await;
    mount_page(&server, Some("c2"), ok(page(vec![issue(3)], Some("c1")))).await;

    let client = client_for(&server, FakeTimer::new());
    let error = tokio::time::timeout(Duration::from_secs(5), client.assigned_issues())
        .await
        .expect("the load stops instead of following the cursors round")
        .unwrap_err();

    assert!(matches!(error, ClientError::Decode(_)), "{error:?}");
    assert_eq!(server.received_requests().await.unwrap().len(), 3);
}

#[tokio::test]
async fn an_unknown_state_type_is_an_error_that_names_it() {
    let server = MockServer::start().await;
    let mut odd = issue(1);
    odd["state"]["type"] = json!("paused");
    mount_page(
        &server,
        None,
        ResponseTemplate::new(200).set_body_json(page(vec![odd], None)),
    )
    .await;

    let error = client_for(&server, FakeTimer::new())
        .assigned_issues()
        .await
        .unwrap_err();

    assert!(error.to_string().contains("paused"), "{error}");
}

#[tokio::test]
async fn a_429_waits_for_retry_after_and_sends_the_request_again() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(429).insert_header("retry-after", "7"))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    mount_page(
        &server,
        None,
        ResponseTemplate::new(200).set_body_json(page(vec![issue(1)], None)),
    )
    .await;
    mount_labels(&server).await;
    let timer = FakeTimer::new();

    let loaded = client_for(&server, timer.clone())
        .assigned_issues()
        .await
        .unwrap();

    assert_eq!(loaded.issues.len(), 1);
    assert_eq!(timer.slept(), [Duration::from_secs(7)]);
    assert_eq!(
        query_names(&server).await,
        ["AssignedIssues", "AssignedIssues", "Labels"]
    );
}

#[tokio::test]
async fn a_rate_limit_that_never_lifts_is_an_error_after_a_few_tries() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(429).insert_header("retry-after", "2"))
        .mount(&server)
        .await;
    let timer = FakeTimer::new();

    let error = client_for(&server, timer.clone())
        .assigned_issues()
        .await
        .unwrap_err();

    assert!(matches!(error, ClientError::RateLimited), "{error:?}");
    assert_eq!(timer.slept(), [Duration::from_secs(2); 3]);
    assert_eq!(server.received_requests().await.unwrap().len(), 4);
}

/// `response` with budget headers as Linear sends them, resetting 600 s after
/// [`START`].
fn budget(response: ResponseTemplate, kind: &str, remaining: u64, limit: u64) -> ResponseTemplate {
    let reset_ms = (START.as_second() + 600) * 1000;
    response
        .insert_header(
            format!("x-ratelimit-{kind}-remaining").as_str(),
            remaining.to_string().as_str(),
        )
        .insert_header(
            format!("x-ratelimit-{kind}-limit").as_str(),
            limit.to_string().as_str(),
        )
        .insert_header(
            format!("x-ratelimit-{kind}-reset").as_str(),
            reset_ms.to_string().as_str(),
        )
}

#[tokio::test]
async fn a_low_request_budget_paces_later_pages_but_not_the_first() {
    let server = MockServer::start().await;
    // 300 of 1500 left, 150 of them the reserve: 150 requests spread over the
    // 600 s to the reset is one every 4 s.
    let low = |body| {
        budget(
            ResponseTemplate::new(200).set_body_json(body),
            "requests",
            300,
            1500,
        )
    };
    mount_page(&server, None, low(page(vec![issue(1)], Some("c1")))).await;
    mount_page(&server, Some("c1"), low(page(vec![issue(2)], None))).await;
    mount_labels(&server).await;
    let timer = FakeTimer::new();

    let loaded = client_for(&server, timer.clone())
        .assigned_issues()
        .await
        .unwrap();

    assert_eq!(loaded.issues.len(), 2);
    // The first page went out at once. The second page waited 600 s / 150;
    // the label lookup then had 596 s left to share, so a little less.
    let slept = timer.slept();
    assert_eq!(slept.len(), 2, "{slept:?}");
    assert_eq!(slept[0], Duration::from_secs(4));
    assert_eq!(slept[1].as_millis(), 3973);
}

#[tokio::test]
async fn a_complexity_budget_at_its_reserve_holds_later_pages_until_the_reset() {
    let server = MockServer::start().await;
    let spent = |body| {
        budget(
            ResponseTemplate::new(200).set_body_json(body),
            "complexity",
            20_000,
            250_000,
        )
        .insert_header("x-complexity", "800")
    };
    mount_page(&server, None, spent(page(vec![issue(1)], Some("c1")))).await;
    mount_page(
        &server,
        Some("c1"),
        ResponseTemplate::new(200).set_body_json(page(vec![issue(2)], None)),
    )
    .await;
    mount_labels(&server).await;
    let timer = FakeTimer::new();

    client_for(&server, timer.clone())
        .assigned_issues()
        .await
        .unwrap();

    assert_eq!(timer.slept(), [Duration::from_secs(600)]);
}

#[tokio::test]
async fn a_ratelimited_graphql_error_is_retried_like_a_429() {
    let server = MockServer::start().await;
    let ratelimited = json!({
        "errors": [{ "message": "Rate limit exceeded", "extensions": { "code": "RATELIMITED" } }]
    });
    Mock::given(method("POST"))
        .respond_with(
            ResponseTemplate::new(400)
                .insert_header("retry-after", "5")
                .set_body_json(ratelimited),
        )
        .up_to_n_times(1)
        .mount(&server)
        .await;
    mount_page(
        &server,
        None,
        ResponseTemplate::new(200).set_body_json(page(Vec::new(), None)),
    )
    .await;
    let timer = FakeTimer::new();

    client_for(&server, timer.clone())
        .assigned_issues()
        .await
        .unwrap();

    assert_eq!(timer.slept(), [Duration::from_secs(5)]);
}
