//! Changing a comment that exists: editing its text, deleting it, and
//! resolving or unresolving its thread. One mutation each, answered with
//! whether Linear did it.

use serde_json::{Value, json};

use super::HttpClient;
use crate::linear::{ClientError, Response};

const UPDATE: &str = include_str!("../graphql/comment_update.graphql");
const DELETE: &str = include_str!("../graphql/comment_delete.graphql");
const RESOLVE: &str = include_str!("../graphql/comment_resolve.graphql");
const UNRESOLVE: &str = include_str!("../graphql/comment_unresolve.graphql");

pub(super) async fn update(client: &HttpClient, id: &str, body: &str) -> Result<(), ClientError> {
    let variables = json!({ "id": id, "input": { "body": body } });
    run(client, UPDATE, "commentUpdate", variables).await
}

pub(super) async fn delete(client: &HttpClient, id: &str) -> Result<(), ClientError> {
    run(client, DELETE, "commentDelete", json!({ "id": id })).await
}

pub(super) async fn resolve(
    client: &HttpClient,
    id: &str,
    resolved: bool,
) -> Result<(), ClientError> {
    let (document, field) = if resolved {
        (RESOLVE, "commentResolve")
    } else {
        (UNRESOLVE, "commentUnresolve")
    };
    run(client, document, field, json!({ "id": id })).await
}

/// Sends `document` with `variables` and reads `success` from its `field`.
pub(super) async fn run(
    client: &HttpClient,
    document: &str,
    field: &str,
    variables: Value,
) -> Result<(), ClientError> {
    let response: Response<Value> = client.execute(document, variables).await?;
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
