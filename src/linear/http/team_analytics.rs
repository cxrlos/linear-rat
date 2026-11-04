//! Loading what team analytics needs for one team over a window of time, and
//! the teams the viewer can administer.

use jiff::{SignedDuration, Timestamp};
use serde::Deserialize;
use serde_json::{Value, json};

use super::analytics::{
    CycleNode, Nodes, SpanNode, lookup_labels, lookup_projects, note, one_copy_each, spans,
};
use super::assigned::{Connection, Decoded, IdNode, IssueNode, TeamNode};
use super::{HttpClient, Urgency};
use crate::analytics::VELOCITY_CYCLES;
use crate::linear::model::{Cycle, Team};
use crate::linear::{AnalyticsWindow, ClientError, Response, TeamAnalyticsIssues};

const ADMINISTRABLE: &str = include_str!("../graphql/administrable_teams.graphql");
const SETTINGS: &str = include_str!("../graphql/team_settings.graphql");
const CYCLES: &str = include_str!("../graphql/team_cycles.graphql");
const RANGE: &str = include_str!("../graphql/team_range_issues.graphql");
const OPEN: &str = include_str!("../graphql/team_open_issues.graphql");
const STARTED: &str = include_str!("../graphql/team_started_issues.graphql");

/// The `first:` of `team_cycles.graphql`.
const CYCLES_LOADED: usize = 20;
/// How many pages of each issue query a load follows at most.
const MAX_PAGES: usize = 10;

pub(super) async fn administrable_teams(client: &HttpClient) -> Result<Vec<String>, ClientError> {
    let Response { data, errors } = client
        .all_pages(
            ADMINISTRABLE,
            json!({}),
            Urgency::Urgent,
            |page: &AdministrablePage| &page.administrable_teams.page_info,
        )
        .await?;
    for error in &errors {
        tracing::warn!(message = %error.message, "the administrable teams query also returned an error");
    }
    Ok(data
        .into_iter()
        .flat_map(|page| page.administrable_teams.nodes)
        .map(|node| node.id)
        .collect())
}

pub(super) async fn load(
    client: &HttpClient,
    team_id: &str,
    window: AnalyticsWindow,
) -> Result<TeamAnalyticsIssues, ClientError> {
    let mut problems = Vec::new();
    let settings: Response<SettingsData> = client
        .send(SETTINGS, &json!({ "teamId": team_id }), Urgency::Urgent)
        .await?;
    note(&mut problems, &settings.errors);
    let settings = settings.data.team;
    let team = Team {
        id: settings.team.id.clone(),
        key: settings.team.key.clone(),
        name: settings.team.name.clone(),
        estimation: settings.team.issue_estimation_type,
    };

    let mut cycles = Vec::new();
    if window.reaches_now() {
        let filter = cycle_filter(&settings, client.timer.now());
        let response: Response<CyclesData> = client
            .send(
                CYCLES,
                &json!({ "teamId": team_id, "cycles": filter }),
                Urgency::Urgent,
            )
            .await?;
        note(&mut problems, &response.errors);
        let nodes = response.data.team.cycles.nodes;
        if nodes.len() >= CYCLES_LOADED {
            problems.push(format!(
                "{} has more than {CYCLES_LOADED} recent cycles; only the first {CYCLES_LOADED} loaded",
                team.key
            ));
        }
        cycles = latest(
            nodes
                .into_iter()
                .map(|cycle| cycle.into_cycle(team_id))
                .collect(),
        );
    }

    let mut found = Decoded::default();
    let range = json!({ "teamId": team_id, "filter": issue_filter(window) });
    let (Response { data, errors }, cut) = client
        .some_pages(RANGE, range, Urgency::Urgent, page_info, MAX_PAGES)
        .await?;
    note(&mut problems, &errors);
    note_cut(&mut problems, cut, &team, "issues in range");
    for node in data.into_iter().flat_map(|page| page.team.issues.nodes) {
        found.add(node);
    }

    if window.reaches_now() {
        let only_team = json!({ "teamId": team_id });
        let (Response { data, errors }, cut) = client
            .some_pages(
                OPEN,
                only_team.clone(),
                Urgency::Urgent,
                page_info,
                MAX_PAGES,
            )
            .await?;
        note(&mut problems, &errors);
        note_cut(&mut problems, cut, &team, "open issues");
        for node in data.into_iter().flat_map(|page| page.team.issues.nodes) {
            found.add(node);
        }

        let (Response { data, errors }, cut) = client
            .some_pages(
                STARTED,
                only_team,
                Urgency::Urgent,
                page_info::<StartedNode>,
                MAX_PAGES,
            )
            .await?;
        note(&mut problems, &errors);
        note_cut(&mut problems, cut, &team, "in-progress issues");
        for node in data.into_iter().flat_map(|page| page.team.issues.nodes) {
            let history = spans(node.state_history.nodes);
            found.add(node.issue);
            if let Some(issue) = found.issues.last_mut() {
                issue.state_history = history;
            }
        }
    }

    let as_of = client.timer.now();
    let issues = one_copy_each(found.issues);
    let labels = lookup_labels(client, &issues, &mut problems).await;
    let projects = lookup_projects(client, &issues, &mut problems).await;
    Ok(TeamAnalyticsIssues {
        team,
        issues,
        cycles,
        labels,
        projects,
        members: found.members,
        as_of,
        problems,
    })
}

