//! What the pickers load and what an edit sends: a team's workflow states,
//! members and labels, a project's milestones, one `issueUpdate`, and the
//! `updatedAt` a text save checks first.

use jiff::Timestamp;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use super::assigned::{Connection, IssueNode, LabelsPage, PersonNode, StateNode, timestamp};
use super::browse::{MilestoneNode, milestones};
use super::{HttpClient, Urgency};
use crate::linear::model::{Issue, Label, Member, Priority, ProjectMilestone, WorkflowState};
use crate::linear::{ClientError, IssueUpdate, Patch, Response};

const TEAM_STATES: &str = include_str!("../graphql/team_states.graphql");
const TEAM_MEMBERS: &str = include_str!("../graphql/team_members.graphql");
const TEAM_LABELS: &str = include_str!("../graphql/team_labels.graphql");
const PROJECT_MILESTONES: &str = include_str!("../graphql/project_milestones.graphql");
const ISSUE_UPDATE: &str = include_str!("../graphql/issue_update.graphql");
const ISSUE_UPDATED_AT: &str = include_str!("../graphql/issue_updated_at.graphql");

pub(super) async fn workflow_states(
    client: &HttpClient,
    team_id: &str,
) -> Result<Vec<WorkflowState>, ClientError> {
    let Response { data: pages, .. } = client
        .all_pages(
            TEAM_STATES,
            json!({ "teamId": team_id }),
            Urgency::Urgent,
            |page: &StatesPage| &page.team.states.page_info,
        )
        .await?;
    Ok(pages
        .into_iter()
        .flat_map(|page| page.team.states.nodes)
        .map(|node| node.into_state(team_id))
        .collect())
}

pub(super) async fn team_members(
    client: &HttpClient,
    team_id: &str,
) -> Result<Vec<Member>, ClientError> {
    let Response { data: pages, .. } = client
        .all_pages(
            TEAM_MEMBERS,
            json!({ "teamId": team_id }),
            Urgency::Urgent,
            |page: &MembersPage| &page.team.members.page_info,
        )
        .await?;
    Ok(pages
        .into_iter()
        .flat_map(|page| page.team.members.nodes)
        .map(PersonNode::into_member)
        .collect())
}

pub(super) async fn team_labels(
    client: &HttpClient,
    team_id: &str,
) -> Result<Vec<Label>, ClientError> {
    let Response { data: pages, .. } = client
        .all_pages(
            TEAM_LABELS,
            json!({ "teamId": team_id }),
            Urgency::Urgent,
            |page: &LabelsPage| &page.issue_labels.page_info,
        )
        .await?;
    Ok(pages
        .into_iter()
        .flat_map(|page| page.issue_labels.nodes)
        .map(Label::from)
        .collect())
}

pub(super) async fn project_milestones(
    client: &HttpClient,
    project_id: &str,
) -> Result<Vec<ProjectMilestone>, ClientError> {
    let Response { data: pages, .. } = client
        .all_pages(
            PROJECT_MILESTONES,
            json!({ "projectId": project_id }),
            Urgency::Urgent,
            |page: &MilestonesPage| &page.project.project_milestones.page_info,
        )
        .await?;
    let nodes = pages
        .into_iter()
        .flat_map(|page| page.project.project_milestones.nodes)
        .collect();
    Ok(milestones(nodes, project_id))
}

pub(super) async fn update_issue(
    client: &HttpClient,
    issue_id: &str,
    update: &IssueUpdate,
) -> Result<Issue, ClientError> {
    let response: Response<UpdateData> = client
        .execute(
            ISSUE_UPDATE,
            json!({ "id": issue_id, "input": input(update) }),
        )
        .await?;
    let payload = response.data.issue_update;
    match payload.issue {
        Some(issue) if payload.success => Ok(issue.into_issue()),
        _ if !response.errors.is_empty() => Err(ClientError::GraphQl(response.errors)),
        _ => Err(ClientError::Decode(
            "Linear did not confirm the update".to_owned(),
        )),
    }
}

