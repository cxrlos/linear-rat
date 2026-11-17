//! The Cycles and Projects screens driven through the app with the fake
//! client: browsing a team's cycles and the viewer's projects, and opening
//! the issues of one in the normal issue list.

// clippy's allow-unwrap-in-tests does not cover helpers outside #[test] functions.
#![allow(clippy::unwrap_used)]

use std::collections::VecDeque;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use insta::assert_snapshot;
use linear_rat::app::{App, Edit, Effect, Event, Screen, State, cycle_title};
use linear_rat::linear::model::Priority;
use linear_rat::linear::{FakeClient, LinearClient, demo};
use ratatui::backend::TestBackend;

fn sized(columns: u16, rows: u16) -> App<TestBackend> {
    App::new(TestBackend::new(columns, rows), State::demo()).unwrap()
}

fn key_event(c: char) -> Event {
    let code = match c {
        '\n' => KeyCode::Enter,
        '\x1b' => KeyCode::Esc,
        c => KeyCode::Char(c),
    };
    Event::Key(KeyEvent::new(code, KeyModifiers::NONE))
}

/// Presses each key of `keys` (`\n` is Enter, `\x1b` Esc) and returns every
/// effect they asked for.
fn press(app: &mut App<TestBackend>, keys: &str) -> Vec<Effect> {
    keys.chars()
        .flat_map(|c| app.handle(key_event(c)).unwrap())
        .collect()
}

/// Carries out every load in `effects` with the fake client, and the loads
/// their results ask for, until none is left. Returns the other effects.
async fn settle(app: &mut App<TestBackend>, effects: Vec<Effect>) -> Vec<Effect> {
    let client = FakeClient;
    let mut queue = VecDeque::from(effects);
    let mut rest = Vec::new();
    while let Some(effect) = queue.pop_front() {
        let event = match effect {
            Effect::LoadTeams => Event::TeamsLoaded(Ok(client.teams().await.unwrap())),
            Effect::LoadTeamOptions(team_id) => {
                Event::TeamOptionsLoaded(Ok(client.team_options(&team_id).await.unwrap()))
            }
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
            Effect::LoadCycles(team_id) => {
                let result = Ok(client.recent_cycles(&team_id).await.unwrap());
                Event::CyclesLoaded { team_id, result }
            }
            Effect::LoadProjects => {
                Event::ProjectsLoaded(Ok(client.viewer_projects().await.unwrap()))
            }
            Effect::LoadCycleIssues(cycle_id) => {
                let result = Ok(client.cycle_issues(&cycle_id).await.unwrap());
                Event::CycleIssuesLoaded { cycle_id, result }
            }
            Effect::LoadProjectIssues(project_id) => {
                let result = Ok(client.project_issues(&project_id).await.unwrap());
                Event::ProjectIssuesLoaded { project_id, result }
            }
            other => {
                rest.push(other);
                continue;
            }
        };
        queue.extend(app.handle(event).unwrap());
    }
    rest
}

