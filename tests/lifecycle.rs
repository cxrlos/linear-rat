//! Archiving, unarchiving, trashing and subscribing to an issue through the
//! app driver: each shows at once, sends one mutation, and rolls back when
//! Linear rejects it. Trash waits for the confirmation modal.

// clippy's allow-unwrap-in-tests does not cover helpers outside #[test] functions.
#![allow(clippy::unwrap_used)]

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use insta::assert_snapshot;
use linear_rat::app::{App, Effect, Event, Lifecycle, LifecycleWrite, Modal, Screen, State};
use linear_rat::linear::model::Issue;
use linear_rat::linear::{FakeClient, IssueDetail, LinearClient};
use ratatui::backend::TestBackend;

/// The issue My Issues focuses first in the demo, which the viewer is
/// subscribed to.
const FIRST: &str = "DES-34";
/// The one after it.
const SECOND: &str = "ENG-51";

const REJECTED: &str = "Linear returned errors: Entity not found (INVALID_INPUT)";

fn app() -> App<TestBackend> {
    App::new(TestBackend::new(80, 24), State::demo()).unwrap()
}

fn key_event(c: char) -> Event {
    let code = match c {
        '\n' => KeyCode::Enter,
        '\x1b' => KeyCode::Esc,
        c => KeyCode::Char(c),
    };
    Event::Key(KeyEvent::new(code, KeyModifiers::NONE))
}

