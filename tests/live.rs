//! Live updates through the app driver: when lrat goes live, how live events
//! merge into the store, and the polls they schedule.

// clippy's allow-unwrap-in-tests does not cover helpers outside #[test] functions.
#![allow(clippy::unwrap_used)]

use jiff::{SignedDuration, Timestamp};
use linear_rat::app::{App, Effect, Event, State};
use linear_rat::linear::model::{Comment, Issue};
use linear_rat::linear::{LiveEvent, PostedComment, TeamChanges, demo};
use ratatui::backend::TestBackend;

/// 2026-11-03T09:00:00Z, the local clock of these tests.
const START: Timestamp = Timestamp::constant(1_793_696_400, 0);

fn at(seconds: i64) -> Timestamp {
    START
        .checked_add(SignedDuration::from_secs(seconds))
        .unwrap()
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

fn polls(effects: &[Effect]) -> usize {
    effects
        .iter()
        .filter(|effect| matches!(effect, Effect::Poll(_)))
        .count()
}

fn go_live(effects: &[Effect]) -> Option<&Vec<String>> {
    effects.iter().find_map(|effect| match effect {
        Effect::GoLive(team_ids) => Some(team_ids),
        _ => None,
    })
}

fn nothing_new() -> Event {
    Event::Polled {
        changes: Vec::new(),
        failure: None,
    }
}

/// The demo app with `live` set, after its first tick and poll at
/// [`START`], and every effect it asked for on the way.
fn synced(live: bool) -> (App<TestBackend>, Vec<Effect>) {
    let mut state = State::demo();
    state.sync.live = live;
    let mut app = App::new(TestBackend::new(80, 24), state).unwrap();
    let mut effects = app.handle(Event::Tick(START)).unwrap();
    effects.extend(app.handle(nothing_new()).unwrap());
    (app, effects)
}

fn focused(app: &App<TestBackend>) -> Issue {
    app.state().my_issues.focused_issue().unwrap().clone()
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

/// A live `issueUpdated` carrying `issue`.
fn live_issue(issue: Issue) -> Event {
    Event::Live(LiveEvent::Issue(TeamChanges {
        team_id: issue.team_id.clone(),
        issues: vec![issue],
        trashed: Vec::new(),
        cycles: Vec::new(),
        labels: Vec::new(),
        members: Vec::new(),
        teams: Vec::new(),
        newest: None,
        as_of: demo::NOW,
    }))
}

#[test]
fn with_live_off_lrat_never_goes_live() {
    let (mut app, mut effects) = synced(false);
    for second in 1..=60 {
        effects.extend(app.handle(Event::Tick(at(second))).unwrap());
    }

    assert_eq!(go_live(&effects), None, "{effects:?}");
}

#[test]
fn with_live_on_lrat_goes_live_once_for_the_teams_it_polls() {
    let (mut app, mut effects) = synced(true);
    let polled: Vec<String> = app.state().store.teams().map(|t| t.id.clone()).collect();
    for second in 1..=30 {
        effects.extend(app.handle(Event::Tick(at(second))).unwrap());
    }

    let live: Vec<&Vec<String>> = effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::GoLive(team_ids) => Some(team_ids),
            _ => None,
        })
        .collect();
    assert_eq!(live, [&polled]);
    assert!(!polled.is_empty());
}

#[test]
fn every_connection_polls_at_once_to_catch_up() {
    let (mut app, _) = synced(true);
    app.handle(Event::Tick(at(1))).unwrap();

    let effects = app.handle(Event::Live(LiveEvent::Connected)).unwrap();
    assert_eq!(polls(&effects), 1, "{effects:?}");

    app.handle(nothing_new()).unwrap();
    app.handle(Event::Tick(at(2))).unwrap();
    let effects = app.handle(Event::Live(LiveEvent::Connected)).unwrap();
    assert_eq!(
        polls(&effects),
        1,
        "a reconnect catches up too: {effects:?}"
    );
}

#[test]
fn a_connection_while_a_poll_is_in_flight_polls_again_after_it() {
    let (mut app, _) = synced(true);
    app.handle(Event::Tick(at(15))).unwrap();

    let effects = app.handle(Event::Live(LiveEvent::Connected)).unwrap();
    assert_eq!(polls(&effects), 0, "{effects:?}");

    let effects = app.handle(nothing_new()).unwrap();
    assert_eq!(polls(&effects), 1, "{effects:?}");
}

