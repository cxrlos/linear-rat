use jiff::{SignedDuration, Timestamp};

use super::timing::elapsed;
use super::{Percentiles, Period};
use crate::linear::model::{Issue, StateType};

/// Whether incoming work is being handled.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TriageHealth {
    /// Ids of the issues in a triage state now, longest waiting first.
    pub waiting: Vec<String>,
    /// Median triage time of the issues that left triage in the period.
    pub median_triage_time: Option<SignedDuration>,
}

/// Triage health: what waits in triage now, and how long triage took for the
/// issues that left it in `period`. Leaving triage counts however it
/// happened, declining (canceling) included.
pub fn triage_health(issues: &[Issue], period: &Period) -> TriageHealth {
    let mut waiting: Vec<&Issue> = issues
        .iter()
        .filter(|issue| issue.state.state_type == StateType::Triage)
        .collect();
    waiting.sort_by(|a, b| {
        a.created_at
            .cmp(&b.created_at)
            .then_with(|| a.id.cmp(&b.id))
    });
    let triage_times = issues
        .iter()
        .filter_map(|issue| {
            let triaged = issue.triaged_at.filter(|&at| period.contains(at))?;
            elapsed(issue.created_at, triaged)
        })
        .collect();
    TriageHealth {
        waiting: waiting.into_iter().map(|issue| issue.id.clone()).collect(),
        median_triage_time: Percentiles::nearest_rank(triage_times).map(|p| p.p50),
    }
}

/// How many days without an update make an open issue stale.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StaleThreshold(u32);

impl StaleThreshold {
    /// Stale after `days` days without an update.
    pub const fn days(days: u32) -> Self {
        Self(days)
    }
}

impl Default for StaleThreshold {
    /// 14 days, the configuration default.
    fn default() -> Self {
        Self(14)
    }
}

/// An open issue with no update for at least the stale threshold.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StaleIssue {
    pub issue_id: String,
    pub since_update: SignedDuration,
}

/// Every open issue last updated at least `threshold` days (of 24 hours)
/// before `now`, longest untouched first.
pub fn stale_issues(
    issues: &[Issue],
    now: Timestamp,
    threshold: StaleThreshold,
) -> Vec<StaleIssue> {
    let limit = SignedDuration::from_hours(24 * i64::from(threshold.0));
    let mut stale: Vec<StaleIssue> = issues
        .iter()
        .filter(|issue| issue.state.state_type.is_open())
        .filter_map(|issue| {
            let since_update = elapsed(issue.updated_at, now).filter(|&d| d >= limit)?;
            Some(StaleIssue {
                issue_id: issue.id.clone(),
                since_update,
            })
        })
        .collect();
    stale.sort_by(|a, b| {
        b.since_update
            .cmp(&a.since_update)
            .then_with(|| a.issue_id.cmp(&b.issue_id))
    });
    stale
}
