//! The pure function that groups and sorts an issue list.

// clippy's allow-unwrap-in-tests does not cover helpers outside #[test] functions.
#![allow(clippy::unwrap_used)]

use jiff::Timestamp;
use jiff::civil::date;
use linear_rat::app::{Arrangement, GroupKey, Grouping, Names, Sort, SortKey, arrange};
use linear_rat::linear::demo;
use linear_rat::linear::model::{Cycle, Issue, Member, Priority, Project, StateType};

fn at(seconds: i64) -> Timestamp {
    Timestamp::from_second(seconds).unwrap()
}

/// A demo issue made over as `id`, with every field a sort or grouping
/// reads set to nothing.
fn issue(id: &str) -> Issue {
    let mut issue = demo::workspace().issues[0].clone();
    issue.id = id.to_owned();
    issue.identifier = id.to_owned();
    issue.priority = Priority::None;
    issue.estimate = None;
    issue.due_date = None;
    issue.project_id = None;
    issue.cycle_id = None;
    issue.assignee_id = None;
    issue.created_at = at(0);
    issue.updated_at = at(0);
    issue
}

fn with(id: &str, change: impl FnOnce(&mut Issue)) -> Issue {
    let mut issue = issue(id);
    change(&mut issue);
    issue
}

fn names() -> Names {
    let cycle = |id: &str, number: u32, starts: i64| Cycle {
        id: id.to_owned(),
        team_id: "team-eng".to_owned(),
        number,
        name: None,
        starts_at: at(starts),
        ends_at: at(starts + 100),
        completed_at: None,
        scope_history: Vec::new(),
        completed_scope_history: Vec::new(),
        issue_count_history: Vec::new(),
        completed_issue_count_history: Vec::new(),
    };
    let member = |id: &str, name: &str| Member {
        id: id.to_owned(),
        name: name.to_owned(),
        display_name: name.to_lowercase(),
    };
    let project = |id: &str, name: &str| Project {
        id: id.to_owned(),
        name: name.to_owned(),
    };
    Names::new(
        &[cycle("cycle-1", 1, 0), cycle("cycle-2", 2, 1_000)],
        &[],
        &[member("m-zoe", "Zoe"), member("m-ada", "Ada")],
        &[],
        at(0),
    )
    .with_projects(&[project("p-z", "Zebra"), project("p-a", "Aardvark")])
}

fn sorted(issues: Vec<Issue>, key: SortKey, descending: bool) -> Vec<String> {
    let arrangement = Arrangement {
        group: Grouping::None,
        sort: Some(Sort::new(key, descending)),
    };
    let groups = arrange(issues, arrangement, &names());
    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0].key, GroupKey::All);
    groups[0]
        .issues
        .iter()
        .map(|issue| issue.id.clone())
        .collect()
}

fn grouped(issues: Vec<Issue>, group: Grouping) -> Vec<(GroupKey, Vec<String>)> {
    let arrangement = Arrangement { group, sort: None };
    arrange(issues, arrangement, &names())
        .into_iter()
        .map(|group| {
            let ids = group.issues.iter().map(|issue| issue.id.clone()).collect();
            (group.key, ids)
        })
        .collect()
}

#[test]
fn no_sort_keeps_the_lists_own_order() {
    let issues = vec![issue("b"), issue("a"), issue("c")];

    let groups = arrange(issues, Arrangement::LINEAR_ORDER, &names());

    let ids: Vec<&str> = groups[0].issues.iter().map(|i| i.id.as_str()).collect();
    assert_eq!(ids, ["b", "a", "c"]);
}

