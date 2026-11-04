//! The real client's edit requests against a local fake HTTP server: picker
//! loads and the `issueUpdate` mutation.

// clippy's allow-unwrap-in-tests does not cover helpers outside #[test] functions.
#![allow(clippy::unwrap_used)]

use std::time::Duration;

use linear_rat::auth::ApiKey;
use linear_rat::linear::model::{Priority, StateType};
use linear_rat::linear::{ClientConfig, ClientError, HttpClient, IssueUpdate, LinearClient, Patch};
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

fn issue_node() -> Value {
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
        "project": null
    })
}

async fn server_answering(body: Value) -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(&server)
        .await;
    server
}

/// The variables of the only request `server` received.
async fn sent_variables(server: &MockServer) -> Value {
    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 1, "expected exactly one request");
    let body: Value = requests[0].body_json().unwrap();
    assert!(
        body["query"]
            .as_str()
            .unwrap()
            .contains("mutation IssueUpdate")
    );
    body["variables"].clone()
}

fn updated() -> Value {
    json!({ "data": { "issueUpdate": { "success": true, "issue": issue_node() } } })
}

#[tokio::test]
async fn unassigning_sends_assignee_id_as_null_and_nothing_else() {
    let server = server_answering(updated()).await;
    let update = IssueUpdate {
        assignee_id: Patch::Clear,
        ..IssueUpdate::default()
    };

    client_for(&server)
        .update_issue("issue-1", &update)
        .await
        .unwrap();

    assert_eq!(
        sent_variables(&server).await,
        json!({ "id": "issue-1", "input": { "assigneeId": null } })
    );
}

#[tokio::test]
async fn a_field_left_unchanged_is_left_out_of_the_input() {
    let server = server_answering(updated()).await;
    let update = IssueUpdate {
        priority: Some(Priority::None),
        ..IssueUpdate::default()
    };

    client_for(&server)
        .update_issue("issue-1", &update)
        .await
        .unwrap();

    assert_eq!(
        sent_variables(&server).await,
        json!({ "id": "issue-1", "input": { "priority": 0 } })
    );
}

#[tokio::test]
async fn a_state_and_an_assignee_are_sent_by_id() {
    let server = server_answering(updated()).await;
    let update = IssueUpdate {
        state_id: Some("state-review".to_owned()),
        assignee_id: Patch::Set("member-ada".to_owned()),
        ..IssueUpdate::default()
    };

    client_for(&server)
        .update_issue("issue-1", &update)
        .await
        .unwrap();

    assert_eq!(
        sent_variables(&server).await,
        json!({
            "id": "issue-1",
            "input": { "stateId": "state-review", "assigneeId": "member-ada" }
        })
    );
}

#[tokio::test]
async fn an_update_returns_the_issue_as_linear_stored_it() {
    let server = server_answering(updated()).await;

    let issue = client_for(&server)
        .update_issue("issue-1", &IssueUpdate::default())
        .await
        .unwrap();

    assert_eq!(issue.identifier, "ENG-1");
    assert_eq!(issue.state.name, "In Review");
    assert_eq!(issue.state.team_id, "team-eng");
    assert_eq!(issue.priority, Priority::High);
    assert_eq!(issue.assignee_id, None);
}

#[tokio::test]
async fn a_rejected_update_is_an_error_with_linears_message() {
    let server = server_answering(json!({
        "data": null,
        "errors": [{
            "message": "Entity not found: WorkflowState",
            "extensions": { "code": "INVALID_INPUT" }
        }]
    }))
    .await;

    let error = client_for(&server)
        .update_issue("issue-1", &IssueUpdate::default())
        .await
        .unwrap_err();

    assert!(matches!(error, ClientError::GraphQl(_)), "{error:?}");
    assert!(error.to_string().contains("Entity not found"), "{error}");
}

#[tokio::test]
async fn an_update_linear_did_not_apply_is_an_error() {
    let server = server_answering(json!({
        "data": { "issueUpdate": { "success": false, "issue": null } }
    }))
    .await;

    let error = client_for(&server)
        .update_issue("issue-1", &IssueUpdate::default())
        .await
        .unwrap_err();

    assert!(error.to_string().contains("did not confirm"), "{error}");
}

