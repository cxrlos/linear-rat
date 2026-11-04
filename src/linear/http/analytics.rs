//! Loading what personal analytics needs for a window of time: the viewer's
//! teams and their cycles, the issues created or completed in the window,
//! the in-progress issues with their state history when the window reaches
//! now, and the names of the labels and projects those issues refer to.

use std::collections::HashMap;

use jiff::Timestamp;
use serde::{Deserialize, Deserializer};
use serde_json::{Value, json};

use super::assigned::{
    Connection, Decoded, IssueNode, LABELS, LabelsPage, TeamNode, optional_timestamp, push_new,
    state_type, timestamp, whole_number,
};
use super::{HttpClient, Urgency};
use crate::linear::model::{Cycle, Issue, Label, Project, StateSpan, StateType, Team, Viewer};
use crate::linear::{AnalyticsIssues, AnalyticsWindow, ClientError, GraphQlError, Response};

const TEAMS: &str = include_str!("../graphql/analytics_teams.graphql");
const ISSUES: &str = include_str!("../graphql/analytics_issues.graphql");
const IN_PROGRESS: &str = include_str!("../graphql/in_progress_issues.graphql");
const PROJECTS: &str = include_str!("../graphql/projects.graphql");

/// The `first:` of a team's cycles in `analytics_teams.graphql`.
const CYCLES_PER_TEAM: usize = 30;

pub(super) async fn load(
    client: &HttpClient,
    window: AnalyticsWindow,
) -> Result<AnalyticsIssues, ClientError> {
    let Response {
        data: team_pages,
        errors: team_errors,
    } = client
        .all_pages(
            TEAMS,
            json!({ "cycles": cycle_filter(window) }),
            Urgency::Urgent,
            |page: &TeamsPage| &page.viewer.teams.page_info,
        )
        .await?;
    let viewer = team_pages
        .first()
        .map(|page| Viewer {
            id: page.viewer.id.clone(),
            name: page.viewer.name.clone(),
        })
        .ok_or_else(|| ClientError::Decode("Linear returned no page".to_owned()))?;
    let Response {
        data: issue_pages,
        errors: issue_errors,
    } = client
        .all_pages(
            ISSUES,
            json!({ "filter": issue_filter(window) }),
            Urgency::Urgent,
            |page: &IssuesPage<IssueNode>| &page.viewer.assigned_issues.page_info,
        )
        .await?;

    let as_of = client.timer.now();
    let mut problems = Vec::new();
    note(&mut problems, &team_errors);
    note(&mut problems, &issue_errors);
    let mut found = Decoded::default();
    for node in issue_pages
        .into_iter()
        .flat_map(|page| page.viewer.assigned_issues.nodes)
    {
        found.add(node);
    }
    if window.reaches_now() {
        let Response {
            data: pages,
            errors,
        } = client
            .all_pages(
                IN_PROGRESS,
                json!({}),
                Urgency::Urgent,
                |page: &IssuesPage<InProgressNode>| &page.viewer.assigned_issues.page_info,
            )
            .await?;
        note(&mut problems, &errors);
        for node in pages
            .into_iter()
            .flat_map(|page| page.viewer.assigned_issues.nodes)
        {
            let history = spans(node.state_history.nodes);
            found.add(node.issue);
            if let Some(issue) = found.issues.last_mut() {
                issue.state_history = history;
            }
        }
    }

    let mut teams = Vec::new();
    let mut cycles = Vec::new();
    for node in team_pages
        .into_iter()
        .flat_map(|page| page.viewer.teams.nodes)
    {
        if node.cycles.nodes.len() >= CYCLES_PER_TEAM {
            problems.push(format!(
                "{} has more than {CYCLES_PER_TEAM} cycles in range; only the first \
                 {CYCLES_PER_TEAM} loaded",
                node.team.key
            ));
        }
        cycles.extend(
            node.cycles
                .nodes
                .into_iter()
                .map(|cycle| cycle.into_cycle(&node.team.id)),
        );
        teams.push(Team {
            id: node.team.id,
            key: node.team.key,
            name: node.team.name,
            estimation: node.team.issue_estimation_type,
        });
    }

    let issues = one_copy_each(found.issues);
    let labels = lookup_labels(client, &issues, &mut problems).await;
    let projects = lookup_projects(client, &issues, &mut problems).await;
    Ok(AnalyticsIssues {
        viewer,
        issues,
        teams,
        cycles,
        labels,
        projects,
        members: found.members,
        as_of,
        problems,
    })
}

