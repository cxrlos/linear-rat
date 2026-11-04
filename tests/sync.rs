//! Keeping My Issues fresh, through the app driver: painting from the
//! snapshot, when polls fall due, how their results merge, and the sync
//! indicator.

// clippy's allow-unwrap-in-tests does not cover helpers outside #[test] functions.
#![allow(clippy::unwrap_used)]

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use jiff::{SignedDuration, Timestamp};
use linear_rat::app::{App, Effect, Event, LoadError, State, SyncFailure, TeamPoll};
use linear_rat::auth::KeySource;
use linear_rat::config::SyncSettings;
use linear_rat::linear::model::Issue;
use linear_rat::linear::{AssignedIssues, TeamChanges, demo};
use linear_rat::snapshot::SnapshotCache;
use ratatui::backend::TestBackend;

/// 2026-11-03T09:00:00Z, the local clock of these tests.
const START: Timestamp = Timestamp::constant(1_793_696_400, 0);

fn at(seconds: i64) -> Timestamp {
    START
        .checked_add(SignedDuration::from_secs(seconds))
        .unwrap()
}

fn app_with(state: State) -> App<TestBackend> {
    App::new(TestBackend::new(80, 24), state).unwrap()
}

fn key(c: char) -> Event {
    Event::Key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE))
}

fn screen(app: &App<TestBackend>) -> String {
    let buffer = app.backend().buffer();
    let width = usize::from(buffer.area.width);
    buffer
        .content()
        .chunks(width)
        .map(|row| row.iter().map(|cell| cell.symbol()).collect::<String>())
        .collect::<Vec<_>>()
        .join("\n")
}

fn tab_row(app: &App<TestBackend>) -> String {
    screen(app).lines().next().unwrap_or_default().to_owned()
}

fn polls(effects: &[Effect]) -> usize {
    effects
        .iter()
        .filter(|effect| matches!(effect, Effect::Poll(_)))
        .count()
}

/// The demo issue the viewer has focused first.
fn focused(app: &App<TestBackend>) -> Issue {
    app.state().my_issues.focused_issue().unwrap().clone()
}

/// A poll that found `issues` in their team.
fn found(issues: Vec<Issue>) -> Event {
    let team_id = issues[0].team_id.clone();
    Event::Polled {
        changes: vec![TeamChanges {
            team_id,
            newest: issues.iter().map(|issue| issue.updated_at).max(),
            issues,
            trashed: Vec::new(),
            cycles: Vec::new(),
            labels: Vec::new(),
            members: Vec::new(),
            teams: Vec::new(),
            as_of: demo::NOW,
        }],
        failure: None,
    }
}

fn nothing_new() -> Event {
    Event::Polled {
        changes: Vec::new(),
        failure: None,
    }
}

fn failed(failure: SyncFailure) -> Event {
    Event::Polled {
        changes: Vec::new(),
        failure: Some(failure),
    }
}

/// `issue` as Linear has it `minutes` later, changed by `change`.
fn later(issue: &Issue, minutes: i64, change: impl FnOnce(&mut Issue)) -> Issue {
    let mut issue = issue.clone();
    issue.updated_at = issue
        .updated_at
        .checked_add(SignedDuration::from_mins(minutes))
        .unwrap();
    change(&mut issue);
    issue
}

/// The demo app after its first tick and poll at [`START`], and what the
/// app asked for when that poll came back.
fn first_poll() -> (App<TestBackend>, Vec<Effect>) {
    let mut app = app_with(State::demo());
    let effects = app.handle(Event::Tick(START)).unwrap();
    assert_eq!(polls(&effects), 1, "{effects:?}");
    let effects = app.handle(nothing_new()).unwrap();
    (app, effects)
}

fn synced() -> App<TestBackend> {
    first_poll().0
}

fn cache_dir() -> PathBuf {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!(
        "sync-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::SeqCst)
    ))
}

#[test]
fn a_second_launch_paints_the_last_known_my_issues_before_any_request_returns() {
    let cache = SnapshotCache::new(cache_dir());
    let first = app_with(State::demo());
    cache.save(&first.state().store, Some(demo::NOW)).unwrap();
    let saved = cache.load().unwrap();

    let state = State::loading(Some(KeySource::Env)).with_snapshot(saved.store, saved.synced_at);
    let mut second = app_with(state);

    assert_eq!(screen(&second), screen(&first));
    let effects = second.handle(Event::Start).unwrap();
    assert_eq!(effects, [Effect::LoadMyIssues]);
}

