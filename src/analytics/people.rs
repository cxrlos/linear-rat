use std::collections::{BTreeMap, BTreeSet};

use super::lifecycle::{completed_at, is_canceled};
use super::{Period, Unit};
use crate::linear::model::{Issue, StateType, Team};

/// The teams the viewer can administer, from `Query.administrableTeams`. The
/// per-person breakdown is only produced for these (ADR 0001).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AdministrableTeams(BTreeSet<String>);

impl AdministrableTeams {
    /// The ids of the teams in `Query.administrableTeams`.
    pub fn new(team_ids: impl IntoIterator<Item = String>) -> Self {
        Self(team_ids.into_iter().collect())
    }

    /// Whether the viewer is a team owner of `team_id`.
    pub fn includes(&self, team_id: &str) -> bool {
        self.0.contains(team_id)
    }
}

/// One assignee's share of a team's work.
#[derive(Debug, Clone, PartialEq)]
pub struct PersonBreakdown {
    /// The assignee's member id; `None` for unassigned issues.
    pub assignee_id: Option<String>,
    /// Issues completed in the period.
    pub completed: u32,
    /// Issues in a started state now.
    pub in_progress: u32,
    /// Issues created in the period, not counting canceled ones.
    pub created: u32,
    /// Scope of the open issues, in [`PerPersonBreakdown::unit`].
    pub open_scope: f64,
}

/// Team analytics split by assignee. Assignees are in id order and unassigned
/// work comes last, so the order says nothing about how much anyone did.
#[derive(Debug, Clone, PartialEq)]
pub struct PerPersonBreakdown {
    pub unit: Unit,
    pub people: Vec<PersonBreakdown>,
}

/// The per-person breakdown of `team`, or `None` unless the viewer
/// administers it. Only issues of `team` are counted, each under its
/// assignee.
pub fn per_person_breakdown(
    team: &Team,
    administrable: &AdministrableTeams,
    issues: &[Issue],
    period: &Period,
) -> Option<PerPersonBreakdown> {
    if !administrable.includes(&team.id) {
        return None;
    }
    let unit = Unit::for_team(team);
    let mut people: BTreeMap<Option<&str>, PersonBreakdown> = BTreeMap::new();
    let team_issues = issues
        .iter()
        .filter(|issue| issue.team_id == team.id && !is_canceled(issue));
    for issue in team_issues {
        let assignee = issue.assignee_id.as_deref();
        let person = people.entry(assignee).or_insert_with(|| PersonBreakdown {
            assignee_id: assignee.map(String::from),
            completed: 0,
            in_progress: 0,
            created: 0,
            open_scope: 0.0,
        });
        if completed_at(issue).is_some_and(|at| period.contains(at)) {
            person.completed += 1;
        }
        if issue.state.state_type == StateType::Started {
            person.in_progress += 1;
        }
        if period.contains(issue.created_at) {
            person.created += 1;
        }
        if issue.state.state_type.is_open() {
            person.open_scope += unit.scope_of(issue);
        }
    }
    let (assigned, unassigned): (Vec<_>, Vec<_>) = people
        .into_values()
        .partition(|person| person.assignee_id.is_some());
    Some(PerPersonBreakdown {
        unit,
        people: assigned.into_iter().chain(unassigned).collect(),
    })
}
