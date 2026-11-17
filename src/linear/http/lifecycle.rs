//! Archiving, unarchiving, trashing and subscribing to one issue: one
//! mutation each, answered with whether Linear did it.

use serde_json::{Value, json};

use super::HttpClient;
use crate::linear::{ClientError, Response};

const ARCHIVE: &str = include_str!("../graphql/issue_archive.graphql");
const UNARCHIVE: &str = include_str!("../graphql/issue_unarchive.graphql");
const TRASH: &str = include_str!("../graphql/issue_delete.graphql");
const SUBSCRIBE: &str = include_str!("../graphql/issue_subscribe.graphql");
const UNSUBSCRIBE: &str = include_str!("../graphql/issue_unsubscribe.graphql");

pub(super) async fn archive(client: &HttpClient, issue_id: &str) -> Result<(), ClientError> {
    run(client, ARCHIVE, "issueArchive", issue_id).await
}

pub(super) async fn unarchive(client: &HttpClient, issue_id: &str) -> Result<(), ClientError> {
    run(client, UNARCHIVE, "issueUnarchive", issue_id).await
}

/// Sends `issueDelete` with the issue's id alone, so Linear trashes it for
/// 30 days and never deletes it for good.
pub(super) async fn trash(client: &HttpClient, issue_id: &str) -> Result<(), ClientError> {
    run(client, TRASH, "issueDelete", issue_id).await
}

pub(super) async fn subscribe(client: &HttpClient, issue_id: &str) -> Result<(), ClientError> {
    run(client, SUBSCRIBE, "issueSubscribe", issue_id).await
}

pub(super) async fn unsubscribe(client: &HttpClient, issue_id: &str) -> Result<(), ClientError> {
    run(client, UNSUBSCRIBE, "issueUnsubscribe", issue_id).await
}

/// Sends `document` for the issue and reads `success` from its `field`.
async fn run(
    client: &HttpClient,
    document: &str,
    field: &str,
    issue_id: &str,
) -> Result<(), ClientError> {
    let response: Response<Value> = client.execute(document, json!({ "id": issue_id })).await?;
    let done = response
        .data
        .get(field)
        .and_then(|payload| payload.get("success"))
        .and_then(Value::as_bool);
    match done {
        Some(true) => Ok(()),
        _ if !response.errors.is_empty() => Err(ClientError::GraphQl(response.errors)),
        _ => Err(ClientError::Decode(format!(
            "Linear did not confirm {field}"
        ))),
    }
}