#[test]
fn nothing_polls_until_the_snapshot_has_been_refreshed_by_the_load() {
    let saved = app_with(State::demo()).state().store.clone();
    let mut app = app_with(State::loading(None).with_snapshot(saved, None));

    let before = app.handle(Event::Tick(START)).unwrap();
    let loaded = app
        .handle(Event::MyIssuesLoaded(Ok(AssignedIssues::demo())))
        .unwrap();

    assert_eq!(polls(&before), 0);
    assert_eq!(polls(&loaded), 1);
}

#[test]
fn a_load_for_another_viewer_throws_the_snapshot_away() {
    let mut snapshot = app_with(State::demo()).state().store.clone();
    let stale = Issue {
        id: "issue-of-someone-else".to_owned(),
        ..snapshot.issues().next().unwrap().clone()
    };
    snapshot.upsert_issue(stale);
    let mut app = app_with(State::loading(None).with_snapshot(snapshot, None));
    let mut other = AssignedIssues::demo();
    other.viewer.id = "viewer-other".to_owned();

    app.handle(Event::MyIssuesLoaded(Ok(other))).unwrap();

    assert!(app.state().store.issue("issue-of-someone-else").is_none());
}

#[test]
fn a_load_for_the_same_viewer_in_another_workspace_throws_the_snapshot_away() {
    let mut snapshot = app_with(State::demo()).state().store.clone();
    let stale = Issue {
        id: "issue-of-another-workspace".to_owned(),
        ..snapshot.issues().next().unwrap().clone()
    };
    snapshot.upsert_issue(stale);
    let mut app = app_with(State::loading(None).with_snapshot(snapshot, None));
    let mut other = AssignedIssues::demo();
    other.workspace.id = "workspace-other".to_owned();

    app.handle(Event::MyIssuesLoaded(Ok(other))).unwrap();

    let store = &app.state().store;
    assert!(store.issue("issue-of-another-workspace").is_none());
    assert_eq!(store.workspace().unwrap().id, "workspace-other");
}

#[test]
fn the_first_poll_starts_each_team_without_a_watermark() {
    let mut app = app_with(State::demo());

    let effects = app.handle(Event::Tick(START)).unwrap();

    let [Effect::Poll(teams)] = effects.as_slice() else {
        panic!("{effects:?}");
    };
    assert_eq!(teams.len(), app.state().store.teams().count());
    assert!(teams.iter().all(|team| team.since.is_none()), "{teams:?}");
}

#[test]
fn later_polls_reach_back_two_minutes_before_the_team_watermark() {
    let mut app = synced();
    let issue = focused(&app);
    let newest = later(&issue, 10, |_| {});
    app.handle(found(vec![newest.clone()])).unwrap();

    let effects = app.handle(Event::Tick(at(15))).unwrap();

    let [Effect::Poll(teams)] = effects.as_slice() else {
        panic!("{effects:?}");
    };
    let since = newest
        .updated_at
        .checked_sub(SignedDuration::from_mins(2))
        .unwrap();
    assert!(
        teams.contains(&TeamPoll {
            team_id: issue.team_id.clone(),
            since: Some(since),
        }),
        "{teams:?}"
    );
}

#[test]
fn polls_come_every_active_interval_while_the_viewer_is_active() {
    let mut app = synced();

    let early = app.handle(Event::Tick(at(14))).unwrap();
    let due = app.handle(Event::Tick(at(15))).unwrap();

    assert_eq!(polls(&early), 0);
    assert_eq!(polls(&due), 1);
}

#[test]
fn an_idle_viewer_is_polled_at_the_idle_interval() {
    let mut app = synced();
    // Two minutes without a key press make the viewer idle.
    app.handle(Event::Tick(at(130))).unwrap();
    app.handle(nothing_new()).unwrap();

    let early = app.handle(Event::Tick(at(130 + 59))).unwrap();
    let due = app.handle(Event::Tick(at(130 + 60))).unwrap();

    assert_eq!(polls(&early), 0);
    assert_eq!(polls(&due), 1);
}

#[test]
fn a_key_press_makes_the_viewer_active_again() {
    let mut app = synced();
    app.handle(Event::Tick(at(130))).unwrap();
    app.handle(nothing_new()).unwrap();

    app.handle(key('j')).unwrap();
    let due = app.handle(Event::Tick(at(130 + 15))).unwrap();

    assert_eq!(polls(&due), 1);
}

