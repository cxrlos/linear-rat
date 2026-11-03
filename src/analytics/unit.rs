use crate::linear::model::{Estimation, Issue, Team};

/// What scope is measured in. Teams that do not use estimates are measured in
/// issue counts, so the numbers still mean something.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unit {
    /// Estimate points; an issue without an estimate counts as zero.
    Points,
    /// Issues; every issue counts as one.
    Issues,
}

impl Unit {
    /// Points when `team` uses estimates, issues otherwise.
    pub fn for_team(team: &Team) -> Self {
        Self::for_teams(std::slice::from_ref(team))
    }

    /// Points when any of `teams` uses estimates, issues otherwise.
    pub fn for_teams(teams: &[Team]) -> Self {
        if teams.iter().any(|t| t.estimation != Estimation::NotUsed) {
            Unit::Points
        } else {
            Unit::Issues
        }
    }

    /// How much `issue` weighs in this unit.
    pub(crate) fn scope_of(self, issue: &Issue) -> f64 {
        match self {
            Unit::Points => issue.estimate.unwrap_or(0.0),
            Unit::Issues => 1.0,
        }
    }
}
