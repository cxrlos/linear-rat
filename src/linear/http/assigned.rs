//! Loading the viewer's open assigned issues: every page of them, then the
//! labels they carry, decoded into the model.

use jiff::Timestamp;
use serde::{Deserialize, Deserializer};
use serde_json::json;

use super::{HttpClient, PageInfo, Urgency};
use crate::linear::model::{
    Cycle, Estimation, Issue, Label, Member, Priority, StateType, Team, Viewer, WorkflowState,
    Workspace,
};
use crate::linear::{AssignedIssues, ClientError, GraphQlError, Response};

const ASSIGNED_ISSUES: &str = include_str!("../graphql/assigned_issues.graphql");
pub(super) const LABELS: &str = include_str!("../graphql/labels.graphql");

pub(super) async fn load(client: &HttpClient) -> Result<AssignedIssues, ClientError> {
    let Response {
        data: pages,
        errors,
    } = client
        .all_pages(
            ASSIGNED_ISSUES,
            json!({}),
            Urgency::Urgent,
            |page: &AssignedPage| &page.viewer.assigned_issues.page_info,
        )
        .await?;
    let (viewer, workspace) = pages
        .first()
        .map(|page| {
            let viewer = Viewer {
                id: page.viewer.id.clone(),
                name: page.viewer.name.clone(),
            };
            let workspace = Workspace {
                id: page.organization.id.clone(),
                name: page.organization.name.clone(),
                url_key: page.organization.url_key.clone(),
            };
            (viewer, workspace)
        })
        .ok_or_else(|| ClientError::Decode("Linear returned no page".to_owned()))?;
    let mut decoded = Decoded::default();
    for node in pages
        .into_iter()
        .flat_map(|page| page.viewer.assigned_issues.nodes)
    {
        decoded.add(node);
    }
    let mut loaded = AssignedIssues {
        viewer,
        workspace,
        issues: Vec::new(),
        cycles: Vec::new(),
        labels: Vec::new(),
        members: Vec::new(),
        teams: Vec::new(),
        as_of: client.timer.now(),
        problems: Vec::new(),
    };
    loaded.note(&errors);

    // The issues are what My Issues is for, so they still show without
    // label names.
    match labels(client, &decoded.label_ids).await {
        Ok(Response { data, errors }) => {
            loaded.note(&errors);
            loaded.labels = data;
        }
        Err(error) => {
            tracing::warn!(%error, "the label lookup failed");
            loaded
                .problems
                .push(format!("label names did not load: {error}"));
        }
    }
    loaded.issues = decoded.issues;
    loaded.cycles = decoded.cycles;
    loaded.members = decoded.members;
    loaded.teams = decoded.teams;
    Ok(loaded)
}

/// The labels with `ids`, archived ones included, as a background request.
/// No ids, no request.
pub(super) async fn labels(
    client: &HttpClient,
    ids: &[String],
) -> Result<Response<Vec<Label>>, ClientError> {
    if ids.is_empty() {
        return Ok(Response {
            data: Vec::new(),
            errors: Vec::new(),
        });
    }
    let Response {
        data: pages,
        errors,
    } = client
        .all_pages(
            LABELS,
            json!({ "ids": ids }),
            Urgency::Background,
            |page: &LabelsPage| &page.issue_labels.page_info,
        )
        .await?;
    let labels = pages
        .into_iter()
        .flat_map(|page| page.issue_labels.nodes)
        .map(|node| Label {
            id: node.id,
            team_id: node.team.map(|team| team.id),
            name: node.name,
        })
        .collect();
    Ok(Response {
        data: labels,
        errors,
    })
}

impl AssignedIssues {
    /// Keeps the errors Linear sent alongside data, so the viewer learns
    /// that something is missing.
    pub(super) fn note(&mut self, errors: &[GraphQlError]) {
        self.problems.extend(
            errors
                .iter()
                .map(|error| format!("Linear left something out: {error}")),
        );
    }
}

/// Issues decoded from their nodes, with the teams, people and cycles they
/// refer to and the ids of their labels, each once.
#[derive(Default)]
pub(super) struct Decoded {
    pub(super) issues: Vec<Issue>,
    pub(super) cycles: Vec<Cycle>,
    pub(super) members: Vec<Member>,
    pub(super) teams: Vec<Team>,
    pub(super) label_ids: Vec<String>,
}