#[test]
fn nothing_polls_while_the_terminal_has_lost_focus() {
    let mut app = synced();

    app.handle(Event::FocusLost).unwrap();
    let unfocused = app.handle(Event::Tick(at(600))).unwrap();
    let back = app.handle(Event::FocusGained).unwrap();

    assert_eq!(polls(&unfocused), 0);
    assert_eq!(polls(&back), 1);
}

#[test]
fn the_intervals_and_the_pause_come_from_the_sync_settings() {
    let mut state = State::demo();
    state.sync = SyncSettings {
        active_poll: Duration::from_secs(5),
        pause_when_unfocused: false,
        ..SyncSettings::default()
    };
    let mut app = app_with(state);
    app.handle(Event::Tick(START)).unwrap();
    app.handle(nothing_new()).unwrap();
    app.handle(Event::FocusLost).unwrap();

    let due = app.handle(Event::Tick(at(5))).unwrap();

    assert_eq!(polls(&due), 1);
}

#[test]
fn r_polls_now_and_not_twice_at_once() {
    let mut app = synced();

    let first = app.handle(key('r')).unwrap();
    let second = app.handle(key('r')).unwrap();

    assert_eq!(polls(&first), 1);
    assert_eq!(polls(&second), 0);
}

#[test]
fn an_issue_reassigned_away_from_the_viewer_leaves_my_issues_on_the_next_poll() {
    let mut app = synced();
    let issue = focused(&app);
    assert!(screen(&app).contains(&issue.identifier));

    let reassigned = later(&issue, 1, |issue| {
        issue.assignee_id = Some("member-someone-else".to_owned());
    });
    app.handle(found(vec![reassigned])).unwrap();

    assert!(
        !screen(&app).contains(&issue.identifier),
        "{}",
        screen(&app)
    );
    assert!(app.state().store.issue(&issue.id).is_some());
}

#[test]
fn an_older_copy_arriving_after_a_newer_one_never_overwrites_it() {
    let mut app = synced();
    let issue = focused(&app);
    let newer = later(&issue, 5, |issue| issue.title = "Newer title".to_owned());
    let older = later(&issue, 2, |issue| issue.title = "Older title".to_owned());

    app.handle(found(vec![newer])).unwrap();
    app.handle(Event::Tick(at(15))).unwrap();
    app.handle(found(vec![older])).unwrap();

    let shown = screen(&app);
    assert!(shown.contains("Newer title"), "{shown}");
    assert!(!shown.contains("Older title"), "{shown}");
}

#[test]
fn a_poll_after_the_active_cycle_ends_stops_showing_it_as_active() {
    let mut app = synced();
    let issue = focused(&app);
    assert!(app.state().my_issues.is_in_active_cycle(&issue));
    let cycle_id = issue.cycle_id.clone().unwrap();
    let ends_at = app
        .state()
        .store
        .cycles()
        .find(|cycle| cycle.id == cycle_id)
        .unwrap()
        .ends_at;

    let effects = app
        .handle(Event::Polled {
            changes: vec![TeamChanges {
                team_id: issue.team_id.clone(),
                issues: Vec::new(),
                trashed: Vec::new(),
                cycles: Vec::new(),
                labels: Vec::new(),
                members: Vec::new(),
                teams: Vec::new(),
                newest: None,
                as_of: ends_at.checked_add(SignedDuration::from_hours(1)).unwrap(),
            }],
            failure: None,
        })
        .unwrap();

    assert!(!app.state().my_issues.is_in_active_cycle(&issue));
    assert!(effects.contains(&Effect::SaveSnapshot), "{effects:?}");
}

#[test]
fn the_focused_issue_keeps_its_focus_when_a_poll_rebuilds_the_list() {
    let mut app = synced();
    app.handle(key('j')).unwrap();
    app.handle(key('j')).unwrap();
    let kept = focused(&app);
    let first = app.state().store.issues().next().unwrap().clone();
    let moved = later(&first, 1, |issue| issue.title = "Renamed".to_owned());

    app.handle(found(vec![moved])).unwrap();

    assert_eq!(focused(&app).id, kept.id);
}