/// Presses each key of `keys` (`\n` is Enter, `\x1b` Esc) and returns every
/// effect they asked for.
fn press(app: &mut App<TestBackend>, keys: &str) -> Vec<Effect> {
    keys.chars()
        .flat_map(|c| app.handle(key_event(c)).unwrap())
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

fn shows(app: &App<TestBackend>, text: &str) -> bool {
    screen(app).iter().any(|line| line.contains(text))
}

fn stored(app: &App<TestBackend>, identifier: &str) -> Option<Issue> {
    app.state()
        .store
        .issues()
        .find(|issue| issue.identifier == identifier)
        .cloned()
}

fn focused(app: &App<TestBackend>) -> String {
    app.state().focused_issue().unwrap().identifier.clone()
}

fn in_my_issues(app: &App<TestBackend>, identifier: &str) -> bool {
    app.state()
        .my_issues
        .groups()
        .iter()
        .flat_map(|group| &group.issues)
        .any(|issue| issue.identifier == identifier)
}

/// The only effect, which must be a lifecycle change.
fn only_change(effects: Vec<Effect>) -> LifecycleWrite {
    match effects.as_slice() {
        [Effect::ChangeLifecycle(write)] => write.clone(),
        other => panic!("expected exactly one lifecycle change, got {other:?}"),
    }
}

fn answered(write: &LifecycleWrite, result: Result<(), &str>) -> Event {
    Event::LifecycleChanged {
        write: write.clone(),
        result: result.map_err(str::to_owned),
    }
}

fn notice(app: &App<TestBackend>) -> String {
    app.state().notice.clone().unwrap_or_default()
}

// Archive and unarchive.

#[test]
fn shift_a_archives_the_focused_issue_and_it_leaves_my_issues_at_once() {
    let mut app = app();

    let write = only_change(press(&mut app, "A"));

    assert_eq!(write.change, Lifecycle::Archive);
    assert_eq!(write.identifier, FIRST);
    assert!(!in_my_issues(&app, FIRST));
    assert!(stored(&app, FIRST).unwrap().archived_at.is_some());
    assert_eq!(focused(&app), SECOND);
    app.handle(answered(&write, Ok(()))).unwrap();
    assert!(!in_my_issues(&app, FIRST));
}

#[test]
fn a_rejected_archive_brings_the_issue_back_with_the_error() {
    let mut app = app();
    let write = only_change(press(&mut app, "A"));

    app.handle(answered(&write, Err(REJECTED))).unwrap();

    assert!(in_my_issues(&app, FIRST));
    assert_eq!(stored(&app, FIRST).unwrap().archived_at, None);
    assert!(notice(&app).starts_with("Could not archive DES-34:"));
    assert!(app.state().row_errors.contains_key(&write.issue_id));
}

#[test]
fn unarchive_is_unavailable_on_an_issue_that_is_not_archived() {
    let mut app = app();

    assert!(press(&mut app, "U").is_empty());
    assert_eq!(stored(&app, FIRST).unwrap().archived_at, None);
}

/// The app showing DES-34 alone in the results, archived.
fn archived_in_results() -> (App<TestBackend>, String) {
    archived_in_results_at(80, 24)
}

fn archived_in_results_at(columns: u16, rows: u16) -> (App<TestBackend>, String) {
    let mut app = App::new(TestBackend::new(columns, rows), State::demo()).unwrap();
    // A wide layout also asks for the detail of the issue focused next.
    let mut effects = press(&mut app, "A");
    effects.retain(|effect| matches!(effect, Effect::ChangeLifecycle(_)));
    let write = only_change(effects);
    app.handle(answered(&write, Ok(()))).unwrap();
    // The palette jumps to an issue My Issues no longer lists in the results.
    press(&mut app, ":DES-34\n");
    assert_eq!(app.state().screen, Screen::Results);
    assert_eq!(focused(&app), FIRST);
    (app, write.issue_id)
}

#[test]
fn shift_u_unarchives_an_archived_issue_and_it_comes_back_to_my_issues() {
    let (mut app, _) = archived_in_results();

    let write = only_change(press(&mut app, "U"));

    assert_eq!(write.change, Lifecycle::Unarchive);
    assert_eq!(stored(&app, FIRST).unwrap().archived_at, None);
    assert!(in_my_issues(&app, FIRST));
}

#[test]
fn a_rejected_unarchive_archives_the_issue_again() {
    let (mut app, _) = archived_in_results();
    let archived_at = stored(&app, FIRST).unwrap().archived_at;
    let write = only_change(press(&mut app, "U"));

    app.handle(answered(&write, Err(REJECTED))).unwrap();

    assert_eq!(stored(&app, FIRST).unwrap().archived_at, archived_at);
    assert!(!in_my_issues(&app, FIRST));
    assert!(notice(&app).starts_with("Could not unarchive DES-34:"));
}

#[test]
fn the_detail_of_an_archived_issue_says_so() {
    let (app, _) = archived_in_results_at(200, 40);

    assert!(shows(&app, "Archived  2026-"), "{:#?}", screen(&app));
}

// Trash.

#[test]
fn shift_x_asks_to_confirm_and_trashes_nothing_until_then() {
    let mut app = app();

    let effects = press(&mut app, "X");

    assert!(effects.is_empty());
    assert!(matches!(app.state().modals.top(), Some(Modal::Confirm(_))));
    assert!(stored(&app, FIRST).is_some());
    assert!(shows(&app, "Trash DES-34"));
    assert_snapshot!(app.backend());
}

#[test]
fn esc_or_n_leaves_the_issue_untouched() {
    for cancel in ["\x1b", "n"] {
        let mut app = app();
        press(&mut app, "X");

        let effects = press(&mut app, cancel);

        assert!(effects.is_empty(), "{cancel:?}");
        assert!(app.state().modals.top().is_none(), "{cancel:?}");
        assert!(in_my_issues(&app, FIRST), "{cancel:?}");
        assert_eq!(focused(&app), FIRST);
    }
}

#[test]
fn y_or_enter_trashes_the_issue_at_once() {
    for confirm in ["y", "\n"] {
        let mut app = app();
        press(&mut app, "X");

        let write = only_change(press(&mut app, confirm));

        assert_eq!(write.change, Lifecycle::Trash, "{confirm:?}");
        assert!(app.state().modals.top().is_none());
        assert!(stored(&app, FIRST).is_none());
        assert_eq!(focused(&app), SECOND);
        app.handle(answered(&write, Ok(()))).unwrap();
        assert!(stored(&app, FIRST).is_none());
    }
}

#[test]
fn a_rejected_trash_restores_the_issue_with_the_error() {
    let mut app = app();
    let before = stored(&app, FIRST).unwrap();
    press(&mut app, "X");
    let write = only_change(press(&mut app, "y"));

    app.handle(answered(&write, Err(REJECTED))).unwrap();

    assert_eq!(stored(&app, FIRST), Some(before));
    assert!(in_my_issues(&app, FIRST));
    assert!(notice(&app).starts_with("Could not trash DES-34:"));
}

#[test]
fn the_palette_offers_trash_behind_the_confirmation() {
    let mut app = app();

    let effects = press(&mut app, ":trash\n");

    assert!(effects.is_empty());
    assert!(matches!(app.state().modals.top(), Some(Modal::Confirm(_))));
}

// Subscribe and unsubscribe.

#[test]
fn shift_s_toggles_the_viewers_subscription() {
    let mut app = app();
    assert!(stored(&app, FIRST).unwrap().subscribed);

    let write = only_change(press(&mut app, "S"));
    assert_eq!(write.change, Lifecycle::Unsubscribe);
    assert!(!stored(&app, FIRST).unwrap().subscribed);
    app.handle(answered(&write, Ok(()))).unwrap();

    let write = only_change(press(&mut app, "S"));
    assert_eq!(write.change, Lifecycle::Subscribe);
    assert!(stored(&app, FIRST).unwrap().subscribed);
}

#[test]
fn a_rejected_unsubscribe_subscribes_again() {
    let mut app = app();
    let write = only_change(press(&mut app, "S"));

    app.handle(answered(&write, Err(REJECTED))).unwrap();

    assert!(stored(&app, FIRST).unwrap().subscribed);
    assert!(notice(&app).starts_with("Could not unsubscribe from DES-34:"));
}

#[test]
fn the_detail_shows_whether_the_viewer_is_subscribed() {
    let mut app = App::new(TestBackend::new(200, 40), State::demo()).unwrap();
    let issue_id = stored(&app, FIRST).unwrap().id;
    app.handle(Event::IssueDetailLoaded {
        result: Ok(IssueDetail::demo(&issue_id)),
        issue_id,
    })
    .unwrap();
    assert!(shows(&app, "Updates   Subscribed"), "{:#?}", screen(&app));

    press(&mut app, "S");

    assert!(shows(&app, "Updates   Not subscribed"));
    assert_snapshot!(app.backend());
}

// Team Issues.

/// Carries out every Team Issues load in `effects` with the fake client.
async fn settle(app: &mut App<TestBackend>, effects: Vec<Effect>) {
    let client = FakeClient;
    let mut queue = std::collections::VecDeque::from(effects);
    while let Some(effect) = queue.pop_front() {
        let event = match effect {
            Effect::LoadTeams => Event::TeamsLoaded(Ok(client.teams().await.unwrap())),
            Effect::LoadTeamOptions(team_id) => {
                Event::TeamOptionsLoaded(Ok(client.team_options(&team_id).await.unwrap()))
            }
            Effect::LoadTeamIssues { query, after } => {
                let result = Ok(client.team_issues(&query, after.as_deref()).await.unwrap());
                Event::TeamIssuesLoaded {
                    query,
                    after,
                    result,
                }
            }
            Effect::CountTeamIssues(query) => {
                let result = Ok(client.count_team_issues(&query).await.unwrap());
                Event::TeamIssuesCounted { query, result }
            }
            _ => continue,
        };
        queue.extend(app.handle(event).unwrap());
    }
}

#[tokio::test]
async fn an_archived_issue_leaves_team_issues_and_comes_back_in_place_on_failure() {
    let mut app = app();
    let effects = press(&mut app, "2");
    settle(&mut app, effects).await;
    press(&mut app, "j");
    let ids = app.state().team_issues.ids().to_vec();
    let archived = app.state().focused_issue().unwrap().id.clone();

    let write = only_change(press(&mut app, "A"));

    assert!(!app.state().team_issues.ids().contains(&archived));
    app.handle(answered(&write, Err(REJECTED))).unwrap();
    assert_eq!(app.state().team_issues.ids(), ids);
}

#[tokio::test]
async fn a_trashed_issue_leaves_team_issues_and_comes_back_on_failure() {
    let mut app = app();
    let effects = press(&mut app, "2");
    settle(&mut app, effects).await;
    let ids = app.state().team_issues.ids().to_vec();
    let trashed = app.state().focused_issue().unwrap().id.clone();
    press(&mut app, "X");

    let write = only_change(press(&mut app, "y"));

    assert!(!app.state().team_issues.ids().contains(&trashed));
    app.handle(answered(&write, Err(REJECTED))).unwrap();
    assert_eq!(app.state().team_issues.ids(), ids);
}

// The demo.

#[tokio::test]
async fn the_demo_client_archives_trashes_and_subscribes() {
    let client = FakeClient;
    let issue = demo_issue(FIRST);

    client.unsubscribe_from_issue(&issue.id).await.unwrap();
    client.archive_issue(&issue.id).await.unwrap();
    client.unarchive_issue(&issue.id).await.unwrap();
    client.trash_issue(&issue.id).await.unwrap();

    let existing = client
        .existing_issues(std::slice::from_ref(&issue.id))
        .await
        .unwrap();
    assert!(existing.is_empty());
    assert!(client.archive_issue(&issue.id).await.is_err());
}

fn demo_issue(identifier: &str) -> Issue {
    linear_rat::linear::demo::workspace()
        .issues
        .into_iter()
        .find(|issue| issue.identifier == identifier)
        .unwrap()
}
