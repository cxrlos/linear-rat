//! The Personal view of the Analytics screen through the app driver: lazy
//! loading when the screen opens, the range picker, caching in the store and
//! what the view renders from the demo workspace.

// clippy's allow-unwrap-in-tests does not cover helpers outside #[test] functions.
#![allow(clippy::unwrap_used)]

use std::future;
use std::time::Duration;

use crossterm::event::{Event as TerminalEvent, KeyCode, KeyEvent, KeyModifiers};
use futures_util::stream::{self, StreamExt};
use insta::assert_snapshot;
use jiff::Timestamp;
use jiff::tz::{Offset, TimeZone};
use linear_rat::app::{App, Effect, Event, State};
use linear_rat::linear::model::Estimation;
use linear_rat::linear::{AnalyticsIssues, AnalyticsWindow, FakeClient};
use linear_rat::runtime;
use ratatui::backend::TestBackend;

/// The Monday twelve weeks back from the demo's now, at midnight UTC.
const TWELVE_WEEKS: &str = "2026-08-17T00:00:00Z";
/// The Monday twenty-six weeks back.
const TWENTY_SIX_WEEKS: &str = "2026-05-11T00:00:00Z";

fn sized(columns: u16, rows: u16) -> App<TestBackend> {
    App::new(TestBackend::new(columns, rows), State::demo()).unwrap()
}

fn key(c: char) -> Event {
    Event::Key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE))
}

fn at(text: &str) -> Timestamp {
    text.parse().unwrap()
}

fn load(since: &str, until: Option<&str>) -> Effect {
    Effect::LoadAnalytics(AnalyticsWindow {
        since: at(since),
        until: until.map(at),
    })
}

/// Hands `app` the demo's answer to every load in `effects`, and to the
/// loads those answers ask for.
fn deliver(app: &mut App<TestBackend>, effects: Vec<Effect>) {
    deliver_with(app, effects, |loaded| loaded);
}

fn deliver_with(
    app: &mut App<TestBackend>,
    effects: Vec<Effect>,
    change: fn(AnalyticsIssues) -> AnalyticsIssues,
) {
    for effect in effects {
        let Effect::LoadAnalytics(window) = effect else {
            continue;
        };
        let result = Ok(change(AnalyticsIssues::demo(window)));
        let more = app
            .handle(Event::AnalyticsLoaded { window, result })
            .unwrap();
        deliver_with(app, more, change);
    }
}

