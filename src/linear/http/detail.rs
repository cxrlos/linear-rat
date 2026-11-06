//! Loading what an issue's detail shows beyond the list: every page of its
//! comments and of its workflow state history, both at once, and its first
//! attachments.

use jiff::Timestamp;
use serde::Deserialize;
use serde_json::{Value, json};

use super::assigned::{optional_timestamp, state_type, timestamp};
use super::{HttpClient, PageInfo, Urgency};
use crate::linear::model::{
    Attachment, Comment, Member, Project, StateSpan, StateType, WorkflowState,
};
use crate::linear::{ClientError, IssueDetail, Response};

const COMMENTS: &str = include_str!("../graphql/issue_comments.graphql");
const STATE_HISTORY: &str = include_str!("../graphql/issue_state_history.graphql");

pub(super) async fn load(
    client: &HttpClient,
    issue_id: &str,
    urgency: Urgency,
) -> Result<IssueDetail, ClientError> {
    let variables = json!({ "id": issue_id });
    let (comments, history) = futures_util::future::try_join(
        client.all_pages(
            COMMENTS,
            variables.clone(),
            urgency,
            |page: &CommentsPage| &page.issue.comments.page_info,
        ),
        client.all_pages(STATE_HISTORY, variables, urgency, |page: &HistoryPage| {
            &page.issue.state_history.page_info
        }),
    )
    .await?;
    let Response {
        data: comment_pages,
        errors: comment_errors,
    } = comments;
    let Response {
        data: history_pages,
        errors: history_errors,
    } = history;

    let mut detail = IssueDetail {
        comments: Vec::new(),
        state_history: Vec::new(),
        workflow_states: Vec::new(),
        members: Vec::new(),
        project: None,
        attachments: Vec::new(),
        problems: comment_errors
            .iter()
            .chain(&history_errors)
            .map(|error| format!("Linear left something out: {error}"))
            .collect(),
    };
    for node in comment_pages
        .into_iter()
        .flat_map(|page| page.issue.comments.nodes)
    {
        if let Some(user) = &node.user
            && !detail.members.iter().any(|known| known.id == user.id)
        {
            detail.members.push(Member {
                id: user.id.clone(),
                name: user.name.clone(),
                display_name: user.display_name.clone(),
            });
        }
        detail.comments.push(Comment {
            id: node.id,
            issue_id: issue_id.to_owned(),
            author_id: node.user.map(|user| user.id),
            body: node.body,
            created_at: node.created_at,
        });
    }
    for (index, page) in history_pages.into_iter().enumerate() {
        if index == 0 {
            detail.attachments = page
                .issue
                .attachments
                .nodes
                .into_iter()
                .map(|node| node.into_attachment(issue_id))
                .collect();
        }
        if let Some(project) = page.issue.project {
            detail.project = Some(Project {
                id: project.id,
                name: project.name,
            });
        }
        for node in page.issue.state_history.nodes {
            // A span whose state was deleted has no type to show it by.
            let Some(state) = node.state else {
                tracing::warn!(state_id = %node.state_id, "a state span has no workflow state");
                continue;
            };
            detail.state_history.push(StateSpan {
                state_id: node.state_id,
                state_type: state.state_type,
                started_at: node.started_at,
                ended_at: node.ended_at,
            });
            if !detail
                .workflow_states
                .iter()
                .any(|known| known.id == state.id)
            {
                detail.workflow_states.push(WorkflowState {
                    id: state.id,
                    team_id: state.team.id,
                    name: state.name,
                    state_type: state.state_type,
                    position: state.position,
                });
            }
        }
    }
    detail.comments.sort_by_key(|comment| comment.created_at);
    detail.state_history.sort_by_key(|span| span.started_at);
    Ok(detail)
}

#[derive(Deserialize)]
struct CommentsPage {
    issue: IssueComments,
}

#[derive(Deserialize)]
struct IssueComments {
    comments: Connection<CommentNode>,
}

#[derive(Deserialize)]
struct HistoryPage {
    issue: IssueHistory,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct IssueHistory {
    project: Option<ProjectNode>,
    state_history: Connection<SpanNode>,
    attachments: Nodes<AttachmentNode>,
}

#[derive(Deserialize)]
struct Nodes<N> {
    nodes: Vec<N>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct AttachmentNode {
    id: String,
    url: String,
    title: String,
    subtitle: Option<String>,
    source_type: Option<String>,
    #[serde(default)]
    metadata: Value,
}

impl AttachmentNode {
    /// The attachment, with a pull request's state when the metadata holds
    /// a `status` string. Linear does not document the metadata's shape per
    /// source, so anything else there is ignored.
    fn into_attachment(self, issue_id: &str) -> Attachment {
        let status = self.metadata.get("status").and_then(Value::as_str);
        Attachment {
            id: self.id,
            issue_id: issue_id.to_owned(),
            status: status.map(str::to_owned),
            url: self.url,
            title: self.title,
            subtitle: self.subtitle,
            source_type: self.source_type,
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Connection<N> {
    nodes: Vec<N>,
    page_info: PageInfo,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CommentNode {
    id: String,
    body: String,
    #[serde(deserialize_with = "timestamp")]
    created_at: Timestamp,
    user: Option<UserNode>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct UserNode {
    id: String,
    name: String,
    display_name: String,
}

#[derive(Deserialize)]
struct ProjectNode {
    id: String,
    name: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SpanNode {
    state_id: String,
    #[serde(deserialize_with = "timestamp")]
    started_at: Timestamp,
    #[serde(deserialize_with = "optional_timestamp")]
    ended_at: Option<Timestamp>,
    state: Option<StateNode>,
}

#[derive(Deserialize)]
struct StateNode {
    id: String,
    name: String,
    #[serde(rename = "type", deserialize_with = "state_type")]
    state_type: StateType,
    position: f64,
    team: TeamNode,
}

#[derive(Deserialize)]
struct TeamNode {
    id: String,
}
