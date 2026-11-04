//! The search overlay: `/`, a query, Enter to ask Linear, and the results
//! as a list that opens each one in its detail.

// clippy's allow-unwrap-in-tests does not cover helpers outside #[test] functions.
#![allow(clippy::unwrap_used)]

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use insta::assert_snapshot;
use linear_rat::app::{App, Effect, Event, State};
use linear_rat::linear::{FakeClient, LinearClient, SearchResults, Urgency};
use ratatui::backend::TestBackend;

fn app() -> App<TestBackend> {
    App::new(TestBackend::new(80, 24), State::demo()).unwrap()
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

/// Opens the search, types `term` and sends it.
fn search_for(app: &mut App<TestBackend>, term: &str) -> Vec<Effect> {
    app.handle(key('/')).unwrap();
    type_text(app, term);
    app.handle(press(KeyCode::Enter)).unwrap()
}

/// Searches for `term` and hands the app what the fake client finds.
async fn searched(app: &mut App<TestBackend>, term: &str) {
    search_for(app, term);
    let result = Ok(found(term).await);
    app.handle(Event::SearchDone {
        term: term.to_owned(),
        result,
    })
    .unwrap();
}

#[test]
fn slash_opens_the_search_and_enter_asks_linear() {
    let mut app = app();

    let effects = search_for(&mut app, "cheese");

    assert_eq!(effects, vec![Effect::Search("cheese".to_owned())]);
    assert!(
        shows(&app, "Searching for \"cheese\""),
        "{:#?}",
        screen(&app)
    );
}

#[test]
fn an_empty_query_asks_for_nothing() {
    let mut app = app();

    let effects = search_for(&mut app, "  ");

    assert!(effects.is_empty(), "{effects:?}");
}

#[tokio::test]
async fn the_results_show_as_a_list_closed_issues_included() {
    let mut app = app();

    searched(&mut app, "cheese").await;

    assert_snapshot!(app.backend());
}

#[tokio::test]
async fn enter_opens_a_result_in_the_detail_and_esc_goes_back_to_the_results() {
    let mut app = app();
    searched(&mut app, "cheese").await;

    app.handle(press(KeyCode::Down)).unwrap();
    app.handle(press(KeyCode::Enter)).unwrap();
    let detail = screen(&app).join("\n");
    app.handle(press(KeyCode::Esc)).unwrap();

    assert_snapshot!(detail);
    assert!(shows(&app, " Search Linear "), "{:#?}", screen(&app));
}

#[tokio::test]
async fn an_opened_result_loads_its_detail_at_once_and_j_and_k_scroll_it() {
    let mut app = App::new(TestBackend::new(80, 12), State::demo()).unwrap();
    searched(&mut app, "cheese").await;
    let first = found("cheese").await.issues.remove(0);

    let effects = app.handle(press(KeyCode::Enter)).unwrap();
    let title = screen(&app)[2].clone();
    app.handle(key('j')).unwrap();
    let scrolled = screen(&app)[2].clone();
    app.handle(key('k')).unwrap();

    assert_eq!(
        effects,
        vec![Effect::LoadIssueDetail {
            issue_id: first.id,
            urgency: Urgency::Urgent,
        }]
    );
    assert!(title.contains(&first.title), "{title}");
    assert!(!scrolled.contains(&first.title), "{scrolled}");
    assert_eq!(screen(&app)[2], title);
}

#[tokio::test]
async fn j_and_k_move_through_the_results() {
    let mut app = app();
    searched(&mut app, "cheese").await;
    let first_two: Vec<String> = found("cheese")
        .await
        .issues
        .into_iter()
        .take(2)
        .map(|issue| issue.identifier)
        .collect();

    app.handle(key('j')).unwrap();
    app.handle(key('j')).unwrap();
    app.handle(key('k')).unwrap();
    app.handle(press(KeyCode::Enter)).unwrap();

    let title_row = &screen(&app)[1];
    assert!(title_row.contains(&first_two[1]), "{title_row}");
}

#[tokio::test]
async fn an_answer_to_an_older_search_does_not_replace_the_newer_one() {
    let mut app = app();
    search_for(&mut app, "cheese");
    app.handle(press(KeyCode::Esc)).unwrap();
    search_for(&mut app, "burrow");

    let result = Ok(found("cheese").await);
    app.handle(Event::SearchDone {
        term: "cheese".to_owned(),
        result,
    })
    .unwrap();

    assert!(
        shows(&app, "Searching for \"burrow\""),
        "{:#?}",
        screen(&app)
    );
}

#[test]
fn a_failed_search_says_why_and_the_query_can_be_sent_again() {
    let mut app = app();
    search_for(&mut app, "cheese");

    app.handle(Event::SearchDone {
        term: "cheese".to_owned(),
        result: Err("Search failed: timed out connecting to Linear".to_owned()),
    })
    .unwrap();
    let shown = shows(&app, "timed out connecting to Linear");
    let effects = app.handle(press(KeyCode::Enter)).unwrap();

    assert!(shown, "{:#?}", screen(&app));
    assert_eq!(effects, vec![Effect::Search("cheese".to_owned())]);
}

#[tokio::test]
async fn nothing_found_says_so() {
    let mut app = app();

    searched(&mut app, "gouda fondue").await;

    assert!(
        shows(&app, "Nothing matches \"gouda fondue\""),
        "{:#?}",
        screen(&app)
    );
}

#[tokio::test]
async fn a_search_keeps_the_place_in_my_issues() {
    let mut app = app();
    type_text(&mut app, "jj");
    let focused = screen(&app)
        .into_iter()
        .find(|line| line.contains('▸'))
        .unwrap();

    searched(&mut app, "cheese").await;
    app.handle(press(KeyCode::Esc)).unwrap();

    assert!(shows(&app, &focused), "{:#?}", screen(&app));
}

#[tokio::test]
async fn the_palette_finds_an_issue_a_search_loaded_and_opens_its_detail() {
    let mut app = app();
    searched(&mut app, "cheese").await;
    let closed = found("cheese")
        .await
        .issues
        .into_iter()
        .find(|issue| !issue.state.state_type.is_open())
        .unwrap();
    app.handle(press(KeyCode::Esc)).unwrap();

    app.handle(key(':')).unwrap();
    type_text(&mut app, &closed.identifier.to_lowercase());
    app.handle(press(KeyCode::Enter)).unwrap();

    let title_row = &screen(&app)[1];
    assert!(title_row.contains(&closed.identifier), "{title_row}");
    assert!(shows(&app, &closed.title), "{:#?}", screen(&app));
}
