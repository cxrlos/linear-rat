//! Looking an issue up by its key.

use serde::Deserialize;
use serde_json::json;

use super::{HttpClient, Urgency};
use crate::linear::{ClientError, IssueLink, Response};

const ISSUE_BY_KEY: &str = include_str!("../graphql/issue_by_key.graphql");

pub(super) async fn issue_by_key(
    client: &HttpClient,
    team_key: &str,
    number: u32,
) -> Result<Option<IssueLink>, ClientError> {
    let Response { data, errors } = client
        .send::<IssueByKeyData>(
            ISSUE_BY_KEY,
            &json!({ "team": team_key, "number": number }),
            Urgency::Urgent,
        )
        .await?;
    for error in &errors {
        tracing::warn!(message = %error.message, "the issue lookup also returned an error");
    }
    Ok(data.issues.nodes.into_iter().next().map(|node| IssueLink {
        identifier: node.identifier,
        title: node.title,
        url: node.url,
    }))
}

#[derive(Deserialize)]
struct IssueByKeyData {
    issues: Nodes,
}

#[derive(Deserialize)]
struct Nodes {
    nodes: Vec<IssueNode>,
}

#[derive(Deserialize)]
struct IssueNode {
    identifier: String,
    title: String,
    url: String,
}
