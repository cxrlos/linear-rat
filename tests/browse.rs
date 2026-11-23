//! The Cycles and Projects screens driven through the app with the fake
//! client: browsing a team's cycles and the viewer's projects, and opening
//! the issues of one in the normal issue list.

// clippy's allow-unwrap-in-tests does not cover helpers outside #[test] functions.
#![allow(clippy::unwrap_used)]

use std::collections::VecDeque;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use insta::assert_snapshot;
use jiff::SignedDuration;
use linear_rat::app::{App, Edit, Effect, Event, Failure, Screen, State, cycle_title};
use linear_rat::linear::model::Priority;
use linear_rat::linear::{ErrorKind, FakeClient, LinearClient, demo};
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
async fn an_edit_from_a_cycle_list_that_linear_rejects_is_rolled_back() {
    let mut app = sized(100, 30);
    run(&mut app, "4").await;
    run(&mut app, "\n").await;
    let focused = app.state().focused_issue().unwrap().clone();
    assert_ne!(focused.priority, Priority::Urgent);
    let effects = press(&mut app, "p1");
    let [Effect::UpdateIssue(write)] = effects.as_slice() else {
        panic!("{effects:?}");
    };

    app.handle(Event::IssueUpdated {
        write: write.clone(),
        result: Err(Failure::new(
            ErrorKind::Other,
            "test",
            "Linear returned errors: Entity not found (INVALID_INPUT)",
        )),
    })
    .unwrap();

    assert_eq!(app.state().screen, Screen::Results);
    let shown = app.state().store.issue(&focused.id).unwrap();
    assert_eq!(shown.priority, focused.priority);
    assert!(
        shows(
            &app,
            &format!("Could not change the priority of {}", focused.identifier)
        ),
        "{:#?}",
        screen(&app)
    );
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

#[tokio::test]
async fn a_cycle_with_many_issues_shows_its_whole_count() {
    let mut app = sized(100, 24);
    run(&mut app, "4").await;
    let team_id = app.state().cycles.team_id().unwrap().to_owned();
    let mut cycle = app.state().cycles.cycles()[0].clone();
    cycle.issue_count_history = vec![140];
    cycle.completed_issue_count_history = vec![123];

    app.handle(Event::CyclesLoaded {
        team_id,
        result: Ok(vec![cycle]),
    })
    .unwrap();

    assert!(shows(&app, " 123 of 140 done"), "{:#?}", screen(&app));
}

/// Moves the clock to `seconds` after the demo's now.
fn tick(app: &mut App<TestBackend>, seconds: i64) -> Vec<Effect> {
    let now = demo::NOW
        .checked_add(SignedDuration::from_secs(seconds))
        .unwrap();
    app.handle(Event::Tick(now)).unwrap()
}

/// The loads of the Cycles and Projects screens among `effects`.
fn browse_loads(effects: Vec<Effect>) -> Vec<Effect> {
    effects
        .into_iter()
        .filter(|effect| {
            matches!(
                effect,
                Effect::LoadCycles(_)
                    | Effect::LoadProjects
                    | Effect::LoadCycleIssues(_)
                    | Effect::LoadProjectIssues(_)
            )
        })
        .collect()
}

#[tokio::test]
async fn refresh_on_the_cycles_screen_reloads_the_teams_cycles_keeping_the_rows() {
    let mut app = sized(100, 24);
    run(&mut app, "4").await;
    let team_id = app.state().cycles.team_id().unwrap().to_owned();
    let before = screen(&app);

    let effects = press(&mut app, "r");

    assert_eq!(browse_loads(effects), [Effect::LoadCycles(team_id)]);
    assert_eq!(screen(&app), before);
    assert!(browse_loads(press(&mut app, "r")).is_empty());
}

#[tokio::test]
async fn refresh_on_the_projects_screen_reloads_the_projects() {
    let mut app = sized(100, 24);
    run(&mut app, "5").await;

    let effects = press(&mut app, "r");

    assert_eq!(browse_loads(effects), [Effect::LoadProjects]);
    assert!(shows(&app, "Nibble 3.0"), "{:#?}", screen(&app));
}

#[tokio::test]
async fn refresh_on_a_cycles_issue_list_reloads_that_list_keeping_the_focus() {
    let mut app = sized(100, 24);
    run(&mut app, "4").await;
    let cycle_id = app.state().cycles.focused().unwrap().id.clone();
    run(&mut app, "\n").await;
    press(&mut app, "j");
    let focused = app.state().results.focused_id().unwrap().to_owned();
    assert_ne!(app.state().results.list().focused_index(), 0);

    let effects = press(&mut app, "r");

    assert_eq!(
        browse_loads(effects.clone()),
        [Effect::LoadCycleIssues(cycle_id)]
    );
    settle(&mut app, effects).await;
    assert_eq!(app.state().screen, Screen::Results);
    assert_eq!(app.state().results.focused_id(), Some(focused.as_str()));
    assert_ne!(app.state().results.list().focused_index(), 0);
}

#[tokio::test]
async fn refresh_on_a_projects_issue_list_reloads_that_list() {
    let mut app = sized(100, 24);
    run(&mut app, "5").await;
    let project_id = app.state().projects.focused().unwrap().project.id.clone();
    run(&mut app, "\n").await;

    let effects = press(&mut app, "r");

    assert_eq!(
        browse_loads(effects),
        [Effect::LoadProjectIssues(project_id)]
    );
}

#[tokio::test]
async fn a_cycle_list_reloaded_after_going_back_does_not_open_again() {
    let mut app = sized(100, 24);
    run(&mut app, "4").await;
    run(&mut app, "\n").await;
    let effects = press(&mut app, "r");

    press(&mut app, "\x1b");
    settle(&mut app, effects).await;

    assert_eq!(app.state().screen, Screen::Cycles);
}

#[tokio::test]
async fn showing_the_cycles_screen_again_reloads_only_once_the_poll_interval_passed() {
    let mut app = sized(100, 24);
    tick(&mut app, 0);
    run(&mut app, "4").await;

    tick(&mut app, 10);
    assert!(browse_loads(press(&mut app, "14")).is_empty());

    tick(&mut app, 20);
    let team_id = app.state().cycles.team_id().unwrap().to_owned();
    let effects = press(&mut app, "14");
    assert_eq!(browse_loads(effects), [Effect::LoadCycles(team_id)]);
    assert!(shows(&app, "active"), "{:#?}", screen(&app));
}

#[tokio::test]
async fn showing_the_projects_screen_again_reloads_only_once_the_poll_interval_passed() {
    let mut app = sized(100, 24);
    tick(&mut app, 0);
    run(&mut app, "5").await;

    tick(&mut app, 10);
    assert!(browse_loads(press(&mut app, "15")).is_empty());

    tick(&mut app, 20);
    assert_eq!(browse_loads(press(&mut app, "15")), [Effect::LoadProjects]);
    assert!(shows(&app, "Nibble 3.0"), "{:#?}", screen(&app));
}

#[tokio::test]
async fn going_back_to_a_stale_cycles_screen_reloads_it() {
    let mut app = sized(100, 24);
    tick(&mut app, 0);
    run(&mut app, "4").await;
    run(&mut app, "\n").await;
    tick(&mut app, 20);

    let effects = press(&mut app, "\x1b");

    assert_eq!(browse_loads(effects).len(), 1);
}

#[tokio::test]
async fn a_reload_keeps_the_focused_project_by_id() {
    let mut app = sized(100, 24);
    run(&mut app, "5").await;
    press(&mut app, "j");
    let focused = app.state().projects.focused().unwrap().project.id.clone();
    press(&mut app, "r");

    let mut list = FakeClient.viewer_projects().await.unwrap();
    list.projects.reverse();
    app.handle(Event::ProjectsLoaded(Ok(list))).unwrap();

    let now = app.state().projects.focused().unwrap();
    assert_eq!(now.project.id, focused);
}

#[tokio::test]
async fn a_reload_keeps_the_focused_cycle_by_id() {
    let mut app = sized(100, 24);
    run(&mut app, "4").await;
    press(&mut app, "j");
    let team_id = app.state().cycles.team_id().unwrap().to_owned();
    let focused = app.state().cycles.focused().unwrap().id.clone();
    press(&mut app, "r");

    let mut cycles = FakeClient.recent_cycles(&team_id).await.unwrap();
    cycles.retain(|cycle| cycle.id != app.state().cycles.cycles()[0].id);
    app.handle(Event::CyclesLoaded {
        team_id,
        result: Ok(cycles),
    })
    .unwrap();

    assert_eq!(app.state().cycles.focused().unwrap().id, focused);
}

#[tokio::test]
async fn a_reload_shows_in_the_sync_indicator_like_a_poll() {
    let mut app = sized(100, 24);
    run(&mut app, "5").await;
    tick(&mut app, 30);
    press(&mut app, "r");

    let list = FakeClient.viewer_projects().await.unwrap();
    app.handle(Event::ProjectsLoaded(Ok(list))).unwrap();

    let at = demo::NOW
        .checked_add(SignedDuration::from_secs(30))
        .unwrap();
    assert_eq!(app.state().sync_status.last_synced(), Some(at));
}

#[tokio::test]
async fn a_failed_reload_keeps_the_rows_and_marks_sync_failing() {
    let mut app = sized(100, 24);
    run(&mut app, "5").await;
    press(&mut app, "r");

    app.handle(Event::ProjectsLoaded(Err(Failure::new(
        ErrorKind::Other,
        "load the projects",
        "Linear could not be reached",
    ))))
    .unwrap();

    assert!(shows(&app, "Nibble 3.0"), "{:#?}", screen(&app));
    assert!(app.state().sync_status.indicator().unwrap().alert);
}
