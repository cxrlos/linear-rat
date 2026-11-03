use jiff::civil::{Date, date};
use jiff::tz::TimeZone;
use jiff::{SignedDuration, Timestamp};
use linear_rat::analytics::*;
use linear_rat::linear::model::*;

// Wednesday 4 November 2026, so the current week starts on Monday 2 November.
const NOW: &str = "2026-11-04T12:00:00Z";

fn at(s: &str) -> Timestamp {
    s.parse()
        .unwrap_or_else(|e| panic!("{s} is not a timestamp: {e}"))
}

fn hours(h: i64) -> SignedDuration {
    SignedDuration::from_hours(h)
}

fn period(range: WeekRange) -> Period {
    Period::new(at(NOW), TimeZone::UTC, range)
}

fn four_weeks() -> Period {
    period(WeekRange::Four)
}

fn state(state_type: StateType) -> WorkflowState {
    WorkflowState {
        id: format!("{state_type:?}").to_lowercase(),
        team_id: "eng".into(),
        name: format!("{state_type:?}"),
        state_type,
        position: 0.0,
    }
}

fn span(state_type: StateType, from: &str, to: Option<&str>) -> StateSpan {
    StateSpan {
        state_id: format!("{state_type:?}").to_lowercase(),
        state_type,
        started_at: at(from),
        ended_at: to.map(at),
    }
}

/// An issue in ENG's backlog, created on 1 January 2026, built up step by step.
struct Build(Issue);

fn issue(id: &str) -> Build {
    let created = at("2026-01-01T09:00:00Z");
    Build(Issue {
        id: id.into(),
        identifier: format!("ENG-{id}"),
        title: format!("Issue {id}"),
        description: None,
        url: format!("https://linear.app/cheese/issue/ENG-{id}"),
        team_id: "eng".into(),
        state: state(StateType::Backlog),
        priority: Priority::None,
        estimate: None,
        assignee_id: None,
        creator_id: None,
        cycle_id: None,
        project_id: None,
        label_ids: Vec::new(),
        created_at: created,
        updated_at: created,
        started_at: None,
        completed_at: None,
        canceled_at: None,
        triaged_at: None,
        state_history: Vec::new(),
    })
}

impl Build {
    fn created(mut self, when: &str) -> Self {
        self.0.created_at = at(when);
        self.0.updated_at = at(when);
        self
    }

    fn started(mut self, when: &str) -> Self {
        self.0.state = state(StateType::Started);
        self.0.started_at = Some(at(when));
        self
    }

    fn completed(mut self, when: &str) -> Self {
        self.0.state = state(StateType::Completed);
        self.0.completed_at = Some(at(when));
        self
    }

    fn canceled(mut self, when: &str) -> Self {
        self.0.state = state(StateType::Canceled);
        self.0.canceled_at = Some(at(when));
        self
    }

    fn estimate(mut self, points: f64) -> Self {
        self.0.estimate = Some(points);
        self
    }

    fn project(mut self, project_id: &str) -> Self {
        self.0.project_id = Some(project_id.into());
        self
    }

    fn labels(mut self, label_ids: &[&str]) -> Self {
        self.0.label_ids = label_ids.iter().map(|&l| l.into()).collect();
        self
    }

    fn priority(mut self, priority: Priority) -> Self {
        self.0.priority = priority;
        self
    }

    fn updated(mut self, when: &str) -> Self {
        self.0.updated_at = at(when);
        self
    }

    fn triaged(mut self, when: &str) -> Self {
        self.0.triaged_at = Some(at(when));
        self
    }

    fn assignee(mut self, member_id: &str) -> Self {
        self.0.assignee_id = Some(member_id.into());
        self
    }

    fn in_state(mut self, state_type: StateType) -> Self {
        self.0.state = state(state_type);
        self
    }

    fn history(mut self, spans: Vec<StateSpan>) -> Self {
        self.0.state_history = spans;
        self
    }

    fn build(self) -> Issue {
        self.0
    }
}

