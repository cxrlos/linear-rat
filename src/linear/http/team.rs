//! Team Issues: the viewer's teams, what a team's filters offer, and its
//! issues one page at a time.

use serde::Deserialize;
use serde_json::{Map, Value, json};

use super::assigned::{
    Connection, CycleNode, Decoded, IdNode, IssueNode, PersonNode, StateNode, TeamNode,
};
use super::{HttpClient, Urgency};
use crate::linear::model::{Cycle, Label, Member, Project, Team, WorkflowState};
use crate::linear::{
    ClientError, GraphQlError, Response, TeamFilter, TeamIssuesPage, TeamOptions, TeamQuery,
};

const TEAMS: &str = include_str!("../graphql/teams.graphql");
const TEAM_OPTIONS: &str = include_str!("../graphql/team_options.graphql");
const TEAM_ISSUES: &str = include_str!("../graphql/team_issues.graphql");
const TEAM_ISSUE_IDS: &str = include_str!("../graphql/team_issue_ids.graphql");

pub(super) async fn teams(client: &HttpClient) -> Result<Vec<Team>, ClientError> {
    let Response { data, errors } = client
        .all_pages(TEAMS, json!({}), Urgency::Urgent, |page: &TeamsPage| {
            &page.viewer.teams.page_info
        })
        .await?;
    warn(&errors, "the teams query");
    Ok(data
        .into_iter()
        .flat_map(|page| page.viewer.teams.nodes)
        .map(|node| Team {
            id: node.id,
            key: node.key,
            name: node.name,
            estimation: node.issue_estimation_type,
        })
        .collect())
}

pub(super) async fn options(
    client: &HttpClient,
    team_id: &str,
) -> Result<TeamOptions, ClientError> {
    let response: Response<OptionsData> = client
        .send(
            TEAM_OPTIONS,
            &json!({ "teamId": team_id }),
            Urgency::Background,
        )
        .await?;
    warn(&response.errors, "the team options query");
    let OptionsData { team, issue_labels } = response.data;
    let cut: Vec<&str> = [
        ("workflow states", team.states.page_info.has_next_page),
        ("members", team.members.page_info.has_next_page),
        ("labels", team.labels.page_info.has_next_page),
        ("cycles", team.cycles.page_info.has_next_page),
        ("projects", team.projects.page_info.has_next_page),
        ("workspace labels", issue_labels.page_info.has_next_page),
    ]
    .into_iter()
    .filter_map(|(name, more)| more.then_some(name))
    .collect();
    let lists = match cut.split_last() {
        None => None,
        Some((last, [])) => Some((*last).to_owned()),
        Some((last, rest)) => Some(format!("{} and {last}", rest.join(", "))),
    };
    let problems = lists
        .map(|lists| {
            format!("The filters may be missing some {lists}: Linear sent only the first page")
        })
        .into_iter()
        .collect();
    let team_label = |node: NamedNode| Label {
        id: node.id,
        team_id: Some(team.id.clone()),
        name: node.name,
    };
    let mut labels: Vec<Label> = team.labels.nodes.into_iter().map(team_label).collect();
    labels.extend(issue_labels.nodes.into_iter().map(|node| Label {
        id: node.id,
        team_id: None,
        name: node.name,
    }));
    Ok(TeamOptions {
        states: team
            .states
            .nodes
            .into_iter()
            .map(|node| WorkflowState {
                id: node.id,
                team_id: team.id.clone(),
                name: node.name,
                state_type: node.state_type,
                position: node.position,
            })
            .collect(),
        members: team
            .members
            .nodes
            .into_iter()
            .map(|node| Member {
                id: node.id,
                name: node.name,
                display_name: node.display_name,
            })
            .collect(),
        labels,
        cycles: team
            .cycles
            .nodes
            .into_iter()
            .map(|node| Cycle {
                id: node.id,
                team_id: team.id.clone(),
                number: node.number,
                name: node.name,
                starts_at: node.starts_at,
                ends_at: node.ends_at,
                completed_at: node.completed_at,
                scope_history: Vec::new(),
                completed_scope_history: Vec::new(),
                issue_count_history: Vec::new(),
                completed_issue_count_history: Vec::new(),
            })
            .collect(),
        projects: team
            .projects
            .nodes
            .into_iter()
            .map(|node| Project {
                id: node.id,
                name: node.name,
            })
            .collect(),
        team_id: team.id,
        problems,
    })
}

