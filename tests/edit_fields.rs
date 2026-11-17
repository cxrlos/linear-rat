//! Changing an issue's cycle, project, project milestone and due date
//! through the app driver: the pickers and the prompt, the one change each
//! sends, and the rollback when Linear rejects it.

// clippy's allow-unwrap-in-tests does not cover helpers outside #[test] functions.
#![allow(clippy::unwrap_used)]

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use insta::assert_snapshot;
use jiff::civil::date;
use linear_rat::app::{Action, App, Edit, Effect, Event, Modal, State, Write};
use linear_rat::keys;
use linear_rat::linear::model::{Cycle, Issue};
use linear_rat::linear::{FakeClient, IssueDetail, IssueUpdate, LinearClient, Patch, demo};
use ratatui::backend::TestBackend;

/// The issue My Issues focuses first in the demo, in team DES: in the
/// active cycle, in no project, due 2026-11-06.
const FIRST: &str = "DES-34";
const DES: &str = "team-des";
/// The third issue of My Issues: in Shared pantries, at its Launch
/// milestone, due 2026-11-20.
const THIRD: &str = "OPS-37";
const OPS: &str = "team-ops";
const SHARED_PANTRIES: &str = "project-2";

fn app() -> App<TestBackend> {
    App::new(TestBackend::new(80, 24), State::demo()).unwrap()
}

/// The demo app focused on OPS-37.
fn on_third() -> App<TestBackend> {
    let mut app = app();
    press(&mut app, "jj");
    assert_eq!(app.state().focused_issue().unwrap().identifier, THIRD);
    app
}

fn key(code: KeyCode) -> Event {
    Event::Key(KeyEvent::new(code, KeyModifiers::NONE))
}

/// Types `keys`, one character each, and returns every effect asked for.
fn press(app: &mut App<TestBackend>, keys: &str) -> Vec<Effect> {
    keys.chars()
        .flat_map(|c| app.handle(key(KeyCode::Char(c))).unwrap())
        .collect()
}

fn enter(app: &mut App<TestBackend>) -> Vec<Effect> {
    app.handle(key(KeyCode::Enter)).unwrap()
}

fn screen(app: &App<TestBackend>) -> Vec<String> {
    let buffer = app.backend().buffer();
    let width = usize::from(buffer.area.width);
    buffer
        .content()
        .chunks(width)
        .map(|row| row.iter().map(|cell| cell.symbol()).collect())
        .collect()
}

fn shows(app: &App<TestBackend>, text: &str) -> bool {
    screen(app).iter().any(|line| line.contains(text))
}

/// The lines inside the open picker's border below the filter, trimmed.
fn picker_lines(app: &App<TestBackend>) -> Vec<String> {
    let lines = screen(app);
    let Some(top) = lines.iter().position(|line| line.contains(" of ")) else {
        return Vec::new();
    };
    let start = lines[top].chars().position(|c| c == '┌').unwrap();
    lines[top + 2..]
        .iter()
        .take_while(|line| line.chars().nth(start) == Some('│'))
        .map(|line| {
            let inner: String = line
                .chars()
                .skip(start + 1)
                .take_while(|&c| c != '│')
                .collect();
            inner.trim().to_owned()
        })
        .collect()
}

fn issue(app: &App<TestBackend>, identifier: &str) -> Issue {
    app.state()
        .store
        .issues()
        .find(|issue| issue.identifier == identifier)
        .cloned()
        .unwrap()
}

/// The only effect, which must be a write.
fn only_write(effects: Vec<Effect>) -> Write {
    match effects.as_slice() {
        [Effect::UpdateIssue(write)] => write.clone(),
        other => panic!("expected exactly one write, got {other:?}"),
    }
}