fn week(week_start: Date, created: u32, completed: u32, completed_scope: f64) -> WeekFlow {
    WeekFlow {
        week_start,
        created,
        completed,
        completed_scope,
    }
}

#[test]
fn percentiles_of_no_samples_are_absent() {
    assert_eq!(Percentiles::nearest_rank(Vec::new()), None);
}

#[test]
fn percentiles_of_one_sample_are_that_sample() {
    assert_eq!(
        Percentiles::nearest_rank(vec![hours(5)]),
        Some(Percentiles {
            p50: hours(5),
            p85: hours(5),
            samples: 1
        })
    );
}

#[test]
fn percentiles_of_two_samples_split_low_and_high() {
    assert_eq!(
        Percentiles::nearest_rank(vec![hours(9), hours(1)]),
        Some(Percentiles {
            p50: hours(1),
            p85: hours(9),
            samples: 2
        })
    );
}

#[test]
fn percentiles_use_the_nearest_rank() {
    let samples = (1..=10).map(hours).collect();

    assert_eq!(
        Percentiles::nearest_rank(samples),
        Some(Percentiles {
            p50: hours(5),
            p85: hours(9),
            samples: 10
        })
    );
}

#[test]
fn weeks_start_on_monday_and_end_with_the_week_containing_now() {
    assert_eq!(
        four_weeks().week_starts(),
        [
            date(2026, 10, 12),
            date(2026, 10, 19),
            date(2026, 10, 26),
            date(2026, 11, 2)
        ]
    );
    assert_eq!(period(WeekRange::Twelve).week_starts().len(), 12);
    assert_eq!(
        period(WeekRange::TwentySix).week_starts().first(),
        Some(&date(2026, 5, 11))
    );
}

#[test]
fn weekly_flow_counts_created_and_completed_issues_per_week() {
    let issues = [
        issue("1")
            .created("2026-10-12T00:30:00Z")
            .completed("2026-10-21T10:00:00Z")
            .build(),
        issue("2").created("2026-10-13T08:00:00Z").build(),
        issue("3")
            .created("2026-10-25T23:59:00Z")
            .completed("2026-11-03T10:00:00Z")
            .build(),
        // Created before the range: only its completion counts.
        issue("4")
            .created("2026-09-01T10:00:00Z")
            .completed("2026-11-02T00:00:00Z")
            .build(),
    ];

    let flow = weekly_flow(&issues, Unit::Issues, &four_weeks());

    assert_eq!(
        flow,
        [
            week(date(2026, 10, 12), 2, 0, 0.0),
            week(date(2026, 10, 19), 1, 1, 1.0),
            week(date(2026, 10, 26), 0, 0, 0.0),
            week(date(2026, 11, 2), 0, 2, 2.0),
        ]
    );
    let net_flow: Vec<i64> = flow.iter().map(WeekFlow::net_flow).collect();
    assert_eq!(net_flow, [2, 0, 0, -2]);
}

#[test]
fn weekly_flow_buckets_by_the_given_time_zone() {
    // 03:00 UTC on Monday 2 November is still Sunday evening in New York.
    let new_york = TimeZone::posix("EST5EDT,M3.2.0,M11.1.0").unwrap();
    let period = Period::new(at(NOW), new_york, WeekRange::Four);
    let issues = [issue("1")
        .created("2026-11-02T03:00:00Z")
        .completed("2026-11-02T04:59:00Z")
        .build()];

    let flow = weekly_flow(&issues, Unit::Issues, &period);

    assert_eq!(flow[2], week(date(2026, 10, 26), 1, 1, 1.0));
    assert_eq!(flow[3], week(date(2026, 11, 2), 0, 0, 0.0));
}

#[test]
fn weekly_flow_skips_canceled_issues_and_duplicates() {
    let issues = [issue("1")
        .created("2026-10-20T10:00:00Z")
        .canceled("2026-10-21T10:00:00Z")
        .build()];

    let flow = weekly_flow(&issues, Unit::Issues, &four_weeks());

    assert!(flow.iter().all(|w| w.created == 0 && w.completed == 0));
}