#[test]
fn a_live_issue_update_shows_at_once() {
    let (mut app, _) = synced(true);
    let issue = focused(&app);
    let frames = app.frames_drawn();

    let effects = app
        .handle(live_issue(later(&issue, 1, |issue| {
            issue.title = "Renamed live".to_owned();
        })))
        .unwrap();

    assert!(app.frames_drawn() > frames);
    assert!(screen(&app).contains("Renamed live"), "{}", screen(&app));
    assert!(effects.contains(&Effect::SaveSnapshot), "{effects:?}");
}

#[test]
fn an_older_live_event_never_overwrites_newer_data() {
    let (mut app, _) = synced(true);
    let issue = focused(&app);
    let newer = later(&issue, 5, |issue| issue.title = "Newer title".to_owned());
    let older = later(&issue, 2, |issue| issue.title = "Older title".to_owned());

    app.handle(live_issue(newer)).unwrap();
    app.handle(live_issue(older)).unwrap();

    let shown = screen(&app);
    assert!(shown.contains("Newer title"), "{shown}");
    assert!(!shown.contains("Older title"), "{shown}");
}

#[test]
fn a_live_event_leaves_the_team_watermark_to_the_poll() {
    let (mut app, _) = synced(true);
    let issue = focused(&app);
    let before = app.state().store.watermark(&issue.team_id);

    app.handle(live_issue(later(&issue, 60, |_| {}))).unwrap();

    assert_eq!(app.state().store.watermark(&issue.team_id), before);
}

#[test]
fn live_events_schedule_one_debounced_poll() {
    let (mut app, _) = synced(true);
    let issue = focused(&app);
    app.handle(Event::Tick(at(1))).unwrap();
    app.handle(live_issue(later(&issue, 1, |_| {}))).unwrap();
    app.handle(Event::Tick(at(4))).unwrap();
    app.handle(live_issue(later(&issue, 2, |_| {}))).unwrap();

    let early = app.handle(Event::Tick(at(8))).unwrap();
    assert_eq!(polls(&early), 0, "the second event pushed the poll back");

    let due = app.handle(Event::Tick(at(9))).unwrap();
    assert_eq!(polls(&due), 1, "{due:?}");

    app.handle(nothing_new()).unwrap();
    let after = app.handle(Event::Tick(at(10))).unwrap();
    assert_eq!(polls(&after), 0, "{after:?}");
}

#[test]
fn a_live_comment_joins_the_comments_already_loaded() {
    let (mut app, _) = synced(true);
    let issue = focused(&app);
    app.handle(Event::IssueDetailLoaded {
        issue_id: issue.id.clone(),
        result: Ok(linear_rat::linear::IssueDetail::demo(&issue.id)),
    })
    .unwrap();
    let before = app.state().store.comments(&issue.id).unwrap().len();

    app.handle(Event::Live(LiveEvent::Comment(PostedComment {
        comment: Comment {
            id: "live-comment".to_owned(),
            issue_id: issue.id.clone(),
            author_id: None,
            body: "Seen live".to_owned(),
            created_at: demo::NOW,
        },
        author: None,
    })))
    .unwrap();

    assert_eq!(
        app.state().store.comments(&issue.id).unwrap().len(),
        before + 1
    );
}

#[test]
fn a_live_event_that_changes_nothing_schedules_no_poll() {
    let (mut app, _) = synced(true);
    let issue = focused(&app);
    let older = later(&issue, -60, |issue| issue.title = "Stale".to_owned());

    app.handle(live_issue(issue)).unwrap();
    app.handle(live_issue(older)).unwrap();

    let effects = app.handle(Event::Tick(at(10))).unwrap();
    assert_eq!(polls(&effects), 0, "{effects:?}");
}

#[test]
fn a_live_comment_schedules_no_poll() {
    let (mut app, _) = synced(true);
    let issue = focused(&app);

    app.handle(Event::Live(LiveEvent::Comment(PostedComment {
        comment: Comment {
            id: "live-comment".to_owned(),
            issue_id: issue.id.clone(),
            author_id: None,
            body: "Seen live".to_owned(),
            created_at: demo::NOW,
        },
        author: None,
    })))
    .unwrap();

    let effects = app.handle(Event::Tick(at(10))).unwrap();
    assert_eq!(polls(&effects), 0, "{effects:?}");
}
