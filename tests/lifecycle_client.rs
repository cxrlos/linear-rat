//! The real client's archive, unarchive, trash and subscription mutations
//! against a local fake HTTP server, and the archived and subscribed state
//! it reads from an issue.

// clippy's allow-unwrap-in-tests does not cover helpers outside #[test] functions.
#![allow(clippy::unwrap_used)]

use std::time::Duration;

use linear_rat::auth::ApiKey;
use linear_rat::linear::{ClientConfig, ClientError, HttpClient, IssueUpdate, LinearClient};
use serde_json::{Value, json};
use wiremock::matchers::method;
use wiremock::{Mock, MockServer, ResponseTemplate};

const KEY: &str = "lin_api_fictional0000000000000000000000000000";

fn client_for(server: &MockServer) -> HttpClient {
    let config = ClientConfig {
        endpoint: format!("{}/graphql", server.uri()),
        connect_timeout: Duration::from_millis(300),
        request_timeout: Duration::from_secs(5),
    };
    HttpClient::with_config(&ApiKey::new(KEY).unwrap(), config).unwrap()
}

async fn server_answering(body: Value) -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(&server)
        .await;
    server
}

/// The query and variables of the only request `server` received.
async fn sent(server: &MockServer) -> (String, Value) {
    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 1, "expected exactly one request");
    let body: Value = requests[0].body_json().unwrap();
    (
        body["query"].as_str().unwrap().to_owned(),
        body["variables"].clone(),
    )
}

fn done(field: &str) -> Value {
    json!({ "data": { field: { "success": true } } })
}

#[tokio::test]
async fn archive_sends_issue_archive_with_the_id_alone() {
    let server = server_answering(done("issueArchive")).await;

    client_for(&server).archive_issue("issue-1").await.unwrap();

    let (query, variables) = sent(&server).await;
    assert!(query.contains("mutation IssueArchive"));
    assert_eq!(variables, json!({ "id": "issue-1" }));
}

#[tokio::test]
async fn unarchive_sends_issue_unarchive() {
    let server = server_answering(done("issueUnarchive")).await;

    client_for(&server)
        .unarchive_issue("issue-1")
        .await
        .unwrap();

    let (query, variables) = sent(&server).await;
    assert!(query.contains("mutation IssueUnarchive"));
    assert_eq!(variables, json!({ "id": "issue-1" }));
}

#[tokio::test]
async fn trash_sends_issue_delete_and_never_a_permanent_delete() {
    let server = server_answering(done("issueDelete")).await;

    client_for(&server).trash_issue("issue-1").await.unwrap();

    let (query, variables) = sent(&server).await;
    assert!(query.contains("mutation IssueDelete"));
    assert!(!query.contains("permanentlyDelete"));
    assert_eq!(variables, json!({ "id": "issue-1" }));
}

#[tokio::test]
async fn subscribe_and_unsubscribe_send_their_mutations() {
    let server = server_answering(done("issueSubscribe")).await;
    client_for(&server)
        .subscribe_to_issue("issue-1")
        .await
        .unwrap();
    let (query, variables) = sent(&server).await;
    assert!(query.contains("mutation IssueSubscribe"));
    assert_eq!(variables, json!({ "id": "issue-1" }));

    let server = server_answering(done("issueUnsubscribe")).await;
    client_for(&server)
        .unsubscribe_from_issue("issue-1")
        .await
        .unwrap();
    let (query, _) = sent(&server).await;
    assert!(query.contains("mutation IssueUnsubscribe"));
}

#[tokio::test]
async fn an_unconfirmed_mutation_is_an_error() {
    let server =
        server_answering(json!({ "data": { "issueArchive": { "success": false } } })).await;

    let error = client_for(&server)
        .archive_issue("issue-1")
        .await
        .unwrap_err();

    assert!(matches!(error, ClientError::Decode(_)), "{error:?}");
}

#[tokio::test]
async fn graphql_errors_come_back_as_they_are() {
    let server = server_answering(json!({
        "data": null,
        "errors": [{ "message": "Entity not found", "extensions": { "code": "INVALID_INPUT" } }]
    }))
    .await;

    let error = client_for(&server)
        .trash_issue("issue-1")
        .await
        .unwrap_err();

    assert!(matches!(error, ClientError::GraphQl(_)), "{error:?}");
}

fn issue_node(archived_at: Value, subscribers: Value) -> Value {
    json!({
        "id": "issue-1",
        "identifier": "ENG-1",
        "title": "Grate the cheddar",
        "description": null,
        "url": "https://linear.app/squeakworks/issue/ENG-1",
        "branchName": "eng-1-grate-the-cheddar",
        "priority": 2.0,
        "estimate": null,
        "labelIds": [],
        "createdAt": "2026-10-01T09:00:00.000Z",
        "updatedAt": "2026-11-02T15:00:01.000Z",
        "startedAt": null,
        "completedAt": null,
        "canceledAt": null,
        "triagedAt": null,
        "team": { "id": "team-eng", "key": "ENG", "name": "Engineering", "issueEstimationType": "fibonacci" },
        "state": { "id": "state-review", "name": "In Review", "type": "started", "position": 1.0 },
        "assignee": null,
        "creator": null,
        "cycle": null,
        "project": null,
        "archivedAt": archived_at,
        "subscribers": subscribers
    })
}

async fn updated(node: Value) -> linear_rat::linear::model::Issue {
    let server =
        server_answering(json!({ "data": { "issueUpdate": { "success": true, "issue": node } } }))
            .await;
    client_for(&server)
        .update_issue("issue-1", &IssueUpdate::default())
        .await
        .unwrap()
}

#[tokio::test]
async fn an_issue_reads_when_it_was_archived_and_whether_the_viewer_is_subscribed() {
    let issue = updated(issue_node(
        json!("2026-11-03T10:00:00.000Z"),
        json!({ "nodes": [{ "id": "viewer" }] }),
    ))
    .await;

    assert_eq!(
        issue.archived_at.map(|at| at.to_string()),
        Some("2026-11-03T10:00:00Z".to_owned())
    );
    assert!(issue.subscribed);
}

#[tokio::test]
async fn an_issue_without_the_viewer_among_its_subscribers_is_not_subscribed() {
    let issue = updated(issue_node(Value::Null, json!({ "nodes": [] }))).await;

    assert_eq!(issue.archived_at, None);
    assert!(!issue.subscribed);
}
