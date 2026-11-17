//! The search overlay: `/`, a query, Enter to ask Linear, and the results
//! in the results view, where Enter opens each one in its detail.

// clippy's allow-unwrap-in-tests does not cover helpers outside #[test] functions.
#![allow(clippy::unwrap_used)]

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use insta::assert_snapshot;
use linear_rat::app::{App, Effect, Event, State};
use linear_rat::linear::{FakeClient, LinearClient, SearchQuery, SearchResults, Urgency};
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
    FakeClient
        .search_issues(&SearchQuery::new(term))
        .await
        .unwrap()
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
        query: SearchQuery::new(term),
        result,
    })
    .unwrap();
}

#[test]
fn slash_opens_the_search_and_enter_asks_linear() {
    let mut app = app();

    let effects = search_for(&mut app, "cheese");

    assert_eq!(effects, vec![Effect::Search(SearchQuery::new("cheese"))]);
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
    let closed = found("cheese")
        .await
        .issues
        .into_iter()
        .find(|issue| !issue.state.state_type.is_open())
        .unwrap();

    searched(&mut app, "cheese").await;

    assert!(!shows(&app, " Search Linear "), "{:#?}", screen(&app));
    assert!(shows(&app, &closed.identifier), "{:#?}", screen(&app));
    assert!(shows(&app, "● Done"), "{:#?}", screen(&app));
}

#[tokio::test]
async fn enter_opens_a_result_in_the_detail_and_esc_goes_back_to_the_results() {
    let mut app = app();
    searched(&mut app, "cheese").await;

    let second = found("cheese").await.issues.remove(1);

    app.handle(key('j')).unwrap();
    app.handle(press(KeyCode::Enter)).unwrap();
    let detail = screen(&app).join("\n");
    app.handle(press(KeyCode::Esc)).unwrap();

    assert_snapshot!(detail);
    let focused = screen(&app)
        .into_iter()
        .find(|line| line.contains('▸'))
        .unwrap_or_default();
    assert!(shows(&app, "Search \"cheese\""), "{:#?}", screen(&app));
    assert!(focused.contains(&second.identifier), "{focused}");
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
        query: SearchQuery::new("cheese"),
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
        query: SearchQuery::new("cheese"),
        result: Err("Search failed: timed out connecting to Linear".to_owned()),
    })
    .unwrap();
    let shown = shows(&app, "timed out connecting to Linear");
    let effects = app.handle(press(KeyCode::Enter)).unwrap();

    assert!(shown, "{:#?}", screen(&app));
    assert_eq!(effects, vec![Effect::Search(SearchQuery::new("cheese"))]);
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
async fn the_palette_finds_an_issue_a_search_loaded_and_enter_opens_its_detail() {
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
    app.handle(press(KeyCode::Enter)).unwrap();

    let title_row = &screen(&app)[1];
    assert!(title_row.contains(&closed.identifier), "{title_row}");
    assert!(shows(&app, &closed.title), "{:#?}", screen(&app));
}

fn ctrl(c: char) -> Event {
    Event::Key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL))
}

/// Carries out the Team Issues loads in `effects` with the fake client
/// until nothing is left to load.
async fn settle(app: &mut App<TestBackend>, effects: Vec<Effect>) {
    let mut queue = std::collections::VecDeque::from(effects);
    while let Some(effect) = queue.pop_front() {
        let event = match effect {
            Effect::LoadTeams => Event::TeamsLoaded(Ok(FakeClient.teams().await.unwrap())),
            Effect::LoadTeamOptions(team_id) => {
                Event::TeamOptionsLoaded(Ok(FakeClient.team_options(&team_id).await.unwrap()))
            }
            Effect::LoadTeamIssues { query, after } => Event::TeamIssuesLoaded {
                result: Ok(FakeClient
                    .team_issues(&query, after.as_deref())
                    .await
                    .unwrap()),
                query,
                after,
            },
            Effect::CountTeamIssues(query) => Event::TeamIssuesCounted {
                result: Ok(FakeClient.count_team_issues(&query).await.unwrap()),
                query,
            },
            _ => continue,
        };
        queue.extend(app.handle(event).unwrap());
    }
}

/// Team Issues on its first team, filtered to the fifth state filter entry
/// (In Progress) and then the label `backend`.
async fn filtered_team_issues() -> App<TestBackend> {
    let mut app = app();
    let effects = type_text(&mut app, "2");
    settle(&mut app, effects).await;
    for keys in ["fs5", "flbackend\n"] {
        let effects = keys
            .chars()
            .flat_map(|c| {
                let event = if c == '\n' {
                    press(KeyCode::Enter)
                } else {
                    key(c)
                };
                app.handle(event).unwrap()
            })
            .collect();
        settle(&mut app, effects).await;
    }
    app
}

