//! An issue's parent, sub-issues and relations for its detail, and creating
//! and deleting a relation.

use serde::Deserialize;
use serde_json::json;

use super::assigned::{Decoded, IssueNode};
use super::{HttpClient, Urgency};
use crate::linear::model::{IssueRelation, RelationType};
use crate::linear::{ClientError, GraphQlError, RelationCreate, Response};

const RELATIONS: &str = include_str!("../graphql/issue_relations.graphql");
const CREATE: &str = include_str!("../graphql/issue_relation_create.graphql");
const DELETE: &str = include_str!("../graphql/issue_relation_delete.graphql");

/// What the detail shows about the issue's place among other issues.
pub(super) struct Related {
    pub(super) relations: Vec<IssueRelation>,
    /// The parent, sub-issues and relation ends, with what they refer to.
    pub(super) issues: Decoded,
    pub(super) errors: Vec<GraphQlError>,
}

pub(super) async fn load(
    client: &HttpClient,
    issue_id: &str,
    urgency: Urgency,
) -> Result<Related, ClientError> {
    let Response { data, errors } = client
        .send::<RelationsData>(RELATIONS, &json!({ "id": issue_id }), urgency)
        .await?;
    let issue = data.issue;
    let mut issues = Decoded::default();
    let mut relations = Vec::new();
    if let Some(parent) = issue.parent {
        issues.add(parent);
    }
    for child in issue.children.nodes {
        issues.add(child);
    }
    for node in issue.relations.nodes {
        if let Some(kind) = relation_type(&node.kind) {
            relations.push(IssueRelation {
                id: node.id,
                kind,
                issue_id: issue_id.to_owned(),
                related_issue_id: node.related_issue.id.clone(),
            });
        }
        issues.add(node.related_issue);
    }
    for node in issue.inverse_relations.nodes {
        if let Some(kind) = relation_type(&node.kind) {
            relations.push(IssueRelation {
                id: node.id,
                kind,
                issue_id: node.issue.id.clone(),
                related_issue_id: issue_id.to_owned(),
            });
        }
        issues.add(node.issue);
    }
    Ok(Related {
        relations,
        issues,
        errors,
    })
}

pub(super) async fn create(
    client: &HttpClient,
    input: &RelationCreate,
) -> Result<IssueRelation, ClientError> {
    let response: Response<CreateData> = client
        .execute(
            CREATE,
            json!({ "input": {
                "issueId": input.issue_id,
                "relatedIssueId": input.related_issue_id,
                "type": type_name(input.kind),
            }}),
        )
        .await?;
    let payload = response.data.issue_relation_create;
    let kind = relation_type(&payload.issue_relation.kind);
    match kind {
        Some(kind) if payload.success => Ok(IssueRelation {
            id: payload.issue_relation.id,
            kind,
            issue_id: payload.issue_relation.issue.id,
            related_issue_id: payload.issue_relation.related_issue.id,
        }),
        _ if !response.errors.is_empty() => Err(ClientError::GraphQl(response.errors)),
        _ => Err(ClientError::Decode(
            "Linear did not confirm the relation".to_owned(),
        )),
    }
}

pub(super) async fn delete(client: &HttpClient, id: &str) -> Result<(), ClientError> {
    let response: Response<DeleteData> = client.execute(DELETE, json!({ "id": id })).await?;
    if response.data.issue_relation_delete.success {
        Ok(())
    } else if !response.errors.is_empty() {
        Err(ClientError::GraphQl(response.errors))
    } else {
        Err(ClientError::Decode(
            "Linear did not confirm the relation was deleted".to_owned(),
        ))
    }
}

/// The relation type Linear calls `name`, or `None` for one lrat does not
/// know.
fn relation_type(name: &str) -> Option<RelationType> {
    match name {
        "blocks" => Some(RelationType::Blocks),
        "duplicate" => Some(RelationType::Duplicate),
        "related" => Some(RelationType::Related),
        "similar" => Some(RelationType::Similar),
        other => {
            tracing::warn!(
                relation_type = other,
                "an issue relation of an unknown type"
            );
            None
        }
    }
}

fn type_name(kind: RelationType) -> &'static str {
    match kind {
        RelationType::Blocks => "blocks",
        RelationType::Duplicate => "duplicate",
        RelationType::Related => "related",
        RelationType::Similar => "similar",
    }
}

#[derive(Deserialize)]
struct RelationsData {
    issue: RelationsIssue,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RelationsIssue {
    parent: Option<IssueNode>,
    children: Nodes<IssueNode>,
    relations: Nodes<OutgoingNode>,
    inverse_relations: Nodes<IncomingNode>,
}

#[derive(Deserialize)]
struct Nodes<N> {
    nodes: Vec<N>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct OutgoingNode {
    id: String,
    #[serde(rename = "type")]
    kind: String,
    related_issue: IssueNode,
}

#[derive(Deserialize)]
struct IncomingNode {
    id: String,
    #[serde(rename = "type")]
    kind: String,
    issue: IssueNode,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CreateData {
    issue_relation_create: CreatePayload,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CreatePayload {
    success: bool,
    issue_relation: RelationNode,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RelationNode {
    id: String,
    #[serde(rename = "type")]
    kind: String,
    issue: IdNode,
    related_issue: IdNode,
}

#[derive(Deserialize)]
struct IdNode {
    id: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct DeleteData {
    issue_relation_delete: DeletePayload,
}

#[derive(Deserialize)]
struct DeletePayload {
    success: bool,
}