impl Decoded {
    /// Adds one issue, and its team, cycle, people and label ids the first
    /// time each shows up.
    pub(super) fn add(&mut self, node: IssueNode) {
        for id in &node.label_ids {
            push_new(&mut self.label_ids, id.clone(), |known| known == id);
        }
        let team_id = node.team.id.clone();
        push_new(
            &mut self.teams,
            Team {
                id: node.team.id.clone(),
                key: node.team.key.clone(),
                name: node.team.name.clone(),
                estimation: node.team.issue_estimation_type,
            },
            |known| known.id == team_id,
        );
        for person in [&node.assignee, &node.creator].into_iter().flatten() {
            push_new(
                &mut self.members,
                Member {
                    id: person.id.clone(),
                    name: person.name.clone(),
                    display_name: person.display_name.clone(),
                },
                |known| known.id == person.id,
            );
        }
        if let Some(cycle) = &node.cycle {
            push_new(
                &mut self.cycles,
                Cycle {
                    id: cycle.id.clone(),
                    team_id: team_id.clone(),
                    number: cycle.number,
                    name: cycle.name.clone(),
                    starts_at: cycle.starts_at,
                    ends_at: cycle.ends_at,
                    completed_at: cycle.completed_at,
                    // My Issues needs no history; analytics loads it.
                    scope_history: Vec::new(),
                    completed_scope_history: Vec::new(),
                    issue_count_history: Vec::new(),
                    completed_issue_count_history: Vec::new(),
                },
                |known| known.id == cycle.id,
            );
        }
        self.issues.push(node.into_issue());
    }
}

impl IssueNode {
    /// The issue this node describes, without its state history.
    pub(super) fn into_issue(self) -> Issue {
        let team_id = self.team.id;
        Issue {
            id: self.id,
            identifier: self.identifier,
            title: self.title,
            description: self.description,
            url: self.url,
            branch_name: self.branch_name,
            state: self.state.into_state(&team_id),
            team_id,
            priority: self.priority,
            estimate: self.estimate,
            assignee_id: self.assignee.map(|person| person.id),
            creator_id: self.creator.map(|person| person.id),
            cycle_id: self.cycle.map(|cycle| cycle.id),
            project_id: self.project.map(|project| project.id),
            label_ids: self.label_ids,
            created_at: self.created_at,
            updated_at: self.updated_at,
            started_at: self.started_at,
            completed_at: self.completed_at,
            canceled_at: self.canceled_at,
            triaged_at: self.triaged_at,
            // `stateHistory` is a connection per issue, too costly for a
            // list; the detail and analytics load it where they need it.
            state_history: Vec::new(),
        }
    }
}

impl StateNode {
    pub(super) fn into_state(self, team_id: &str) -> WorkflowState {
        WorkflowState {
            id: self.id,
            team_id: team_id.to_owned(),
            name: self.name,
            state_type: self.state_type,
            position: self.position,
        }
    }
}

impl PersonNode {
    pub(super) fn into_member(self) -> Member {
        Member {
            id: self.id,
            name: self.name,
            display_name: self.display_name,
        }
    }
}

pub(super) fn push_new<T>(list: &mut Vec<T>, item: T, is_same: impl Fn(&T) -> bool) {
    if !list.iter().any(is_same) {
        list.push(item);
    }
}

