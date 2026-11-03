//! Loading the viewer's open assigned issues: every page of them, then the
//! labels they carry, decoded into the model.

use jiff::Timestamp;
use serde::{Deserialize, Deserializer};
use serde_json::json;

use super::{HttpClient, PageInfo, Urgency};
use crate::linear::model::{
    Cycle, Estimation, Issue, Label, Member, Priority, StateType, Team, Viewer, WorkflowState,
};
use crate::linear::{AssignedIssues, ClientError, GraphQlError, Response};

const ASSIGNED_ISSUES: &str = include_str!("../graphql/assigned_issues.graphql");
const LABELS: &str = include_str!("../graphql/labels.graphql");

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
    let viewer = pages
        .first()
        .map(|page| Viewer {
            id: page.viewer.id.clone(),
            name: page.viewer.name.clone(),
        })
        .ok_or_else(|| ClientError::Decode("Linear returned no page".to_owned()))?;
    let mut loaded = AssignedIssues {
        viewer,
        issues: Vec::new(),
        cycles: Vec::new(),
        labels: Vec::new(),
        members: Vec::new(),
        teams: Vec::new(),
        as_of: client.timer.now(),
        problems: Vec::new(),
    };
    loaded.note(&errors);
    let mut label_ids: Vec<String> = Vec::new();
    for node in pages
        .into_iter()
        .flat_map(|page| page.viewer.assigned_issues.nodes)
    {
        for id in &node.label_ids {
            push_new(&mut label_ids, id.clone(), |known| known == id);
        }
        loaded.add(node);
    }

    if label_ids.is_empty() {
        return Ok(loaded);
    }
    let labels = client
        .all_pages(
            LABELS,
            json!({ "ids": label_ids }),
            Urgency::Background,
            |page: &LabelsPage| &page.issue_labels.page_info,
        )
        .await;
    // The issues are what My Issues is for, so they still show without
    // label names.
    match labels {
        Ok(Response {
            data: pages,
            errors,
        }) => {
            loaded.note(&errors);
            loaded.labels = pages
                .into_iter()
                .flat_map(|page| page.issue_labels.nodes)
                .map(|node| Label {
                    id: node.id,
                    team_id: node.team.map(|team| team.id),
                    name: node.name,
                })
                .collect();
        }
        Err(error) => {
            tracing::warn!(%error, "the label lookup failed");
            loaded
                .problems
                .push(format!("label names did not load: {error}"));
        }
    }
    Ok(loaded)
}

impl AssignedIssues {
    /// Keeps the errors Linear sent alongside data, so the viewer learns
    /// that something is missing.
    fn note(&mut self, errors: &[GraphQlError]) {
        self.problems.extend(
            errors
                .iter()
                .map(|error| format!("Linear left something out: {error}")),
        );
    }

    /// Adds one issue, and its team, cycle and people the first time each
    /// shows up.
    fn add(&mut self, node: IssueNode) {
        let team_id = node.team.id.clone();
        push_new(
            &mut self.teams,
            Team {
                id: node.team.id,
                key: node.team.key,
                name: node.team.name,
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
        self.issues.push(Issue {
            id: node.id,
            identifier: node.identifier,
            title: node.title,
            description: node.description,
            url: node.url,
            branch_name: node.branch_name,
            state: WorkflowState {
                id: node.state.id,
                team_id: team_id.clone(),
                name: node.state.name,
                state_type: node.state.state_type,
                position: node.state.position,
            },
            team_id,
            priority: node.priority,
            estimate: node.estimate,
            assignee_id: node.assignee.map(|person| person.id),
            creator_id: node.creator.map(|person| person.id),
            cycle_id: node.cycle.map(|cycle| cycle.id),
            project_id: node.project.map(|project| project.id),
            label_ids: node.label_ids,
            created_at: node.created_at,
            updated_at: node.updated_at,
            started_at: node.started_at,
            completed_at: node.completed_at,
            canceled_at: node.canceled_at,
            triaged_at: node.triaged_at,
            // `stateHistory` is a connection per issue, too costly for a
            // list; the detail and analytics load it where they need it.
            state_history: Vec::new(),
        });
    }
}

fn push_new<T>(list: &mut Vec<T>, item: T, is_same: impl Fn(&T) -> bool) {
    if !list.iter().any(is_same) {
        list.push(item);
    }
}

#[derive(Deserialize)]
struct AssignedPage {
    viewer: ViewerWithIssues,
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
struct LabelsPage {
    issue_labels: Connection<LabelNode>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Connection<N> {
    nodes: Vec<N>,
    page_info: PageInfo,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct IssueNode {
    id: String,
    identifier: String,
    title: String,
    description: Option<String>,
    url: String,
    branch_name: String,
    #[serde(deserialize_with = "priority")]
    priority: Priority,
    estimate: Option<f64>,
    label_ids: Vec<String>,
    #[serde(deserialize_with = "timestamp")]
    created_at: Timestamp,
    #[serde(deserialize_with = "timestamp")]
    updated_at: Timestamp,
    #[serde(deserialize_with = "optional_timestamp")]
    started_at: Option<Timestamp>,
    #[serde(deserialize_with = "optional_timestamp")]
    completed_at: Option<Timestamp>,
    #[serde(deserialize_with = "optional_timestamp")]
    canceled_at: Option<Timestamp>,
    #[serde(deserialize_with = "optional_timestamp")]
    triaged_at: Option<Timestamp>,
    team: TeamNode,
    state: StateNode,
    assignee: Option<PersonNode>,
    creator: Option<PersonNode>,
    cycle: Option<CycleNode>,
    project: Option<IdNode>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct TeamNode {
    id: String,
    key: String,
    name: String,
    #[serde(deserialize_with = "estimation")]
    issue_estimation_type: Estimation,
}

#[derive(Deserialize)]
struct StateNode {
    id: String,
    name: String,
    #[serde(rename = "type", deserialize_with = "state_type")]
    state_type: StateType,
    position: f64,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PersonNode {
    id: String,
    name: String,
    display_name: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CycleNode {
    id: String,
    #[serde(deserialize_with = "whole_number")]
    number: u32,
    name: Option<String>,
    #[serde(deserialize_with = "timestamp")]
    starts_at: Timestamp,
    #[serde(deserialize_with = "timestamp")]
    ends_at: Timestamp,
    #[serde(deserialize_with = "optional_timestamp")]
    completed_at: Option<Timestamp>,
}

#[derive(Deserialize)]
struct IdNode {
    id: String,
}

#[derive(Deserialize)]
struct LabelNode {
    id: String,
    name: String,
    team: Option<IdNode>,
}

fn timestamp<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Timestamp, D::Error> {
    let text = String::deserialize(deserializer)?;
    text.parse()
        .map_err(|error| serde::de::Error::custom(format!("bad timestamp `{text}`: {error}")))
}

fn optional_timestamp<'de, D: Deserializer<'de>>(
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
fn whole_number<'de, D: Deserializer<'de>>(deserializer: D) -> Result<u32, D::Error> {
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
fn state_type<'de, D: Deserializer<'de>>(deserializer: D) -> Result<StateType, D::Error> {
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