#[tokio::test]
async fn workflow_states_load_for_the_team_asked_for() {
    let server = server_answering(json!({
        "data": { "team": { "states": {
            "nodes": [
                { "id": "s1", "name": "Todo", "type": "unstarted", "position": 0.0 },
                { "id": "s2", "name": "Duplicate", "type": "duplicate", "position": 1.0 }
            ],
            "pageInfo": { "hasNextPage": false, "endCursor": null }
        } } }
    }))
    .await;

    let states = client_for(&server)
        .workflow_states("team-eng")
        .await
        .unwrap();

    let requests = server.received_requests().await.unwrap();
    let body: Value = requests[0].body_json().unwrap();
    assert_eq!(body["variables"]["teamId"], "team-eng");
    let names: Vec<(&str, StateType, &str)> = states
        .iter()
        .map(|s| (s.name.as_str(), s.state_type, s.team_id.as_str()))
        .collect();
    assert_eq!(
        names,
        [
            ("Todo", StateType::Unstarted, "team-eng"),
            ("Duplicate", StateType::Canceled, "team-eng")
        ]
    );
}

#[tokio::test]
async fn team_members_follow_every_page() {
    let server = MockServer::start().await;
    let page = |name: &str, next: Option<&str>| {
        json!({ "data": { "team": { "members": {
            "nodes": [{ "id": format!("member-{name}"), "name": name, "displayName": name }],
            "pageInfo": { "hasNextPage": next.is_some(), "endCursor": next }
        } } } })
    };
    Mock::given(method("POST"))
        .and(wiremock::matchers::body_partial_json(
            json!({ "variables": { "after": null } }),
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(page("ada", Some("c1"))))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(wiremock::matchers::body_partial_json(
            json!({ "variables": { "after": "c1" } }),
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(page("jun", None)))
        .mount(&server)
        .await;

    let members = client_for(&server).team_members("team-eng").await.unwrap();

    let ids: Vec<&str> = members.iter().map(|m| m.id.as_str()).collect();
    assert_eq!(ids, ["member-ada", "member-jun"]);
}

#[tokio::test]
async fn labels_go_as_the_added_and_removed_ids_only() {
    let server = server_answering(updated()).await;
    let update = IssueUpdate {
        added_label_ids: vec!["label-bug".to_owned()],
        removed_label_ids: vec!["label-feature".to_owned(), "label-ux".to_owned()],
        ..IssueUpdate::default()
    };

    client_for(&server)
        .update_issue("issue-1", &update)
        .await
        .unwrap();

    assert_eq!(
        sent_variables(&server).await,
        json!({
            "id": "issue-1",
            "input": {
                "addedLabelIds": ["label-bug"],
                "removedLabelIds": ["label-feature", "label-ux"]
            }
        })
    );
}

#[tokio::test]
async fn an_estimate_is_sent_as_whole_points_and_cleared_as_null() {
    let server = server_answering(updated()).await;
    let client = client_for(&server);
    for estimate in [Patch::Set(5), Patch::Clear] {
        let update = IssueUpdate {
            estimate,
            ..IssueUpdate::default()
        };
        client.update_issue("issue-1", &update).await.unwrap();
    }

    let requests = server.received_requests().await.unwrap();
    let inputs: Vec<Value> = requests
        .iter()
        .map(|request| request.body_json::<Value>().unwrap()["variables"]["input"].clone())
        .collect();
    assert_eq!(
        inputs,
        [json!({ "estimate": 5 }), json!({ "estimate": null })]
    );
}

#[tokio::test]
async fn team_labels_follow_every_page_with_the_workspace_labels() {
    let server = MockServer::start().await;
    let page = |id: &str, team: Value, next: Option<&str>| {
        json!({ "data": { "issueLabels": {
            "nodes": [{ "id": id, "name": id, "team": team }],
            "pageInfo": { "hasNextPage": next.is_some(), "endCursor": next }
        } } })
    };
    Mock::given(method("POST"))
        .and(wiremock::matchers::body_partial_json(
            json!({ "variables": { "after": null, "teamId": "team-eng" } }),
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(page(
            "label-backend",
            json!({ "id": "team-eng" }),
            Some("c1"),
        )))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(wiremock::matchers::body_partial_json(
            json!({ "variables": { "after": "c1" } }),
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(page(
            "label-bug",
            Value::Null,
            None,
        )))
        .mount(&server)
        .await;

    let labels = client_for(&server).team_labels("team-eng").await.unwrap();

    let labels: Vec<(&str, Option<&str>)> = labels
        .iter()
        .map(|label| (label.id.as_str(), label.team_id.as_deref()))
        .collect();
    assert_eq!(
        labels,
        [("label-backend", Some("team-eng")), ("label-bug", None)]
    );
    let requests = server.received_requests().await.unwrap();
    let body: Value = requests[0].body_json().unwrap();
    assert!(body["query"].as_str().unwrap().contains("query TeamLabels"));
}
