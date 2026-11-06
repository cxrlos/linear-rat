use std::collections::{HashMap, HashSet};

use jiff::{SignedDuration, Timestamp};
use linear_rat::linear::demo::{self, DemoWorkspace};
use linear_rat::linear::model::{Cycle, Estimation, Issue, StateType};

fn data() -> DemoWorkspace {
    demo::workspace()
}

fn active_cycles(data: &DemoWorkspace) -> Vec<&Cycle> {
    data.cycles
        .iter()
        .filter(|cycle| cycle.starts_at <= demo::NOW && demo::NOW < cycle.ends_at)
        .collect()
}

fn cycle_of<'a>(data: &'a DemoWorkspace, issue: &Issue) -> Option<&'a Cycle> {
    let id = issue.cycle_id.as_ref()?;
    data.cycles.iter().find(|cycle| &cycle.id == id)
}

#[test]
fn is_the_same_every_time() {
    assert_eq!(data(), data());
}

#[test]
fn has_three_teams_and_about_150_issues() {
    let data = data();

    let keys: Vec<&str> = data.teams.iter().map(|team| team.key.as_str()).collect();
    assert_eq!(keys, ["ENG", "DES", "OPS"]);
    assert!(
        (140..=160).contains(&data.issues.len()),
        "{}",
        data.issues.len()
    );
}

#[test]
fn ops_uses_no_estimates_and_its_issues_have_none() {
    let data = data();
    let ops = data.teams.iter().find(|team| team.key == "OPS").unwrap();

    assert_eq!(ops.estimation, Estimation::NotUsed);
    assert!(
        data.issues
            .iter()
            .filter(|issue| issue.team_id == ops.id)
            .all(|issue| issue.estimate.is_none())
    );
}

#[test]
fn covers_every_state_type_in_every_team() {
    let data = data();
    let all = [
        StateType::Triage,
        StateType::Backlog,
        StateType::Unstarted,
        StateType::Started,
        StateType::Completed,
        StateType::Canceled,
    ];
    for team in &data.teams {
        let present: HashSet<StateType> = data
            .issues
            .iter()
            .filter(|issue| issue.team_id == team.id)
            .map(|issue| issue.state.state_type)
            .collect();
        for state_type in all {
            assert!(
                present.contains(&state_type),
                "{} lacks {state_type:?}",
                team.key
            );
        }
    }
}

#[test]
fn identifiers_are_unique_and_numbered_in_creation_order() {
    let data = data();
    for team in &data.teams {
        let mut issues: Vec<&Issue> = data
            .issues
            .iter()
            .filter(|issue| issue.team_id == team.id)
            .collect();
        issues.sort_by_key(|issue| issue.created_at);
        for (index, issue) in issues.iter().enumerate() {
            assert_eq!(issue.identifier, format!("{}-{}", team.key, index + 1));
            assert!(issue.url.ends_with(&issue.identifier));
        }
    }
}

#[test]
fn timestamps_follow_the_workflow_and_never_pass_now() {
    let data = data();
    for issue in &data.issues {
        let id = &issue.identifier;
        let state_type = issue.state.state_type;
        assert!(issue.created_at <= issue.updated_at, "{id}");
        assert!(issue.updated_at <= demo::NOW, "{id}");
        assert_eq!(
            issue.completed_at.is_some(),
            state_type == StateType::Completed,
            "{id}"
        );
        assert_eq!(
            issue.canceled_at.is_some(),
            state_type == StateType::Canceled,
            "{id}"
        );
        if matches!(state_type, StateType::Started | StateType::Completed) {
            assert!(issue.started_at.is_some(), "{id}");
        }
        if matches!(
            state_type,
            StateType::Triage | StateType::Backlog | StateType::Unstarted
        ) {
            assert!(issue.started_at.is_none(), "{id}");
        }
        if state_type == StateType::Triage {
            assert!(issue.triaged_at.is_none(), "{id}");
        }
        let mut order = vec![issue.created_at];
        order.extend(issue.triaged_at);
        order.extend(issue.started_at);
        order.extend(issue.completed_at.or(issue.canceled_at));
        assert!(order.is_sorted(), "{id}: {order:?}");
    }
}

