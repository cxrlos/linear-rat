//! The Cycles and Projects screens: a team's recent cycles, the viewer's
//! projects, and the issues of one cycle or project.

use serde::Deserialize;
use serde_json::json;

use super::analytics::CycleNode;
use super::assigned::{Connection, Decoded, IssueNode, PersonNode};
use super::{HttpClient, Urgency};
use crate::linear::model::{
    Cycle, Project, ProjectMilestone, ProjectOverview, ProjectStatus, ProjectStatusType,
};
use crate::linear::{ClientError, ProjectList, Response, SearchResults};

const RECENT_CYCLES: &str = include_str!("../graphql/recent_cycles.graphql");
const VIEWER_PROJECTS: &str = include_str!("../graphql/viewer_projects.graphql");
const CYCLE_ISSUES: &str = include_str!("../graphql/cycle_issues.graphql");
const PROJECT_ISSUES: &str = include_str!("../graphql/project_issues.graphql");

/// How many pages of a cycle's or project's issues to follow: 200 issues.
const ISSUE_PAGES: usize = 4;

pub(super) async fn recent_cycles(
    client: &HttpClient,
    team_id: &str,
) -> Result<Vec<Cycle>, ClientError> {
    let response: Response<CyclesData> = client
        .send(
            RECENT_CYCLES,
            &json!({ "teamId": team_id }),
            Urgency::Urgent,
        )
        .await?;
    warn(&response.errors, "the recent cycles query");
    Ok(response
        .data
        .team
        .cycles
        .nodes
        .into_iter()
        .map(|node| node.into_cycle(team_id))
        .collect())
}

pub(super) async fn projects(client: &HttpClient) -> Result<ProjectList, ClientError> {
    let response: Response<ProjectsData> = client
        .send(VIEWER_PROJECTS, &json!({}), Urgency::Urgent)
        .await?;
    warn(&response.errors, "the projects query");
    let connection = response.data.projects;
    let mut problems = Vec::new();
    if connection.page_info.has_next_page {
        problems.push("Showing the 50 most recently updated projects".to_owned());
    }
    let mut projects = Vec::new();
    for node in connection.nodes {
        if node.project_milestones.page_info.has_next_page {
            problems.push(format!("{} has more milestones than lrat shows", node.name));
        }
        projects.push(node.into_overview());
    }
    Ok(ProjectList { projects, problems })
}

pub(super) async fn cycle_issues(
    client: &HttpClient,
    cycle_id: &str,
) -> Result<SearchResults, ClientError> {
    let (response, more) = client
        .some_pages(
            CYCLE_ISSUES,
            json!({ "cycleId": cycle_id }),
            Urgency::Urgent,
            |page: &CycleIssuesData| &page.cycle.issues.page_info,
            ISSUE_PAGES,
        )
        .await?;
    let nodes = response
        .data
        .into_iter()
        .flat_map(|page| page.cycle.issues.nodes);
    Ok(listed(nodes, &response.errors, more))
}

pub(super) async fn project_issues(
    client: &HttpClient,
    project_id: &str,
) -> Result<SearchResults, ClientError> {
    let (response, more) = client
        .some_pages(
            PROJECT_ISSUES,
            json!({ "projectId": project_id }),
            Urgency::Urgent,
            |page: &ProjectIssuesData| &page.project.issues.page_info,
            ISSUE_PAGES,
        )
        .await?;
    let nodes = response
        .data
        .into_iter()
        .flat_map(|page| page.project.issues.nodes);
    Ok(listed(nodes, &response.errors, more))
}

/// The issues of `nodes` and what they refer to, with what Linear could not
/// give: errors that came with the data, and the pages past the last one
/// followed.
fn listed(
    nodes: impl Iterator<Item = IssueNode>,
    errors: &[crate::linear::GraphQlError],
    more: bool,
) -> SearchResults {
    let mut decoded = Decoded::default();
    for node in nodes {
        decoded.add(node);
    }
    let mut problems: Vec<String> = errors
        .iter()
        .map(|error| format!("Linear left something out: {error}"))
        .collect();
    if more {
        problems.push(format!(
            "Showing the first {} issues",
            ISSUE_PAGES * ISSUES_PER_PAGE
        ));
    }
    SearchResults {
        issues: decoded.issues,
        cycles: decoded.cycles,
        labels: Vec::new(),
        members: decoded.members,
        teams: decoded.teams,
        problems,
    }
}

/// The page size of the issue documents' `first:`.
const ISSUES_PER_PAGE: usize = 50;

fn warn(errors: &[crate::linear::GraphQlError], what: &str) {
    for error in errors {
        tracing::warn!(message = %error.message, "{what} also returned an error");
    }
}

#[derive(Deserialize)]
struct CyclesData {
    team: CyclesTeam,
}

#[derive(Deserialize)]
struct CyclesTeam {
    cycles: Bounded<CycleNode>,
}

#[derive(Deserialize)]
struct ProjectsData {
    projects: Bounded<ProjectNode>,
}

/// A connection read as one page.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Bounded<N> {
    nodes: Vec<N>,
    page_info: HasNextPage,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct HasNextPage {
    has_next_page: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ProjectNode {
    id: String,
    name: String,
    status: StatusNode,
    target_date: Option<jiff::civil::Date>,
    progress: f64,
    lead: Option<PersonNode>,
    project_milestones: Bounded<MilestoneNode>,
}

impl ProjectNode {
    fn into_overview(self) -> ProjectOverview {
        ProjectOverview {
            milestones_complete: !self.project_milestones.page_info.has_next_page,
            milestones: milestones(self.project_milestones.nodes, &self.id),
            project: Project {
                id: self.id,
                name: self.name,
            },
            status: ProjectStatus {
                name: self.status.name,
                status_type: self.status.status_type,
            },
            target_date: self.target_date,
            lead: self.lead.map(PersonNode::into_member),
            progress: self.progress,
        }
    }
}

#[derive(Deserialize)]
struct StatusNode {
    name: String,
    #[serde(rename = "type")]
    status_type: ProjectStatusType,
}

/// A project milestone as the Projects screen and the milestone picker ask
/// for it.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct MilestoneNode {
    id: String,
    name: String,
    sort_order: f64,
}

/// The milestones of project `project_id` in the project's own order.
pub(super) fn milestones(mut nodes: Vec<MilestoneNode>, project_id: &str) -> Vec<ProjectMilestone> {
    nodes.sort_by(|a, b| a.sort_order.total_cmp(&b.sort_order));
    nodes
        .into_iter()
        .map(|node| ProjectMilestone {
            id: node.id,
            name: node.name,
            project_id: project_id.to_owned(),
        })
        .collect()
}

#[derive(Deserialize)]
struct CycleIssuesData {
    cycle: WithIssues,
}

#[derive(Deserialize)]
struct ProjectIssuesData {
    project: WithIssues,
}

#[derive(Deserialize)]
struct WithIssues {
    issues: Connection<IssueNode>,
}
