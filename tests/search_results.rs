//! Search results and palette jumps shown in the normal panes: a list of the
//! results with the focused one's detail, every issue action on the focused
//! result, and Esc back to where the search started.

// clippy's allow-unwrap-in-tests does not cover helpers outside #[test] functions.
#![allow(clippy::unwrap_used)]

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use insta::assert_snapshot;
use linear_rat::app::{App, Effect, Event, State};
use linear_rat::linear::model::Issue;
use linear_rat::linear::{FakeClient, LinearClient, SearchResults, Urgency};
use ratatui::backend::TestBackend;

fn app() -> App<TestBackend> {
    App::new(TestBackend::new(80, 24), State::demo()).unwrap()
}

fn wide() -> App<TestBackend> {
    App::new(TestBackend::new(200, 60), State::demo()).unwrap()
}

fn key(c: char) -> Event {
    Event::Key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE))
}

fn press(code: KeyCode) -> Event {
    Event::Key(KeyEvent::new(code, KeyModifiers::NONE))
}

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

async fn found(term: &str) -> SearchResults {
    FakeClient.search_issues(term).await.unwrap()
}

/// Searches for `term` and hands the app what the fake client finds,
/// returning the effects of that answer.
async fn searched(app: &mut App<TestBackend>, term: &str) -> Vec<Effect> {
    app.handle(key('/')).unwrap();
    type_text(app, term);
    app.handle(press(KeyCode::Enter)).unwrap();
    let result = Ok(found(term).await);
    app.handle(Event::SearchDone {
        term: term.to_owned(),
        result,
    })
    .unwrap()
}

async fn results(term: &str) -> Vec<Issue> {
    found(term).await.issues
}

#[tokio::test]
async fn results_show_as_a_list_in_the_panes() {
    let mut app = app();

    searched(&mut app, "cheese").await;

    assert!(!shows(&app, " Search Linear "), "{:#?}", screen(&app));
    assert_snapshot!(app.backend());
}

#[tokio::test]
async fn a_wide_terminal_shows_the_focused_results_detail_beside_the_list() {
    let mut app = wide();

    let effects = searched(&mut app, "cheese").await;

    let first = results("cheese").await.remove(0);
    assert_eq!(
        effects,
        vec![Effect::LoadIssueDetail {
            issue_id: first.id,
            urgency: Urgency::Background,
        }]
    );
    assert_snapshot!(app.backend());
}

#[tokio::test]
async fn esc_from_the_results_returns_to_the_screen_and_focus_the_search_started_from() {
    let mut app = app();
    app.handle(key('2')).unwrap();
    app.handle(key('1')).unwrap();
    type_text(&mut app, "jj");
    let before = screen(&app);

    searched(&mut app, "cheese").await;
    app.handle(press(KeyCode::Esc)).unwrap();

    assert_eq!(screen(&app), before);
}

#[tokio::test]
async fn esc_from_results_started_on_team_issues_returns_there() {
    let mut app = app();
    app.handle(key('2')).unwrap();

    searched(&mut app, "cheese").await;
    app.handle(press(KeyCode::Esc)).unwrap();

    assert!(shows(&app, "┌Team Issues"), "{:#?}", screen(&app));
}

#[tokio::test]
async fn a_search_from_the_results_still_returns_to_where_the_first_one_started() {
    let mut app = app();
    app.handle(key('2')).unwrap();
    searched(&mut app, "cheese").await;

    searched(&mut app, "burrow").await;
    app.handle(press(KeyCode::Esc)).unwrap();

    assert!(shows(&app, "┌Team Issues"), "{:#?}", screen(&app));
}

#[tokio::test]
async fn copy_works_on_the_focused_result() {
    let mut app = app();
    searched(&mut app, "cheese").await;
    let second = results("cheese").await.remove(1);

    app.handle(key('j')).unwrap();
    let effects = type_text(&mut app, "yi");

    assert_eq!(effects, vec![Effect::Copy(second.identifier.clone())]);
    assert!(
        shows(&app, &format!("Copied {}", second.identifier)),
        "{:#?}",
        screen(&app)
    );
}

#[tokio::test]
async fn an_edit_works_on_the_focused_result() {
    let mut app = app();
    searched(&mut app, "cheese").await;
    let first = results("cheese").await.remove(0);

    type_text(&mut app, "p");
    let picker = screen(&app);
    let effects = type_text(&mut app, "1");

    let write = match effects.as_slice() {
        [Effect::UpdateIssue(write)] => write.clone(),
        other => panic!("expected exactly one write, got {other:?}"),
    };
    assert!(
        picker.iter().any(|line| line.contains(&first.identifier)),
        "{picker:#?}"
    );
    assert_eq!(write.issue_id, first.id);
    assert_snapshot!(app.backend());
}

#[tokio::test]
async fn comment_and_links_work_on_the_focused_result() {
    let mut app = app();
    searched(&mut app, "cheese").await;

    let comment = type_text(&mut app, "c");
    app.handle(press(KeyCode::Esc)).unwrap();
    type_text(&mut app, "o");

    assert!(
        matches!(comment.as_slice(), [Effect::OpenEditor { .. }]),
        "{comment:?}"
    );
    assert!(shows(&app, "Open"), "{:#?}", screen(&app));
}

#[tokio::test]
async fn a_palette_jump_to_an_issue_outside_my_issues_opens_it_in_the_results_view() {
    let mut app = app();
    app.handle(key('2')).unwrap();
    searched(&mut app, "cheese").await;
    app.handle(press(KeyCode::Esc)).unwrap();
    let closed = results("cheese")
        .await
        .into_iter()
        .find(|issue| !issue.state.state_type.is_open())
        .unwrap();

    app.handle(key(':')).unwrap();
    type_text(&mut app, &closed.identifier.to_lowercase());
    app.handle(press(KeyCode::Enter)).unwrap();
    let jumped = screen(&app);
    let effects = type_text(&mut app, "yi");
    app.handle(press(KeyCode::Esc)).unwrap();

    assert!(
        focused_row_of(&jumped).contains(&closed.identifier),
        "{jumped:#?}"
    );
    assert_eq!(effects, vec![Effect::Copy(closed.identifier.clone())]);
    assert!(shows(&app, "┌Team Issues"), "{:#?}", screen(&app));
}

fn focused_row_of(lines: &[String]) -> String {
    lines
        .iter()
        .find(|line| line.contains('▸'))
        .cloned()
        .unwrap_or_default()
}