/// Linear's answer to `write`, as the fake client gives it, on a thread of
/// its own so each answer starts from the demo workspace.
fn accepted(write: &Write) -> Event {
    let issue = std::thread::scope(|scope| {
        scope
            .spawn(|| {
                tokio::runtime::Builder::new_current_thread()
                    .build()
                    .unwrap()
                    .block_on(FakeClient.update_issue(&write.issue_id, &write.update()))
            })
            .join()
            .unwrap()
    })
    .unwrap();
    Event::IssueUpdated {
        write: write.clone(),
        result: Ok(Box::new(issue)),
    }
}

fn rejected(write: &Write) -> Event {
    Event::IssueUpdated {
        write: write.clone(),
        result: Err("Linear returned errors: Entity not found (INVALID_INPUT)".to_owned()),
    }
}

/// Team `team_id`'s options as the fake gives them, with two upcoming
/// cycles after the active one, in reverse order.
fn options_loaded(team_id: &str) -> Event {
    let mut options = demo::team_options(&demo::workspace(), team_id);
    let active = options
        .cycles
        .iter()
        .find(|cycle| cycle.completed_at.is_none())
        .cloned()
        .unwrap();
    let after = |cycle: &Cycle, number: u32| {
        let length = cycle.ends_at.duration_since(cycle.starts_at);
        Cycle {
            id: format!("{team_id}-cycle-{number}"),
            number,
            starts_at: cycle.ends_at,
            ends_at: cycle.ends_at.checked_add(length).unwrap(),
            ..cycle.clone()
        }
    };
    let next = after(&active, active.number + 1);
    let later = after(&next, active.number + 2);
    options.cycles.push(later);
    options.cycles.push(next);
    Event::TeamOptionsLoaded(Ok(options))
}

fn milestones_loaded(project_id: &str) -> Event {
    Event::ProjectMilestonesLoaded {
        project_id: project_id.to_owned(),
        result: Ok(demo::workspace()
            .milestones
            .into_iter()
            .filter(|milestone| milestone.project_id == project_id)
            .collect()),
    }
}

// The cycle picker.

#[test]
fn shift_c_opens_the_cycle_picker_and_asks_for_the_teams_options() {
    let mut app = app();

    let effects = press(&mut app, "C");

    assert_eq!(effects, vec![Effect::LoadTeamOptions(DES.to_owned())]);
    assert!(shows(&app, "Cycle of DES-34"));
    assert!(shows(&app, "Loading…"));
}

#[test]
fn the_cycle_picker_lists_the_active_next_and_upcoming_cycles_then_none() {
    let mut app = app();
    press(&mut app, "C");

    app.handle(options_loaded(DES)).unwrap();

    assert_eq!(
        picker_lines(&app),
        [
            "1 Cycle 7 (active) ✓",
            "2 Cycle 8 (next)",
            "3 Cycle 9",
            "4 No cycle"
        ]
    );
    assert_snapshot!(app.backend());
}

#[test]
fn a_new_cycle_shows_at_once_and_sends_only_the_cycle() {
    let mut app = app();
    press(&mut app, "C");
    app.handle(options_loaded(DES)).unwrap();

    let write = only_write(press(&mut app, "2"));

    assert_eq!(
        write.update(),
        IssueUpdate {
            cycle_id: Patch::Set(format!("{DES}-cycle-8")),
            ..IssueUpdate::default()
        }
    );
    let cycle = format!("{DES}-cycle-8");
    assert_eq!(issue(&app, FIRST).cycle_id.as_deref(), Some(cycle.as_str()));
    app.handle(accepted(&write)).unwrap();
    assert_eq!(issue(&app, FIRST).cycle_id.as_deref(), Some(cycle.as_str()));
}

#[test]
fn no_cycle_clears_the_cycle() {
    let mut app = app();
    press(&mut app, "C");
    app.handle(options_loaded(DES)).unwrap();

    let write = only_write(press(&mut app, "4"));

    assert_eq!(write.update().cycle_id, Patch::Clear);
    assert_eq!(issue(&app, FIRST).cycle_id, None);
}

