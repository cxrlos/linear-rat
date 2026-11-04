//! The Team view of the Analytics screen through the app driver: the view
//! toggle, the team switcher, loading and caching per team, the stale
//! threshold from config, and what the view renders from the demo workspace
//! for teams the viewer administers and one they do not.

// clippy's allow-unwrap-in-tests does not cover helpers outside #[test] functions.
#![allow(clippy::unwrap_used)]

use std::path::Path;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use insta::assert_snapshot;
use jiff::Timestamp;
use linear_rat::app::{App, Effect, Event, State};
use linear_rat::config;
use linear_rat::linear::model::Team;
use linear_rat::linear::{AnalyticsIssues, AnalyticsWindow, TeamAnalyticsIssues, demo};
use ratatui::backend::TestBackend;

/// The Monday twelve weeks back from the demo's now, at midnight UTC.
const TWELVE_WEEKS: &str = "2026-08-17T00:00:00Z";
/// The Monday twenty-six weeks back.
const TWENTY_SIX_WEEKS: &str = "2026-05-11T00:00:00Z";

fn key(c: char) -> Event {
    Event::Key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE))
}

fn enter() -> Event {
    Event::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
}

fn at(text: &str) -> Timestamp {
    text.parse().unwrap()
}

fn team_load(team_id: &str, since: &str, until: Option<&str>) -> Effect {
    Effect::LoadTeamAnalytics {
        team_id: team_id.to_owned(),
        window: AnalyticsWindow {
            since: at(since),
            until: until.map(at),
        },
    }
}

/// Hands `app` the demo's answer to every load in `effects`, and to the
/// loads those answers ask for.
fn deliver(app: &mut App<TestBackend>, effects: Vec<Effect>) {
    for effect in effects {
        let event = match effect {
            Effect::LoadAnalytics(window) => Event::AnalyticsLoaded {
                window,
                result: Ok(AnalyticsIssues::demo(window)),
            },
            Effect::LoadTeamAnalytics { team_id, window } => Event::TeamAnalyticsLoaded {
                result: Ok(TeamAnalyticsIssues::demo(&team_id, window).unwrap()),
                team_id,
                window,
            },
            Effect::LoadAdministrableTeams => {
                Event::AdministrableTeamsLoaded(Ok(demo::administrable_team_ids()))
            }
            _ => continue,
        };
        let more = app.handle(event).unwrap();
        deliver(app, more);
    }
}

/// Analytics open on the demo workspace with the personal load delivered.
fn personal(state: State, columns: u16, rows: u16) -> App<TestBackend> {
    let mut app = App::new(TestBackend::new(columns, rows), state).unwrap();
    let effects = app.handle(key('3')).unwrap();
    deliver(&mut app, effects);
    app
}

/// The Team view open on the demo's first team, ENG, with every load
/// delivered.
fn team_view(columns: u16, rows: u16) -> App<TestBackend> {
    let mut app = personal(State::demo(), columns, rows);
    let effects = app.handle(key('v')).unwrap();
    deliver(&mut app, effects);
    app
}

/// The Team view switched to the `nth` of the viewer's teams (0 is ENG).
fn team_view_of(nth: usize, columns: u16, rows: u16) -> App<TestBackend> {
    let mut app = team_view(columns, rows);
    app.handle(key('t')).unwrap();
    for _ in 0..nth {
        app.handle(key('j')).unwrap();
    }
    let effects = app.handle(enter()).unwrap();
    deliver(&mut app, effects);
    app
}

