use std::collections::{HashMap, HashSet};

use jiff::Timestamp;

use crate::linear::model::{Cycle, Estimation, Issue, Label, Member, Team};

/// What issue rows and details look up by id: cycle, label and member
/// names, which cycles are running, and which teams use estimates.
#[derive(Debug, Default)]
pub struct Names {
    cycle_names: HashMap<String, String>,
    active_cycles: HashSet<String>,
    label_names: HashMap<String, String>,
    member_names: HashMap<String, String>,
    teams_without_estimates: HashSet<String>,
}

impl Names {
    /// The names in these lists. A cycle is active when `now` falls inside
    /// it.
    pub fn new<'a>(
        cycles: impl IntoIterator<Item = &'a Cycle>,
        labels: impl IntoIterator<Item = &'a Label>,
        members: impl IntoIterator<Item = &'a Member>,
        teams: impl IntoIterator<Item = &'a Team>,
        now: Timestamp,
    ) -> Self {
        let mut names = Self::default();
        for cycle in cycles {
            if cycle.starts_at <= now && now < cycle.ends_at {
                names.active_cycles.insert(cycle.id.clone());
            }
            let name = cycle
                .name
                .clone()
                .unwrap_or_else(|| format!("Cycle {}", cycle.number));
            names.cycle_names.insert(cycle.id.clone(), name);
        }
        names.label_names = labels
            .into_iter()
            .map(|label| (label.id.clone(), label.name.clone()))
            .collect();
        names.member_names = members
            .into_iter()
            .map(|member| (member.id.clone(), member.name.clone()))
            .collect();
        names.teams_without_estimates = teams
            .into_iter()
            .filter(|team| team.estimation == Estimation::NotUsed)
            .map(|team| team.id.clone())
            .collect();
        names
    }

    /// The cycle's name, or `Cycle <number>` when it has none.
    pub fn cycle_name(&self, cycle_id: &str) -> Option<&str> {
        self.cycle_names.get(cycle_id).map(String::as_str)
    }

    /// Whether `issue` belongs to a cycle running now.
    pub fn is_in_active_cycle(&self, issue: &Issue) -> bool {
        issue
            .cycle_id
            .as_ref()
            .is_some_and(|id| self.active_cycles.contains(id))
    }

    /// Whether `issue`'s team uses estimates. Estimates are hidden when not.
    pub fn uses_estimates(&self, issue: &Issue) -> bool {
        !self.teams_without_estimates.contains(&issue.team_id)
    }

    /// The name of a label, by id.
    pub fn label_name(&self, label_id: &str) -> Option<&str> {
        self.label_names.get(label_id).map(String::as_str)
    }

    /// The full name of a workspace member, by id.
    pub fn member_name(&self, member_id: &str) -> Option<&str> {
        self.member_names.get(member_id).map(String::as_str)
    }
}
