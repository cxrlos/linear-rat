use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use insta::assert_snapshot;
use linear_rat::app::{App, Effect, Event};
use ratatui::backend::TestBackend;

fn app() -> App<TestBackend> {
    App::new(TestBackend::new(80, 24)).unwrap()
}

fn key(c: char) -> Event {
    Event::Key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE))
}

#[test]
fn opens_on_my_issues_with_the_three_screen_tabs() {
    let app = app();

    assert_snapshot!(app.backend());
}

#[test]
fn redraws_at_the_new_size_after_a_resize() {
    let mut app = app();

    let frames = app.frames_drawn();
    app.backend_mut().resize(200, 60);
    let effects = app
        .handle(Event::Resize {
            columns: 200,
            rows: 60,
        })
        .unwrap();

    assert!(effects.is_empty());
    assert_eq!(app.frames_drawn(), frames + 1);
    assert_snapshot!(app.backend());
}

#[test]
fn q_asks_to_quit() {
    let mut app = app();

    assert_eq!(app.handle(key('q')).unwrap(), vec![Effect::Quit]);
}

#[test]
fn draws_nothing_when_an_event_changes_nothing() {
    let mut app = app();
    let frames = app.frames_drawn();

    app.handle(key('x')).unwrap();
    app.handle(key('q')).unwrap();

    assert_eq!(app.frames_drawn(), frames);
}