#[derive(Deserialize)]
struct AssignedPage {
    viewer: ViewerWithIssues,
    organization: WorkspaceNode,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct WorkspaceNode {
    id: String,
    name: String,
    url_key: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ViewerWithIssues {
    id: String,
    name: String,
    assigned_issues: Connection<IssueNode>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct LabelsPage {
    pub(super) issue_labels: Connection<LabelNode>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Connection<N> {
    pub(super) nodes: Vec<N>,
    pub(super) page_info: PageInfo,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct IssueNode {
    pub(super) id: String,
    pub(super) identifier: String,
    pub(super) title: String,
    pub(super) description: Option<String>,
    pub(super) url: String,
    pub(super) branch_name: String,
    #[serde(deserialize_with = "priority")]
    pub(super) priority: Priority,
    pub(super) estimate: Option<f64>,
    pub(super) label_ids: Vec<String>,
    #[serde(deserialize_with = "timestamp")]
    pub(super) created_at: Timestamp,
    #[serde(deserialize_with = "timestamp")]
    pub(super) updated_at: Timestamp,
    #[serde(deserialize_with = "optional_timestamp")]
    pub(super) started_at: Option<Timestamp>,
    #[serde(deserialize_with = "optional_timestamp")]
    pub(super) completed_at: Option<Timestamp>,
    #[serde(deserialize_with = "optional_timestamp")]
    pub(super) canceled_at: Option<Timestamp>,
    #[serde(deserialize_with = "optional_timestamp")]
    pub(super) triaged_at: Option<Timestamp>,
    pub(super) team: TeamNode,
    pub(super) state: StateNode,
    pub(super) assignee: Option<PersonNode>,
    pub(super) creator: Option<PersonNode>,
    pub(super) cycle: Option<CycleNode>,
    pub(super) project: Option<IdNode>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct TeamNode {
    pub(super) id: String,
    pub(super) key: String,
    pub(super) name: String,
    #[serde(deserialize_with = "estimation")]
    pub(super) issue_estimation_type: Estimation,
}

#[derive(Deserialize)]
pub(super) struct StateNode {
    pub(super) id: String,
    pub(super) name: String,
    #[serde(rename = "type", deserialize_with = "state_type")]
    pub(super) state_type: StateType,
    pub(super) position: f64,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct PersonNode {
    pub(super) id: String,
    pub(super) name: String,
    pub(super) display_name: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct CycleNode {
    pub(super) id: String,
    #[serde(deserialize_with = "whole_number")]
    pub(super) number: u32,
    pub(super) name: Option<String>,
    #[serde(deserialize_with = "timestamp")]
    pub(super) starts_at: Timestamp,
    #[serde(deserialize_with = "timestamp")]
    pub(super) ends_at: Timestamp,
    #[serde(deserialize_with = "optional_timestamp")]
    pub(super) completed_at: Option<Timestamp>,
}

#[derive(Deserialize)]
pub(super) struct IdNode {
    pub(super) id: String,
}

#[derive(Deserialize)]
pub(super) struct LabelNode {
    pub(super) id: String,
    pub(super) name: String,
    pub(super) team: Option<IdNode>,
}

impl From<LabelNode> for Label {
    fn from(node: LabelNode) -> Self {
        Label {
            id: node.id,
            team_id: node.team.map(|team| team.id),
            name: node.name,
        }
    }
}

pub(super) fn timestamp<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Timestamp, D::Error> {
    let text = String::deserialize(deserializer)?;
    text.parse()
        .map_err(|error| serde::de::Error::custom(format!("bad timestamp `{text}`: {error}")))
}

pub(super) fn optional_timestamp<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<Timestamp>, D::Error> {
    match Option::<String>::deserialize(deserializer)? {
        Some(text) => text
            .parse()
            .map(Some)
            .map_err(|error| serde::de::Error::custom(format!("bad timestamp `{text}`: {error}"))),
        None => Ok(None),
    }
}

/// Linear sends whole numbers such as a cycle's number as floats.
pub(super) fn whole_number<'de, D: Deserializer<'de>>(deserializer: D) -> Result<u32, D::Error> {
    let number = f64::deserialize(deserializer)?;
    if number.fract() == 0.0 && (0.0..=f64::from(u32::MAX)).contains(&number) {
        Ok(number as u32)
    } else {
        Err(serde::de::Error::custom(format!(
            "expected a whole number, got {number}"
        )))
    }
}

/// Linear stores priority as 0 (none) and 1 (urgent) to 4 (low).
fn priority<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Priority, D::Error> {
    match whole_number(deserializer)? {
        0 => Ok(Priority::None),
        1 => Ok(Priority::Urgent),
        2 => Ok(Priority::High),
        3 => Ok(Priority::Medium),
        4 => Ok(Priority::Low),
        other => Err(serde::de::Error::custom(format!(
            "unknown priority {other}"
        ))),
    }
}

/// Linear's `duplicate` state type is a kind of canceled.
pub(super) fn state_type<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<StateType, D::Error> {
    let name = String::deserialize(deserializer)?;
    match name.as_str() {
        "triage" => Ok(StateType::Triage),
        "backlog" => Ok(StateType::Backlog),
        "unstarted" => Ok(StateType::Unstarted),
        "started" => Ok(StateType::Started),
        "completed" => Ok(StateType::Completed),
        "canceled" | "duplicate" => Ok(StateType::Canceled),
        _ => Err(serde::de::Error::custom(format!(
            "unknown workflow state type `{name}`"
        ))),
    }
}

fn estimation<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Estimation, D::Error> {
    let name = String::deserialize(deserializer)?;
    match name.as_str() {
        "notUsed" => Ok(Estimation::NotUsed),
        "exponential" => Ok(Estimation::Exponential),
        "fibonacci" => Ok(Estimation::Fibonacci),
        "linear" => Ok(Estimation::Linear),
        "tShirt" => Ok(Estimation::TShirt),
        _ => Err(serde::de::Error::custom(format!(
            "unknown estimation type `{name}`"
        ))),
    }
}