fn screen(app: &App<TestBackend>) -> String {
    let buffer = app.backend().buffer();
    let width = usize::from(buffer.area.width);
    buffer
        .content()
        .chunks(width)
        .map(|row| row.iter().map(|cell| cell.symbol()).collect::<String>())
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn the_team_view_asks_once_for_administered_teams_and_the_first_teams_range() {
    let mut app = personal(State::demo(), 80, 24);

    let effects = app.handle(key('v')).unwrap();

    assert_eq!(
        effects,
        [
            Effect::LoadAdministrableTeams,
            team_load("team-eng", TWELVE_WEEKS, None),
        ]
    );
}

#[test]
fn back_in_the_personal_view_nothing_more_is_asked_for() {
    let mut app = team_view(80, 24);

    assert!(app.handle(key('v')).unwrap().is_empty());
    assert!(app.handle(key('v')).unwrap().is_empty(), "ENG is cached");
}

#[test]
fn a_longer_range_asks_the_team_only_for_the_weeks_it_adds() {
    let mut app = team_view(80, 24);

    let effects = app.handle(key(']')).unwrap();

    assert!(effects.contains(&team_load("team-eng", TWENTY_SIX_WEEKS, Some(TWELVE_WEEKS))));
    assert!(!effects.contains(&Effect::LoadAdministrableTeams));
}

#[test]
fn switching_team_asks_for_that_teams_range() {
    let mut app = team_view(80, 24);
    app.handle(key('t')).unwrap();
    app.handle(key('j')).unwrap();

    let effects = app.handle(enter()).unwrap();

    assert_eq!(effects, [team_load("team-des", TWELVE_WEEKS, None)]);
}

#[test]
fn switching_back_to_a_loaded_team_asks_for_nothing() {
    let mut app = team_view_of(1, 80, 24);
    app.handle(key('t')).unwrap();
    app.handle(key('k')).unwrap();

    assert!(app.handle(enter()).unwrap().is_empty());
    assert!(screen(&app).contains("ENG Engineering"));
}

#[test]
fn the_team_switch_is_only_in_the_team_view() {
    let mut app = personal(State::demo(), 80, 24);

    app.handle(key('t')).unwrap();

    assert!(!screen(&app).contains("Engineering"), "{}", screen(&app));
}

#[test]
fn a_failed_team_load_shows_and_is_asked_again() {
    let mut app = personal(State::demo(), 80, 24);
    let effects = app.handle(key('v')).unwrap();
    for effect in effects {
        if let Effect::LoadTeamAnalytics { team_id, window } = effect {
            app.handle(Event::TeamAnalyticsLoaded {
                team_id,
                window,
                result: Err("Linear is down".to_owned()),
            })
            .unwrap();
        }
    }
    assert!(screen(&app).contains("Could not load analytics: Linear is down"));

    app.handle(key('v')).unwrap();
    let effects = app.handle(key('v')).unwrap();

    assert!(effects.contains(&team_load("team-eng", TWELVE_WEEKS, None)));
}

#[test]
fn the_per_person_breakdown_shows_for_an_administered_team() {
    let app = team_view(200, 80);

    assert!(screen(&app).contains("Per person"));
}

#[test]
fn the_per_person_breakdown_is_absent_for_a_team_the_viewer_does_not_administer() {
    let app = team_view_of(1, 200, 80);

    assert!(screen(&app).contains("DES Design"));
    assert!(!screen(&app).contains("Per person"));
}

#[test]
fn the_stale_threshold_is_read_from_config() {
    let loaded = config::parse("[analytics]\nstale-days = 30\n", Path::new("config.toml"));
    let mut app = personal(State::demo().with_config(&loaded), 200, 80);
    let effects = app.handle(key('v')).unwrap();
    deliver(&mut app, effects);

    let text = screen(&app);
    assert!(text.contains("no update in 30 days"), "{text}");
    assert!(!text.contains("no update in 14 days"));
    // The default lists nine ENG issues idle 14 days or more; only those idle
    // 30 days or more stay stale at the configured threshold.
    assert!(
        text.contains("Stale issues  3, no update in 30 days"),
        "{text}"
    );
    assert!(!text.contains("ENG-46 Add pagination cursors"), "{text}");
}

#[test]
fn the_team_view_waits_for_the_viewers_teams_and_says_when_there_are_none() {
    let mut app = App::new(TestBackend::new(80, 24), State::demo()).unwrap();
    let effects = app.handle(key('3')).unwrap();
    assert_eq!(
        app.handle(key('v')).unwrap(),
        [Effect::LoadAdministrableTeams],
        "no team known yet"
    );
    assert!(screen(&app).contains("Loading your teams from Linear…"));

    for effect in effects {
        if let Effect::LoadAnalytics(window) = effect {
            let result = Ok(AnalyticsIssues {
                teams: Vec::new(),
                ..AnalyticsIssues::demo(window)
            });
            app.handle(Event::AnalyticsLoaded { window, result })
                .unwrap();
        }
    }

    assert!(screen(&app).contains("You are not a member of any team."));
}

#[test]
fn administered_team_in_points_at_80x24() {
    assert_snapshot!(team_view(80, 24).backend());
}

#[test]
fn administered_team_in_points_at_200x60() {
    assert_snapshot!(team_view(200, 60).backend());
}

#[test]
fn team_not_administered_at_200x60() {
    assert_snapshot!(team_view_of(1, 200, 60).backend());
}

#[test]
fn administered_team_without_estimates_at_200x60() {
    assert_snapshot!(team_view_of(2, 200, 60).backend());
}

#[test]
fn the_team_switcher_at_80x24() {
    let mut app = team_view(80, 24);

    app.handle(key('t')).unwrap();

    assert_snapshot!(app.backend());
}

#[test]
fn a_long_team_name_is_cut_so_the_range_picker_stays_in_the_header() {
    const LONG: &str = "Platform Infrastructure and Developer Experience";
    let rename = |team: &mut Team| {
        if team.id == "team-eng" {
            team.name = LONG.to_owned();
        }
    };
    let mut app = App::new(TestBackend::new(80, 24), State::demo()).unwrap();
    let mut effects = app.handle(key('3')).unwrap();
    effects.extend(app.handle(key('v')).unwrap());
    while let Some(effect) = effects.pop() {
        let event = match effect {
            Effect::LoadAnalytics(window) => {
                let mut result = AnalyticsIssues::demo(window);
                result.teams.iter_mut().for_each(rename);
                Event::AnalyticsLoaded {
                    window,
                    result: Ok(result),
                }
            }
            Effect::LoadTeamAnalytics { team_id, window } => {
                let mut result = TeamAnalyticsIssues::demo(&team_id, window).unwrap();
                rename(&mut result.team);
                Event::TeamAnalyticsLoaded {
                    result: Ok(result),
                    team_id,
                    window,
                }
            }
            Effect::LoadAdministrableTeams => {
                Event::AdministrableTeamsLoaded(Ok(demo::administrable_team_ids()))
            }
            _ => continue,
        };
        effects.extend(app.handle(event).unwrap());
    }

    let text = screen(&app);
    let header = text.lines().nth(2).unwrap();
    assert!(header.contains("t ENG Platform Infr…   Range"), "{header}");
    assert!(header.contains("26 weeks │"), "{header}");
}