/// Analytics open on the demo workspace with its loads delivered.
fn opened(columns: u16, rows: u16) -> App<TestBackend> {
    let mut app = sized(columns, rows);
    let effects = app.handle(key('3')).unwrap();
    deliver(&mut app, effects);
    app
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

#[test]
fn opening_analytics_asks_for_the_default_range_up_to_now() {
    let mut app = sized(80, 24);

    let effects = app.handle(key('3')).unwrap();

    assert_eq!(effects, [load(TWELVE_WEEKS, None)]);
}

#[test]
fn weeks_start_on_monday_in_the_viewers_time_zone() {
    let state = State::demo().with_time_zone(TimeZone::fixed(Offset::constant(10)));
    let mut app = App::new(TestBackend::new(80, 24), state).unwrap();

    let effects = app.handle(key('3')).unwrap();

    assert_eq!(effects, [load("2026-08-16T14:00:00Z", None)]);
}

#[test]
fn a_longer_range_loads_only_the_weeks_it_adds() {
    let mut app = opened(80, 24);

    let effects = app.handle(key(']')).unwrap();

    assert_eq!(effects, [load(TWENTY_SIX_WEEKS, Some(TWELVE_WEEKS))]);
}

#[test]
fn a_range_already_loaded_loads_nothing() {
    let mut app = opened(80, 24);

    assert!(app.handle(key('[')).unwrap().is_empty());
    assert!(app.handle(key(']')).unwrap().is_empty());
    let effects = app.handle(key(']')).unwrap();
    deliver(&mut app, effects);
    for step in ['[', '[', ']', ']'] {
        let effects = app.handle(key(step)).unwrap();
        assert!(effects.is_empty(), "{step}: {effects:?}");
    }
}

#[test]
fn coming_back_to_analytics_loads_nothing_new() {
    let mut app = opened(80, 24);

    assert!(app.handle(key('1')).unwrap().is_empty());
    assert!(app.handle(key('3')).unwrap().is_empty());
}

#[test]
fn a_range_asked_for_is_not_asked_for_again_while_it_loads() {
    let mut app = sized(80, 24);
    app.handle(key('3')).unwrap();

    let longer = app.handle(key(']')).unwrap();
    let shorter = app.handle(key('[')).unwrap();
    let longer_again = app.handle(key(']')).unwrap();

    assert_eq!(longer, [load(TWENTY_SIX_WEEKS, Some(TWELVE_WEEKS))]);
    assert!(shorter.is_empty(), "{shorter:?}");
    assert!(longer_again.is_empty(), "{longer_again:?}");
}

#[test]
fn a_failed_load_is_shown_and_asked_for_again_next_time() {
    let mut app = sized(80, 24);
    let effects = app.handle(key('3')).unwrap();
    let Effect::LoadAnalytics(window) = effects[0] else {
        panic!("{effects:?}");
    };

    app.handle(Event::AnalyticsLoaded {
        window,
        result: Err("could not reach Linear".to_owned()),
    })
    .unwrap();

    let shown = screen(&app);
    assert!(
        shown.contains("Could not load analytics: could not reach Linear"),
        "{shown}"
    );
    app.handle(key('1')).unwrap();
    assert_eq!(
        app.handle(key('3')).unwrap(),
        [Effect::LoadAnalytics(window)]
    );
}

#[test]
fn opening_analytics_before_my_issues_load_waits_for_them() {
    let mut app = App::new(TestBackend::new(80, 24), State::loading(None)).unwrap();

    assert!(app.handle(key('3')).unwrap().is_empty());
    let effects = app
        .handle(Event::MyIssuesLoaded(Ok(
            linear_rat::linear::AssignedIssues::demo(),
        )))
        .unwrap();

    assert_eq!(effects, [load(TWELVE_WEEKS, None)]);
}

#[test]
fn the_view_opens_on_the_configured_range() {
    let loaded = linear_rat::config::parse(
        "[analytics]\ndefault-range-weeks = 4\n",
        std::path::Path::new("config.toml"),
    );
    let state = State::demo().with_config(&loaded);
    let mut app = App::new(TestBackend::new(80, 24), state).unwrap();

    let effects = app.handle(key('3')).unwrap();

    assert_eq!(effects, [load("2026-10-12T00:00:00Z", None)]);
}

fn with_range(columns: u16, rows: u16, steps: &str) -> App<TestBackend> {
    let mut app = opened(columns, rows);
    for step in steps.chars() {
        let effects = app.handle(key(step)).unwrap();
        deliver(&mut app, effects);
    }
    app
}

#[test]
fn four_weeks_at_80x24() {
    assert_snapshot!(with_range(80, 24, "[").backend());
}

#[test]
fn twelve_weeks_at_80x24() {
    assert_snapshot!(with_range(80, 24, "").backend());
}

#[test]
fn twenty_six_weeks_at_80x24() {
    assert_snapshot!(with_range(80, 24, "]").backend());
}

#[test]
fn four_weeks_at_200x60() {
    assert_snapshot!(with_range(200, 60, "[").backend());
}

#[test]
fn twelve_weeks_at_200x60() {
    assert_snapshot!(with_range(200, 60, "").backend());
}

#[test]
fn twenty_six_weeks_at_200x60() {
    assert_snapshot!(with_range(200, 60, "]").backend());
}

#[test]
fn j_scrolls_down_to_the_splits_at_80x24() {
    let mut app = opened(80, 24);
    let top = screen(&app);

    for _ in 0..200 {
        app.handle(key('j')).unwrap();
    }

    let bottom = screen(&app);
    assert_ne!(top, bottom);
    assert!(bottom.contains("Completed by priority"), "{bottom}");
    assert_snapshot!(app.backend());
    for _ in 0..200 {
        app.handle(key('k')).unwrap();
    }
    assert_eq!(screen(&app), top);
}

fn scrolled_to_bottom(app: &mut App<TestBackend>) {
    for _ in 0..200 {
        app.handle(key('j')).unwrap();
    }
}

#[test]
fn a_resize_keeps_the_scrolled_view_on_its_content() {
    let mut app = with_range(80, 24, "]");
    scrolled_to_bottom(&mut app);
    let mut fresh = with_range(200, 60, "]");
    scrolled_to_bottom(&mut fresh);

    app.backend_mut().resize(200, 60);
    app.handle(Event::Resize {
        columns: 200,
        rows: 60,
    })
    .unwrap();

    assert_eq!(screen(&app), screen(&fresh));
    app.handle(key('k')).unwrap();
    fresh.handle(key('k')).unwrap();
    assert_eq!(screen(&app), screen(&fresh));
}

#[test]
fn a_shorter_range_keeps_the_scrolled_view_on_its_content() {
    let mut app = with_range(80, 24, "]");
    scrolled_to_bottom(&mut app);
    let mut fresh = with_range(80, 24, "[");
    scrolled_to_bottom(&mut fresh);

    for step in "[[".chars() {
        app.handle(key(step)).unwrap();
    }

    assert_eq!(screen(&app), screen(&fresh));
    app.handle(key('k')).unwrap();
    fresh.handle(key('k')).unwrap();
    assert_eq!(screen(&app), screen(&fresh));
}

#[test]
fn without_estimates_scope_is_counted_in_issues() {
    let mut app = sized(200, 60);
    let effects = app.handle(key('3')).unwrap();
    deliver_with(&mut app, effects, |mut loaded| {
        for team in &mut loaded.teams {
            team.estimation = Estimation::NotUsed;
        }
        loaded
    });

    let shown = screen(&app);
    assert!(!shown.contains("Points"), "{shown}");
    assert!(!shown.contains(" pts"), "{shown}");
    assert!(shown.contains("scope is counted in issues"), "{shown}");
}

#[test]
fn with_estimates_points_show_per_week() {
    let shown = screen(&opened(200, 60));

    assert!(shown.contains("Points"), "{shown}");
    assert!(shown.contains(" pts"), "{shown}");
}

#[test]
fn the_cheatsheet_lists_the_analytics_keys() {
    let mut app = opened(80, 24);

    app.handle(key('?')).unwrap();

    let shown = screen(&app);
    for action in ["Show a shorter range", "Show a longer range", "Scroll down"] {
        assert!(shown.contains(action), "{action}:\n{shown}");
    }
}

#[test]
fn the_range_keys_can_be_remapped() {
    let loaded = linear_rat::config::parse(
        "[keys.analytics]\nlonger-range = \"L\"\n",
        std::path::Path::new("config.toml"),
    );
    assert!(loaded.errors.is_empty(), "{:?}", loaded.errors);
    let mut app = App::new(TestBackend::new(80, 24), State::demo().with_config(&loaded)).unwrap();
    let effects = app.handle(key('3')).unwrap();
    deliver(&mut app, effects);

    assert!(app.handle(key(']')).unwrap().is_empty());
    assert_eq!(
        app.handle(key('L')).unwrap(),
        [load(TWENTY_SIX_WEEKS, Some(TWELVE_WEEKS))]
    );
}

#[tokio::test]
async fn the_runtime_loads_analytics_through_the_client_when_the_screen_opens() {
    let mut app = sized(80, 24);
    let three = KeyEvent::new(KeyCode::Char('3'), KeyModifiers::NONE);
    let input = stream::iter(vec![Ok(TerminalEvent::Key(three))]).chain(stream::pending());

    let running = runtime::run(&mut app, Some(&FakeClient), None, input, future::pending());
    let stopped = tokio::time::timeout(Duration::from_millis(200), running).await;

    assert!(stopped.is_err(), "the runtime stopped: {stopped:?}");
    // The tab row also carries the sync indicator of the runtime's polls.
    let below_tabs = |shown: String| shown.lines().skip(1).collect::<Vec<_>>().join("\n");
    assert_eq!(
        below_tabs(screen(&app)),
        below_tabs(screen(&opened(80, 24)))
    );
}