#[test]
fn state_history_ends_in_the_current_state() {
    let data = data();
    for issue in &data.issues {
        let id = &issue.identifier;
        let history = &issue.state_history;
        assert_eq!(history[0].started_at, issue.created_at, "{id}");
        for pair in history.windows(2) {
            assert_eq!(pair[0].ended_at, Some(pair[1].started_at), "{id}");
            assert!(pair[0].started_at <= pair[1].started_at, "{id}");
        }
        let last = history.last().unwrap();
        assert_eq!(last.state_id, issue.state.id, "{id}");
        assert_eq!(last.state_type, issue.state.state_type, "{id}");
        assert_eq!(last.ended_at, None, "{id}");
        for span in history {
            let state = data
                .workflow_states
                .iter()
                .find(|state| state.id == span.state_id)
                .unwrap();
            assert_eq!(state.state_type, span.state_type, "{id}");
            assert_eq!(state.team_id, issue.team_id, "{id}");
        }
        let entered = |state_type: StateType| {
            history
                .iter()
                .find(|span| span.state_type == state_type)
                .map(|span| span.started_at)
        };
        assert_eq!(entered(StateType::Started), issue.started_at, "{id}");
        assert_eq!(entered(StateType::Completed), issue.completed_at, "{id}");
        assert_eq!(entered(StateType::Canceled), issue.canceled_at, "{id}");
        if let Some(triaged_at) = issue.triaged_at {
            assert_eq!(history[0].state_type, StateType::Triage, "{id}");
            assert_eq!(history[0].ended_at, Some(triaged_at), "{id}");
        }
    }
}

#[test]
fn every_team_has_six_past_cycles_and_one_active() {
    let data = data();
    let active = active_cycles(&data);
    for team in &data.teams {
        let cycles: Vec<&Cycle> = data
            .cycles
            .iter()
            .filter(|cycle| cycle.team_id == team.id)
            .collect();
        let past = cycles
            .iter()
            .filter(|cycle| cycle.ends_at <= demo::NOW && cycle.completed_at.is_some())
            .count();
        assert_eq!(past, 6, "{}", team.key);
        assert_eq!(cycles.len(), 7, "{}", team.key);
        assert_eq!(
            active
                .iter()
                .filter(|cycle| cycle.team_id == team.id)
                .count(),
            1,
            "{}",
            team.key
        );
    }
}

#[test]
fn cycle_history_has_one_entry_per_day_and_matches_its_issues() {
    let data = data();
    let day = SignedDuration::from_hours(24);
    for cycle in &data.cycles {
        let last_day = cycle.ends_at.min(demo::NOW);
        let days = usize::try_from(last_day.duration_since(cycle.starts_at).as_hours())
            .unwrap()
            .div_ceil(24);
        assert_eq!(cycle.scope_history.len(), days, "cycle {}", cycle.id);
        assert_eq!(cycle.completed_scope_history.len(), days);
        assert_eq!(cycle.issue_count_history.len(), days);
        assert_eq!(cycle.completed_issue_count_history.len(), days);
        for history in [&cycle.scope_history, &cycle.completed_scope_history] {
            assert!(history.iter().all(|points| *points >= 0.0));
        }
        assert!(cycle.completed_issue_count_history.is_sorted());

        let at_end = (cycle.starts_at + day * i32::try_from(days).unwrap()).min(demo::NOW);
        let issues: Vec<&Issue> = data
            .issues
            .iter()
            .filter(|issue| issue.cycle_id.as_ref() == Some(&cycle.id))
            .filter(|issue| issue.canceled_at.is_none_or(|canceled| canceled > at_end))
            .collect();
        let completed = issues
            .iter()
            .filter(|issue| issue.completed_at.is_some_and(|done| done <= at_end))
            .count();
        assert_eq!(
            cycle.issue_count_history.last(),
            Some(&(issues.len() as u32))
        );
        assert_eq!(
            cycle.completed_issue_count_history.last(),
            Some(&(completed as u32))
        );
        let points: f64 = issues.iter().filter_map(|issue| issue.estimate).sum();
        assert_eq!(cycle.scope_history.last(), Some(&points));
    }
}

#[test]
fn some_cycles_grow_after_they_start() {
    let data = data();
    let grown = data
        .cycles
        .iter()
        .filter(|cycle| cycle.issue_count_history.first() < cycle.issue_count_history.last())
        .count();

    assert!(grown >= 3, "{grown}");
}

