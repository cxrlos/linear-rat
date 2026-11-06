//! What the viewer writes in `$EDITOR`: one `commentCreate` and one
//! `issueCreate`.

use jiff::Timestamp;
use serde::Deserialize;
use serde_json::json;

use super::HttpClient;
use super::assigned::{IssueNode, PersonNode, timestamp};
use super::edit::priority_number;
use crate::linear::model::{Comment, Issue};
use crate::linear::{ClientError, GraphQlError, IssueCreate, PostedComment, Response};

const COMMENT_CREATE: &str = include_str!("../graphql/comment_create.graphql");
const ISSUE_CREATE: &str = include_str!("../graphql/issue_create.graphql");

pub(super) async fn create_comment(
    client: &HttpClient,
    issue_id: &str,
    body: &str,
) -> Result<PostedComment, ClientError> {
    let response: Response<CommentData> = client
        .execute(
            COMMENT_CREATE,
            json!({ "input": { "issueId": issue_id, "body": body } }),
        )
        .await?;
    let payload = response.data.comment_create;
    if !payload.success {
        return Err(unconfirmed(response.errors, "comment"));
    }
    let node = payload.comment;
    Ok(PostedComment {
        comment: Comment {
            id: node.id,
            issue_id: issue_id.to_owned(),
            author_id: node.user.as_ref().map(|user| user.id.clone()),
            body: node.body,
            created_at: node.created_at,
        },
        author: node.user.map(PersonNode::into_member),
    })
}

pub(super) async fn create_issue(
    client: &HttpClient,
    input: &IssueCreate,
) -> Result<Issue, ClientError> {
    let response: Response<IssueData> = client
        .execute(
            ISSUE_CREATE,
            json!({ "input": {
                "teamId": input.team_id,
                "stateId": input.state_id,
                "assigneeId": input.assignee_id,
                "priority": priority_number(input.priority),
                "title": input.title,
                "description": input.description,
            } }),
        )
        .await?;
    let payload = response.data.issue_create;
    match payload.issue {
        Some(issue) if payload.success => Ok(issue.into_issue()),
        _ => Err(unconfirmed(response.errors, "new issue")),
    }
}

/// Why Linear's answer does not confirm the `what`: the errors that came
/// with it, or a plain message when there are none.
fn unconfirmed(errors: Vec<GraphQlError>, what: &str) -> ClientError {
    if errors.is_empty() {
        ClientError::Decode(format!("Linear did not confirm the {what}"))
    } else {
        ClientError::GraphQl(errors)
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CommentData {
    comment_create: CommentPayload,
}

#[derive(Deserialize)]
struct CommentPayload {
    success: bool,
    comment: CommentNode,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CommentNode {
    id: String,
    body: String,
    #[serde(deserialize_with = "timestamp")]
    created_at: Timestamp,
    user: Option<PersonNode>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct IssueData {
    issue_create: IssuePayload,
}

#[derive(Deserialize)]
struct IssuePayload {
    success: bool,
    issue: Option<IssueNode>,
}
