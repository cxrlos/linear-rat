//! Changing an issue's workflow state, assignee and priority through the app
//! driver: the pickers, the one mutation each edit sends, and the rollback
//! when Linear rejects it.

// clippy's allow-unwrap-in-tests does not cover helpers outside #[test] functions.
#![allow(clippy::unwrap_used)]

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use insta::assert_snapshot;
use linear_rat::app::{App, Edit, Effect, Event, State, Store, Write};
use linear_rat::linear::model::{Issue, Priority};
use linear_rat::linear::{FakeClient, IssueUpdate, LinearClient, Patch, demo};
use ratatui::backend::TestBackend;

/// The issue My Issues focuses first in the demo, in team DES.
const FIRST: &str = "DES-34";
const DES: &str = "team-des";

fn app() -> App<TestBackend> {
    App::new(TestBackend::new(80, 24), State::demo()).unwrap()
}

fn wide() -> App<TestBackend> {
    App::new(TestBackend::new(200, 60), State::demo()).unwrap()
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

fn screen(app: &App<TestBackend>) -> Vec<String> {
    let buffer = app.backend().buffer();
    let width = usize::from(buffer.area.width);
    buffer
        .content()
        .chunks(width)
        .map(|row| row.iter().map(|cell| cell.symbol()).collect())
        .collect()
}

/// The list row of the issue with `identifier`, if it is on screen.
fn row(app: &App<TestBackend>, identifier: &str) -> Option<String> {
    screen(app)
        .into_iter()
        .find(|line| line.contains(&format!("{identifier} ")))
}

/// The lines inside the open picker's border below the filter, trimmed.
fn picker_lines(app: &App<TestBackend>) -> Vec<String> {
    let lines = screen(app);
    let Some(top) = lines.iter().position(|line| line.contains(" of DES-")) else {
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

fn shows(app: &App<TestBackend>, text: &str) -> bool {
    screen(app).iter().any(|line| line.contains(text))
}

fn demo_issue(identifier: &str) -> Issue {
    demo::workspace()
        .issues
        .into_iter()
        .find(|issue| issue.identifier == identifier)
        .unwrap()
}

fn states_loaded(team_id: &str) -> Event {
    Event::WorkflowStatesLoaded {
        team_id: team_id.to_owned(),
        result: Ok(demo::workspace()
            .workflow_states
            .into_iter()
            .filter(|state| state.team_id == team_id)
            .collect()),
    }
}

fn members_loaded(team_id: &str) -> Event {
    let ids = demo::team_member_ids(team_id);
    Event::TeamMembersLoaded {
        team_id: team_id.to_owned(),
        result: Ok(demo::workspace()
            .members
            .into_iter()
            .filter(|member| ids.contains(&member.id))
            .collect()),
    }
}

/// The only effect, which must be a write.
fn only_write(effects: Vec<Effect>) -> Write {
    match effects.as_slice() {
        [Effect::UpdateIssue(write)] => write.clone(),
        other => panic!("expected exactly one write, got {other:?}"),
    }
}

/// Linear's answer to `write`, as the fake client gives it. It runs on a
/// thread of its own, because the fake remembers edits per thread and each
/// answer here must start from the demo workspace.
fn accepted(write: &Write) -> Event {
    let issue = std::thread::scope(|scope| {
        scope
            .spawn(|| {
                tokio::runtime::Builder::new_current_thread()
                    .build()
                    .unwrap()
                    .block_on(FakeClient.update_issue(&write.issue_id, &write.edit.update()))
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

#[test]
fn s_opens_the_state_picker_and_asks_for_the_teams_states() {
    let mut app = app();

    let effects = press(&mut app, "s");

    assert_eq!(
        effects,
        vec![Effect::LoadWorkflowStates {
            team_id: DES.to_owned()
        }]
    );
    assert_snapshot!(app.backend());
}

#[test]
fn the_state_picker_lists_the_states_by_type_once_they_load() {
    let mut app = app();
    press(&mut app, "s");

    app.handle(states_loaded(DES)).unwrap();

    assert_snapshot!(app.backend());
}

#[test]
fn a_second_state_picker_for_the_same_team_needs_no_load() {
    let mut app = app();
    press(&mut app, "s");
    app.handle(states_loaded(DES)).unwrap();
    app.handle(key(KeyCode::Esc)).unwrap();

    let effects = press(&mut app, "s");

    assert!(effects.is_empty(), "{effects:?}");
    assert!(shows(&app, "In Review"));
}

#[test]
fn typing_filters_the_picker_and_numbers_count_what_is_left() {
    let mut app = app();
    press(&mut app, "s");
    app.handle(states_loaded(DES)).unwrap();

    press(&mut app, "rev");

    assert!(shows(&app, "> rev"));
    assert_eq!(picker_lines(&app), ["1 ◐ In Review"]);
    let write = only_write(press(&mut app, "1"));
    assert_eq!(
        write.edit.update(),
        IssueUpdate {
            state_id: Some(format!("{DES}-state-in-review")),
            ..IssueUpdate::default()
        }
    );
}

#[test]
fn backspace_widens_the_filter_again() {
    let mut app = app();
    press(&mut app, "p");
    press(&mut app, "lo");
    assert_eq!(picker_lines(&app), ["1 ▮▯▯ Low"]);

    app.handle(key(KeyCode::Backspace)).unwrap();
    app.handle(key(KeyCode::Backspace)).unwrap();

    assert_eq!(picker_lines(&app).len(), 5);
    assert!(shows(&app, "type to filter"));
}

#[test]
fn a_number_past_the_last_entry_types_into_the_filter() {
    let mut app = app();
    press(&mut app, "p");

    let effects = press(&mut app, "7");

    assert!(effects.is_empty(), "{effects:?}");
    assert!(shows(&app, "> 7"));
    assert!(shows(&app, "No matches"));
}

#[test]
fn a_number_picks_that_entry_and_the_row_changes_at_once() {
    let mut app = app();
    press(&mut app, "s");
    app.handle(states_loaded(DES)).unwrap();

    let write = only_write(press(&mut app, "3"));

    assert_eq!(write.identifier, FIRST);
    assert!(matches!(&write.edit, Edit::State(state) if state.name == "Todo"));
    assert!(matches!(&write.previous, Edit::State(state) if state.name == "In Progress"));
    assert!(!shows(&app, "State of"));
    assert!(row(&app, FIRST).unwrap().contains("○ Todo"));
}

#[test]
fn enter_chooses_the_selected_entry_and_arrows_move_it() {
    let mut app = app();
    press(&mut app, "p");

    app.handle(key(KeyCode::Down)).unwrap();
    app.handle(key(KeyCode::Down)).unwrap();
    app.handle(key(KeyCode::Up)).unwrap();
    let effects = app.handle(key(KeyCode::Enter)).unwrap();

    let write = only_write(effects);
    assert_eq!(write.edit, Edit::Priority(Priority::Medium));
    assert_eq!(
        write.edit.update(),
        IssueUpdate {
            priority: Some(Priority::Medium),
            ..IssueUpdate::default()
        }
    );
    assert!(row(&app, FIRST).unwrap().contains("Medium"));
}

#[test]
fn choosing_the_value_the_issue_has_sends_nothing() {
    let mut app = app();
    press(&mut app, "p");

    let effects = app.handle(key(KeyCode::Enter)).unwrap();

    assert!(effects.is_empty(), "{effects:?}");
    assert!(!shows(&app, "Priority of"));
}

#[test]
fn esc_closes_a_picker_without_an_edit() {
    let mut app = app();
    press(&mut app, "p");

    let effects = app.handle(key(KeyCode::Esc)).unwrap();

    assert!(effects.is_empty());
    assert!(!shows(&app, "Priority of"));
    assert!(row(&app, FIRST).unwrap().contains("High"));
}

#[test]
fn the_priority_picker_lists_every_priority_with_the_current_one_marked() {
    let mut app = app();

    let effects = press(&mut app, "p");

    assert!(effects.is_empty(), "{effects:?}");
    assert_snapshot!(app.backend());
}

#[test]
fn the_assignee_picker_offers_me_and_unassigned_before_the_members_load() {
    let mut app = app();

    let effects = press(&mut app, "a");

    assert_eq!(
        effects,
        vec![Effect::LoadTeamMembers {
            team_id: DES.to_owned()
        }]
    );
    assert_snapshot!(app.backend());
}

#[test]
fn the_assignee_picker_lists_the_team_after_me_and_unassigned() {
    let mut app = app();
    press(&mut app, "a");

    app.handle(members_loaded(DES)).unwrap();

    assert_snapshot!(app.backend());
}

#[test]
fn unassigning_clears_the_assignee_rather_than_leaving_it() {
    let mut app = app();
    press(&mut app, "a");

    let write = only_write(press(&mut app, "2"));

    assert_eq!(write.edit, Edit::Assignee(None));
    assert_eq!(
        write.edit.update(),
        IssueUpdate {
            assignee_id: Patch::Clear,
            ..IssueUpdate::default()
        }
    );
    assert_eq!(row(&app, FIRST), None, "no longer assigned to the viewer");
}

#[test]
fn a_failed_member_load_says_so_in_the_picker_and_the_notice() {
    let mut app = app();
    press(&mut app, "a");

    app.handle(Event::TeamMembersLoaded {
        team_id: DES.to_owned(),
        result: Err("Linear did not answer in time".to_owned()),
    })
    .unwrap();

    assert!(shows(&app, "Could not load: Linear did not answer in time"));
    assert!(shows(
        &app,
        "Could not load the members of DES: Linear did not answer in time"
    ));
    assert!(shows(&app, "Unassigned"));
}

#[test]
fn pickers_open_from_the_detail_too() {
    let mut app = app();
    app.handle(key(KeyCode::Enter)).unwrap();

    press(&mut app, "p");
    let write = only_write(press(&mut app, "1"));

    assert_eq!(write.edit, Edit::Priority(Priority::Urgent));
    assert!(shows(&app, "Priority  !!! Urgent"), "{:#?}", screen(&app));
}

#[test]
fn focus_follows_the_edited_issue_when_its_group_reorders() {
    let mut app = app();
    press(&mut app, "j");
    let second = row(&app, "ENG-51").unwrap();
    assert!(second.contains('▸'));

    press(&mut app, "p1");

    assert!(row(&app, "ENG-51").unwrap().contains('▸'));
}

/// Each edit with the keys that make it on the first issue.
const EDITS: [(&str, &str); 3] = [("state", "s3"), ("assignee", "a3"), ("priority", "p1")];

fn edited(keys: &str) -> (App<TestBackend>, Write) {
    let mut app = app();
    let mut effects = press(&mut app, &keys[..1]);
    effects.retain(|effect| {
        !matches!(
            effect,
            Effect::LoadWorkflowStates { .. } | Effect::LoadTeamMembers { .. }
        )
    });
    assert!(effects.is_empty());
    app.handle(states_loaded(DES)).unwrap();
    app.handle(members_loaded(DES)).unwrap();
    let write = only_write(press(&mut app, &keys[1..]));
    (app, write)
}

#[test]
fn each_edit_sends_one_mutation_and_keeps_linears_answer() {
    for (name, keys) in EDITS {
        let (mut app, write) = edited(keys);

        let effects = app.handle(accepted(&write)).unwrap();

        assert!(effects.is_empty(), "{name}: {effects:?}");
        assert_snapshot!(format!("{name}_accepted"), app.backend());
    }
}

#[test]
fn each_rejected_edit_restores_the_previous_value_with_the_error() {
    for (name, keys) in EDITS {
        let (mut app, write) = edited(keys);

        app.handle(rejected(&write)).unwrap();

        let first = row(&app, FIRST).unwrap();
        assert!(first.contains("◐ In Progress"), "{name}: {first}");
        assert!(first.contains("High"), "{name}: {first}");
        assert!(first.contains('✘'), "{name}: {first}");
        assert!(
            shows(&app, &format!("Could not change the {name} of {FIRST}")),
            "{name}"
        );
        assert_snapshot!(format!("{name}_rejected"), app.backend());
    }
}

#[test]
fn the_detail_says_why_the_last_edit_was_not_saved() {
    let mut app = wide();
    let write = only_write(press(&mut app, "p1"));

    app.handle(rejected(&write)).unwrap();

    assert_snapshot!(app.backend());
}

#[test]
fn the_next_edit_of_the_issue_clears_its_error() {
    let (mut app, write) = edited("p1");
    app.handle(rejected(&write)).unwrap();

    press(&mut app, "p3");

    assert!(!row(&app, FIRST).unwrap().contains('✘'));
}

#[test]
fn a_rollback_leaves_a_field_that_moved_on_since() {
    let mut store = Store::default();
    let issue = demo_issue(FIRST);
    store.upsert_issue(issue.clone());
    let write = Write {
        issue_id: issue.id.clone(),
        identifier: FIRST.to_owned(),
        edit: Edit::Priority(Priority::Urgent),
        previous: Edit::Priority(issue.priority),
    };
    store.apply_edit(&issue.id, &write.edit);
    let mut newer = issue.clone();
    newer.priority = Priority::Low;
    newer.updated_at += jiff::SignedDuration::from_secs(60);
    store.upsert_issue(newer);

    let restored = store.roll_back(&write);

    assert!(!restored);
    assert_eq!(store.issue(&issue.id).unwrap().priority, Priority::Low);
}

#[test]
fn edits_through_the_fake_client_build_on_each_other() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap();
    let issue = demo_issue("ENG-51");
    let first = IssueUpdate {
        priority: Some(Priority::Urgent),
        ..IssueUpdate::default()
    };
    let second = IssueUpdate {
        assignee_id: Patch::Clear,
        ..IssueUpdate::default()
    };

    runtime
        .block_on(FakeClient.update_issue(&issue.id, &first))
        .unwrap();
    let after = runtime
        .block_on(FakeClient.update_issue(&issue.id, &second))
        .unwrap();

    assert_eq!(after.priority, Priority::Urgent);
    assert_eq!(after.assignee_id, None);
    assert!(after.updated_at > issue.updated_at);
}