/// When the issue was last updated, read as the viewer waits on a save.
pub(super) async fn issue_updated_at(
    client: &HttpClient,
    issue_id: &str,
) -> Result<Timestamp, ClientError> {
    let response: Response<UpdatedAtData> = client
        .execute(ISSUE_UPDATED_AT, json!({ "id": issue_id }))
        .await?;
    Ok(response.data.issue.updated_at)
}

/// `IssueUpdateInput` with only the fields `update` changes. A cleared field
/// is sent as null; an unchanged one is left out.
fn input(update: &IssueUpdate) -> Value {
    let mut input = Map::new();
    if let Some(state_id) = &update.state_id {
        input.insert("stateId".to_owned(), json!(state_id));
    }
    match &update.assignee_id {
        Patch::Unchanged => {}
        Patch::Clear => {
            input.insert("assigneeId".to_owned(), Value::Null);
        }
        Patch::Set(id) => {
            input.insert("assigneeId".to_owned(), json!(id));
        }
    }
    if let Some(priority) = update.priority {
        input.insert("priority".to_owned(), json!(priority_number(priority)));
    }
    match update.estimate {
        Patch::Unchanged => {}
        Patch::Clear => {
            input.insert("estimate".to_owned(), Value::Null);
        }
        Patch::Set(points) => {
            input.insert("estimate".to_owned(), json!(points));
        }
    }
    if !update.added_label_ids.is_empty() {
        input.insert("addedLabelIds".to_owned(), json!(update.added_label_ids));
    }
    if !update.removed_label_ids.is_empty() {
        input.insert(
            "removedLabelIds".to_owned(),
            json!(update.removed_label_ids),
        );
    }
    if let Some(title) = &update.title {
        input.insert("title".to_owned(), json!(title));
    }
    if let Some(description) = &update.description {
        input.insert("description".to_owned(), json!(description));
    }
    patch(&mut input, "dueDate", &update.due_date);
    patch(&mut input, "cycleId", &update.cycle_id);
    patch(&mut input, "projectId", &update.project_id);
    patch(
        &mut input,
        "projectMilestoneId",
        &update.project_milestone_id,
    );
    patch(&mut input, "parentId", &update.parent_id);
    Value::Object(input)
}

/// Puts `value` in `input` as `field`: null when cleared, left out when
/// unchanged.
fn patch<T: Serialize>(input: &mut Map<String, Value>, field: &str, value: &Patch<T>) {
    match value {
        Patch::Unchanged => {}
        Patch::Clear => {
            input.insert(field.to_owned(), Value::Null);
        }
        Patch::Set(value) => {
            input.insert(field.to_owned(), json!(value));
        }
    }
}

/// Linear stores priority as 0 (none) and 1 (urgent) to 4 (low).
pub(super) fn priority_number(priority: Priority) -> u8 {
    match priority {
        Priority::None => 0,
        Priority::Urgent => 1,
        Priority::High => 2,
        Priority::Medium => 3,
        Priority::Low => 4,
    }
}

#[derive(Deserialize)]
struct StatesPage {
    team: TeamStates,
}

#[derive(Deserialize)]
struct TeamStates {
    states: Connection<StateNode>,
}

#[derive(Deserialize)]
struct MembersPage {
    team: TeamMembers,
}

#[derive(Deserialize)]
struct TeamMembers {
    members: Connection<PersonNode>,
}

#[derive(Deserialize)]
struct MilestonesPage {
    project: ProjectMilestones,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ProjectMilestones {
    project_milestones: Connection<MilestoneNode>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct UpdateData {
    issue_update: UpdatePayload,
}

#[derive(Deserialize)]
struct UpdatePayload {
    success: bool,
    issue: Option<IssueNode>,
}

#[derive(Deserialize)]
struct UpdatedAtData {
    issue: UpdatedAtNode,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct UpdatedAtNode {
    #[serde(deserialize_with = "timestamp")]
    updated_at: Timestamp,
}
