//! The real client's `commentCreate` and `issueCreate` against a local fake
//! HTTP server: what goes over the wire and how the answer is read.

// clippy's allow-unwrap-in-tests does not cover helpers outside #[test] functions.
#![allow(clippy::unwrap_used)]

use std::time::Duration;

use linear_rat::auth::ApiKey;
use linear_rat::linear::model::Priority;
use linear_rat::linear::{ClientConfig, ClientError, HttpClient, IssueCreate, LinearClient};
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

/// The body of the only request `server` received.
async fn sent(server: &MockServer) -> Value {
    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 1, "expected exactly one request");
    requests[0].body_json().unwrap()
}

fn new_issue() -> IssueCreate {
    IssueCreate {
        team_id: "team-eng".to_owned(),
        state_id: "state-todo".to_owned(),
        assignee_id: None,
        priority: Priority::High,
        title: "Grate the cheddar".to_owned(),
        description: "Finely, please.".to_owned(),
    }
}

fn issue_node() -> Value {
    json!({
        "id": "issue-1",
        "identifier": "ENG-1",
        "title": "Grate the cheddar",
        "description": "Finely, please.",
        "url": "https://linear.app/squeakworks/issue/ENG-1",
        "branchName": "eng-1-grate-the-cheddar",
        "priority": 2.0,
        "estimate": null,
        "labelIds": [],
        "createdAt": "2026-11-02T15:00:01.000Z",
        "updatedAt": "2026-11-02T15:00:01.000Z",
        "startedAt": null,
        "completedAt": null,
        "canceledAt": null,
        "triagedAt": null,
        "team": { "id": "team-eng", "key": "ENG", "name": "Engineering", "issueEstimationType": "fibonacci" },
        "state": { "id": "state-todo", "name": "Todo", "type": "unstarted", "position": 0.0 },
        "assignee": null,
        "creator": { "id": "user-mira", "name": "Mira Gouda", "displayName": "mira" },
        "cycle": null,
        "project": null
    })
}

#[tokio::test]
async fn a_comment_sends_the_issue_and_body_and_reads_back_the_stored_comment() {
    let server = server_answering(json!({ "data": { "commentCreate": {
        "success": true,
        "comment": {
            "id": "comment-9",
            "body": "Rind looks *great*",
            "createdAt": "2026-11-02T15:00:01.000Z",
            "user": { "id": "user-mira", "name": "Mira Gouda", "displayName": "mira" }
        }
    } } }))
    .await;

    let posted = client_for(&server)
        .create_comment("issue-1", "Rind looks *great*")
        .await
        .unwrap();

    let body = sent(&server).await;
    assert!(
        body["query"]
            .as_str()
            .unwrap()
            .contains("mutation CommentCreate")
    );
    assert_eq!(
        body["variables"],
        json!({ "input": { "issueId": "issue-1", "body": "Rind looks *great*" } })
    );
    assert_eq!(posted.comment.id, "comment-9");
    assert_eq!(posted.comment.issue_id, "issue-1");
    assert_eq!(posted.comment.author_id.as_deref(), Some("user-mira"));
    assert_eq!(posted.author.unwrap().name, "Mira Gouda");
}

#[tokio::test]
async fn a_comment_linear_rejects_is_an_error_with_its_message() {
    let server = server_answering(json!({
        "data": null,
        "errors": [{ "message": "Entity not found: Issue", "extensions": { "code": "INVALID_INPUT" } }]
    }))
    .await;

    let error = client_for(&server)
        .create_comment("issue-gone", "Hello")
        .await
        .unwrap_err();

    assert!(
        matches!(&error, ClientError::GraphQl(errors) if errors[0].message.contains("Entity not found")),
        "{error:?}"
    );
}

#[tokio::test]
async fn an_issue_sends_every_field_of_the_form_and_reads_back_linears_copy() {
    let server = server_answering(json!({ "data": { "issueCreate": {
        "success": true,
        "issue": issue_node()
    } } }))
    .await;

    let issue = client_for(&server)
        .create_issue(&new_issue())
        .await
        .unwrap();

    let body = sent(&server).await;
    assert!(
        body["query"]
            .as_str()
            .unwrap()
            .contains("mutation IssueCreate")
    );
    assert_eq!(
        body["variables"],
        json!({ "input": {
            "teamId": "team-eng",
            "stateId": "state-todo",
            "assigneeId": null,
            "priority": 2,
            "title": "Grate the cheddar",
            "description": "Finely, please.",
        } })
    );
    assert_eq!(issue.identifier, "ENG-1");
    assert_eq!(issue.priority, Priority::High);
    assert_eq!(issue.creator_id.as_deref(), Some("user-mira"));
}

#[tokio::test]
async fn an_issue_linear_does_not_confirm_is_an_error() {
    let server = server_answering(json!({ "data": { "issueCreate": {
        "success": false,
        "issue": null
    } } }))
    .await;

    let error = client_for(&server)
        .create_issue(&new_issue())
        .await
        .unwrap_err();

    assert!(
        matches!(&error, ClientError::Decode(message) if message.contains("did not confirm")),
        "{error:?}"
    );
}
