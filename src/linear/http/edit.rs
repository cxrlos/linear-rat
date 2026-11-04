//! What the pickers load and what an edit sends: a team's workflow states
//! and members, and one `issueUpdate`.

use serde::Deserialize;
use serde_json::{Map, Value, json};

use super::assigned::{Connection, IssueNode, PersonNode, StateNode};
use super::{HttpClient, Urgency};
use crate::linear::model::{Issue, Member, Priority, WorkflowState};
use crate::linear::{ClientError, IssueUpdate, Patch, Response};

const TEAM_STATES: &str = include_str!("../graphql/team_states.graphql");
const TEAM_MEMBERS: &str = include_str!("../graphql/team_members.graphql");
const ISSUE_UPDATE: &str = include_str!("../graphql/issue_update.graphql");

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
    Value::Object(input)
}

/// Linear stores priority as 0 (none) and 1 (urgent) to 4 (low).
fn priority_number(priority: Priority) -> u8 {
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
#[serde(rename_all = "camelCase")]
struct UpdateData {
    issue_update: UpdatePayload,
}

#[derive(Deserialize)]
struct UpdatePayload {
    success: bool,
    issue: Option<IssueNode>,
}