pub(super) async fn issues(
    client: &HttpClient,
    query: &TeamQuery,
    after: Option<&str>,
) -> Result<TeamIssuesPage, ClientError> {
    let variables = json!({
        "teamId": query.team_id,
        "filter": issue_filter(&query.filter),
        "after": after,
    });
    let response: Response<IssuesData<IssueNode>> = client
        .send(TEAM_ISSUES, &variables, Urgency::Urgent)
        .await?;
    let Connection { nodes, page_info } = response.data.team.issues;
    let mut page = TeamIssuesPage {
        issues: Vec::new(),
        cycles: Vec::new(),
        members: Vec::new(),
        next: None,
        as_of: client.timer.now(),
        problems: response
            .errors
            .iter()
            .map(|error| format!("Linear left something out: {error}"))
            .collect(),
    };
    // Each issue names its team, which the caller already has.
    let mut decoded = Decoded::default();
    for node in nodes {
        decoded.add(node);
    }
    page.issues = decoded.issues;
    page.cycles = decoded.cycles;
    page.members = decoded.members;
    page.next = match (page_info.has_next_page, page_info.end_cursor) {
        (false, _) => None,
        (true, Some(cursor)) if after != Some(cursor.as_str()) => Some(cursor),
        (true, _) => {
            return Err(ClientError::Decode(
                "Linear said there are more issues but gave no new cursor".to_owned(),
            ));
        }
    };
    Ok(page)
}

pub(super) async fn count(client: &HttpClient, query: &TeamQuery) -> Result<usize, ClientError> {
    let variables = json!({
        "teamId": query.team_id,
        "filter": issue_filter(&query.filter),
    });
    let Response { data, errors } = client
        .all_pages(
            TEAM_ISSUE_IDS,
            variables,
            Urgency::Background,
            |page: &IssuesData<IdNode>| &page.team.issues.page_info,
        )
        .await?;
    warn(&errors, "the team issue count");
    Ok(data.iter().map(|page| page.team.issues.nodes.len()).sum())
}

/// Linear's `IssueFilter` for `filter`, with only the fields that are set.
/// `null` when none is, which leaves the issues unfiltered.
fn issue_filter(filter: &TeamFilter) -> Value {
    let by_id = |id: &String| json!({ "id": { "eq": id } });
    let mut fields = Map::new();
    let set = [
        ("state", filter.state.as_ref().map(by_id)),
        ("assignee", filter.assignee.as_ref().map(by_id)),
        (
            "labels",
            filter
                .label
                .as_ref()
                .map(|id| json!({ "some": { "id": { "eq": id } } })),
        ),
        ("cycle", filter.cycle.as_ref().map(by_id)),
        ("project", filter.project.as_ref().map(by_id)),
    ];
    for (name, value) in set {
        if let Some(value) = value {
            fields.insert(name.to_owned(), value);
        }
    }
    if fields.is_empty() {
        Value::Null
    } else {
        Value::Object(fields)
    }
}

fn warn(errors: &[GraphQlError], what: &str) {
    for error in errors {
        tracing::warn!(message = %error.message, "{what} also returned an error");
    }
}

#[derive(Deserialize)]
struct TeamsPage {
    viewer: ViewerTeams,
}

#[derive(Deserialize)]
struct ViewerTeams {
    teams: Connection<TeamNode>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct OptionsData {
    team: OptionsTeam,
    issue_labels: Bounded<NamedNode>,
}

#[derive(Deserialize)]
struct OptionsTeam {
    id: String,
    states: Bounded<StateNode>,
    members: Bounded<PersonNode>,
    labels: Bounded<NamedNode>,
    cycles: Bounded<CycleNode>,
    projects: Bounded<NamedNode>,
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
struct NamedNode {
    id: String,
    name: String,
}

#[derive(Deserialize)]
struct IssuesData<N> {
    team: TeamIssues<N>,
}

#[derive(Deserialize)]
struct TeamIssues<N> {
    issues: Connection<N>,
}