#[test]
fn updated_and_created_sort_newest_or_oldest_first() {
    let issues = || {
        vec![
            with("old", |i| (i.updated_at, i.created_at) = (at(1), at(30))),
            with("new", |i| (i.updated_at, i.created_at) = (at(3), at(10))),
            with("mid", |i| (i.updated_at, i.created_at) = (at(2), at(20))),
        ]
    };

    assert_eq!(
        sorted(issues(), SortKey::Updated, true),
        ["new", "mid", "old"]
    );
    assert_eq!(
        sorted(issues(), SortKey::Updated, false),
        ["old", "mid", "new"]
    );
    assert_eq!(
        sorted(issues(), SortKey::Created, true),
        ["old", "mid", "new"]
    );
    assert_eq!(
        sorted(issues(), SortKey::Created, false),
        ["new", "mid", "old"]
    );
}

#[test]
fn priority_sorts_urgent_or_low_first_with_no_priority_last() {
    let issues = || {
        vec![
            issue("none"),
            with("low", |i| i.priority = Priority::Low),
            with("urgent", |i| i.priority = Priority::Urgent),
            with("medium", |i| i.priority = Priority::Medium),
        ]
    };

    assert_eq!(
        sorted(issues(), SortKey::Priority, false),
        ["urgent", "medium", "low", "none"]
    );
    assert_eq!(
        sorted(issues(), SortKey::Priority, true),
        ["low", "medium", "urgent", "none"]
    );
}

#[test]
fn due_date_sorts_soonest_or_latest_first_with_no_due_date_last() {
    let issues = || {
        vec![
            issue("undated"),
            with("later", |i| i.due_date = Some(date(2026, 12, 24))),
            with("sooner", |i| i.due_date = Some(date(2026, 11, 20))),
        ]
    };

    assert_eq!(
        sorted(issues(), SortKey::DueDate, false),
        ["sooner", "later", "undated"]
    );
    assert_eq!(
        sorted(issues(), SortKey::DueDate, true),
        ["later", "sooner", "undated"]
    );
}

#[test]
fn estimate_sorts_largest_or_smallest_first_with_no_estimate_last() {
    let issues = || {
        vec![
            issue("unestimated"),
            with("small", |i| i.estimate = Some(1.0)),
            with("large", |i| i.estimate = Some(8.0)),
        ]
    };

    assert_eq!(
        sorted(issues(), SortKey::Estimate, true),
        ["large", "small", "unestimated"]
    );
    assert_eq!(
        sorted(issues(), SortKey::Estimate, false),
        ["small", "large", "unestimated"]
    );
}

#[test]
fn issues_that_tie_on_the_sort_keep_their_own_order() {
    let issues = vec![
        with("b", |i| i.priority = Priority::High),
        with("a", |i| i.priority = Priority::High),
    ];

    assert_eq!(sorted(issues, SortKey::Priority, false), ["b", "a"]);
}

#[test]
fn state_groups_follow_how_work_moves_and_leave_out_empty_ones() {
    let state = |id: &str, state_type| with(id, |i| i.state.state_type = state_type);
    let issues = vec![
        state("t", StateType::Triage),
        state("s", StateType::Started),
        state("d", StateType::Completed),
        state("b", StateType::Backlog),
        state("s2", StateType::Started),
    ];

    let keys: Vec<GroupKey> = grouped(issues, Grouping::State)
        .into_iter()
        .map(|(key, _)| key)
        .collect();

    assert_eq!(
        keys,
        [
            GroupKey::State(StateType::Started),
            GroupKey::State(StateType::Backlog),
            GroupKey::State(StateType::Triage),
            GroupKey::State(StateType::Completed),
        ]
    );
}

#[test]
fn priority_groups_run_from_urgent_to_no_priority() {
    let issues = vec![
        issue("none"),
        with("low", |i| i.priority = Priority::Low),
        with("urgent", |i| i.priority = Priority::Urgent),
    ];

    assert_eq!(
        grouped(issues, Grouping::Priority),
        [
            (
                GroupKey::Priority(Priority::Urgent),
                vec!["urgent".to_owned()]
            ),
            (GroupKey::Priority(Priority::Low), vec!["low".to_owned()]),
            (GroupKey::Priority(Priority::None), vec!["none".to_owned()]),
        ]
    );
}