#[test]
fn weekly_flow_does_not_count_a_reopened_issue_as_completed() {
    let issues = [issue("1")
        .created("2026-10-20T10:00:00Z")
        .in_state(StateType::Started)
        .history(vec![
            span(
                StateType::Started,
                "2026-10-20T10:00:00Z",
                Some("2026-10-21T10:00:00Z"),
            ),
            span(
                StateType::Completed,
                "2026-10-21T10:00:00Z",
                Some("2026-10-22T10:00:00Z"),
            ),
            span(StateType::Started, "2026-10-22T10:00:00Z", None),
        ])
        .build()];

    let flow = weekly_flow(&issues, Unit::Issues, &four_weeks());

    assert_eq!(flow[1], week(date(2026, 10, 19), 1, 0, 0.0));
}

fn team(estimation: Estimation) -> Team {
    Team {
        id: "eng".into(),
        key: "ENG".into(),
        name: "Engineering".into(),
        estimation,
    }
}

#[test]
fn teams_that_use_estimates_measure_scope_in_points() {
    assert_eq!(Unit::for_team(&team(Estimation::Fibonacci)), Unit::Points);
    assert_eq!(Unit::for_team(&team(Estimation::NotUsed)), Unit::Issues);
    assert_eq!(
        Unit::for_teams(&[team(Estimation::NotUsed), team(Estimation::TShirt)]),
        Unit::Points
    );
    assert_eq!(Unit::for_teams(&[]), Unit::Issues);
}

#[test]
fn weekly_flow_sums_estimate_points_completed_per_week() {
    let issues = [
        issue("1")
            .estimate(3.0)
            .completed("2026-10-27T10:00:00Z")
            .build(),
        issue("2")
            .estimate(5.0)
            .completed("2026-10-28T10:00:00Z")
            .build(),
        issue("3").completed("2026-10-29T10:00:00Z").build(),
    ];

    let flow = weekly_flow(&issues, Unit::Points, &four_weeks());

    assert_eq!(flow[2], week(date(2026, 10, 26), 0, 3, 8.0));
}

#[test]
fn completion_days_cover_the_range_up_to_today() {
    let issues = [
        issue("1").completed("2026-10-12T10:00:00Z").build(),
        issue("2").completed("2026-11-03T08:00:00Z").build(),
        issue("3").completed("2026-11-03T22:00:00Z").build(),
        issue("4").canceled("2026-11-03T22:00:00Z").build(),
        issue("5").completed("2026-10-11T23:00:00Z").build(),
    ];

    let days = completion_days(&issues, &four_weeks());

    assert_eq!(days.len(), 3 * 7 + 3);
    assert_eq!(
        days.first(),
        Some(&DayCount {
            day: date(2026, 10, 12),
            completed: 1
        })
    );
    assert_eq!(
        days[days.len() - 2],
        DayCount {
            day: date(2026, 11, 3),
            completed: 2
        }
    );
    assert_eq!(days.iter().map(|d| d.completed).sum::<u32>(), 3);
}

fn days(d: i64) -> SignedDuration {
    hours(24 * d)
}

#[test]
fn cycle_time_runs_from_first_start_to_completion() {
    let issues = [
        issue("1")
            .started("2026-10-20T10:00:00Z")
            .completed("2026-10-22T10:00:00Z")
            .build(),
        issue("2")
            .started("2026-10-26T10:00:00Z")
            .completed("2026-10-30T10:00:00Z")
            .build(),
        // Completed before the range.
        issue("3")
            .started("2026-09-01T10:00:00Z")
            .completed("2026-09-30T10:00:00Z")
            .build(),
        // Never started, so it has no cycle time.
        issue("4").completed("2026-10-30T10:00:00Z").build(),
        issue("5")
            .started("2026-10-20T10:00:00Z")
            .canceled("2026-10-30T10:00:00Z")
            .build(),
    ];

    assert_eq!(
        cycle_time(&issues, &four_weeks()),
        Some(Percentiles {
            p50: days(2),
            p85: days(4),
            samples: 2
        })
    );
}

