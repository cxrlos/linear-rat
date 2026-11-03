use jiff::Timestamp;
use jiff::civil::Date;
use jiff::tz::TimeZone;

use super::Unit;
use crate::linear::model::Cycle;

/// How many finished cycles velocity looks back over.
pub const VELOCITY_CYCLES: usize = 6;

/// One day of a cycle's burn-up.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BurnUpDay {
    pub day: Date,
    pub scope: f64,
    pub completed: f64,
}

/// A cycle's scope and completed scope, day by day, from its history arrays.
#[derive(Debug, Clone, PartialEq)]
pub struct BurnUp {
    pub cycle_id: String,
    pub unit: Unit,
    /// From the cycle's first day to the last day Linear has history for.
    pub days: Vec<BurnUpDay>,
    /// Scope added after the first day: the sum of every day-over-day
    /// increase in scope. Scope removed on another day does not offset it.
    pub scope_creep: f64,
}

/// The burn-up of `cycle`, with days as calendar days in `time_zone`. Points
/// come from `scopeHistory` and `completedScopeHistory`, issue counts from
/// `issueCountHistory` and `completedIssueCountHistory`.
pub fn burn_up(cycle: &Cycle, unit: Unit, time_zone: &TimeZone) -> BurnUp {
    let (scope, completed) = histories(cycle, unit);
    let first_day = cycle.starts_at.to_zoned(time_zone.clone()).date();
    let days = std::iter::successors(Some(first_day), |day| day.tomorrow().ok())
        .zip(scope.iter().zip(&completed))
        .map(|(day, (&scope, &completed))| BurnUpDay {
            day,
            scope,
            completed,
        })
        .collect();
    let scope_creep = scope
        .windows(2)
        .map(|pair| (pair[1] - pair[0]).max(0.0))
        .sum();
    BurnUp {
        cycle_id: cycle.id.clone(),
        unit,
        days,
        scope_creep,
    }
}

/// The cycle of `team_id` running at `now`, if any.
pub fn active_cycle<'a>(cycles: &'a [Cycle], team_id: &str, now: Timestamp) -> Option<&'a Cycle> {
    cycles.iter().find(|cycle| {
        cycle.team_id == team_id
            && cycle.completed_at.is_none()
            && cycle.starts_at <= now
            && now < cycle.ends_at
    })
}

/// What one finished cycle delivered against what it committed to.
#[derive(Debug, Clone, PartialEq)]
pub struct CycleVelocity {
    pub cycle_id: String,
    pub number: u32,
    /// Scope completed by the end of the cycle: its velocity.
    pub completed: f64,
    /// Scope on the cycle's first day.
    pub committed: f64,
    /// `completed / committed`, or `None` when nothing was committed. The
    /// history arrays do not say which issues were completed, so scope added
    /// after day one and completed counts too, and the share can pass 1.
    pub committed_completed_share: Option<f64>,
}

/// Velocity of the last [`VELOCITY_CYCLES`] cycles of `team_id` that had
/// finished by `now`, oldest first.
pub fn velocity(cycles: &[Cycle], team_id: &str, unit: Unit, now: Timestamp) -> Vec<CycleVelocity> {
    let mut finished: Vec<&Cycle> = cycles
        .iter()
        .filter(|cycle| cycle.team_id == team_id)
        .filter(|cycle| cycle.completed_at.is_some() || cycle.ends_at <= now)
        .collect();
    finished.sort_by_key(|cycle| cycle.starts_at);
    let recent = &finished[finished.len().saturating_sub(VELOCITY_CYCLES)..];
    recent
        .iter()
        .map(|cycle| {
            let (scope, completed) = histories(cycle, unit);
            let committed = scope.first().copied().unwrap_or(0.0);
            let completed = completed.last().copied().unwrap_or(0.0);
            CycleVelocity {
                cycle_id: cycle.id.clone(),
                number: cycle.number,
                completed,
                committed,
                committed_completed_share: (committed > 0.0).then(|| completed / committed),
            }
        })
        .collect()
}

/// The daily scope and completed scope of `cycle` in `unit`.
fn histories(cycle: &Cycle, unit: Unit) -> (Vec<f64>, Vec<f64>) {
    match unit {
        Unit::Points => (
            cycle.scope_history.clone(),
            cycle.completed_scope_history.clone(),
        ),
        Unit::Issues => (
            cycle
                .issue_count_history
                .iter()
                .map(|&n| f64::from(n))
                .collect(),
            cycle
                .completed_issue_count_history
                .iter()
                .map(|&n| f64::from(n))
                .collect(),
        ),
    }
}