#[test]
fn project_groups_go_by_name_with_no_project_last() {
    let issues = vec![
        issue("loose"),
        with("z", |i| i.project_id = Some("p-z".to_owned())),
        with("a", |i| i.project_id = Some("p-a".to_owned())),
    ];

    assert_eq!(
        grouped(issues, Grouping::Project),
        [
            (
                GroupKey::Project(Some("p-a".to_owned())),
                vec!["a".to_owned()]
            ),
            (
                GroupKey::Project(Some("p-z".to_owned())),
                vec!["z".to_owned()]
            ),
            (GroupKey::Project(None), vec!["loose".to_owned()]),
        ]
    );
}

#[test]
fn cycle_groups_go_newest_first_with_no_cycle_last() {
    let issues = vec![
        issue("loose"),
        with("one", |i| i.cycle_id = Some("cycle-1".to_owned())),
        with("two", |i| i.cycle_id = Some("cycle-2".to_owned())),
    ];

    assert_eq!(
        grouped(issues, Grouping::Cycle),
        [
            (
                GroupKey::Cycle(Some("cycle-2".to_owned())),
                vec!["two".to_owned()]
            ),
            (
                GroupKey::Cycle(Some("cycle-1".to_owned())),
                vec!["one".to_owned()]
            ),
            (GroupKey::Cycle(None), vec!["loose".to_owned()]),
        ]
    );
}

#[test]
fn assignee_groups_go_by_name_with_unassigned_last() {
    let issues = vec![
        issue("nobody"),
        with("zoe", |i| i.assignee_id = Some("m-zoe".to_owned())),
        with("ada", |i| i.assignee_id = Some("m-ada".to_owned())),
    ];

    assert_eq!(
        grouped(issues, Grouping::Assignee),
        [
            (
                GroupKey::Assignee(Some("m-ada".to_owned())),
                vec!["ada".to_owned()]
            ),
            (
                GroupKey::Assignee(Some("m-zoe".to_owned())),
                vec!["zoe".to_owned()]
            ),
            (GroupKey::Assignee(None), vec!["nobody".to_owned()]),
        ]
    );
}

#[test]
fn no_grouping_is_one_list_without_a_header() {
    let groups = grouped(vec![issue("a"), issue("b")], Grouping::None);

    assert_eq!(groups.len(), 1);
    assert!(!groups[0].0.has_header());
}

#[test]
fn sorting_applies_inside_each_group() {
    let issues = vec![
        with("low", |i| {
            i.priority = Priority::Low;
            i.state.state_type = StateType::Started;
        }),
        with("backlog", |i| i.state.state_type = StateType::Backlog),
        with("urgent", |i| {
            i.priority = Priority::Urgent;
            i.state.state_type = StateType::Started;
        }),
    ];
    let arrangement = Arrangement {
        group: Grouping::State,
        sort: Some(Sort::new(SortKey::Priority, false)),
    };

    let groups = arrange(issues, arrangement, &names());

    let ids: Vec<Vec<&str>> = groups
        .iter()
        .map(|group| group.issues.iter().map(|i| i.id.as_str()).collect())
        .collect();
    assert_eq!(ids, [vec!["urgent", "low"], vec!["backlog"]]);
}

#[test]
fn sort_names_read_with_or_without_a_direction() {
    assert_eq!(
        Sort::from_name("priority"),
        Some(Sort::new(SortKey::Priority, false))
    );
    assert_eq!(
        Sort::from_name("updated"),
        Some(Sort::new(SortKey::Updated, true))
    );
    assert_eq!(
        Sort::from_name("due-date-desc"),
        Some(Sort::new(SortKey::DueDate, true))
    );
    assert_eq!(
        Sort::from_name("estimate-asc"),
        Some(Sort::new(SortKey::Estimate, false))
    );
    assert_eq!(Sort::from_name("estimate-up"), None);
    for sort in Sort::all() {
        assert_eq!(Sort::from_name(&sort.name()), Some(sort));
    }
}