#[test]
fn cycle_time_of_a_reopened_issue_starts_at_its_first_start() {
    let issues = [issue("1")
        .started("2026-10-26T10:00:00Z")
        .completed("2026-10-30T10:00:00Z")
        .history(vec![
            span(
                StateType::Started,
                "2026-10-16T10:00:00Z",
                Some("2026-10-20T10:00:00Z"),
            ),
            span(
                StateType::Completed,
                "2026-10-20T10:00:00Z",
                Some("2026-10-26T10:00:00Z"),
            ),
            span(
                StateType::Started,
                "2026-10-26T10:00:00Z",
                Some("2026-10-30T10:00:00Z"),
            ),
            span(StateType::Completed, "2026-10-30T10:00:00Z", None),
        ])
        .build()];

    assert_eq!(
        cycle_time(&issues, &four_weeks()).map(|p| p.p50),
        Some(days(14))
    );
}

#[test]
fn cycle_time_without_completed_issues_is_absent() {
    let issues = [issue("1").started("2026-10-26T10:00:00Z").build()];

    assert_eq!(cycle_time(&issues, &four_weeks()), None);
}

#[test]
fn lead_time_runs_from_creation_to_completion() {
    let issues = [
        issue("1")
            .created("2026-10-20T10:00:00Z")
            .completed("2026-10-21T10:00:00Z")
            .build(),
        issue("2")
            .created("2026-10-01T10:00:00Z")
            .started("2026-10-25T10:00:00Z")
            .completed("2026-10-31T10:00:00Z")
            .build(),
        issue("3")
            .created("2026-10-01T10:00:00Z")
            .canceled("2026-10-31T10:00:00Z")
            .build(),
    ];

    assert_eq!(
        lead_time(&issues, &four_weeks()),
        Some(Percentiles {
            p50: days(1),
            p85: days(30),
            samples: 2
        })
    );
}

#[test]
fn issue_age_counts_from_entering_the_current_started_state() {
    let issues = [
        issue("1").started("2026-11-01T12:00:00Z").build(),
        // In Review since 2 November, after a day In Progress.
        issue("2")
            .started("2026-10-01T12:00:00Z")
            .history(vec![
                span(
                    StateType::Backlog,
                    "2026-09-01T12:00:00Z",
                    Some("2026-10-01T12:00:00Z"),
                ),
                span(
                    StateType::Started,
                    "2026-10-01T12:00:00Z",
                    Some("2026-11-02T12:00:00Z"),
                ),
                span(StateType::Started, "2026-11-02T12:00:00Z", None),
            ])
            .build(),
        issue("3").build(),
        issue("4").completed("2026-11-01T12:00:00Z").build(),
    ];

    assert_eq!(
        issue_ages(&issues, at(NOW)),
        [
            IssueAge {
                issue_id: "1".into(),
                age: days(3)
            },
            IssueAge {
                issue_id: "2".into(),
                age: days(2)
            },
        ]
    );
}

fn share<K>(key: K, issues: u32, scope: f64) -> Share<K> {
    Share { key, issues, scope }
}