#[test]
fn the_reconciliation_after_startup_drops_issues_that_disappeared() {
    let (mut app, effects) = first_poll();
    let issue = focused(&app);

    let [Effect::Reconcile(checked)] = effects.as_slice() else {
        panic!("{effects:?}");
    };
    let existing = checked
        .iter()
        .filter(|id| **id != issue.id)
        .cloned()
        .collect();
    let update = app
        .handle(Event::Reconciled {
            checked: checked.clone(),
            result: Ok(existing),
        })
        .unwrap();

    assert!(app.state().store.issue(&issue.id).is_none());
    assert!(!screen(&app).contains(&issue.identifier));
    assert_eq!(update, [Effect::SaveSnapshot]);
}

#[test]
fn the_reconciliation_comes_again_only_after_a_few_hours() {
    let (mut app, effects) = first_poll();
    let [Effect::Reconcile(checked)] = effects.as_slice() else {
        panic!("{effects:?}");
    };
    app.handle(Event::Reconciled {
        checked: checked.clone(),
        result: Ok(checked.clone()),
    })
    .unwrap();

    let reconciles = |effects: Vec<Effect>| {
        effects
            .iter()
            .filter(|effect| matches!(effect, Effect::Reconcile(_)))
            .count()
    };
    let mut soon = 0;
    let mut hours_later = 0;
    for (tick, count) in [(at(3600), &mut soon), (at(3 * 3600 + 1), &mut hours_later)] {
        app.handle(Event::Tick(tick)).unwrap();
        *count += reconciles(app.handle(nothing_new()).unwrap());
        *count += reconciles(app.handle(Event::Tick(tick)).unwrap());
    }

    assert_eq!(soon, 0);
    assert_eq!(hours_later, 1);
}

#[test]
fn the_sync_indicator_says_how_long_ago_the_last_poll_finished() {
    let mut app = synced();

    app.handle(Event::Tick(at(12))).unwrap();

    let tabs = tab_row(&app);
    assert!(tabs.starts_with(" [1] My Issues"), "{tabs}");
    assert!(tabs.trim_end().ends_with("synced 12s ago"), "{tabs}");
}

#[test]
fn the_sync_indicator_says_when_sync_is_failing_or_throttled() {
    let mut app = synced();
    app.handle(Event::Tick(at(15))).unwrap();
    app.handle(failed(SyncFailure::Failed("connection refused".to_owned())))
        .unwrap();
    app.handle(Event::Tick(at(20))).unwrap();
    let failing = tab_row(&app);

    app.handle(failed(SyncFailure::Throttled)).unwrap();
    let throttled = tab_row(&app);

    app.handle(Event::Tick(at(30))).unwrap();
    app.handle(nothing_new()).unwrap();
    let recovered = tab_row(&app);

    assert!(
        failing.trim_end().ends_with("sync failing, synced 20s ago"),
        "{failing}"
    );
    assert!(
        throttled.trim_end().ends_with("throttled, synced 20s ago"),
        "{throttled}"
    );
    assert!(
        recovered.trim_end().ends_with("synced 0s ago"),
        "{recovered}"
    );
}

#[test]
fn the_sync_indicator_is_muted_when_fresh_and_accented_when_failing() {
    let mut app = synced();
    let fresh = indicator_style(&app);

    app.handle(Event::Tick(at(15))).unwrap();
    app.handle(failed(SyncFailure::Throttled)).unwrap();
    let alert = indicator_style(&app);

    let theme = &app.state().theme;
    assert_eq!(fresh.fg, Some(theme.muted));
    assert_eq!(alert.fg, Some(theme.accent));
}

/// The style of the last character of the tab row, where the indicator ends.
fn indicator_style(app: &App<TestBackend>) -> ratatui::style::Style {
    let buffer = app.backend().buffer();
    let row: Vec<_> = buffer
        .content()
        .iter()
        .take(usize::from(buffer.area.width))
        .collect();
    let last = row.iter().rev().find(|cell| cell.symbol() != " ").unwrap();
    last.style()
}

#[test]
fn a_failed_poll_says_why_once_in_the_notice() {
    let mut app = synced();
    app.handle(Event::Tick(at(15))).unwrap();

    app.handle(failed(SyncFailure::Failed("connection refused".to_owned())))
        .unwrap();

    assert!(
        screen(&app).contains("Sync failed: connection refused"),
        "{}",
        screen(&app)
    );
}

