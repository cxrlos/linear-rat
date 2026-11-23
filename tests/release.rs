//! The release notice: the check as an effect and its answer as an event,
//! the notice on the tab row and in the cheatsheet, the palette actions,
//! the runtime asking a fake GitHub, and the source itself.

// clippy's allow-unwrap-in-tests does not cover helpers outside #[test] functions.
#![allow(clippy::unwrap_used)]

use std::fs;
use std::future;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use futures_util::stream;
use insta::assert_snapshot;
use jiff::Timestamp;
use linear_rat::app::{App, Effect, Event, Release, State};
use linear_rat::linear::FakeClient;
use linear_rat::release::{
    self, Channel, Record, ReleaseCache, ReleaseCheck, ReleaseSource, release_url,
};
use linear_rat::runtime;
use ratatui::backend::TestBackend;
use semver::Version;
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn version(text: &str) -> Version {
    Version::parse(text).unwrap()
}

/// A release newer than any lrat will be.
fn newer() -> Version {
    version("99.0.0")
}

fn now() -> Timestamp {
    "2026-11-18T09:00:00Z".parse().unwrap()
}

fn hours_ago(hours: i64) -> Timestamp {
    now()
        .checked_sub(jiff::SignedDuration::from_hours(hours))
        .unwrap()
}

/// The demo state checking for releases through `channel`, with `record`
/// from the cache.
fn state(check: bool, channel: Channel, record: Record) -> State {
    State::demo().with_release(Release::new(check, channel, record))
}

fn app_at(width: u16, state: State) -> App<TestBackend> {
    App::new(TestBackend::new(width, 24), state).unwrap()
}

/// Wide enough for every tab and the notice.
fn app(state: State) -> App<TestBackend> {
    app_at(120, state)
}

/// `state` as if its last sync finished 12 seconds before [`now`], so the
/// first tick shows the sync indicator.
fn synced(state: State) -> State {
    let store = state.store.clone();
    let synced_at = now()
        .checked_sub(jiff::SignedDuration::from_secs(12))
        .unwrap();
    state.with_snapshot(store, Some(synced_at))
}

fn with_latest(latest: Version) -> Record {
    Record {
        checked_at: Some(hours_ago(1)),
        latest: Some(latest),
        skipped: None,
    }
}

fn key(c: char) -> Event {
    Event::Key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE))
}

fn enter() -> Event {
    Event::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
}

fn lines(app: &App<TestBackend>) -> Vec<String> {
    let buffer = app.backend().buffer();
    let width = usize::from(buffer.area.width);
    buffer
        .content()
        .chunks(width)
        .map(|row| row.iter().map(|cell| cell.symbol()).collect())
        .collect()
}

fn tab_row(app: &App<TestBackend>) -> String {
    lines(app).remove(0)
}

fn shows(app: &App<TestBackend>, text: &str) -> bool {
    lines(app).iter().any(|line| line.contains(text))
}

const NOTICE: &str = "v99.0.0 available";

#[test]
fn a_tick_asks_for_a_check_when_none_ran_yet() {
    let mut app = app(state(true, Channel::CargoBin, Record::default()));

    let effects = app.handle(Event::Tick(now())).unwrap();

    assert!(effects.contains(&Effect::CheckRelease), "{effects:?}");
}

#[test]
fn a_check_under_a_day_old_asks_for_nothing() {
    let record = Record {
        checked_at: Some(hours_ago(23)),
        ..Record::default()
    };
    let mut app = app(state(true, Channel::CargoBin, record));

    let effects = app.handle(Event::Tick(now())).unwrap();

    assert!(!effects.contains(&Effect::CheckRelease), "{effects:?}");
}

#[test]
fn a_check_a_day_old_is_due_again_and_asked_for_once() {
    let record = Record {
        checked_at: Some(hours_ago(24)),
        ..Record::default()
    };
    let mut app = app(state(true, Channel::CargoBin, record));

    let first = app.handle(Event::Tick(now())).unwrap();
    let second = app.handle(Event::Tick(now())).unwrap();

    assert!(first.contains(&Effect::CheckRelease), "{first:?}");
    assert!(!second.contains(&Effect::CheckRelease), "{second:?}");
}

#[test]
fn with_the_check_off_a_tick_asks_for_nothing() {
    let mut app = app(state(false, Channel::CargoBin, Record::default()));

    let effects = app.handle(Event::Tick(now())).unwrap();

    assert!(!effects.contains(&Effect::CheckRelease), "{effects:?}");
}

#[test]
fn the_default_state_never_checks() {
    let mut app = app(State::demo());

    let effects = app.handle(Event::Tick(now())).unwrap();

    assert!(!effects.contains(&Effect::CheckRelease), "{effects:?}");
}

