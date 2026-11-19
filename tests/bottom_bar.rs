//! The bottom bar on each screen, at 80 columns and wide: moves, actions,
//! and a third row with the issue edits or the screen's own keys.

// clippy's allow-unwrap-in-tests does not cover helpers outside #[test] functions.
#![allow(clippy::unwrap_used)]

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use insta::assert_snapshot;
use linear_rat::app::{App, Effect, Event, State};
use linear_rat::linear::{FakeClient, LinearClient};
use ratatui::backend::TestBackend;
use ratatui::style::Color;

const WIDE: (u16, u16) = (160, 40);

fn sized((columns, rows): (u16, u16)) -> App<TestBackend> {
    App::new(TestBackend::new(columns, rows), State::demo()).unwrap()
}

fn press(app: &mut App<TestBackend>, keys: &str) -> Vec<Effect> {
    keys.chars()
        .flat_map(|c| {
            app.handle(Event::Key(KeyEvent::new(
                KeyCode::Char(c),
                KeyModifiers::NONE,
            )))
            .unwrap()
        })
        .collect()
}

/// Carries out the Team Issues loads in `effects` with the fake client,
/// and the loads their results ask for.
async fn settle(app: &mut App<TestBackend>, effects: Vec<Effect>) {
    let client = FakeClient;
    let mut queue = std::collections::VecDeque::from(effects);
    while let Some(effect) = queue.pop_front() {
        let event = match effect {
            Effect::LoadTeams => Event::TeamsLoaded(Ok(client.teams().await.unwrap())),
            Effect::LoadTeamIssues { query, after } => {
                let result = Ok(client.team_issues(&query, after.as_deref()).await.unwrap());
                Event::TeamIssuesLoaded {
                    query,
                    after,
                    result,
                }
            }
            Effect::CountTeamIssues(query) => {
                let result = Ok(client.count_team_issues(&query).await.unwrap());
                Event::TeamIssuesCounted { query, result }
            }
            _ => continue,
        };
        queue.extend(app.handle(event).unwrap());
    }
}

fn row(app: &App<TestBackend>, y: u16) -> String {
    let buffer = app.backend().buffer();
    (0..buffer.area.width)
        .map(|x| buffer[(x, y)].symbol())
        .collect()
}

/// The bar's three rows, trailing blanks trimmed.
fn bar(app: &App<TestBackend>) -> String {
    let height = app.backend().buffer().area.height;
    (height - 3..height)
        .map(|y| row(app, y).trim_end().to_owned())
        .collect::<Vec<_>>()
        .join("\n")
}

fn my_issues(size: (u16, u16)) -> App<TestBackend> {
    sized(size)
}

async fn team_issues(size: (u16, u16)) -> App<TestBackend> {
    let mut app = sized(size);
    let effects = press(&mut app, "2");
    settle(&mut app, effects).await;
    app
}

fn analytics(size: (u16, u16)) -> App<TestBackend> {
    let mut app = sized(size);
    press(&mut app, "3");
    app
}

fn priority_picker(size: (u16, u16)) -> App<TestBackend> {
    let mut app = sized(size);
    press(&mut app, "p");
    app
}

/// The color `text` starts with on row `y`.
fn fg_of(app: &App<TestBackend>, y: u16, text: &str) -> Option<Color> {
    let line = row(app, y);
    let x = line[..line.find(text)?].chars().count();
    let cell = app.backend().buffer().cell((u16::try_from(x).ok()?, y))?;
    Some(cell.fg)
}

#[test]
fn my_issues_names_every_issue_edit_at_80_columns() {
    let app = my_issues((80, 24));

    assert_eq!(
        row(&app, 23).trim_end(),
        " Edit │ s state  a assignee  p priority  L labels  e estimate  E text  c comment"
    );
}