/// Presses `keys` and carries out the loads they ask for.
async fn run(app: &mut App<TestBackend>, keys: &str) -> Vec<Effect> {
    let effects = press(app, keys);
    settle(app, effects).await
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

/// The row a line containing `text` is on.
fn row_of(app: &App<TestBackend>, text: &str) -> usize {
    screen(app)
        .iter()
        .position(|line| line.contains(text))
        .unwrap_or_else(|| panic!("{text} is not on screen: {:#?}", screen(app)))
}

#[tokio::test]
async fn the_cycles_screen_lists_the_first_teams_recent_cycles() {
    let mut app = sized(100, 24);

    run(&mut app, "4").await;

    assert_eq!(app.state().screen, Screen::Cycles);
    assert!(shows(&app, "active"), "{:#?}", screen(&app));
    assert_snapshot!(app.backend());
}

#[tokio::test]
async fn the_projects_screen_lists_status_target_date_lead_and_progress() {
    let mut app = sized(100, 24);

    run(&mut app, "5").await;

    assert_eq!(app.state().screen, Screen::Projects);
    assert!(shows(&app, "Nibble 3.0"), "{:#?}", screen(&app));
    assert_snapshot!(app.backend());
}

#[tokio::test]
async fn the_projects_screen_shows_loading_until_projects_arrive() {
    let mut app = sized(80, 24);

    let effects = press(&mut app, "5");

    assert_eq!(effects, vec![Effect::LoadProjects]);
    assert!(shows(&app, "Loading projects"), "{:#?}", screen(&app));
}

#[tokio::test]
async fn enter_on_a_cycle_lists_its_issues_and_esc_returns_with_the_focus_kept() {
    let mut app = sized(100, 24);
    run(&mut app, "4j").await;
    let before = screen(&app);
    let focused = app.state().cycles.focused().unwrap().clone();

    run(&mut app, "\n").await;

    assert_eq!(app.state().screen, Screen::Results);
    let ids = app.state().results.ids();
    assert!(!ids.is_empty());
    for id in ids {
        let issue = app.state().store.issue(id).unwrap();
        assert_eq!(issue.cycle_id.as_deref(), Some(focused.id.as_str()));
    }
    run(&mut app, "\x1b").await;
    assert_eq!(screen(&app), before);
}

#[tokio::test]
async fn enter_on_a_project_lists_its_issues_grouped_by_milestone() {
    let mut app = sized(100, 30);
    run(&mut app, "5").await;

    run(&mut app, "\n").await;

    assert_eq!(app.state().screen, Screen::Results);
    let beta = row_of(&app, "Beta (");
    let launch = row_of(&app, "Launch (");
    let none = row_of(&app, "No milestone (");
    assert!(beta < launch && launch < none, "{:#?}", screen(&app));
    assert_snapshot!(app.backend());
}

#[tokio::test]
async fn an_edit_from_a_project_list_is_sent_like_one_from_any_list() {
    let mut app = sized(100, 30);
    run(&mut app, "5").await;
    run(&mut app, "\n").await;
    let focused = app.state().focused_issue().unwrap().clone();

    let effects = press(&mut app, "p1");

    let [Effect::UpdateIssue(write)] = effects.as_slice() else {
        panic!("{effects:?}");
    };
    assert_eq!(write.issue_id, focused.id);
    assert_eq!(write.edit, Edit::Priority(Priority::Urgent));
    let shown = app.state().store.issue(&focused.id).unwrap();
    assert_eq!(shown.priority, Priority::Urgent);
}

#[tokio::test]
async fn t_switches_the_team_of_the_cycles_screen() {
    let mut app = sized(100, 24);
    run(&mut app, "4").await;
    let teams = demo::workspace().teams;

    let effects = press(&mut app, "t2");

    assert_eq!(effects, vec![Effect::LoadCycles(teams[1].id.clone())]);
    settle(&mut app, effects).await;
    assert!(shows(&app, &teams[1].name), "{:#?}", screen(&app));
}

#[tokio::test]
async fn the_cheatsheet_on_the_cycles_screen_lists_its_keys() {
    let mut app = sized(100, 30);
    run(&mut app, "4").await;

    press(&mut app, "?");

    assert_snapshot!(app.backend());
}

/// The answers to the issue-list loads in `effects`, as events not yet
/// handled.
async fn answers(effects: Vec<Effect>) -> Vec<Event> {
    let client = FakeClient;
    let mut events = Vec::new();
    for effect in effects {
        events.push(match effect {
            Effect::LoadCycleIssues(cycle_id) => {
                let result = Ok(client.cycle_issues(&cycle_id).await.unwrap());
                Event::CycleIssuesLoaded { cycle_id, result }
            }
            Effect::LoadProjectIssues(project_id) => {
                let result = Ok(client.project_issues(&project_id).await.unwrap());
                Event::ProjectIssuesLoaded { project_id, result }
            }
            other => panic!("not an issue list: {other:?}"),
        });
    }
    events
}

#[tokio::test]
async fn a_cycles_issues_arriving_after_enter_on_another_cycle_do_not_open() {
    let mut app = sized(100, 24);
    run(&mut app, "4").await;
    let first = answers(press(&mut app, "\n")).await;
    let second = answers(press(&mut app, "j\n")).await;
    let wanted = cycle_title(app.state().cycles.focused().unwrap());

    for event in first {
        app.handle(event).unwrap();
    }
    assert_eq!(app.state().screen, Screen::Cycles);

    for event in second {
        app.handle(event).unwrap();
    }
    assert_eq!(app.state().screen, Screen::Results);
    assert_eq!(app.state().results.title(), wanted);
}

#[tokio::test]
async fn a_projects_issues_arriving_after_enter_on_another_project_do_not_open() {
    let mut app = sized(100, 24);
    run(&mut app, "5").await;
    let first = answers(press(&mut app, "\n")).await;
    let second = answers(press(&mut app, "j\n")).await;
    let wanted = app.state().projects.focused().unwrap().project.name.clone();

    for event in first {
        app.handle(event).unwrap();
    }
    assert_eq!(app.state().screen, Screen::Projects);

    for event in second {
        app.handle(event).unwrap();
    }
    assert_eq!(app.state().screen, Screen::Results);
    assert_eq!(app.state().results.title(), wanted);
}