#[test]
fn an_unreachable_linear_keeps_the_snapshot_on_screen_and_marks_sync_failing() {
    let saved = app_with(State::demo()).state().store.clone();
    let mut app = app_with(State::loading(None).with_snapshot(saved, Some(at(-60))));
    let before = screen(&app);

    app.handle(Event::Tick(START)).unwrap();
    app.handle(Event::MyIssuesLoaded(Err(LoadError::Unreachable(
        "timed out".to_owned(),
    ))))
    .unwrap();

    let after = screen(&app);
    assert_eq!(after.lines().nth(3), before.lines().nth(3));
    assert!(
        tab_row(&app)
            .trim_end()
            .ends_with("sync failing, synced 1m ago")
    );
    assert!(after.contains("Sync failed: timed out"), "{after}");
}

fn loads(effects: &[Effect]) -> usize {
    effects
        .iter()
        .filter(|effect| matches!(effect, Effect::LoadMyIssues))
        .count()
}

/// An app painting the demo snapshot whose own load could not reach Linear
/// at [`START`].
fn unconfirmed_snapshot() -> App<TestBackend> {
    let saved = app_with(State::demo()).state().store.clone();
    let mut app = app_with(State::loading(None).with_snapshot(saved, Some(at(-60))));
    app.handle(Event::Tick(START)).unwrap();
    app.handle(Event::MyIssuesLoaded(Err(LoadError::Unreachable(
        "timed out".to_owned(),
    ))))
    .unwrap();
    app
}

#[test]
fn after_a_failed_load_the_snapshot_is_loaded_again_each_poll_interval_and_never_polled() {
    let mut app = unconfirmed_snapshot();

    let early = app.handle(Event::Tick(at(14))).unwrap();
    let due = app.handle(Event::Tick(at(15))).unwrap();
    let in_flight = app.handle(Event::Tick(at(30))).unwrap();
    app.handle(Event::MyIssuesLoaded(Err(LoadError::Unreachable(
        "timed out".to_owned(),
    ))))
    .unwrap();
    let again = app.handle(Event::Tick(at(45))).unwrap();

    assert_eq!(loads(&early), 0);
    assert_eq!(loads(&due), 1);
    assert_eq!(loads(&in_flight), 0);
    assert_eq!(loads(&again), 1);
    let all = [early, due, in_flight, again].concat();
    assert_eq!(polls(&all), 0, "{all:?}");
}

#[test]
fn r_loads_an_unconfirmed_snapshot_again_now() {
    let mut app = unconfirmed_snapshot();

    let effects = app.handle(key('r')).unwrap();

    assert_eq!(effects, [Effect::LoadMyIssues]);
}

#[test]
fn a_retried_load_for_another_workspace_throws_the_snapshot_away_and_then_polls() {
    let mut app = unconfirmed_snapshot();
    let snapshot_issue = focused(&app);
    app.handle(Event::Tick(at(15))).unwrap();
    let mut other = AssignedIssues::demo();
    other.workspace.id = "workspace-other".to_owned();
    other.issues.retain(|issue| issue.id != snapshot_issue.id);

    let effects = app.handle(Event::MyIssuesLoaded(Ok(other))).unwrap();

    assert!(app.state().store.issue(&snapshot_issue.id).is_none());
    assert_eq!(polls(&effects), 1, "{effects:?}");
}

#[test]
fn a_rejected_key_still_shows_its_error_screen_over_the_snapshot() {
    let saved = app_with(State::demo()).state().store.clone();
    let mut app = app_with(State::loading(Some(KeySource::Env)).with_snapshot(saved, None));

    app.handle(Event::MyIssuesLoaded(Err(LoadError::KeyRejected)))
        .unwrap();

    assert!(
        screen(&app).contains("API key rejected"),
        "{}",
        screen(&app)
    );
}

#[test]
fn no_reconciliation_starts_while_sync_is_failing() {
    let mut app = app_with(State::demo());
    app.handle(Event::Tick(START)).unwrap();

    let effects = app
        .handle(failed(SyncFailure::Failed("connection refused".to_owned())))
        .unwrap();
    let recovered = app
        .handle(Event::Tick(at(15)))
        .unwrap()
        .into_iter()
        .chain(app.handle(nothing_new()).unwrap())
        .collect::<Vec<_>>();

    assert!(effects.is_empty(), "{effects:?}");
    assert!(
        recovered
            .iter()
            .any(|effect| matches!(effect, Effect::Reconcile(_))),
        "{recovered:?}"
    );
}