#[test]
fn splits_divide_completed_work_by_project_label_and_priority() {
    let issues = [
        issue("1")
            .project("cheese")
            .labels(&["bug", "ui"])
            .priority(Priority::High)
            .estimate(3.0)
            .completed("2026-10-20T10:00:00Z")
            .build(),
        issue("2")
            .project("cheese")
            .labels(&["bug"])
            .priority(Priority::Urgent)
            .estimate(2.0)
            .completed("2026-10-21T10:00:00Z")
            .build(),
        issue("3")
            .priority(Priority::High)
            .estimate(1.0)
            .completed("2026-10-22T10:00:00Z")
            .build(),
        // Open, canceled, or completed before the range: not in the split.
        issue("4").project("cheese").build(),
        issue("5")
            .project("cheese")
            .canceled("2026-10-22T10:00:00Z")
            .build(),
        issue("6")
            .project("cheese")
            .completed("2026-09-22T10:00:00Z")
            .build(),
    ];

    let splits = splits(&issues, Unit::Points, &four_weeks());

    assert_eq!(
        splits.by_project,
        [
            share(Some("cheese".to_string()), 2, 5.0),
            share(None, 1, 1.0)
        ]
    );
    assert_eq!(
        splits.by_label,
        [
            share(Some("bug".to_string()), 2, 5.0),
            share(None, 1, 1.0),
            share(Some("ui".to_string()), 1, 3.0),
        ]
    );
    assert_eq!(
        splits.by_priority,
        [
            share(Priority::Urgent, 1, 2.0),
            share(Priority::High, 2, 4.0)
        ]
    );
}

/// ENG's cycle `number`, two weeks from `starts`, with daily point histories
/// and issue counts of a tenth of the points.
fn cycle(number: u32, starts: &str, scope: &[f64], completed: &[f64]) -> Cycle {
    let starts_at = at(starts);
    let ends_at = starts_at + SignedDuration::from_hours(24 * 14);
    let tenth = |points: &[f64]| points.iter().map(|p| (p / 10.0) as u32).collect();
    Cycle {
        id: format!("c{number}"),
        team_id: "eng".into(),
        number,
        name: None,
        starts_at,
        ends_at,
        completed_at: (ends_at < at(NOW)).then_some(ends_at),
        scope_history: scope.to_vec(),
        completed_scope_history: completed.to_vec(),
        issue_count_history: tenth(scope),
        completed_issue_count_history: tenth(completed),
    }
}

fn burn_up_day(day: Date, scope: f64, completed: f64) -> BurnUpDay {
    BurnUpDay {
        day,
        scope,
        completed,
    }
}

#[test]
fn burn_up_follows_the_cycle_history_one_day_at_a_time() {
    let active = cycle(
        9,
        "2026-10-26T08:00:00Z",
        &[100.0, 100.0, 130.0, 120.0],
        &[0.0, 20.0, 40.0],
    );

    let burn_up = burn_up(&active, Unit::Points, &TimeZone::UTC);

    assert_eq!(burn_up.cycle_id, "c9");
    assert_eq!(burn_up.unit, Unit::Points);
    // The shorter history decides how many days are known.
    assert_eq!(
        burn_up.days,
        [
            burn_up_day(date(2026, 10, 26), 100.0, 0.0),
            burn_up_day(date(2026, 10, 27), 100.0, 20.0),
            burn_up_day(date(2026, 10, 28), 130.0, 40.0),
        ]
    );
}

#[test]
fn scope_creep_adds_up_every_daily_increase_after_the_first_day() {
    let active = cycle(
        9,
        "2026-10-26T08:00:00Z",
        &[100.0, 100.0, 130.0, 120.0, 150.0],
        &[0.0, 0.0, 0.0, 0.0, 0.0],
    );

    assert_eq!(
        burn_up(&active, Unit::Points, &TimeZone::UTC).scope_creep,
        60.0
    );
    assert_eq!(
        burn_up(&active, Unit::Issues, &TimeZone::UTC).scope_creep,
        6.0
    );
}

#[test]
fn burn_up_counts_issues_for_teams_without_estimates() {
    let active = cycle(9, "2026-10-26T08:00:00Z", &[100.0, 120.0], &[0.0, 30.0]);

    let burn_up = burn_up(&active, Unit::Issues, &TimeZone::UTC);

    assert_eq!(burn_up.unit, Unit::Issues);
    assert_eq!(
        burn_up.days,
        [
            burn_up_day(date(2026, 10, 26), 10.0, 0.0),
            burn_up_day(date(2026, 10, 27), 12.0, 3.0),
        ]
    );
}

