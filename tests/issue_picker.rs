//! The issue picker through the app driver: fuzzy matching on the loaded
//! issues, the search Linear is asked after a pause, and what one or several
//! chosen issues go back as.

// clippy's allow-unwrap-in-tests does not cover helpers outside #[test] functions.
#![allow(clippy::unwrap_used)]

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use insta::assert_snapshot;
use linear_rat::app::{
    App, Effect, Event, IssuePicker, IssuePurpose, Modal, Mode, Scope, Screen, State,
    open_issue_picker,
};
use linear_rat::linear::model::Issue;
use linear_rat::linear::{FakeClient, LinearClient, SearchResults, demo};
use ratatui::backend::TestBackend;

fn picker_app(mode: Mode, scope: Scope) -> App<TestBackend> {
    let mut state = State::demo();
    open_issue_picker(
        &mut state,
        IssuePicker::new(IssuePurpose::Show, mode, scope, None),
    );
    App::new(TestBackend::new(80, 24), state).unwrap()
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

/// The rows inside the picker's border below the filter, trimmed.
fn picker_lines(app: &App<TestBackend>) -> Vec<String> {
    let lines = screen(app);
    let Some(top) = lines
        .iter()
        .position(|line| line.contains(" Go to an issue "))
    else {
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

/// The id of the issue the open picker has selected.
fn selected(app: &App<TestBackend>) -> Option<String> {
    match app.state().modals.top() {
        Some(Modal::IssuePicker(picker)) => picker.list.selected().cloned(),
        _ => None,
    }
}

fn demo_issue(identifier: &str) -> Issue {
    demo::workspace()
        .issues
        .into_iter()
        .find(|issue| issue.identifier == identifier)
        .unwrap()
}

fn found(term: &str, issues: Vec<Issue>) -> Event {
    Event::SearchDone {
        term: term.to_owned(),
        result: Ok(SearchResults {
            issues,
            ..SearchResults::default()
        }),
    }
}

/// The searches in `effects`.
fn searches(effects: &[Effect]) -> Vec<&str> {
    effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::Search(term) => Some(term.as_str()),
            _ => None,
        })
        .collect()
}

#[test]
fn typing_fuzzy_matches_key_and_title_with_an_exact_key_first() {
    let mut app = picker_app(Mode::One, Scope::default());

    press(&mut app, "ENG-6");

    let lines = picker_lines(&app);
    assert!(
        lines[0].contains("ENG-6 Migrate burrow storage"),
        "{lines:?}"
    );
    assert!(
        lines.iter().any(|line| line.contains("ENG-60 ")),
        "{lines:?}"
    );

    press(&mut app, " ");
    for _ in 0..6 {
        app.handle(key(KeyCode::Backspace)).unwrap();
    }
    press(&mut app, "chscv");
    let lines = picker_lines(&app);
    assert!(
        lines[0].contains("OPS-36 Plan the office cheese cave move"),
        "{lines:?}"
    );
}

#[test]
fn every_keystroke_asks_for_a_pause_and_only_the_pause_on_the_current_query_searches() {
    let mut app = picker_app(Mode::One, Scope::default());

    let effects = press(&mut app, "cave");
    assert_eq!(
        effects,
        ["ca", "cav", "cave"]
            .map(|query| Effect::SearchAfterPause(query.to_owned()))
            .to_vec()
    );

    let stale = app.handle(Event::TypingPaused("cav".to_owned())).unwrap();
    assert!(stale.is_empty());
    let effects = app.handle(Event::TypingPaused("cave".to_owned())).unwrap();
    assert_eq!(searches(&effects), ["cave"]);
    assert!(picker_lines(&app).iter().any(|line| line == "Loading…"));

    let again = app.handle(Event::TypingPaused("cave".to_owned())).unwrap();
    assert!(
        again.is_empty(),
        "a second pause on the same query sends nothing"
    );
}

#[test]
fn a_one_character_query_never_searches() {
    let mut app = picker_app(Mode::One, Scope::default());

    let effects = press(&mut app, "c");

    assert!(effects.is_empty());
}

#[test]
fn an_answer_to_an_older_query_is_dropped_and_the_newest_merges_without_moving_the_selection() {
    let mut app = picker_app(Mode::One, Scope::default());
    press(&mut app, "ca");
    app.handle(Event::TypingPaused("ca".to_owned())).unwrap();
    press(&mut app, "ve");
    app.handle(Event::TypingPaused("cave".to_owned())).unwrap();
    app.handle(key(KeyCode::Down)).unwrap();
    let selected_before = selected(&app);
    assert!(selected_before.is_some());

    app.handle(found("ca", vec![demo_issue("ENG-1")])).unwrap();
    assert!(
        !picker_lines(&app)
            .iter()
            .any(|line| line.contains("ENG-1 ")),
        "the answer for \"ca\" is not shown"
    );
    assert!(picker_lines(&app).iter().any(|line| line == "Loading…"));

    let extra = demo::workspace()
        .issues
        .into_iter()
        .find(|issue| issue.identifier == "OPS-1")
        .unwrap();
    app.handle(found("cave", vec![extra])).unwrap();

    let lines = picker_lines(&app);
    assert!(
        lines.iter().any(|line| line.contains("OPS-1 ")),
        "{lines:?}"
    );
    assert!(!lines.iter().any(|line| line == "Loading…"));
    assert_eq!(selected(&app), selected_before);
}

#[test]
fn a_failed_search_says_why_and_keeps_the_local_matches() {
    let mut app = picker_app(Mode::One, Scope::default());
    press(&mut app, "cave");
    app.handle(Event::TypingPaused("cave".to_owned())).unwrap();

    app.handle(Event::SearchDone {
        term: "cave".to_owned(),
        result: Err("Search failed: offline".to_owned()),
    })
    .unwrap();

    let lines = picker_lines(&app);
    assert!(
        lines.iter().any(|line| line.contains("OPS-36 ")),
        "{lines:?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line.contains("Could not load: Search failed: offline")),
        "{lines:?}"
    );
}

#[test]
fn excluded_issues_are_never_offered() {
    let excluded = demo_issue("OPS-36").id;
    let mut app = picker_app(
        Mode::One,
        Scope {
            excluded: vec![excluded],
            only: None,
        },
    );
    press(&mut app, "cave");
    app.handle(Event::TypingPaused("cave".to_owned())).unwrap();
    app.handle(found("cave", vec![demo_issue("OPS-36")]))
        .unwrap();

    assert!(
        !picker_lines(&app)
            .iter()
            .any(|line| line.contains("OPS-36"))
    );
}

#[test]
fn a_picker_limited_to_some_issues_never_searches() {
    let only = vec![demo_issue("OPS-36").id, demo_issue("ENG-6").id];
    let mut app = picker_app(
        Mode::One,
        Scope {
            excluded: Vec::new(),
            only: Some(only),
        },
    );

    let effects = press(&mut app, "cave");

    assert!(effects.is_empty());
    let lines = picker_lines(&app);
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert!(lines[0].contains("OPS-36"));
}

#[test]
fn enter_in_single_mode_goes_to_the_selected_issue() {
    let mut app = picker_app(Mode::One, Scope::default());
    press(&mut app, "ENG-6");

    app.handle(key(KeyCode::Enter)).unwrap();

    assert!(app.state().modals.top().is_none());
    assert_eq!(app.state().focused_issue().unwrap().identifier, "ENG-6");
}

#[test]
fn a_number_in_single_mode_chooses_that_issue() {
    let mut app = picker_app(Mode::One, Scope::default());
    press(&mut app, "cave ");

    press(&mut app, "1");

    assert!(app.state().modals.top().is_none());
    assert_eq!(app.state().focused_issue().unwrap().identifier, "OPS-36");
}

#[test]
fn several_mode_toggles_with_tab_and_numbers_and_enter_returns_every_marked_issue() {
    let mut app = picker_app(Mode::Several, Scope::default());
    press(&mut app, "ENG-6");
    app.handle(key(KeyCode::Tab)).unwrap();
    for _ in 0..5 {
        app.handle(key(KeyCode::Backspace)).unwrap();
    }
    press(&mut app, "cave ");
    press(&mut app, "1");
    assert!(app.state().modals.top().is_some(), "a number toggles");

    app.handle(key(KeyCode::Enter)).unwrap();

    assert!(app.state().modals.top().is_none());
    assert_eq!(app.state().screen, Screen::Results);
    let shown: Vec<&str> = app
        .state()
        .results
        .ids()
        .iter()
        .map(|id| app.state().store.issue(id).unwrap().identifier.as_str())
        .collect();
    assert_eq!(shown, ["ENG-6", "OPS-36"]);
}

#[tokio::test]
async fn the_demo_search_answers_the_picker() {
    let mut app = picker_app(Mode::One, Scope::default());
    press(&mut app, "runbook");
    let effects = app
        .handle(Event::TypingPaused("runbook".to_owned()))
        .unwrap();
    let [Effect::Search(term)] = effects.as_slice() else {
        panic!("expected one search, got {effects:?}");
    };
    let result = FakeClient.search_issues(term).await.unwrap();
    let expected: Vec<String> = result
        .issues
        .iter()
        .map(|issue| issue.identifier.clone())
        .collect();
    assert!(!expected.is_empty(), "the demo has issues about runbooks");

    app.handle(Event::SearchDone {
        term: term.clone(),
        result: Ok(result),
    })
    .unwrap();

    let lines = picker_lines(&app);
    for identifier in expected.iter().take(5) {
        assert!(
            lines
                .iter()
                .any(|line| line.contains(&format!("{identifier} "))),
            "{identifier} in {lines:?}"
        );
    }
}

#[test]
fn single_mode() {
    let mut app = picker_app(Mode::One, Scope::default());
    press(&mut app, "ENG-6");
    assert_snapshot!(app.backend());
}

#[test]
fn several_mode() {
    let mut app = picker_app(Mode::Several, Scope::default());
    press(&mut app, "cave ");
    press(&mut app, "1");
    assert_snapshot!(app.backend());
}

#[test]
fn waiting_on_the_search() {
    let mut app = picker_app(Mode::One, Scope::default());
    press(&mut app, "cave");
    app.handle(Event::TypingPaused("cave".to_owned())).unwrap();
    assert_snapshot!(app.backend());
}

#[test]
fn typing_further_selects_the_best_match_again() {
    let mut app = picker_app(Mode::One, Scope::default());

    press(&mut app, "ENG-60");
    assert_eq!(selected(&app).as_deref(), Some("issue-eng-60"));
    app.handle(key(KeyCode::Backspace)).unwrap();

    assert_eq!(selected(&app).as_deref(), Some("issue-eng-6"));
}