#[test]
fn work_in_a_cycle_happens_inside_it() {
    let data = data();
    for issue in &data.issues {
        let Some(cycle) = cycle_of(&data, issue) else {
            continue;
        };
        let id = &issue.identifier;
        assert_eq!(cycle.team_id, issue.team_id, "{id}");
        assert!(issue.created_at < cycle.ends_at, "{id}");
        for moment in [issue.started_at, issue.completed_at, issue.canceled_at]
            .into_iter()
            .flatten()
        {
            assert!(moment >= cycle.starts_at, "{id}");
            assert!(moment < cycle.ends_at, "{id}");
        }
        if cycle.ends_at <= demo::NOW {
            assert!(
                !issue.state.state_type.is_open(),
                "{id} is open in a past cycle"
            );
        }
    }
}

#[test]
fn references_point_at_things_that_exist() {
    let data = data();
    let teams: HashSet<&str> = data.teams.iter().map(|team| team.id.as_str()).collect();
    let members: HashSet<&str> = data.members.iter().map(|m| m.id.as_str()).collect();
    let projects: HashSet<&str> = data.projects.iter().map(|p| p.id.as_str()).collect();
    let labels: HashMap<&str, Option<&str>> = data
        .labels
        .iter()
        .map(|label| (label.id.as_str(), label.team_id.as_deref()))
        .collect();
    assert!(members.contains(data.viewer.id.as_str()));
    for issue in &data.issues {
        let id = &issue.identifier;
        assert!(teams.contains(issue.team_id.as_str()), "{id}");
        assert_eq!(issue.state.team_id, issue.team_id, "{id}");
        for member in [&issue.assignee_id, &issue.creator_id]
            .into_iter()
            .flatten()
        {
            assert!(members.contains(member.as_str()), "{id}");
        }
        if let Some(project) = &issue.project_id {
            assert!(projects.contains(project.as_str()), "{id}");
        }
        for label in &issue.label_ids {
            let owner = labels.get(label.as_str()).unwrap();
            assert!(owner.is_none_or(|team| team == issue.team_id), "{id}");
        }
    }
    assert!(!data.comments.is_empty());
    for comment in &data.comments {
        let issue = data
            .issues
            .iter()
            .find(|i| i.id == comment.issue_id)
            .unwrap();
        assert!(issue.created_at <= comment.created_at);
        assert!(comment.created_at <= issue.updated_at);
        assert!(members.contains(comment.author_id.as_deref().unwrap()));
    }
}

#[test]
fn the_viewer_has_open_work_in_every_open_state_type() {
    let data = data();
    let active: Vec<&str> = active_cycles(&data).iter().map(|c| c.id.as_str()).collect();
    let mine: Vec<&Issue> = data
        .issues
        .iter()
        .filter(|issue| issue.assignee_id.as_ref() == Some(&data.viewer.id))
        .collect();
    for state_type in [
        StateType::Started,
        StateType::Unstarted,
        StateType::Backlog,
        StateType::Triage,
    ] {
        let count = mine
            .iter()
            .filter(|issue| issue.state.state_type == state_type)
            .count();
        assert!(count >= 2, "{state_type:?}: {count}");
    }
    let in_active_cycle = mine
        .iter()
        .filter(|issue| issue.state.state_type.is_open())
        .filter(|issue| {
            issue
                .cycle_id
                .as_deref()
                .is_some_and(|c| active.contains(&c))
        })
        .count();
    assert!(in_active_cycle >= 3, "{in_active_cycle}");
    assert!(
        mine.iter()
            .any(|issue| issue.state.state_type == StateType::Completed),
        "the viewer needs history for personal analytics"
    );
}

#[test]
fn every_date_is_anchored_before_now() {
    let data = data();
    let earliest = demo::NOW - SignedDuration::from_hours(24 * 120);
    let dates = data
        .issues
        .iter()
        .map(|issue| issue.created_at)
        .chain(data.cycles.iter().map(|cycle| cycle.starts_at));

    assert!(
        dates
            .into_iter()
            .all(|date: Timestamp| earliest <= date && date <= demo::NOW)
    );
}

#[test]
fn every_issue_has_a_git_safe_branch_name_starting_with_its_identifier() {
    for issue in data().issues {
        let prefix = format!("{}-", issue.identifier.to_lowercase());
        assert!(
            issue.branch_name.starts_with(&prefix),
            "{}",
            issue.branch_name
        );
        assert!(issue.branch_name.len() <= 60, "{}", issue.branch_name);
        assert!(
            issue
                .branch_name
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-'),
            "{}",
            issue.branch_name
        );
    }
}