#[test]
fn burn_up_days_are_calendar_days_in_the_time_zone() {
    // 02:00 UTC on 26 October is still 25 October in New York.
    let new_york = TimeZone::posix("EST5EDT,M3.2.0,M11.1.0").unwrap();
    let active = cycle(9, "2026-10-26T02:00:00Z", &[10.0], &[0.0]);

    assert_eq!(
        burn_up(&active, Unit::Points, &new_york).days[0].day,
        date(2026, 10, 25)
    );
}

#[test]
fn active_cycle_is_the_team_cycle_running_now() {
    let cycles = [
        cycle(7, "2026-09-28T08:00:00Z", &[], &[]),
        cycle(8, "2026-10-12T08:00:00Z", &[], &[]),
        cycle(9, "2026-10-26T08:00:00Z", &[], &[]),
        cycle(10, "2026-11-09T08:00:00Z", &[], &[]),
    ];

    assert_eq!(
        active_cycle(&cycles, "eng", at(NOW)).map(|c| c.number),
        Some(9)
    );
    assert_eq!(active_cycle(&cycles, "des", at(NOW)), None);
}

#[test]
fn velocity_covers_the_last_six_finished_cycles_oldest_first() {
    let mut cycles: Vec<Cycle> = (1..=9)
        .map(|n| {
            let starts =
                at("2026-07-06T08:00:00Z") + SignedDuration::from_hours(24 * 14 * i64::from(n - 1));
            cycle(n, &starts.to_string(), &[20.0, 30.0], &[0.0, f64::from(n)])
        })
        .collect();
    cycles.push(Cycle {
        team_id: "des".into(),
        ..cycle(99, "2026-09-14T08:00:00Z", &[10.0], &[10.0])
    });

    let velocity = velocity(&cycles, "eng", Unit::Points, at(NOW));

    let numbers: Vec<u32> = velocity.iter().map(|v| v.number).collect();
    assert_eq!(numbers, [3, 4, 5, 6, 7, 8]);
    assert_eq!(
        velocity[0],
        CycleVelocity {
            cycle_id: "c3".into(),
            number: 3,
            completed: 3.0,
            committed: 20.0,
            committed_completed_share: Some(0.15),
        }
    );
}

#[test]
fn committed_completed_share_compares_completed_scope_with_day_one_scope() {
    let cycles = [
        cycle(1, "2026-09-14T08:00:00Z", &[20.0, 40.0], &[0.0, 30.0]),
        cycle(2, "2026-09-28T08:00:00Z", &[], &[]),
    ];

    let velocity = velocity(&cycles, "eng", Unit::Points, at(NOW));

    // Points added after day one and completed push the share past 1.
    assert_eq!(velocity[0].committed_completed_share, Some(1.5));
    assert_eq!(
        velocity[1],
        CycleVelocity {
            cycle_id: "c2".into(),
            number: 2,
            completed: 0.0,
            committed: 0.0,
            committed_completed_share: None,
        }
    );
}

#[test]
fn velocity_counts_issues_for_teams_without_estimates() {
    let cycles = [cycle(
        1,
        "2026-09-14T08:00:00Z",
        &[20.0, 40.0],
        &[0.0, 30.0],
    )];

    let velocity = velocity(&cycles, "eng", Unit::Issues, at(NOW));

    assert_eq!((velocity[0].committed, velocity[0].completed), (2.0, 3.0));
}

