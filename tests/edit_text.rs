//! Editing an issue's title and description in `$EDITOR` through the app
//! driver: the buffer the editor opens on, what a save sends, what cancels
//! it, and the rollback when the save is refused or rejected.

// clippy's allow-unwrap-in-tests does not cover helpers outside #[test] functions.
#![allow(clippy::unwrap_used)]

use std::path::Path;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use insta::assert_snapshot;
use linear_rat::app::{App, Effect, Event, Failure, State, Write, Writing};
use linear_rat::config;
use linear_rat::linear::model::Issue;
use linear_rat::linear::{ErrorKind, IssueUpdate, demo};
use ratatui::backend::TestBackend;

/// The issue My Issues focuses first in the demo.
const FIRST: &str = "DES-34";

fn app() -> App<TestBackend> {
    App::new(TestBackend::new(80, 24), State::demo()).unwrap()
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

fn demo_issue(identifier: &str) -> Issue {
    demo::workspace()
        .issues
        .into_iter()
        .find(|issue| issue.identifier == identifier)
        .unwrap()
}

/// Opens the editor on the focused issue and returns what it is writing
/// and the buffer it opens on.
fn open_editor(app: &mut App<TestBackend>) -> (Writing, String) {
    match press(app, "E").as_slice() {
        [Effect::OpenEditor { writing, text }] => (writing.clone(), text.clone()),
        other => panic!("expected the editor to open, got {other:?}"),
    }
}

/// Opens the editor, saves `buffer` in it, and returns the effects.
fn save(app: &mut App<TestBackend>, buffer: &str) -> Vec<Effect> {
    let (writing, _) = open_editor(app);
    app.handle(Event::EditorClosed {
        writing,
        result: Ok(buffer.to_owned()),
    })
    .unwrap()
}

/// The only effect, which must be a checked text write; with the buffer it
/// keeps.
fn only_text_write(effects: Vec<Effect>) -> (Write, String) {
    match effects.as_slice() {
        [Effect::UpdateIssueText { write, text, .. }] => (write.clone(), text.clone()),
        other => panic!("expected exactly one text write, got {other:?}"),
    }
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

fn description(issue: &Issue) -> String {
    issue.description.clone().unwrap_or_default()
}

#[test]
fn the_text_edit_opens_the_editor_on_the_title_a_blank_line_and_the_description() {
    let mut app = app();
    let issue = demo_issue(FIRST);

    let (writing, text) = open_editor(&mut app);

    let Writing::Text(opened) = writing else {
        panic!("{writing:?}");
    };
    assert_eq!(opened.identifier, FIRST);
    assert_eq!(text, format!("{}\n\n{}", issue.title, description(&issue)));
}

#[test]
fn changing_only_the_title_sends_only_the_title_and_shows_at_once() {
    let mut app = app();
    let issue = demo_issue(FIRST);
    let buffer = format!("Autumn release, take two\n\n{}\n", description(&issue));

    let (write, text) = only_text_write(save(&mut app, &buffer));

    assert_eq!(
        write.update(),
        IssueUpdate {
            title: Some("Autumn release, take two".to_owned()),
            ..IssueUpdate::default()
        }
    );
    assert_eq!(text, buffer);
    assert!(shows(&app, "Autumn release, take two"));
}

#[test]
fn changing_only_the_description_sends_only_the_description() {
    let mut app = app();
    let issue = demo_issue(FIRST);
    let buffer = format!("{}\n\nA new description.\n\n- with a list\n", issue.title);

    let (write, _) = only_text_write(save(&mut app, &buffer));

    assert_eq!(
        write.update(),
        IssueUpdate {
            description: Some("A new description.\n\n- with a list".to_owned()),
            ..IssueUpdate::default()
        }
    );
}

#[test]
fn the_save_checks_against_the_updated_at_the_editor_opened_on() {
    let mut app = app();
    let issue = demo_issue(FIRST);

    let effects = save(&mut app, "New title\n\nNew body");

    let [Effect::UpdateIssueText { seen, .. }] = effects.as_slice() else {
        panic!("{effects:?}");
    };
    assert_eq!(*seen, issue.updated_at);
}

#[test]
fn unchanged_empty_or_untitled_buffers_send_nothing_and_say_why() {
    let issue = demo_issue(FIRST);
    let cases = [
        (
            format!("{}\n\n{}\n\n", issue.title, description(&issue)),
            "nothing changed",
        ),
        (" \n\n".to_owned(), "the buffer was empty"),
        (
            "\nA description with no title".to_owned(),
            "the first line, the title, was empty",
        ),
    ];
    for (buffer, why) in cases {
        let mut app = app();

        let effects = save(&mut app, &buffer);

        assert!(effects.is_empty(), "{why}: {effects:?}");
        assert!(
            shows(&app, &format!("{FIRST} was not changed: {why}")),
            "{why}"
        );
    }
}

#[test]
fn a_refused_save_rolls_back_and_names_the_kept_file() {
    let mut app = app();
    let (write, _) = only_text_write(save(&mut app, "Autumn release, take two\n\nBody"));

    app.handle(Event::IssueUpdated {
        write,
        result: Err(failed(
            "it changed in Linear while the editor was open, so nothing was sent; your text \
             is in /tmp/lrat-DES-34-1-0.md",
        )),
    })
    .unwrap();

    assert!(!shows(&app, "Autumn release, take two"));
    assert_snapshot!(app.backend());
}

#[test]
fn an_accepted_save_keeps_linears_copy() {
    let mut app = app();
    let (write, _) = only_text_write(save(&mut app, "Autumn release, take two\n\nBody"));
    let mut stored = demo_issue(FIRST);
    stored.title = "Autumn release, take two".to_owned();
    stored.description = Some("Body".to_owned());

    let effects = app
        .handle(Event::IssueUpdated {
            write,
            result: Ok(Box::new(stored)),
        })
        .unwrap();

    assert!(effects.is_empty());
    assert!(shows(&app, "Autumn release, take two"));
}

#[test]
fn the_cheatsheet_lists_the_text_edit() {
    let mut app = App::new(TestBackend::new(100, 80), State::demo()).unwrap();

    press(&mut app, "?");

    assert!(shows(&app, "title and description"));
}

#[test]
fn the_text_edit_can_be_rebound() {
    let loaded = config::parse("[keys.list]\nedit-text = \"T\"\n", Path::new("config.toml"));
    let mut app = App::new(TestBackend::new(80, 24), State::demo().with_config(&loaded)).unwrap();

    assert!(press(&mut app, "E").is_empty());
    assert!(matches!(
        press(&mut app, "T").as_slice(),
        [Effect::OpenEditor {
            writing: Writing::Text(_),
            ..
        }]
    ));
}

/// The issue's `updatedAt` moves when a write lrat sent lands, so a text
/// edit opened before then would be refused as if a teammate had changed
/// the issue. The editor waits until the issue's writes are answered.
#[test]
fn the_editor_does_not_open_while_a_write_to_the_issue_is_in_flight() {
    let mut app = app();
    press(&mut app, "p");
    app.handle(key(KeyCode::Down)).unwrap();
    let picked = app.handle(key(KeyCode::Enter)).unwrap();
    let [Effect::UpdateIssue(write)] = picked.as_slice() else {
        panic!("{picked:?}");
    };

    let effects = press(&mut app, "E");

    assert!(effects.is_empty(), "{effects:?}");
    assert!(shows(&app, &format!("{FIRST} is still being saved")));

    app.handle(Event::IssueUpdated {
        write: write.clone(),
        result: Err(failed("Linear returned errors")),
    })
    .unwrap();
    open_editor(&mut app);
}

#[test]
fn a_second_text_edit_waits_for_the_first_save() {
    let mut app = app();
    let (write, _) = only_text_write(save(&mut app, "Autumn release, take two\n\nBody"));

    assert!(press(&mut app, "E").is_empty());

    let mut stored = demo_issue(FIRST);
    stored.title = "Autumn release, take two".to_owned();
    stored.description = Some("Body".to_owned());
    app.handle(Event::IssueUpdated {
        write,
        result: Ok(Box::new(stored)),
    })
    .unwrap();
    open_editor(&mut app);
}

/// A failure with `message`, as the runtime sends one.
fn failed(message: &str) -> Failure {
    Failure::new(ErrorKind::Other, "test", message)
}
