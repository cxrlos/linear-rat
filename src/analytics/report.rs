use super::{
    AdministrableTeams, BurnUp, CycleVelocity, DayCount, IssueAge, PerPersonBreakdown, Percentiles,
    Period, Splits, StaleIssue, StaleThreshold, TriageHealth, Unit, WeekFlow, active_cycle,
    burn_up, completion_days, cycle_time, issue_ages, lead_time, open_issue_trend,
    per_person_breakdown, splits, stale_issues, triage_health, velocity, weekly_flow,
};
use crate::linear::model::{Cycle, Issue, Team, Viewer};

/// Everything the Personal view of the Analytics screen shows.
#[derive(Debug, Clone, PartialEq)]
pub struct PersonalAnalytics {
    pub unit: Unit,
    pub weeks: Vec<WeekFlow>,
    pub completion_days: Vec<DayCount>,
    pub cycle_time: Option<Percentiles>,
    pub lead_time: Option<Percentiles>,
    pub in_progress: Vec<IssueAge>,
    pub splits: Splits,
}

/// Personal analytics over the issues assigned to `viewer`, so "created"
/// counts the viewer's issues created in the period. Scope is in points when
/// any of `teams` (the viewer's teams) uses estimates.
pub fn personal_analytics(
    viewer: &Viewer,
    teams: &[Team],
    issues: &[Issue],
    period: &Period,
) -> PersonalAnalytics {
    let unit = Unit::for_teams(teams);
    let viewer_issues: Vec<Issue> = issues
        .iter()
        .filter(|issue| issue.assignee_id.as_deref() == Some(viewer.id.as_str()))
        .cloned()
        .collect();
    PersonalAnalytics {
        unit,
        weeks: weekly_flow(&viewer_issues, unit, period),
        completion_days: completion_days(&viewer_issues, period),
        cycle_time: cycle_time(&viewer_issues, period),
        lead_time: lead_time(&viewer_issues, period),
        in_progress: issue_ages(&viewer_issues, period.now()),
        splits: splits(&viewer_issues, unit, period),
    }
}

/// Everything the Team view of the Analytics screen shows.
#[derive(Debug, Clone, PartialEq)]
pub struct TeamAnalytics {
    pub unit: Unit,
    pub weeks: Vec<WeekFlow>,
    /// Issues open at the end of each week, aligned with `weeks`.
    pub open_trend: Vec<u32>,
    pub cycle_time: Option<Percentiles>,
    pub lead_time: Option<Percentiles>,
    pub in_progress: Vec<IssueAge>,
    pub splits: Splits,
    /// The burn-up of the active cycle, if one is running.
    pub burn_up: Option<BurnUp>,
    pub velocity: Vec<CycleVelocity>,
    pub triage: TriageHealth,
    pub stale: Vec<StaleIssue>,
    /// Present only for a team the viewer administers.
    pub per_person: Option<PerPersonBreakdown>,
}

/// Team analytics over the issues and cycles of `team`; anything belonging to
/// another team is ignored.
pub fn team_analytics(
    team: &Team,
    administrable: &AdministrableTeams,
    issues: &[Issue],
    cycles: &[Cycle],
    stale_after: StaleThreshold,
    period: &Period,
) -> TeamAnalytics {
    let unit = Unit::for_team(team);
    let team_issues: Vec<Issue> = issues
        .iter()
        .filter(|issue| issue.team_id == team.id)
        .cloned()
        .collect();
    TeamAnalytics {
        unit,
        weeks: weekly_flow(&team_issues, unit, period),
        open_trend: open_issue_trend(&team_issues, period),
        cycle_time: cycle_time(&team_issues, period),
        lead_time: lead_time(&team_issues, period),
        in_progress: issue_ages(&team_issues, period.now()),
        splits: splits(&team_issues, unit, period),
        burn_up: active_cycle(cycles, &team.id, period.now())
            .map(|cycle| burn_up(cycle, unit, period.time_zone())),
        velocity: velocity(cycles, &team.id, unit, period.now()),
        triage: triage_health(&team_issues, period),
        stale: stale_issues(&team_issues, period.now(), stale_after),
        per_person: per_person_breakdown(team, administrable, &team_issues, period),
    }
}
