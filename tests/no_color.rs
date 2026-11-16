//! `NO_COLOR`: when set and non-empty, lrat draws with no color at all and
//! modifiers alone carry focus and emphasis.

use std::ffi::OsStr;
use std::path::Path;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use insta::assert_debug_snapshot;
use linear_rat::app::{App, Effect, Event, State};
use linear_rat::config;
use linear_rat::linear::IssueDetail;
use linear_rat::ui::theme::Theme;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::style::{Color, Modifier};

fn no_color_demo() -> App<TestBackend> {
    let loaded = config::parse("[theme]\nname = \"nord\"\n", Path::new("config.toml"))
        .with_no_color(Some(OsStr::new("1")));
    App::new(TestBackend::new(80, 24), State::demo().with_config(&loaded)).unwrap()
}

fn press(app: &mut App<TestBackend>, code: KeyCode) -> Vec<Effect> {
    app.handle(Event::Key(KeyEvent::new(code, KeyModifiers::NONE)))
        .unwrap()
}

fn assert_colorless(buffer: &Buffer) {
    for (index, cell) in buffer.content().iter().enumerate() {
        assert!(
            cell.fg == Color::Reset && cell.bg == Color::Reset,
            "cell {index} ({:?}) has a color: {cell:?}",
            cell.symbol()
        );
    }
}

fn has_modifier(buffer: &Buffer, modifier: Modifier) -> bool {
    buffer
        .content()
        .iter()
        .any(|cell| cell.modifier.contains(modifier))
}

#[test]
fn my_issues_draws_no_color_and_reverses_the_focused_row() {
    let app = no_color_demo();
    let buffer = app.backend().buffer();

    assert_colorless(buffer);
    assert!(has_modifier(buffer, Modifier::REVERSED));
    assert_debug_snapshot!(buffer);
}

#[test]
fn the_issue_detail_draws_no_color() {
    let mut app = no_color_demo();

    for effect in press(&mut app, KeyCode::Enter) {
        if let Effect::LoadIssueDetail { issue_id, .. } = effect {
            let result = Ok(IssueDetail::demo(&issue_id));
            app.handle(Event::IssueDetailLoaded { issue_id, result })
                .unwrap();
        }
    }
    let buffer = app.backend().buffer();

    assert_colorless(buffer);
    assert_debug_snapshot!(buffer);
}

#[test]
fn the_cheatsheet_draws_no_color_and_keeps_usable_keys_bold() {
    let mut app = no_color_demo();

    press(&mut app, KeyCode::Char('?'));
    let buffer = app.backend().buffer();

    assert_colorless(buffer);
    assert!(has_modifier(buffer, Modifier::BOLD));
    assert_debug_snapshot!(buffer);
}

#[test]
fn no_color_unset_or_empty_keeps_the_configured_theme() {
    let nord = config::parse("[theme]\nname = \"nord\"\n", Path::new("config.toml"));

    let unset = nord.clone().with_no_color(None);
    let empty = nord.clone().with_no_color(Some(OsStr::new("")));

    assert_eq!(unset.theme, Theme::built_in("nord").unwrap());
    assert_eq!(empty.theme, Theme::built_in("nord").unwrap());
}

#[test]
fn no_color_set_beats_the_configured_theme() {
    for config in ["", "[theme]\nname = \"nord\"\n"] {
        let loaded = config::parse(config, Path::new("config.toml"));

        let resolved = loaded.with_no_color(Some(OsStr::new("1")));

        assert_eq!(resolved.theme, Theme::no_color(), "{config}");
    }
}