/// Keeps the errors Linear sent alongside data, so the viewer learns that
/// something is missing.
pub(super) fn note(problems: &mut Vec<String>, errors: &[GraphQlError]) {
    problems.extend(
        errors
            .iter()
            .map(|error| format!("Linear left something out: {error}")),
    );
}

/// Created or completed in `window`.
fn issue_filter(window: AnalyticsWindow) -> Value {
    let mut range = json!({ "gte": window.since.to_string() });
    if let Some(until) = window.until {
        range["lt"] = Value::String(until.to_string());
    }
    json!({ "or": [{ "createdAt": range }, { "completedAt": range }] })
}

/// Overlapping `window`, and started by now when the window reaches now.
fn cycle_filter(window: AnalyticsWindow) -> Value {
    let mut filter = json!({ "endsAt": { "gt": window.since.to_string() } });
    match window.until {
        Some(until) => filter["startsAt"] = json!({ "lt": until.to_string() }),
        None => filter["isFuture"] = json!({ "eq": false }),
    }
    filter
}

/// An issue that came back twice (created in the window and in progress)
/// keeps the copy that carries its state history.
pub(super) fn one_copy_each(issues: Vec<Issue>) -> Vec<Issue> {
    let mut kept: Vec<Issue> = Vec::with_capacity(issues.len());
    let mut index: HashMap<String, usize> = HashMap::new();
    for issue in issues {
        match index.get(&issue.id) {
            Some(&at) if !issue.state_history.is_empty() => kept[at] = issue,
            Some(_) => {}
            None => {
                index.insert(issue.id.clone(), kept.len());
                kept.push(issue);
            }
        }
    }
    kept
}

/// The spans oldest first, leaving out any whose workflow state Linear did
/// not return.
pub(super) fn spans(nodes: Vec<SpanNode>) -> Vec<StateSpan> {
    let mut spans: Vec<StateSpan> = nodes
        .into_iter()
        .filter_map(|node| {
            Some(StateSpan {
                state_id: node.state_id,
                state_type: node.state?.state_type,
                started_at: node.started_at,
                ended_at: node.ended_at,
            })
        })
        .collect();
    spans.sort_by_key(|span| span.started_at);
    spans
}

/// The labels `issues` carry. A failed lookup becomes a problem: the
/// analytics still show, with label ids for names.
pub(super) async fn lookup_labels(
    client: &HttpClient,
    issues: &[Issue],
    problems: &mut Vec<String>,
) -> Vec<Label> {
    let mut ids: Vec<String> = Vec::new();
    for id in issues.iter().flat_map(|issue| &issue.label_ids) {
        push_new(&mut ids, id.clone(), |known| known == id);
    }
    if ids.is_empty() {
        return Vec::new();
    }
    let result = client
        .all_pages(
            LABELS,
            json!({ "ids": ids }),
            Urgency::Background,
            |page: &LabelsPage| &page.issue_labels.page_info,
        )
        .await;
    match result {
        Ok(Response { data, errors }) => {
            problems.extend(
                errors
                    .iter()
                    .map(|e| format!("Linear left something out: {e}")),
            );
            data.into_iter()
                .flat_map(|page| page.issue_labels.nodes)
                .map(|node| Label {
                    id: node.id,
                    team_id: node.team.map(|team| team.id),
                    name: node.name,
                })
                .collect()
        }
        Err(error) => {
            tracing::warn!(%error, "the label lookup failed");
            problems.push(format!("label names did not load: {error}"));
            Vec::new()
        }
    }
}

/// The projects `issues` belong to, as [`lookup_labels`] does for labels.
pub(super) async fn lookup_projects(
    client: &HttpClient,
    issues: &[Issue],
    problems: &mut Vec<String>,
) -> Vec<Project> {
    let mut ids: Vec<String> = Vec::new();
    for id in issues.iter().filter_map(|issue| issue.project_id.as_ref()) {
        push_new(&mut ids, id.clone(), |known| known == id);
    }
    if ids.is_empty() {
        return Vec::new();
    }
    let result = client
        .all_pages(
            PROJECTS,
            json!({ "ids": ids }),
            Urgency::Background,
            |page: &ProjectsPage| &page.projects.page_info,
        )
        .await;
    match result {
        Ok(Response { data, errors }) => {
            problems.extend(
                errors
                    .iter()
                    .map(|e| format!("Linear left something out: {e}")),
            );
            data.into_iter()
                .flat_map(|page| page.projects.nodes)
                .map(|node| Project {
                    id: node.id,
                    name: node.name,
                })
                .collect()
        }
        Err(error) => {
            tracing::warn!(%error, "the project lookup failed");
            problems.push(format!("project names did not load: {error}"));
            Vec::new()
        }
    }
}