#[test]
fn search_and_the_palette_show_on_every_screen() {
    let app = my_issues((80, 24));
    assert!(bar(&app).contains("/ search"), "{}", bar(&app));
    assert!(bar(&app).contains(": palette"), "{}", bar(&app));

    let app = analytics((80, 24));
    assert!(bar(&app).contains("/ search"), "{}", bar(&app));
    assert!(bar(&app).contains(": palette"), "{}", bar(&app));
}

#[tokio::test]
async fn team_issues_leads_its_third_row_with_its_own_keys_and_leaves_out_edits_that_do_not_fit() {
    let app = team_issues((80, 24)).await;

    let third = row(&app, 23);
    assert!(
        third.starts_with(" View │ t team  f filter  m more  s state"),
        "{third}"
    );
    assert!(!third.contains("comment"), "{third}");
}

#[test]
fn the_bar_is_three_rows_on_every_screen() {
    let app = my_issues((80, 24));
    assert!(row(&app, 20).starts_with('└'), "{}", row(&app, 20));
    assert!(row(&app, 21).starts_with(" Move"), "{}", row(&app, 21));

    let app = analytics((80, 24));
    assert!(row(&app, 21).starts_with(" Move"), "{}", bar(&app));
}

#[test]
fn edits_are_muted_without_an_issue_and_accented_with_one() {
    let empty = App::new(TestBackend::new(80, 24), State::default()).unwrap();
    assert_eq!(fg_of(&empty, 23, "s state"), Some(Color::DarkGray));

    let demo = my_issues((80, 24));
    assert_eq!(fg_of(&demo, 23, "s state"), Some(Color::Cyan));
}

#[test]
fn a_single_choice_picker_offers_enter_and_hides_the_toggle() {
    let app = priority_picker((80, 24));

    let bar = bar(&app);
    assert!(bar.contains("Enter choose"), "{bar}");
    assert!(!bar.contains("Tab"), "{bar}");
    assert_eq!(fg_of(&app, 22, "Enter"), Some(Color::Cyan));
}

#[test]
fn a_multi_select_picker_offers_the_toggle() {
    let mut app = sized((80, 24));
    press(&mut app, "L");

    let bar = bar(&app);
    assert!(bar.contains("Enter choose"), "{bar}");
    assert!(bar.contains("Tab toggle"), "{bar}");
}

#[test]
fn a_key_that_applies_but_cannot_run_now_stays_muted() {
    let mut app = priority_picker((80, 24));
    press(&mut app, "zzz");

    let bar = bar(&app);
    assert!(bar.contains("Enter choose"), "{bar}");
    assert_eq!(fg_of(&app, 22, "Enter"), Some(Color::DarkGray));
}

#[test]
fn the_search_overlay_shows_no_moves_it_cannot_make() {
    let mut app = sized((80, 24));
    press(&mut app, "/");

    let bar = bar(&app);
    assert!(!bar.contains("move"), "{bar}");
    assert!(bar.contains("Esc close"), "{bar}");
}

#[test]
fn bar_on_my_issues_at_80() {
    assert_snapshot!(bar(&my_issues((80, 24))));
}

#[test]
fn bar_on_my_issues_wide() {
    assert_snapshot!(bar(&my_issues(WIDE)));
}

#[tokio::test]
async fn bar_on_team_issues_at_80() {
    assert_snapshot!(bar(&team_issues((80, 24)).await));
}

#[tokio::test]
async fn bar_on_team_issues_wide() {
    assert_snapshot!(bar(&team_issues(WIDE).await));
}

#[test]
fn bar_on_analytics_at_80() {
    assert_snapshot!(bar(&analytics((80, 24))));
}

#[test]
fn bar_on_analytics_wide() {
    assert_snapshot!(bar(&analytics(WIDE)));
}

#[test]
fn bar_in_a_picker_at_80() {
    assert_snapshot!(bar(&priority_picker((80, 24))));
}

#[test]
fn bar_in_a_picker_wide() {
    assert_snapshot!(bar(&priority_picker(WIDE)));
}