#[test]
fn triage_health_lists_waiting_issues_and_the_median_triage_time() {
    let issues = [
        issue("1")
            .created("2026-10-30T10:00:00Z")
            .in_state(StateType::Triage)
            .build(),
        issue("2")
            .created("2026-10-20T10:00:00Z")
            .in_state(StateType::Triage)
            .build(),
        issue("3")
            .created("2026-10-20T10:00:00Z")
            .triaged("2026-10-20T12:00:00Z")
            .build(),
        // Declined from triage still counts as handled.
        issue("4")
            .created("2026-10-20T10:00:00Z")
            .triaged("2026-10-21T10:00:00Z")
            .canceled("2026-10-21T10:00:00Z")
            .build(),
        issue("5")
            .created("2026-10-20T10:00:00Z")
            .triaged("2026-10-23T10:00:00Z")
            .build(),
        // Triaged before the range.
        issue("6")
            .created("2026-09-01T10:00:00Z")
            .triaged("2026-09-30T10:00:00Z")
            .build(),
    ];

    assert_eq!(
        triage_health(&issues, &four_weeks()),
        TriageHealth {
            waiting: vec!["2".into(), "1".into()],
            median_triage_time: Some(days(1)),
        }
    );
}

#[test]
fn triage_health_without_triaged_issues_has_no_median() {
    assert_eq!(
        triage_health(&[], &four_weeks()),
        TriageHealth {
            waiting: Vec::new(),
            median_triage_time: None,
        }
    );
}

#[test]
fn stale_issues_are_open_issues_not_updated_for_the_threshold() {
    let issues = [
        issue("1").updated("2026-10-21T12:00:00Z").build(),
        issue("2").updated("2026-10-21T12:00:01Z").build(),
        issue("3")
            .started("2026-09-01T12:00:00Z")
            .updated("2026-09-01T12:00:00Z")
            .build(),
        issue("4").completed("2026-09-01T12:00:00Z").build(),
        issue("5").canceled("2026-09-01T12:00:00Z").build(),
    ];

    let stale = stale_issues(&issues, at(NOW), StaleThreshold::default());

    assert_eq!(
        stale,
        [
            StaleIssue {
                issue_id: "3".into(),
                since_update: days(64)
            },
            StaleIssue {
                issue_id: "1".into(),
                since_update: days(14)
            },
        ]
    );
}

#[test]
fn stale_threshold_is_configurable() {
    let issues = [issue("1").updated("2026-11-01T12:00:00Z").build()];

    assert_eq!(
        stale_issues(&issues, at(NOW), StaleThreshold::days(3)).len(),
        1
    );
    assert!(stale_issues(&issues, at(NOW), StaleThreshold::days(4)).is_empty());
}

fn person(
    assignee: Option<&str>,
    completed: u32,
    in_progress: u32,
    created: u32,
    open_scope: f64,
) -> PersonBreakdown {
    PersonBreakdown {
        assignee_id: assignee.map(String::from),
        completed,
        in_progress,
        created,
        open_scope,
    }
}

fn mixed_team_work() -> Vec<Issue> {
    vec![
        issue("1")
            .assignee("ana")
            .estimate(3.0)
            .created("2026-10-20T10:00:00Z")
            .completed("2026-10-22T10:00:00Z")
            .build(),
        issue("2")
            .assignee("ana")
            .estimate(5.0)
            .started("2026-10-26T10:00:00Z")
            .build(),
        issue("3")
            .assignee("ana")
            .estimate(2.0)
            .created("2026-10-27T10:00:00Z")
            .build(),
        issue("4")
            .assignee("bo")
            .estimate(8.0)
            .started("2026-10-26T10:00:00Z")
            .build(),
        issue("5")
            .assignee("bo")
            .created("2026-10-27T10:00:00Z")
            .canceled("2026-10-28T10:00:00Z")
            .build(),
        issue("6").estimate(1.0).build(),
        // Another team's issue.
        Build(Issue {
            team_id: "des".into(),
            ..issue("7").assignee("ana").estimate(13.0).build()
        })
        .build(),
    ]
}

#[test]
fn per_person_breakdown_is_absent_for_a_team_the_viewer_does_not_administer() {
    let administrable = AdministrableTeams::new(["des".to_string()]);

    let breakdown = per_person_breakdown(
        &team(Estimation::Fibonacci),
        &administrable,
        &mixed_team_work(),
        &four_weeks(),
    );

    assert_eq!(breakdown, None);
}

