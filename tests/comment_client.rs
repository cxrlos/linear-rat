//! The real client's comment changes against a local fake HTTP server:
//! replies, edits, deletes, resolving and unresolving, what each sends and
//! how the answer is read.

// clippy's allow-unwrap-in-tests does not cover helpers outside #[test] functions.
#![allow(clippy::unwrap_used)]

use std::time::Duration;

use linear_rat::auth::ApiKey;
use linear_rat::linear::{ClientConfig, ClientError, HttpClient, LinearClient};
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

fn query(body: &Value) -> &str {
    body["query"].as_str().unwrap()
}

#[tokio::test]
async fn a_reply_sends_its_parent_and_reads_it_back() {
    let server = server_answering(json!({ "data": { "commentCreate": {
        "success": true,
        "comment": {
            "id": "comment-10",
            "body": "Agreed",
            "createdAt": "2026-11-02T15:00:01.000Z",
            "parentId": "comment-9",
            "user": { "id": "user-mira", "name": "Mira Gouda", "displayName": "mira" }
        }
    } } }))
    .await;

    let posted = client_for(&server)
        .create_comment("issue-1", "Agreed", Some("comment-9"))
        .await
        .unwrap();

    let body = sent(&server).await;
    assert_eq!(
        body["variables"],
        json!({ "input": { "issueId": "issue-1", "body": "Agreed", "parentId": "comment-9" } })
    );
    assert_eq!(posted.comment.parent_id.as_deref(), Some("comment-9"));
}

#[tokio::test]
async fn an_edit_sends_the_comment_and_its_new_text() {
    let server =
        server_answering(json!({ "data": { "commentUpdate": { "success": true } } })).await;

    client_for(&server)
        .update_comment("comment-9", "Rind looks fine")
        .await
        .unwrap();

    let body = sent(&server).await;
    assert!(query(&body).contains("mutation CommentUpdate"));
    assert_eq!(
        body["variables"],
        json!({ "id": "comment-9", "input": { "body": "Rind looks fine" } })
    );
}

#[tokio::test]
async fn a_delete_sends_the_comment() {
    let server =
        server_answering(json!({ "data": { "commentDelete": { "success": true } } })).await;

    client_for(&server)
        .delete_comment("comment-9")
        .await
        .unwrap();

    let body = sent(&server).await;
    assert!(query(&body).contains("mutation CommentDelete"));
    assert_eq!(body["variables"], json!({ "id": "comment-9" }));
}

#[tokio::test]
async fn resolving_and_unresolving_send_their_own_mutations() {
    let server =
        server_answering(json!({ "data": { "commentResolve": { "success": true } } })).await;
    client_for(&server)
        .resolve_comment("comment-9", true)
        .await
        .unwrap();
    let body = sent(&server).await;
    assert!(query(&body).contains("mutation CommentResolve"));
    assert_eq!(body["variables"], json!({ "id": "comment-9" }));

    let server =
        server_answering(json!({ "data": { "commentUnresolve": { "success": true } } })).await;
    client_for(&server)
        .resolve_comment("comment-9", false)
        .await
        .unwrap();
    let body = sent(&server).await;
    assert!(query(&body).contains("mutation CommentUnresolve"));
}

#[tokio::test]
async fn a_change_linear_does_not_confirm_is_an_error() {
    let server =
        server_answering(json!({ "data": { "commentDelete": { "success": false } } })).await;

    let error = client_for(&server)
        .delete_comment("comment-9")
        .await
        .unwrap_err();

    assert!(matches!(error, ClientError::Decode(_)), "{error:?}");
}

#[tokio::test]
async fn a_change_linear_rejects_is_an_error_with_its_message() {
    let server = server_answering(json!({
        "data": null,
        "errors": [{ "message": "Entity not found: Comment", "extensions": { "code": "INVALID_INPUT" } }]
    }))
    .await;

    let error = client_for(&server)
        .update_comment("comment-gone", "Hello")
        .await
        .unwrap_err();

    assert!(
        matches!(&error, ClientError::GraphQl(errors) if errors[0].message.contains("Entity not found")),
        "{error:?}"
    );
}