#[test]
fn a_rejected_cycle_edit_restores_the_cycle_with_the_error() {
    let mut app = app();
    let before = issue(&app, FIRST).cycle_id;
    press(&mut app, "C");
    app.handle(options_loaded(DES)).unwrap();
    let write = only_write(press(&mut app, "4"));

    app.handle(rejected(&write)).unwrap();

    assert_eq!(issue(&app, FIRST).cycle_id, before);
    assert!(shows(&app, "Could not change the cycle of DES-34"));
}

#[test]
fn the_cycle_picker_counts_from_the_latest_tick_not_the_last_load() {
    let mut app = app();
    let active_ends = demo::team_options(&demo::workspace(), DES)
        .cycles
        .iter()
        .find(|cycle| cycle.completed_at.is_none())
        .unwrap()
        .ends_at;
    app.handle(Event::Tick(
        active_ends
            .checked_add(jiff::SignedDuration::from_hours(1))
            .unwrap(),
    ))
    .unwrap();
    press(&mut app, "C");

    app.handle(options_loaded(DES)).unwrap();

    assert_eq!(
        picker_lines(&app),
        ["1 Cycle 8 (active)", "2 Cycle 9 (next)", "3 No cycle"]
    );
}

#[test]
fn the_cycle_picker_says_when_the_options_could_not_load() {
    let mut app = app();
    press(&mut app, "C");

    app.handle(Event::TeamOptionsLoaded(Err("timed out".to_owned())))
        .unwrap();

    assert!(shows(&app, "Could not load: timed out"));
}

// The project picker.

#[test]
fn the_project_picker_lists_the_teams_projects_then_none() {
    let mut app = on_third();
    let effects = press(&mut app, "P");
    assert_eq!(effects, vec![Effect::LoadTeamOptions(OPS.to_owned())]);

    app.handle(options_loaded(OPS)).unwrap();

    let lines = picker_lines(&app);
    assert!(
        lines.contains(&"4 Shared pantries ✓".to_owned()),
        "{lines:?}"
    );
    assert_eq!(lines.last().map(String::as_str), Some("5 No project"));
    assert_snapshot!(app.backend());
}

#[test]
fn a_new_project_clears_the_milestone_in_the_store_and_the_input() {
    let mut app = on_third();
    press(&mut app, "P");
    app.handle(options_loaded(OPS)).unwrap();

    let write = only_write(press(&mut app, "1"));

    let moved = issue(&app, THIRD);
    assert_eq!(moved.project_id.as_deref(), Some("project-3"));
    assert_eq!(moved.project_milestone_id, None);
    assert_eq!(
        write.update(),
        IssueUpdate {
            project_id: Patch::Set("project-3".to_owned()),
            project_milestone_id: Patch::Clear,
            ..IssueUpdate::default()
        }
    );
    app.handle(accepted(&write)).unwrap();
    assert_eq!(issue(&app, THIRD).project_milestone_id, None);
}

#[test]
fn choosing_the_issues_own_project_keeps_its_milestone_and_sends_nothing() {
    let mut app = on_third();
    press(&mut app, "P");
    app.handle(options_loaded(OPS)).unwrap();

    let effects = press(&mut app, "4");

    assert!(effects.is_empty(), "{effects:?}");
    assert_eq!(
        issue(&app, THIRD).project_milestone_id.as_deref(),
        Some("milestone-2-3")
    );
}

#[test]
fn a_rejected_project_edit_restores_the_project_and_the_milestone() {
    let mut app = on_third();
    press(&mut app, "P");
    app.handle(options_loaded(OPS)).unwrap();
    let write = only_write(press(&mut app, "5"));
    assert_eq!(issue(&app, THIRD).project_id, None);

    app.handle(rejected(&write)).unwrap();

    let restored = issue(&app, THIRD);
    assert_eq!(restored.project_id.as_deref(), Some(SHARED_PANTRIES));
    assert_eq!(
        restored.project_milestone_id.as_deref(),
        Some("milestone-2-3")
    );
    assert!(shows(&app, "Could not change the project of OPS-37"));
}