fn page_info<N>(page: &TeamIssuesPage<N>) -> &super::PageInfo {
    &page.team.issues.page_info
}

fn note_cut(problems: &mut Vec<String>, cut: bool, team: &Team, what: &str) {
    if cut {
        problems.push(format!(
            "{} has too many {what} to load; team analytics count only the first {MAX_PAGES} pages",
            team.key
        ));
    }
}

/// Created, completed or canceled in `window`, so the open-issue trend also
/// sees issues closed in it.
fn issue_filter(window: AnalyticsWindow) -> Value {
    let mut range = json!({ "gte": window.since.to_string() });
    if let Some(until) = window.until {
        range["lt"] = Value::String(until.to_string());
    }
    json!({ "or": [
        { "createdAt": range },
        { "completedAt": range },
        { "canceledAt": range },
    ] })
}

/// Started by `now`, and ending after the start of the cycle that the last
/// [`VELOCITY_CYCLES`] cycles and the active one would reach back to at the
/// team's cycle length and cooldown.
fn cycle_filter(settings: &SettingsNode, now: Timestamp) -> Value {
    let weeks = (settings.cycle_duration + settings.cycle_cooldown_time).max(1.0);
    let cycles = (VELOCITY_CYCLES + 1) as f64;
    let seconds = (weeks * 7.0 * 86_400.0 * cycles).min(f64::from(u32::MAX));
    let since = now
        .checked_sub(SignedDuration::from_secs(seconds as i64))
        .unwrap_or(Timestamp::MIN);
    json!({
        "isFuture": { "eq": false },
        "endsAt": { "gt": since.to_string() },
    })
}

/// The active cycle and the [`VELOCITY_CYCLES`] before it, newest first.
fn latest(mut cycles: Vec<Cycle>) -> Vec<Cycle> {
    cycles.sort_by_key(|cycle| std::cmp::Reverse(cycle.number));
    cycles.truncate(VELOCITY_CYCLES + 1);
    cycles
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct AdministrablePage {
    administrable_teams: Connection<IdNode>,
}

#[derive(Deserialize)]
struct SettingsData {
    team: SettingsNode,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SettingsNode {
    #[serde(flatten)]
    team: TeamNode,
    cycle_duration: f64,
    cycle_cooldown_time: f64,
}

#[derive(Deserialize)]
struct CyclesData {
    team: TeamCycles,
}

#[derive(Deserialize)]
struct TeamCycles {
    cycles: Nodes<CycleNode>,
}

#[derive(Deserialize)]
struct TeamIssuesPage<N> {
    team: TeamIssues<N>,
}

#[derive(Deserialize)]
struct TeamIssues<N> {
    issues: Connection<N>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct StartedNode {
    #[serde(flatten)]
    issue: IssueNode,
    state_history: Nodes<SpanNode>,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 2026-08-17T00:00:00Z and 2026-10-12T00:00:00Z.
    const SINCE: Timestamp = Timestamp::constant(1_786_924_800, 0);
    const UNTIL: Timestamp = Timestamp::constant(1_791_763_200, 0);

    #[test]
    fn an_older_window_bounds_created_completed_and_canceled() {
        let range = json!({ "gte": "2026-08-17T00:00:00Z", "lt": "2026-10-12T00:00:00Z" });

        assert_eq!(
            issue_filter(AnalyticsWindow {
                since: SINCE,
                until: Some(UNTIL),
            }),
            json!({ "or": [
                { "createdAt": range },
                { "completedAt": range },
                { "canceledAt": range },
            ] })
        );
    }

    #[test]
    fn cycles_reach_back_seven_cycles_with_their_cooldown() {
        let settings = SettingsNode {
            team: TeamNode {
                id: "team-eng".to_owned(),
                key: "ENG".to_owned(),
                name: "Engineering".to_owned(),
                issue_estimation_type: crate::linear::model::Estimation::Fibonacci,
            },
            cycle_duration: 2.0,
            cycle_cooldown_time: 0.0,
        };

        // Seven two-week cycles back from 12 October 2026 is 98 days.
        assert_eq!(
            cycle_filter(&settings, UNTIL),
            json!({
                "isFuture": { "eq": false },
                "endsAt": { "gt": "2026-07-06T00:00:00Z" },
            })
        );
    }
}