#[derive(Deserialize)]
struct TeamsPage {
    viewer: ViewerWithTeams,
}

#[derive(Deserialize)]
struct ViewerWithTeams {
    id: String,
    name: String,
    teams: Connection<TeamWithCycles>,
}

#[derive(Deserialize)]
struct TeamWithCycles {
    #[serde(flatten)]
    team: TeamNode,
    cycles: Nodes<CycleNode>,
}

#[derive(Deserialize)]
pub(super) struct Nodes<N> {
    pub(super) nodes: Vec<N>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct CycleNode {
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
    scope_history: Vec<f64>,
    completed_scope_history: Vec<f64>,
    #[serde(deserialize_with = "whole_numbers")]
    issue_count_history: Vec<u32>,
    #[serde(deserialize_with = "whole_numbers")]
    completed_issue_count_history: Vec<u32>,
}

impl CycleNode {
    pub(super) fn into_cycle(self, team_id: &str) -> Cycle {
        Cycle {
            id: self.id,
            team_id: team_id.to_owned(),
            number: self.number,
            name: self.name,
            starts_at: self.starts_at,
            ends_at: self.ends_at,
            completed_at: self.completed_at,
            scope_history: self.scope_history,
            completed_scope_history: self.completed_scope_history,
            issue_count_history: self.issue_count_history,
            completed_issue_count_history: self.completed_issue_count_history,
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct IssuesPage<N> {
    viewer: ViewerWithIssues<N>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ViewerWithIssues<N> {
    assigned_issues: Connection<N>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct InProgressNode {
    #[serde(flatten)]
    issue: IssueNode,
    state_history: Nodes<SpanNode>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct SpanNode {
    state_id: String,
    #[serde(deserialize_with = "timestamp")]
    started_at: Timestamp,
    #[serde(deserialize_with = "optional_timestamp")]
    ended_at: Option<Timestamp>,
    state: Option<SpanState>,
}

#[derive(Deserialize)]
struct SpanState {
    #[serde(rename = "type", deserialize_with = "state_type")]
    state_type: StateType,
}

#[derive(Deserialize)]
struct ProjectsPage {
    projects: Connection<ProjectNode>,
}

#[derive(Deserialize)]
struct ProjectNode {
    id: String,
    name: String,
}

/// Linear sends the counts in a cycle's history as floats.
fn whole_numbers<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<u32>, D::Error> {
    Vec::<f64>::deserialize(deserializer)?
        .into_iter()
        .map(|number| {
            if number.fract() == 0.0 && (0.0..=f64::from(u32::MAX)).contains(&number) {
                Ok(number as u32)
            } else {
                Err(serde::de::Error::custom(format!(
                    "expected a whole number, got {number}"
                )))
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 2026-08-17T00:00:00Z and 2026-10-12T00:00:00Z.
    const SINCE: Timestamp = Timestamp::constant(1_786_924_800, 0);
    const UNTIL: Timestamp = Timestamp::constant(1_791_763_200, 0);

    fn window(until: Option<Timestamp>) -> AnalyticsWindow {
        AnalyticsWindow {
            since: SINCE,
            until,
        }
    }

    #[test]
    fn a_window_up_to_now_filters_on_its_start_only() {
        assert_eq!(
            issue_filter(window(None)),
            json!({ "or": [
                { "createdAt": { "gte": "2026-08-17T00:00:00Z" } },
                { "completedAt": { "gte": "2026-08-17T00:00:00Z" } },
            ] })
        );
        assert_eq!(
            cycle_filter(window(None)),
            json!({ "endsAt": { "gt": "2026-08-17T00:00:00Z" }, "isFuture": { "eq": false } })
        );
    }

    #[test]
    fn an_older_window_ends_where_the_loaded_one_starts() {
        let until = Some(UNTIL);
        let range = json!({ "gte": "2026-08-17T00:00:00Z", "lt": "2026-10-12T00:00:00Z" });

        assert_eq!(
            issue_filter(window(until)),
            json!({ "or": [{ "createdAt": range }, { "completedAt": range }] })
        );
        assert_eq!(
            cycle_filter(window(until)),
            json!({
                "endsAt": { "gt": "2026-08-17T00:00:00Z" },
                "startsAt": { "lt": "2026-10-12T00:00:00Z" },
            })
        );
    }
}
