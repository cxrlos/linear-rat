//! The real client's notification query and mutations against a local fake
//! HTTP server: which notifications it keeps, what a delta asks for, and the
//! read, snooze and archive mutations.

// clippy's allow-unwrap-in-tests does not cover helpers outside #[test] functions.
#![allow(clippy::unwrap_used)]

use std::time::Duration;

use jiff::Timestamp;
use linear_rat::auth::ApiKey;
use linear_rat::linear::{
    ClientConfig, ClientError, HttpClient, LinearClient, NotificationUpdate, Patch, Urgency,
};
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

fn issue() -> Value {
    json!({
        "id": "issue-7",
        "identifier": "ENG-7",
        "title": "Rind sensor drifts",
        "description": null,
        "url": "https://linear.app/cheddar-works/issue/ENG-7",
        "branchName": "remy/eng-7",
        "priority": 2,
        "estimate": null,
        "labelIds": [],
        "createdAt": "2026-10-01T10:00:00.000Z",
        "updatedAt": "2026-11-01T10:00:00.000Z",
        "startedAt": null,
        "completedAt": null,
        "canceledAt": null,
        "triagedAt": null,
        "team": { "id": "team-eng", "key": "ENG", "name": "Engineering", "issueEstimationType": "notUsed" },
        "state": { "id": "state-todo", "name": "Todo", "type": "unstarted", "position": 1 },
        "assignee": { "id": "viewer", "name": "Remy Fromage", "displayName": "remy" },
        "creator": null,
        "cycle": null,
        "project": null,
        "archivedAt": null
    })
}

fn page() -> Value {
    json!({ "data": { "notifications": {
        "nodes": [
            {
                "__typename": "IssueNotification",
                "id": "notification-1",
                "type": "issueNewComment",
                "createdAt": "2026-11-02T09:00:00.000Z",
                "updatedAt": "2026-11-02T09:00:00.000Z",
                "readAt": null,
                "snoozedUntilAt": null,
                "archivedAt": null,
                "actor": { "id": "member-emile", "name": "Emile Brie", "displayName": "emile" },
                "commentId": "comment-3",
                "reactionEmoji": null,
                "issue": issue()
            },
            {
                "__typename": "ProjectNotification",
                "id": "notification-2",
                "type": "projectUpdateCreated",
                "createdAt": "2026-11-02T08:00:00.000Z",
                "updatedAt": "2026-11-02T08:00:00.000Z",
                "readAt": null,
                "snoozedUntilAt": null,
                "archivedAt": null,
                "actor": null
            }
        ],
        "pageInfo": { "hasNextPage": false, "endCursor": null }
    } } })
}

#[tokio::test]
async fn the_first_load_keeps_issue_notifications_with_their_issue_and_actor() {
    let server = server_answering(page()).await;

    let loaded = client_for(&server).notifications(None).await.unwrap();

    assert_eq!(loaded.notifications.len(), 1);
    let notification = &loaded.notifications[0];
    assert_eq!(notification.id, "notification-1");
    assert_eq!(notification.kind, "issueNewComment");
    assert_eq!(notification.issue_id, "issue-7");
    assert_eq!(notification.team_id, "team-eng");
    assert_eq!(notification.comment_id.as_deref(), Some("comment-3"));
    assert_eq!(notification.actor_id.as_deref(), Some("member-emile"));
    assert_eq!(loaded.skipped, 1);
    assert_eq!(loaded.issues[0].identifier, "ENG-7");
    assert!(
        loaded
            .members
            .iter()
            .any(|member| member.id == "member-emile")
    );
    assert!(loaded.teams.iter().any(|team| team.id == "team-eng"));

    let (query, variables) = sent(&server).await;
    assert!(query.contains("query Notifications"));
    assert_eq!(
        variables["filter"],
        Value::Null,
        "the first load filters nothing"
    );
}

#[tokio::test]
async fn a_delta_asks_for_what_changed_since_archived_ones_included() {
    let server = server_answering(page()).await;
    let since: Timestamp = "2026-11-02T08:30:00Z".parse().unwrap();

    client_for(&server)
        .notifications(Some(since))
        .await
        .unwrap();

    let (_, variables) = sent(&server).await;
    assert_eq!(
        variables["filter"],
        json!({ "updatedAt": { "gt": "2026-11-02T08:30:00Z" } })
    );
    assert_eq!(variables["includeArchived"], json!(true));
}

fn done(field: &str) -> Value {
    json!({ "data": { field: { "success": true } } })
}

#[tokio::test]
async fn read_unread_and_snooze_send_notification_update() {
    let at: Timestamp = "2026-11-02T10:00:00Z".parse().unwrap();
    let server = server_answering(done("notificationUpdate")).await;
    let read = NotificationUpdate {
        read_at: Patch::Set(at),
        ..NotificationUpdate::default()
    };
    client_for(&server)
        .update_notification("notification-1", &read)
        .await
        .unwrap();
    let (query, variables) = sent(&server).await;
    assert!(query.contains("mutation NotificationUpdate"));
    assert_eq!(
        variables,
        json!({ "id": "notification-1", "input": { "readAt": "2026-11-02T10:00:00Z" } })
    );

    let server = server_answering(done("notificationUpdate")).await;
    let unread = NotificationUpdate {
        read_at: Patch::Clear,
        ..NotificationUpdate::default()
    };
    client_for(&server)
        .update_notification("notification-1", &unread)
        .await
        .unwrap();
    let (_, variables) = sent(&server).await;
    assert_eq!(variables["input"], json!({ "readAt": null }));

    let server = server_answering(done("notificationUpdate")).await;
    let snooze = NotificationUpdate {
        snoozed_until_at: Some(at),
        ..NotificationUpdate::default()
    };
    client_for(&server)
        .update_notification("notification-1", &snooze)
        .await
        .unwrap();
    let (_, variables) = sent(&server).await;
    assert_eq!(
        variables["input"],
        json!({ "snoozedUntilAt": "2026-11-02T10:00:00Z" })
    );
}

#[tokio::test]
async fn archive_and_mark_issue_read_send_their_mutations() {
    let server = server_answering(done("notificationArchive")).await;
    client_for(&server)
        .archive_notification("notification-1", Urgency::Background)
        .await
        .unwrap();
    let (query, variables) = sent(&server).await;
    assert!(query.contains("mutation NotificationArchive"));
    assert_eq!(variables, json!({ "id": "notification-1" }));

    let at: Timestamp = "2026-11-02T10:00:00Z".parse().unwrap();
    let server = server_answering(done("notificationMarkReadAll")).await;
    client_for(&server)
        .mark_issue_notifications_read("issue-7", at)
        .await
        .unwrap();
    let (query, variables) = sent(&server).await;
    assert!(query.contains("mutation NotificationMarkReadAll"));
    assert_eq!(
        variables,
        json!({ "issueId": "issue-7", "readAt": "2026-11-02T10:00:00Z" })
    );
}

#[tokio::test]
async fn an_unconfirmed_notification_write_is_an_error() {
    let server =
        server_answering(json!({ "data": { "notificationArchive": { "success": false } } })).await;

    let error = client_for(&server)
        .archive_notification("notification-1", Urgency::Urgent)
        .await
        .unwrap_err();

    assert!(matches!(error, ClientError::Decode(_)), "{error:?}");
}
