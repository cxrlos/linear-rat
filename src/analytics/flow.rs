use jiff::civil::Date;

use super::lifecycle::{completed_at, is_canceled};
use super::{Period, Unit};
use crate::linear::model::Issue;

/// One week of created and completed issues.
#[derive(Debug, Clone, PartialEq)]
pub struct WeekFlow {
    pub week_start: Date,
    /// Issues created this week, not counting canceled ones.
    pub created: u32,
    /// Issues completed this week: the week's throughput.
    pub completed: u32,
    /// Scope completed this week, in the unit passed to [`weekly_flow`].
    pub completed_scope: f64,
}

impl WeekFlow {
    /// Created minus completed; positive means the backlog is growing.
    pub fn net_flow(&self) -> i64 {
        i64::from(self.created) - i64::from(self.completed)
    }
}

/// Created and completed issues for every week of `period`, oldest first.
/// Canceled issues count as neither.
pub fn weekly_flow(issues: &[Issue], unit: Unit, period: &Period) -> Vec<WeekFlow> {
    let mut weeks: Vec<WeekFlow> = period
        .week_starts()
        .iter()
        .map(|&week_start| WeekFlow {
            week_start,
            created: 0,
            completed: 0,
            completed_scope: 0.0,
        })
        .collect();
    for issue in issues.iter().filter(|i| !is_canceled(i)) {
        if let Some(week) = period.week_of(issue.created_at) {
            weeks[week].created += 1;
        }
        if let Some(week) = completed_at(issue).and_then(|at| period.week_of(at)) {
            weeks[week].completed += 1;
            weeks[week].completed_scope += unit.scope_of(issue);
        }
    }
    weeks
}

/// Issues completed on one calendar day, a cell of the completion heatmap.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DayCount {
    pub day: Date,
    pub completed: u32,
}

/// Issues completed per day, from the first Monday of `period` up to and
/// including today. Canceled issues do not count.
pub fn completion_days(issues: &[Issue], period: &Period) -> Vec<DayCount> {
    let today = period.day_of(period.now());
    let Some(&first) = period.week_starts().first() else {
        return Vec::new();
    };
    let mut days: Vec<DayCount> = std::iter::successors(Some(first), |day| {
        day.tomorrow().ok().filter(|next| next <= &today)
    })
    .map(|day| DayCount { day, completed: 0 })
    .collect();
    for at in issues.iter().filter_map(completed_at) {
        let day = period.day_of(at);
        if let Ok(index) = days.binary_search_by_key(&day, |d| d.day) {
            days[index].completed += 1;
        }
    }
    days
}