#[test]
fn a_newer_release_shows_the_notice_and_is_saved() {
    let mut app = app(state(true, Channel::CargoBin, Record::default()));
    app.handle(Event::Tick(now())).unwrap();

    let effects = app.handle(Event::ReleaseChecked(Some(newer()))).unwrap();

    assert!(tab_row(&app).contains(NOTICE), "{}", tab_row(&app));
    assert_eq!(
        effects,
        vec![Effect::SaveRelease(Record {
            checked_at: Some(now()),
            latest: Some(newer()),
            skipped: None,
        })]
    );
}

#[test]
fn the_same_or_an_older_release_shows_nothing() {
    for latest in [release::running(), version("0.0.1")] {
        let mut app = app(state(true, Channel::CargoBin, Record::default()));
        app.handle(Event::Tick(now())).unwrap();

        app.handle(Event::ReleaseChecked(Some(latest.clone())))
            .unwrap();

        assert!(!tab_row(&app).contains("available"), "{latest}");
    }
}

#[test]
fn a_cached_newer_release_shows_at_startup_without_a_check() {
    let mut app = app(state(true, Channel::CargoBin, with_latest(newer())));

    assert!(tab_row(&app).contains(NOTICE), "{}", tab_row(&app));
    let effects = app.handle(Event::Tick(now())).unwrap();
    assert!(!effects.contains(&Effect::CheckRelease), "{effects:?}");
}

#[test]
fn a_skipped_release_shows_nothing_until_a_newer_one() {
    let record = Record {
        skipped: Some(newer()),
        ..with_latest(newer())
    };
    let mut app = app(state(true, Channel::CargoBin, record));
    assert!(!tab_row(&app).contains("available"), "{}", tab_row(&app));

    app.handle(Event::ReleaseChecked(Some(version("99.0.1"))))
        .unwrap();

    assert!(
        tab_row(&app).contains("v99.0.1 available"),
        "{}",
        tab_row(&app)
    );
}

#[test]
fn a_failed_check_adds_no_notice_and_no_error() {
    let mut app = app(state(true, Channel::CargoBin, Record::default()));
    app.handle(Event::Tick(now())).unwrap();

    let effects = app.handle(Event::ReleaseChecked(None)).unwrap();

    assert!(!tab_row(&app).contains("available"), "{}", tab_row(&app));
    assert!(app.state().notice.is_none());
    assert_eq!(app.state().errors.len(), 0);
    // It counts as the day's check, so the next is tried a day later.
    assert_eq!(
        effects,
        vec![Effect::SaveRelease(Record {
            checked_at: Some(now()),
            ..Record::default()
        })]
    );
}

#[test]
fn aur_and_nix_installs_never_show_the_notice() {
    for channel in [Channel::Aur, Channel::Nix] {
        let app = app(state(true, channel, with_latest(newer())));

        assert!(!tab_row(&app).contains("available"), "{channel:?}");
    }
}

#[test]
fn the_tab_row_shows_the_notice_left_of_the_sync_indicator_when_wide() {
    let mut app = app_at(
        160,
        synced(state(true, Channel::CargoBin, with_latest(newer()))),
    );
    app.handle(Event::Tick(now())).unwrap();

    assert_snapshot!(tab_row(&app));
}

#[test]
fn the_tab_row_at_80_columns() {
    let mut app = app_at(
        80,
        synced(state(true, Channel::CargoBin, with_latest(newer()))),
    );
    app.handle(Event::Tick(now())).unwrap();

    assert_snapshot!(tab_row(&app));
}

#[test]
fn the_notice_goes_first_when_the_tab_row_is_too_narrow() {
    let mut app = app_at(
        64,
        synced(state(true, Channel::CargoBin, with_latest(newer()))),
    );
    app.handle(Event::Tick(now())).unwrap();

    assert!(!tab_row(&app).contains("available"), "{}", tab_row(&app));
    assert!(tab_row(&app).contains("synced"), "{}", tab_row(&app));
}

#[test]
fn the_cheatsheet_shows_the_full_line_with_how_to_upgrade() {
    // Tall enough that the line below every pane needs no scrolling.
    let state = state(true, Channel::Homebrew, with_latest(newer()));
    let mut app = App::new(TestBackend::new(120, 32), state).unwrap();

    app.handle(key('?')).unwrap();

    assert!(
        shows(&app, "lrat v99.0.0 is available: brew upgrade linear-rat"),
        "{:#?}",
        lines(&app)
    );
}

/// Opens the palette and types `query`.
fn find(app: &mut App<TestBackend>, query: &str) {
    app.handle(key(':')).unwrap();
    for c in query.chars() {
        app.handle(key(c)).unwrap();
    }
}

#[test]
fn without_a_newer_release_the_palette_actions_cannot_run() {
    let mut app = app(state(true, Channel::CargoBin, Record::default()));
    find(&mut app, "skip this release");

    let effects = app.handle(enter()).unwrap();

    assert_eq!(effects, vec![]);
    assert!(shows(&app, " Palette "), "{:#?}", lines(&app));
}

