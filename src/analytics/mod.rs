//! Pure analytics: turns issues, cycles and team settings into the metrics the
//! Analytics screen renders. No I/O and no clock; the caller passes in "now",
//! the time zone and the range.

mod cycles;
mod flow;
mod health;
mod lifecycle;
mod people;
mod percentile;
mod period;
mod report;
mod split;
mod timing;
mod unit;

pub use cycles::{
    BurnUp, BurnUpDay, CycleVelocity, VELOCITY_CYCLES, active_cycle, burn_up, velocity,
};
pub use flow::{DayCount, WeekFlow, completion_days, open_issue_trend, weekly_flow};
pub use health::{StaleIssue, StaleThreshold, TriageHealth, stale_issues, triage_health};
pub use people::{AdministrableTeams, PerPersonBreakdown, PersonBreakdown, per_person_breakdown};
pub use percentile::Percentiles;
pub use period::{Period, WeekRange};
pub use report::{PersonalAnalytics, TeamAnalytics, personal_analytics, team_analytics};
pub use split::{Share, Splits, splits};
pub use timing::{IssueAge, cycle_time, issue_ages, lead_time};
pub use unit::Unit;
