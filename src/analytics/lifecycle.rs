//! When an issue reached each stage, read from public fields only.

use jiff::Timestamp;

use crate::linear::model::{Issue, StateType};

/// When the issue was completed, if it is completed now. A reopened issue
/// (back in an open state, `completedAt` cleared) is not completed, and a
/// canceled issue, which is also how Linear marks duplicates, never is.
pub(crate) fn completed_at(issue: &Issue) -> Option<Timestamp> {
    match issue.state.state_type {
        StateType::Completed => issue.completed_at,
        _ => None,
    }
}

pub(crate) fn is_canceled(issue: &Issue) -> bool {
    issue.state.state_type == StateType::Canceled
}

/// When the issue first entered a started state: the earliest started span in
/// its history, or `startedAt` when the history was not loaded.
pub(crate) fn first_started_at(issue: &Issue) -> Option<Timestamp> {
    issue
        .state_history
        .iter()
        .filter(|span| span.state_type == StateType::Started)
        .map(|span| span.started_at)
        .min()
        .or(issue.started_at)
}

/// When an in-progress issue entered the started state it is in now: its open
/// span in the history, or `startedAt` when the history was not loaded.
pub(crate) fn in_current_state_since(issue: &Issue) -> Option<Timestamp> {
    issue
        .state_history
        .iter()
        .rev()
        .find(|span| span.ended_at.is_none() && span.state_type == StateType::Started)
        .map(|span| span.started_at)
        .or(issue.started_at)
}