#[test]
fn skipping_the_release_hides_the_notice_and_records_it() {
    let mut app = app(state(true, Channel::CargoBin, with_latest(newer())));
    find(&mut app, "skip this release");

    let effects = app.handle(enter()).unwrap();

    assert!(!tab_row(&app).contains("available"), "{}", tab_row(&app));
    assert_eq!(
        effects,
        vec![Effect::SaveRelease(Record {
            skipped: Some(newer()),
            ..with_latest(newer())
        })]
    );
}

#[test]
fn whats_new_names_the_release_and_opens_its_page() {
    let mut app = app(state(true, Channel::CargoBin, with_latest(newer())));
    find(&mut app, "what's new");
    assert!(shows(&app, "What's new in v99.0.0"), "{:#?}", lines(&app));

    let effects = app.handle(enter()).unwrap();

    assert_eq!(effects, vec![Effect::OpenUrl(release_url(&newer()))]);
    assert_eq!(
        release_url(&newer()),
        "https://github.com/cxrlos/linear-rat/releases/tag/v99.0.0"
    );
}

/// A fresh directory for one test.
fn scratch_dir() -> PathBuf {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!(
        "release-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::SeqCst)
    ));
    let _ = fs::remove_dir_all(&dir);
    dir
}

async fn github(tag: &str, expected_requests: u64) -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/repos/cxrlos/linear-rat/releases/latest"))
        .and(header(
            "user-agent",
            concat!("lrat/", env!("CARGO_PKG_VERSION")),
        ))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(serde_json::json!({ "tag_name": tag, "draft": false })),
        )
        .expect(expected_requests)
        .mount(&server)
        .await;
    server
}

fn source(server: &MockServer) -> ReleaseSource {
    ReleaseSource::new(format!(
        "{}/repos/cxrlos/linear-rat/releases/latest",
        server.uri()
    ))
    .unwrap()
}

#[tokio::test]
async fn the_source_reads_the_latest_tag_with_lrats_user_agent_and_no_key() {
    let server = github("v0.3.1", 1).await;

    let latest = source(&server).latest().await.unwrap();

    assert_eq!(latest, version("0.3.1"));
    let requests = server.received_requests().await.unwrap();
    assert!(
        requests
            .iter()
            .all(|request| !request.headers.contains_key("authorization"))
    );
}

#[tokio::test]
async fn a_github_error_is_an_error() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(403))
        .mount(&server)
        .await;

    let error = source(&server).latest().await.unwrap_err();

    assert_eq!(format!("{error:#}"), "GitHub answered 403 Forbidden");
}

/// Runs the TUI on the demo state with the check on and `record` in the
/// cache, against `server`, until it settles.
async fn run_checking(server: &MockServer, record: &Record) -> (App<TestBackend>, ReleaseCache) {
    let cache = ReleaseCache::new(scratch_dir());
    cache.save(record).unwrap();
    let check = ReleaseCheck {
        source: source(server),
        cache: cache.clone(),
    };
    let state = State::demo().with_release(Release::new(true, Channel::CargoBin, cache.load()));
    let mut app = App::new(TestBackend::new(100, 24), state).unwrap();
    let running = runtime::run_with_job_control(
        &mut app,
        Some(&FakeClient),
        None,
        Some(&check),
        stream::pending(),
        future::pending(),
        stream::pending(),
    );
    let stopped = tokio::time::timeout(Duration::from_millis(300), running).await;
    assert!(stopped.is_err(), "the runtime stopped: {stopped:?}");
    (app, cache)
}

#[tokio::test]
async fn the_runtime_checks_once_when_due_and_caches_the_answer() {
    let server = github("v99.0.0", 1).await;

    let (app, cache) = run_checking(&server, &Record::default()).await;

    assert!(tab_row(&app).contains(NOTICE), "{}", tab_row(&app));
    let saved = cache.load();
    assert_eq!(saved.latest, Some(newer()));
    assert!(saved.checked_at.is_some());
}

#[tokio::test]
async fn the_runtime_asks_nothing_when_the_cache_is_fresh() {
    let server = github("v99.0.0", 0).await;
    let record = Record {
        checked_at: Some(Timestamp::now()),
        ..Record::default()
    };

    let (app, cache) = run_checking(&server, &record).await;

    assert!(!tab_row(&app).contains("available"), "{}", tab_row(&app));
    assert_eq!(cache.load(), record);
}

#[tokio::test]
async fn a_check_github_fails_shows_nothing_and_logs_no_error() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(500))
        .expect(1)
        .mount(&server)
        .await;

    let (app, cache) = run_checking(&server, &Record::default()).await;

    assert!(!tab_row(&app).contains("available"), "{}", tab_row(&app));
    assert_eq!(app.state().errors.len(), 0);
    assert!(cache.load().checked_at.is_some());
}