// The milestone picker.

#[test]
fn without_a_project_the_milestone_picker_does_not_open() {
    let mut app = app();

    let effects = press(&mut app, "M");

    assert!(effects.is_empty(), "{effects:?}");
    assert!(app.state().modals.top().is_none());
}

#[test]
fn shift_m_asks_for_the_projects_milestones_and_lists_them_once_loaded() {
    let mut app = on_third();

    let effects = press(&mut app, "M");

    assert_eq!(
        effects,
        vec![Effect::LoadProjectMilestones {
            project_id: SHARED_PANTRIES.to_owned()
        }]
    );
    assert!(shows(&app, "Loading…"));
    app.handle(milestones_loaded(SHARED_PANTRIES)).unwrap();
    assert_eq!(
        picker_lines(&app),
        ["1 Design review", "2 Beta", "3 Launch ✓", "4 No milestone"]
    );
    assert_snapshot!(app.backend());
}

#[test]
fn a_second_milestone_picker_for_the_same_project_needs_no_load() {
    let mut app = on_third();
    press(&mut app, "M");
    app.handle(milestones_loaded(SHARED_PANTRIES)).unwrap();
    app.handle(key(KeyCode::Esc)).unwrap();

    let effects = press(&mut app, "M");

    assert!(effects.is_empty(), "{effects:?}");
}

#[test]
fn a_new_milestone_sends_only_the_milestone_and_rolls_back_when_rejected() {
    let mut app = on_third();
    press(&mut app, "M");
    app.handle(milestones_loaded(SHARED_PANTRIES)).unwrap();

    let write = only_write(press(&mut app, "2"));

    assert_eq!(
        write.update(),
        IssueUpdate {
            project_milestone_id: Patch::Set("milestone-2-2".to_owned()),
            ..IssueUpdate::default()
        }
    );
    assert_eq!(
        issue(&app, THIRD).project_milestone_id.as_deref(),
        Some("milestone-2-2")
    );
    app.handle(rejected(&write)).unwrap();
    assert_eq!(
        issue(&app, THIRD).project_milestone_id.as_deref(),
        Some("milestone-2-3")
    );
    assert!(shows(&app, "Could not change the milestone of OPS-37"));
}

#[test]
fn a_milestone_edit_keeps_linears_answer() {
    let mut app = on_third();
    press(&mut app, "M");
    app.handle(milestones_loaded(SHARED_PANTRIES)).unwrap();
    let write = only_write(press(&mut app, "4"));
    assert_eq!(write.update().project_milestone_id, Patch::Clear);

    app.handle(accepted(&write)).unwrap();

    assert_eq!(issue(&app, THIRD).project_milestone_id, None);
}

#[test]
fn the_milestone_picker_says_when_the_milestones_could_not_load() {
    let mut app = on_third();
    press(&mut app, "M");

    app.handle(Event::ProjectMilestonesLoaded {
        project_id: SHARED_PANTRIES.to_owned(),
        result: Err("timed out".to_owned()),
    })
    .unwrap();

    assert!(shows(&app, "Could not load: timed out"));
    assert!(
        app.state()
            .notice
            .as_deref()
            .is_some_and(|notice| notice.starts_with("Could not load the milestones of"))
    );
}

// The due date.

#[test]
fn d_opens_the_due_date_prompt_with_the_due_date_it_has() {
    let mut app = app();

    let effects = press(&mut app, "d");

    assert!(effects.is_empty(), "{effects:?}");
    assert!(matches!(app.state().modals.top(), Some(Modal::DueDate(_))));
    assert_snapshot!(app.backend());
}

