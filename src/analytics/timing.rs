use jiff::{SignedDuration, Timestamp};

use super::lifecycle::{completed_at, first_started_at, in_current_state_since};
use super::{Percentiles, Period};
use crate::linear::model::{Issue, StateType};

/// Cycle time of the issues completed in `period`: from first entering a
/// started state to completion, so a reopened issue keeps its first start.
/// Issues completed without ever starting have no cycle time.
pub fn cycle_time(issues: &[Issue], period: &Period) -> Option<Percentiles> {
    durations_to_completion(issues, period, first_started_at)
}

/// Lead time of the issues completed in `period`: from creation to completion.
pub fn lead_time(issues: &[Issue], period: &Period) -> Option<Percentiles> {
    durations_to_completion(issues, period, |issue| Some(issue.created_at))
}

fn durations_to_completion(
    issues: &[Issue],
    period: &Period,
    from: impl Fn(&Issue) -> Option<Timestamp>,
) -> Option<Percentiles> {
    let samples = issues
        .iter()
        .filter_map(|issue| {
            let done = completed_at(issue).filter(|&at| period.contains(at))?;
            elapsed(from(issue)?, done)
        })
        .collect();
    Percentiles::nearest_rank(samples)
}

/// How long an in-progress issue has been in its current started state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssueAge {
    pub issue_id: String,
    pub age: SignedDuration,
}

/// The issue age of every issue in a started state, oldest first.
pub fn issue_ages(issues: &[Issue], now: Timestamp) -> Vec<IssueAge> {
    let mut ages: Vec<IssueAge> = issues
        .iter()
        .filter(|issue| issue.state.state_type == StateType::Started)
        .filter_map(|issue| {
            Some(IssueAge {
                issue_id: issue.id.clone(),
                age: elapsed(in_current_state_since(issue)?, now)?,
            })
        })
        .collect();
    ages.sort_by(|a, b| b.age.cmp(&a.age).then_with(|| a.issue_id.cmp(&b.issue_id)));
    ages
}

/// The time from `start` to `end`, or `None` when the data has them reversed.
pub(crate) fn elapsed(start: Timestamp, end: Timestamp) -> Option<SignedDuration> {
    Some(end.duration_since(start)).filter(|d| !d.is_negative())
}
