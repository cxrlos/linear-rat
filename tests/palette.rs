//! The palette: `:` or `Ctrl+K`, then type to find an action or a loaded
//! issue, and Enter to run or jump to it.

// clippy's allow-unwrap-in-tests does not cover helpers outside #[test] functions.
#![allow(clippy::unwrap_used)]

use std::path::Path;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use insta::assert_snapshot;
use linear_rat::app::{App, Effect, Event, State};
use linear_rat::config;
use ratatui::backend::TestBackend;

fn app() -> App<TestBackend> {
    App::new(TestBackend::new(80, 24), State::demo()).unwrap()
}

fn app_with(config: &str) -> App<TestBackend> {
    let loaded = config::parse(config, Path::new("config.toml"));
    App::new(TestBackend::new(80, 24), State::demo().with_config(&loaded)).unwrap()
}

fn key(c: char) -> Event {
    Event::Key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE))
}

fn ctrl(c: char) -> Event {
    Event::Key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL))
}

fn press(code: KeyCode) -> Event {
    Event::Key(KeyEvent::new(code, KeyModifiers::NONE))
}

/// Types `text` one key at a time and returns every effect asked for.
fn type_text(app: &mut App<TestBackend>, text: &str) -> Vec<Effect> {
    text.chars()
        .flat_map(|c| app.handle(key(c)).unwrap())
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

/// The row of the focused issue in My Issues, marked with `▸`.
fn focused_row(app: &App<TestBackend>) -> String {
    screen(app)
        .into_iter()
        .find(|line| line.contains('▸'))
        .unwrap_or_default()
}

#[test]
fn colon_opens_the_palette_with_every_action_and_its_keys() {
    let mut app = app();

    app.handle(key(':')).unwrap();

    assert_snapshot!(app.backend());
}

#[test]
fn ctrl_k_opens_the_palette_too() {
    let mut app = app();

    app.handle(ctrl('k')).unwrap();

    assert!(shows(&app, " Palette "), "{:#?}", screen(&app));
}

#[test]
fn the_palette_shows_the_keys_the_viewer_rebound() {
    let mut app = app_with(
        r#"
[keys.global]
quit = "Q"
show-cheatsheet = { append = ["<F1>"] }
"#,
    );
    app.handle(key(':')).unwrap();

    type_text(&mut app, "quit");
    let quit = screen(&app);
    app.handle(press(KeyCode::Esc)).unwrap();
    app.handle(key(':')).unwrap();
    type_text(&mut app, "every key");

    assert_snapshot!(format!(
        "{}\n\n{}",
        quit.join("\n"),
        screen(&app).join("\n")
    ));
}

#[test]
fn typing_part_of_a_title_finds_a_loaded_issue() {
    let mut app = app();
    app.handle(key(':')).unwrap();

    type_text(&mut app, "cheese cave");

    assert_snapshot!(app.backend());
}

#[test]
fn enter_on_an_issue_focuses_it_in_my_issues() {
    let mut app = app();
    app.handle(key(':')).unwrap();
    type_text(&mut app, "cheese cave");

    app.handle(press(KeyCode::Enter)).unwrap();

    assert!(!shows(&app, " Palette "));
    assert!(
        focused_row(&app).contains("OPS-36"),
        "{}",
        focused_row(&app)
    );
}

#[test]
fn an_exact_identifier_comes_before_longer_ones_that_start_the_same() {
    let mut app = app();
    app.handle(key(':')).unwrap();
    type_text(&mut app, "eng-6");

    app.handle(press(KeyCode::Enter)).unwrap();

    assert!(
        focused_row(&app).contains("ENG-6 "),
        "{}",
        focused_row(&app)
    );
}

#[test]
fn a_jump_from_another_screen_shows_my_issues() {
    let mut app = app();
    app.handle(key('2')).unwrap();
    app.handle(key(':')).unwrap();
    type_text(&mut app, "eng-57");

    app.handle(press(KeyCode::Enter)).unwrap();

    assert!(
        focused_row(&app).contains("ENG-57"),
        "{}",
        focused_row(&app)
    );
}

#[test]
fn enter_runs_the_highlighted_action() {
    let mut app = app();
    app.handle(key(':')).unwrap();
    type_text(&mut app, "quit");

    let effects = app.handle(press(KeyCode::Enter)).unwrap();

    assert_eq!(effects, vec![Effect::Quit]);
}

#[test]
fn an_action_runs_on_the_screen_under_the_palette() {
    let mut app = app();
    app.handle(key(':')).unwrap();
    type_text(&mut app, "move to the bottom");

    app.handle(press(KeyCode::Enter)).unwrap();

    assert!(!shows(&app, " Palette "));
    assert!(
        focused_row(&app).contains("DES-36"),
        "{}",
        focused_row(&app)
    );
}

#[test]
fn down_and_ctrl_n_move_the_highlight_and_up_moves_it_back() {
    let mut app = app();
    app.handle(key(':')).unwrap();
    type_text(&mut app, "copy");
    // Copy the identifier, the URL, the branch name, a Markdown link.

    app.handle(press(KeyCode::Down)).unwrap();
    app.handle(ctrl('n')).unwrap();
    app.handle(press(KeyCode::Up)).unwrap();
    let effects = app.handle(press(KeyCode::Enter)).unwrap();

    assert_eq!(
        effects,
        vec![Effect::Copy(
            "https://linear.app/squeakworks/issue/DES-34".to_owned()
        )]
    );
}

#[test]
fn letters_go_to_the_query_rather_than_the_keymap() {
    let mut app = app();
    app.handle(key(':')).unwrap();

    let effects = type_text(&mut app, "jkq");

    assert!(effects.is_empty(), "{effects:?}");
    assert!(shows(&app, "jkq"), "{:#?}", screen(&app));
}

#[test]
fn backspace_deletes_the_last_character() {
    let mut app = app();
    app.handle(key(':')).unwrap();
    type_text(&mut app, "quix");

    app.handle(press(KeyCode::Backspace)).unwrap();
    type_text(&mut app, "t");
    let effects = app.handle(press(KeyCode::Enter)).unwrap();

    assert_eq!(effects, vec![Effect::Quit]);
}

#[test]
fn nothing_matching_says_so_and_enter_does_nothing() {
    let mut app = app();
    app.handle(key(':')).unwrap();
    type_text(&mut app, "zzzzqqq");

    let effects = app.handle(press(KeyCode::Enter)).unwrap();

    assert!(effects.is_empty());
    assert!(shows(&app, "Nothing matches"), "{:#?}", screen(&app));
}

#[test]
fn esc_closes_the_palette() {
    let mut app = app();
    app.handle(key(':')).unwrap();

    app.handle(press(KeyCode::Esc)).unwrap();

    assert!(!shows(&app, " Palette "));
}