#[test]
fn a_typed_date_shows_at_once_and_sends_only_the_due_date() {
    let mut app = app();
    press(&mut app, "d2026-12-24");

    let write = only_write(enter(&mut app));

    assert_eq!(
        write.update(),
        IssueUpdate {
            due_date: Patch::Set(date(2026, 12, 24)),
            ..IssueUpdate::default()
        }
    );
    assert!(app.state().modals.top().is_none());
    assert_eq!(issue(&app, FIRST).due_date, Some(date(2026, 12, 24)));
    app.handle(accepted(&write)).unwrap();
    assert_eq!(issue(&app, FIRST).due_date, Some(date(2026, 12, 24)));
}

#[test]
fn a_relative_date_counts_from_the_viewers_today() {
    let mut app = app();
    // The demo's today is Monday 2026-11-02 in UTC, the tests' time zone.
    press(&mut app, "dtue");

    let write = only_write(enter(&mut app));

    assert_eq!(write.edit, Edit::DueDate(Some(date(2026, 11, 3))));
    assert_eq!(write.previous, Edit::DueDate(Some(date(2026, 11, 6))));
}

#[test]
fn before_any_load_or_tick_today_does_not_read_the_clock() {
    let state = State::default();

    assert_eq!(state.today(), State::default().today());
    assert_eq!(state.today(), date(1970, 1, 1));
}

#[test]
fn clear_removes_the_due_date_and_a_rejection_puts_it_back() {
    let mut app = app();
    press(&mut app, "dclear");

    let write = only_write(enter(&mut app));

    assert_eq!(write.update().due_date, Patch::Clear);
    assert_eq!(issue(&app, FIRST).due_date, None);
    app.handle(rejected(&write)).unwrap();
    assert_eq!(issue(&app, FIRST).due_date, Some(date(2026, 11, 6)));
    assert!(shows(&app, "Could not change the due date of DES-34"));
}

#[test]
fn enter_is_available_while_the_due_date_prompt_is_open() {
    let mut app = app();
    let accept = keys::spec(Action::Accept).unwrap();

    press(&mut app, "d");

    assert!((accept.available)(app.state()));
}

#[test]
fn input_it_cannot_read_keeps_the_prompt_open_with_the_reason() {
    let mut app = app();
    press(&mut app, "dsoon");

    let effects = enter(&mut app);

    assert!(effects.is_empty(), "{effects:?}");
    assert!(matches!(app.state().modals.top(), Some(Modal::DueDate(_))));
    assert_snapshot!(app.backend());
    press(&mut app, "x");
    assert!(!shows(&app, "✘ Use"));
}

#[test]
fn esc_closes_the_prompt_without_an_edit() {
    let mut app = app();
    press(&mut app, "d2026-12-24");

    let effects = app.handle(key(KeyCode::Esc)).unwrap();

    assert!(effects.is_empty(), "{effects:?}");
    assert_eq!(issue(&app, FIRST).due_date, Some(date(2026, 11, 6)));
}

// What the list and the detail show.

#[test]
fn the_detail_shows_the_milestone_and_the_due_date_beside_project_and_cycle() {
    let mut app = App::new(TestBackend::new(200, 40), State::demo()).unwrap();
    press(&mut app, "jj");
    let issue_id = issue(&app, THIRD).id;

    app.handle(Event::IssueDetailLoaded {
        result: Ok(IssueDetail::demo(&issue_id)),
        issue_id,
    })
    .unwrap();

    assert!(shows(&app, "Milestone Launch"));
    assert!(shows(&app, "Due       2026-11-20"));
    assert_snapshot!(app.backend());
}

#[test]
fn an_overdue_open_issue_says_so_in_its_detail() {
    let mut app = App::new(TestBackend::new(200, 40), State::demo()).unwrap();

    press(&mut app, "j");

    assert!(shows(&app, "due Oct 30"));
    assert!(shows(&app, "Due       2026-10-30 (overdue)"));
}
