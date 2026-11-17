//! The `?` cheatsheet as a grid of panes by purpose: Move, Edit the issue,
//! Open & copy, App and the screen's own keys.

// clippy's allow-unwrap-in-tests does not cover helpers outside #[test] functions.
#![allow(clippy::unwrap_used)]

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use insta::assert_snapshot;
use linear_rat::app::{App, Effect, Event, State};
use linear_rat::linear::{FakeClient, LinearClient};
use ratatui::backend::TestBackend;
use ratatui::style::Color;

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

/// The demo on My Issues at `columns` by `rows`, with the cheatsheet open.
fn cheatsheet(columns: u16, rows: u16) -> App<TestBackend> {
    let mut app = App::new(TestBackend::new(columns, rows), State::demo()).unwrap();
    press(&mut app, "?");
    app
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

fn text(app: &App<TestBackend>) -> String {
    screen(app).join("\n")
}

/// The color `needle` starts with on the first row that has it.
fn fg_of(app: &App<TestBackend>, needle: &str) -> Option<Color> {
    let rows = screen(app);
    let (y, line) = rows
        .iter()
        .enumerate()
        .find(|(_, line)| line.contains(needle))?;
    let x = line[..line.find(needle)?].chars().count();
    let cell = app
        .backend()
        .buffer()
        .cell((u16::try_from(x).ok()?, u16::try_from(y).ok()?))?;
    Some(cell.fg)
}

#[test]
fn at_80x24_every_pane_of_my_issues_fits_without_scrolling() {
    let mut app = cheatsheet(80, 24);
    let before = text(&app);

    for needle in [
        "Move",
        "Edit the issue",
        "Open & copy",
        "App",
        "C-z",
        "new issue",
        "copy branch / link",
    ] {
        assert!(before.contains(needle), "{needle} in\n{before}");
    }
    press(&mut app, "j");
    assert_eq!(text(&app), before, "nothing below to scroll to");
}

#[test]
fn at_80_columns_the_panes_flow_into_two_columns() {
    let app = cheatsheet(80, 24);

    let rows = screen(&app);
    let row = rows.iter().find(|row| row.contains("Move")).unwrap();
    assert!(row.contains("Edit the issue"), "{row}");
    assert!(!row.contains("Open & copy"), "{row}");
}

#[test]
fn a_wide_terminal_puts_more_panes_side_by_side() {
    let app = cheatsheet(160, 40);

    let rows = screen(&app);
    let row = rows.iter().find(|row| row.contains("Move")).unwrap();
    for title in ["Edit the issue", "Open & copy"] {
        assert!(row.contains(title), "{title} in {row}");
    }
}

#[test]
fn a_narrow_terminal_stacks_the_panes_in_one_column_that_scrolls() {
    let mut app = cheatsheet(50, 24);
    let rows = screen(&app);
    let row = rows.iter().find(|row| row.contains("Move")).unwrap();
    assert!(!row.contains("Edit the issue"), "{row}");
    assert!(!text(&app).contains("C-z"), "{}", text(&app));

    press(&mut app, &"j".repeat(40));

    assert!(text(&app).contains("C-z"), "{}", text(&app));
}

#[test]
fn pairs_share_a_line_with_a_short_description() {
    let app = cheatsheet(80, 24);

    let text = text(&app);
    for pair in [
        "j/k      down / up",
        "gg/G     top / bottom",
        "C-d/C-u  half page",
        "yi/yu  copy key / URL",
    ] {
        assert!(text.contains(pair), "{pair} in\n{text}");
    }
}

#[test]
fn keys_that_cannot_run_now_are_muted() {
    let app = cheatsheet(80, 24);

    assert_eq!(fg_of(&app, "1/2/3"), Some(Color::DarkGray), "already on 1");
    assert_eq!(fg_of(&app, "s  workflow state"), Some(Color::Cyan));
}

/// Carries out the Team Issues loads in `effects` with the fake client.
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

#[tokio::test]
async fn team_issues_adds_a_pane_of_its_own_keys() {
    let mut app = App::new(TestBackend::new(160, 40), State::demo()).unwrap();
    let effects = press(&mut app, "2");
    settle(&mut app, effects).await;

    press(&mut app, "?");

    let text = text(&app);
    assert!(text.contains("Team Issues  "), "{text}");
    assert!(text.contains("fs"), "{text}");
    assert_snapshot!(text);
}

#[test]
fn analytics_adds_a_pane_of_its_own_keys() {
    let mut app = App::new(TestBackend::new(80, 24), State::demo()).unwrap();
    press(&mut app, "3?");

    let text = text(&app);
    assert!(text.contains("[/]"), "{text}");
    assert_snapshot!(text);
}

#[test]
fn cheatsheet_at_80x24() {
    assert_snapshot!(text(&cheatsheet(80, 24)));
}

#[test]
fn cheatsheet_wide() {
    assert_snapshot!(text(&cheatsheet(160, 40)));
}

#[test]
fn cheatsheet_narrow() {
    assert_snapshot!(text(&cheatsheet(50, 24)));
}