#[test]
fn per_person_breakdown_splits_a_team_owners_team_by_assignee() {
    let administrable = AdministrableTeams::new(["eng".to_string()]);

    let breakdown = per_person_breakdown(
        &team(Estimation::Fibonacci),
        &administrable,
        &mixed_team_work(),
        &four_weeks(),
    );

    assert_eq!(
        breakdown,
        Some(PerPersonBreakdown {
            unit: Unit::Points,
            people: vec![
                person(Some("ana"), 1, 1, 2, 7.0),
                person(Some("bo"), 0, 1, 0, 8.0),
                person(None, 0, 0, 0, 1.0),
            ],
        })
    );
}

#[test]
fn per_person_breakdown_counts_open_issues_for_teams_without_estimates() {
    let administrable = AdministrableTeams::new(["eng".to_string()]);

    let breakdown = per_person_breakdown(
        &team(Estimation::NotUsed),
        &administrable,
        &mixed_team_work(),
        &four_weeks(),
    )
    .unwrap();

    assert_eq!(breakdown.unit, Unit::Issues);
    let open: Vec<f64> = breakdown.people.iter().map(|p| p.open_scope).collect();
    assert_eq!(open, [2.0, 1.0, 1.0]);
}

#[test]
fn personal_analytics_cover_only_issues_assigned_to_the_viewer() {
    let viewer = Viewer {
        id: "ana".into(),
        name: "Ana".into(),
    };
    let issues = mixed_team_work();

    let personal = personal_analytics(
        &viewer,
        &[team(Estimation::Fibonacci)],
        &issues,
        &four_weeks(),
    );

    assert_eq!(personal.unit, Unit::Points);
    let created: u32 = personal.weeks.iter().map(|w| w.created).sum();
    let completed: f64 = personal.weeks.iter().map(|w| w.completed_scope).sum();
    assert_eq!((created, completed), (2, 3.0));
    assert_eq!(
        personal
            .completion_days
            .iter()
            .map(|d| d.completed)
            .sum::<u32>(),
        1
    );
    assert_eq!(personal.cycle_time, None);
    assert_eq!(personal.lead_time.map(|p| p.p50), Some(days(2)));
    let in_progress: Vec<&str> = personal
        .in_progress
        .iter()
        .map(|a| a.issue_id.as_str())
        .collect();
    assert_eq!(in_progress, ["2"]);
    assert_eq!(personal.splits.by_priority, [share(Priority::None, 1, 3.0)]);
}

#[test]
fn team_analytics_leave_out_the_per_person_breakdown_for_other_teams() {
    let cycles = [cycle(9, "2026-10-26T08:00:00Z", &[10.0], &[0.0])];
    let issues = mixed_team_work();
    let analytics = |administrable: &AdministrableTeams| {
        team_analytics(
            &team(Estimation::NotUsed),
            administrable,
            &issues,
            &cycles,
            StaleThreshold::default(),
            &four_weeks(),
        )
    };

    let member = analytics(&AdministrableTeams::default());
    let owner = analytics(&AdministrableTeams::new(["eng".to_string()]));

    assert_eq!(member.per_person, None);
    assert_eq!(owner.per_person.map(|p| p.people.len()), Some(3));
    assert_eq!(member.unit, Unit::Issues);
    assert_eq!(member.burn_up.map(|b| b.days.len()), Some(1));
    assert!(member.velocity.is_empty());
    let created: u32 = member.weeks.iter().map(|w| w.created).sum();
    assert_eq!(created, 2);
    let in_progress: Vec<&str> = member
        .in_progress
        .iter()
        .map(|a| a.issue_id.as_str())
        .collect();
    assert_eq!(in_progress, ["2", "4"]);
    assert_eq!(member.triage.waiting, Vec::<String>::new());
    // Issues 2, 4 and 6 have not changed since 1 January; issue 7 is DES's.
    let stale: Vec<&str> = member.stale.iter().map(|s| s.issue_id.as_str()).collect();
    assert_eq!(stale, ["2", "4", "6"]);
}
