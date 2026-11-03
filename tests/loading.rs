//! Loading My Issues through the app driver: the loading message, the load
//! result arriving as an event, and the error screens.

// clippy's allow-unwrap-in-tests does not cover helpers outside #[test] functions.
#![allow(clippy::unwrap_used)]

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use insta::assert_snapshot;
use linear_rat::app::{App, Effect, Event, LoadError, State};
use linear_rat::auth::KeySource;
use linear_rat::linear::{FakeClient, LinearClient};
use ratatui::backend::TestBackend;

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

fn loading(key_source: Option<KeySource>) -> App<TestBackend> {
    App::new(TestBackend::new(80, 24), State::loading(key_source)).unwrap()
}

fn failed(error: LoadError) -> App<TestBackend> {
    App::new(TestBackend::new(80, 24), State::failed(error)).unwrap()
}

#[test]
fn starts_with_a_loading_message_and_asks_the_runtime_for_the_issues() {
    let mut app = loading(None);

    let effects = app.handle(Event::Start).unwrap();

    assert_eq!(effects, [Effect::LoadMyIssues]);
    assert_snapshot!(app.backend());
}

#[tokio::test]
async fn the_loaded_issues_fill_my_issues_grouped_as_in_the_demo() {
    let mut app = loading(None);
    let loaded = FakeClient.assigned_issues().await.unwrap();

    let effects = app.handle(Event::MyIssuesLoaded(Ok(loaded))).unwrap();

    assert!(effects.is_empty());
    // The demo screen itself is pinned by the app snapshots.
    let demo = App::new(TestBackend::new(80, 24), State::demo()).unwrap();
    assert_eq!(screen(&app), screen(&demo));
}

#[test]
fn a_state_that_is_not_loading_asks_for_nothing_at_start() {
    let mut app = failed(LoadError::NoKey);

    assert!(app.handle(Event::Start).unwrap().is_empty());
}

#[test]
fn a_missing_key_says_how_to_add_one() {
    let app = failed(LoadError::NoKey);

    assert_snapshot!(app.backend());
}

#[test]
fn a_rejected_key_from_the_environment_says_to_replace_or_unset_it() {
    let mut app = loading(Some(KeySource::Env));

    app.handle(Event::MyIssuesLoaded(Err(LoadError::KeyRejected)))
        .unwrap();

    assert_snapshot!(app.backend());
}

#[test]
fn a_rejected_key_from_the_keyring_says_to_log_in_again() {
    let mut app = loading(Some(KeySource::Keyring {
        account: "squeakworks".to_owned(),
    }));

    app.handle(Event::MyIssuesLoaded(Err(LoadError::KeyRejected)))
        .unwrap();

    let screen = screen(&app);
    assert!(screen.contains("API key rejected"), "{screen}");
    assert!(
        screen.contains("the OS keyring (service linear-rat"),
        "{screen}"
    );
    assert!(screen.contains("squeakworks"), "{screen}");
    assert!(screen.contains("run `lrat auth login`"), "{screen}");
    assert!(!screen.contains("unset LINEAR_API_KEY"), "{screen}");
}

#[test]
fn an_unavailable_keyring_says_to_use_the_environment_instead() {
    let error = LoadError::KeyringUnavailable(
        "the OS keyring is unavailable (no Secret Service); \
         set LINEAR_API_KEY to use lrat without it"
            .to_owned(),
    );

    let screen = screen(&failed(error));

    assert!(screen.contains("OS keyring unavailable"), "{screen}");
    assert!(screen.contains("The OS keyring is unavailable"), "{screen}");
    assert!(screen.contains("set LINEAR_API_KEY"), "{screen}");
}

#[test]
fn a_network_failure_says_what_to_try() {
    let mut app = loading(Some(KeySource::Env));

    app.handle(Event::MyIssuesLoaded(Err(LoadError::Unreachable(
        "timed out connecting to Linear".to_owned(),
    ))))
    .unwrap();

    let screen = screen(&app);
    assert!(screen.contains("Cannot reach Linear"), "{screen}");
    assert!(
        screen.contains("Timed out connecting to Linear."),
        "{screen}"
    );
    assert!(screen.contains("network connection"), "{screen}");
}

#[test]
fn q_still_quits_from_an_error_screen() {
    let mut app = failed(LoadError::NoKey);

    let effects = app
        .handle(Event::Key(KeyEvent::new(
            KeyCode::Char('q'),
            KeyModifiers::NONE,
        )))
        .unwrap();

    assert_eq!(effects, [Effect::Quit]);
}

#[test]
fn with_no_list_the_bottom_bar_offers_only_what_can_run() {
    for app in [loading(None), failed(LoadError::NoKey)] {
        let screen = screen(&app);
        let bar = screen.lines().last().unwrap();

        assert!(bar.contains("q quit"), "{bar}");
        assert!(!bar.contains("down"), "{bar}");
        assert!(!bar.contains("open"), "{bar}");
    }
}

#[tokio::test]
async fn issues_that_loaded_with_problems_show_and_say_what_is_missing() {
    let mut app = loading(None);
    let mut loaded = FakeClient.assigned_issues().await.unwrap();
    loaded.problems = vec![
        "label names did not load: Linear answered with HTTP 503".to_owned(),
        "Linear left something out: Cycle could not be loaded".to_owned(),
    ];

    app.handle(Event::MyIssuesLoaded(Ok(loaded))).unwrap();

    let screen = screen(&app);
    assert!(screen.contains("Started (9)"), "{screen}");
    assert!(
        screen.contains("label names did not load: Linear answered with HTTP 503 (and 1 more;"),
        "{screen}"
    );
}