/// The team and filter Team Issues shows.
fn shown_query(app: &App<TestBackend>) -> linear_rat::linear::TeamQuery {
    app.state().team_issues.query().unwrap().clone()
}

#[test]
fn tab_looks_in_comments_too_and_enter_says_so() {
    let mut app = app();
    app.handle(key('/')).unwrap();
    type_text(&mut app, "standup");

    app.handle(press(KeyCode::Tab)).unwrap();
    let effects = app.handle(press(KeyCode::Enter)).unwrap();

    assert_eq!(
        effects,
        vec![Effect::Search(SearchQuery {
            include_comments: true,
            ..SearchQuery::new("standup")
        })]
    );
}

#[test]
fn from_my_issues_the_prompt_offers_comments_but_no_team() {
    let mut app = app();

    app.handle(key('/')).unwrap();
    let ctrl_t = app.handle(ctrl('t')).unwrap();

    assert!(shows(&app, "[ ] Include comments"), "{:#?}", screen(&app));
    assert!(!shows(&app, "In ENG"), "{:#?}", screen(&app));
    assert!(ctrl_t.is_empty(), "{ctrl_t:?}");
    assert_eq!(
        search_for_typed(&mut app, "cheese"),
        vec![Effect::Search(SearchQuery::new("cheese"))]
    );
}

/// Types `term` into the open prompt and sends it.
fn search_for_typed(app: &mut App<TestBackend>, term: &str) -> Vec<Effect> {
    type_text(app, term);
    app.handle(press(KeyCode::Enter)).unwrap()
}

#[tokio::test]
async fn from_team_issues_the_search_keeps_to_its_team_and_filter() {
    let mut app = filtered_team_issues().await;
    let within = shown_query(&app);

    app.handle(key('/')).unwrap();
    let prompt = app.backend().to_string();
    let effects = search_for_typed(&mut app, "cheese");

    assert_snapshot!(prompt);
    assert_eq!(
        effects,
        vec![Effect::Search(SearchQuery {
            within: Some(within),
            ..SearchQuery::new("cheese")
        })]
    );
}

#[tokio::test]
async fn ctrl_t_searches_the_whole_workspace_again() {
    let mut app = filtered_team_issues().await;
    app.handle(key('/')).unwrap();

    app.handle(ctrl('t')).unwrap();
    app.handle(press(KeyCode::Tab)).unwrap();
    let prompt = app.backend().to_string();
    let effects = search_for_typed(&mut app, "cheese");

    assert_snapshot!(prompt);
    assert_eq!(
        effects,
        vec![Effect::Search(SearchQuery {
            include_comments: true,
            ..SearchQuery::new("cheese")
        })]
    );
}

#[tokio::test]
async fn both_options_reset_each_time_the_prompt_opens() {
    let mut app = filtered_team_issues().await;
    let within = shown_query(&app);
    app.handle(key('/')).unwrap();
    app.handle(ctrl('t')).unwrap();
    app.handle(press(KeyCode::Tab)).unwrap();
    app.handle(press(KeyCode::Esc)).unwrap();

    app.handle(key('/')).unwrap();
    let effects = search_for_typed(&mut app, "cheese");

    assert_eq!(
        effects,
        vec![Effect::Search(SearchQuery {
            within: Some(within),
            ..SearchQuery::new("cheese")
        })]
    );
}

#[tokio::test]
async fn the_results_title_says_what_was_searched() {
    let mut app = filtered_team_issues().await;
    let within = shown_query(&app);
    let title = app.state().focused_issue().unwrap().title.clone();
    let term = title.split_whitespace().last().unwrap().to_lowercase();
    app.handle(key('/')).unwrap();
    app.handle(press(KeyCode::Tab)).unwrap();
    type_text(&mut app, &term);
    let effects = app.handle(press(KeyCode::Enter)).unwrap();
    let [Effect::Search(query)] = &effects[..] else {
        panic!("{effects:?}");
    };
    assert_eq!(query.within.as_ref(), Some(&within));

    let result = FakeClient.search_issues(query).await;
    app.handle(Event::SearchDone {
        query: query.clone(),
        result: result.map_err(|error| error.to_string()),
    })
    .unwrap();

    assert!(
        shows(&app, &format!("Search \"{term}\" in ENG (with comments)")),
        "{:#?}",
        screen(&app)
    );
}
