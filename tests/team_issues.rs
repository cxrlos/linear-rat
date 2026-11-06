//! Team Issues driven through the app with the fake client: the team
//! switcher, the filters, "N of M" and loading more on demand.

// clippy's allow-unwrap-in-tests does not cover helpers outside #[test] functions.
#![allow(clippy::unwrap_used)]

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use insta::assert_snapshot;
use std::path::Path;

use linear_rat::app::{App, Effect, Event, State};
use linear_rat::config;
use linear_rat::linear::{
    FakeClient, LinearClient, TeamFilter, TeamIssuesPage, TeamQuery, Urgency, demo,
};
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
    let mut queue = std::collections::VecDeque::from(effects);
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
            other => {
                rest.push(other);
                continue;
            }
        };
        queue.extend(app.handle(event).unwrap());
    }
    rest
}

/// The app on Team Issues once its first page and count are in.
async fn team_issues(columns: u16, rows: u16) -> App<TestBackend> {
    settled(sized(columns, rows)).await
}

/// `app` switched to Team Issues once its first page and count are in.
async fn settled(mut app: App<TestBackend>) -> App<TestBackend> {
    let effects = press(&mut app, "2");
    settle(&mut app, effects).await;
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

/// The header row: team, "N of M" and filters.
fn header(app: &App<TestBackend>) -> String {
    screen(app)[1].trim_end().to_owned()
}

/// N and M from the header's "N of M".
fn n_of_m(app: &App<TestBackend>) -> (usize, usize) {
    let header = header(app);
    let (_, after_team) = header.split_once(" · ").unwrap();
    let mut words = after_team.split_whitespace();
    let n = words.next().unwrap().parse().unwrap();
    assert_eq!(words.next(), Some("of"), "{header}");
    let m = words.next().unwrap().parse().unwrap();
    (n, m)
}

fn query(team_id: &str, filter: TeamFilter) -> TeamQuery {
    TeamQuery {
        team_id: team_id.to_owned(),
        filter,
    }
}

fn eng() -> TeamQuery {
    query("team-eng", TeamFilter::default())
}

#[test]
fn opening_team_issues_asks_once_for_the_viewers_teams() {
    let mut app = sized(80, 24);

    assert_eq!(press(&mut app, "2"), vec![Effect::LoadTeams]);
    assert!(screen(&app)[2].contains("Loading your teams"));
    assert_eq!(press(&mut app, "12"), Vec::new());
}

#[test]
fn coming_back_after_the_teams_failed_asks_for_them_again() {
    let mut app = sized(80, 24);
    press(&mut app, "2");
    app.handle(Event::TeamsLoaded(Err(
        "could not load your teams: Linear did not answer in time".to_owned(),
    )))
    .unwrap();

    assert_eq!(press(&mut app, "12"), vec![Effect::LoadTeams]);
    assert!(screen(&app)[2].contains("Loading your teams"));
}

#[tokio::test]
async fn the_first_team_shows_its_first_page_and_asks_for_it_and_its_count() {
    let mut app = sized(80, 24);
    press(&mut app, "2");
    let teams = FakeClient.teams().await.unwrap();

    let effects = app.handle(Event::TeamsLoaded(Ok(teams))).unwrap();

    assert_eq!(
        effects,
        vec![
            Effect::LoadTeamOptions("team-eng".to_owned()),
            Effect::LoadTeamIssues {
                query: eng(),
                after: None
            },
            Effect::CountTeamIssues(eng()),
        ]
    );
}

#[tokio::test]
async fn a_filter_opened_after_its_options_failed_asks_again_and_fills_in() {
    let mut app = sized(80, 24);
    press(&mut app, "2");
    let teams = FakeClient.teams().await.unwrap();
    let effects = app.handle(Event::TeamsLoaded(Ok(teams))).unwrap();
    app.handle(Event::TeamOptionsLoaded(Err(
        "could not load the team's filters: Linear did not answer in time".to_owned(),
    )))
    .unwrap();
    let rest: Vec<Effect> = effects
        .into_iter()
        .filter(|effect| !matches!(effect, Effect::LoadTeamOptions(_)))
        .collect();
    settle(&mut app, rest).await;

    let effects = press(&mut app, "fs");
    assert_eq!(
        effects,
        vec![Effect::LoadTeamOptions("team-eng".to_owned())]
    );
    assert_eq!(
        press(&mut app, "\x1bfs"),
        Vec::new(),
        "one request at a time"
    );

    settle(&mut app, effects).await;
    assert!(
        screen(&app).iter().any(|row| row.contains("In Progress")),
        "{}",
        screen(&app).join("\n")
    );
}

#[tokio::test]
async fn filter_options_linear_cut_short_say_so_in_the_notice() {
    let mut app = team_issues(80, 24).await;
    let mut options = FakeClient.team_options("team-eng").await.unwrap();
    options.problems =
        vec!["The filters may be missing some labels: Linear sent only the first page".to_owned()];

    app.handle(Event::TeamOptionsLoaded(Ok(options))).unwrap();

    assert!(
        screen(&app)[21].contains("The filters may be missing some labels"),
        "{}",
        screen(&app)[21]
    );
}

#[tokio::test]
async fn loading_more_after_the_count_failed_counts_again() {
    let mut app = sized(80, 24);
    press(&mut app, "2");
    let teams = FakeClient.teams().await.unwrap();
    let effects = app.handle(Event::TeamsLoaded(Ok(teams))).unwrap();
    let rest: Vec<Effect> = effects
        .into_iter()
        .filter(|effect| !matches!(effect, Effect::CountTeamIssues(_)))
        .collect();
    settle(&mut app, rest).await;
    app.handle(Event::TeamIssuesCounted {
        query: eng(),
        result: Err("could not count the team's issues: Linear did not answer in time".to_owned()),
    })
    .unwrap();
    assert!(header(&app).contains("50 of … m more"), "{}", header(&app));

    let effects = press(&mut app, "m");

    assert_eq!(
        effects,
        vec![
            Effect::LoadTeamIssues {
                query: eng(),
                after: Some("50".to_owned())
            },
            Effect::CountTeamIssues(eng()),
        ]
    );
    settle(&mut app, effects).await;
    assert_eq!(n_of_m(&app), (70, 70));
}

#[tokio::test]
async fn the_header_shows_n_of_m_and_the_key_that_loads_more() {
    let app = team_issues(80, 24).await;

    // The demo's Engineering team has 70 issues; a page holds 50.
    assert_eq!(n_of_m(&app), (50, 70));
    assert_eq!(
        header(&app),
        "t ENG Engineering · 50 of 70 m more · f Filters none"
    );
    assert_snapshot!(app.backend());
}

#[tokio::test]
async fn a_wide_terminal_shows_the_team_list_beside_the_focused_issue() {
    let app = team_issues(200, 60).await;

    assert_snapshot!(app.backend());
}

#[tokio::test]
async fn m_loads_the_next_page_until_every_issue_is_in() {
    let mut app = team_issues(80, 24).await;

    let effects = press(&mut app, "m");

    assert_eq!(
        effects,
        vec![Effect::LoadTeamIssues {
            query: eng(),
            after: Some("50".to_owned())
        }]
    );
    assert!(
        header(&app).contains("50 of 70 loading more"),
        "{}",
        header(&app)
    );
    assert_eq!(press(&mut app, "m"), Vec::new(), "one page at a time");

    settle(&mut app, effects).await;

    assert_eq!(n_of_m(&app), (70, 70));
    assert!(!header(&app).contains("more"), "{}", header(&app));
    assert_eq!(press(&mut app, "m"), Vec::new());
}

#[tokio::test]
async fn the_team_switcher_lists_the_viewers_teams_and_switching_reloads() {
    let mut app = team_issues(80, 24).await;

    press(&mut app, "t");
    assert_snapshot!("team_switcher", app.backend());

    let effects = press(&mut app, "2");
    let des = query("team-des", TeamFilter::default());
    assert_eq!(
        effects,
        vec![
            Effect::LoadTeamOptions("team-des".to_owned()),
            Effect::LoadTeamIssues {
                query: des.clone(),
                after: None
            },
            Effect::CountTeamIssues(des),
        ]
    );
    assert!(screen(&app)[3].contains("Loading issues"));

    settle(&mut app, effects).await;
    assert!(
        header(&app).starts_with("t DES Design · "),
        "{}",
        header(&app)
    );
}

#[tokio::test]
async fn typing_narrows_the_team_switcher_and_enter_switches_to_what_is_left() {
    let mut app = team_issues(80, 24).await;

    press(&mut app, "tops");
    assert!(
        screen(&app)
            .iter()
            .any(|row| row.contains("OPS Operations"))
    );
    assert!(!screen(&app).iter().any(|row| row.contains("DES Design")));
    let effects = press(&mut app, "\n");

    assert_eq!(
        effects.first(),
        Some(&Effect::LoadTeamOptions("team-ops".to_owned()))
    );
}

#[tokio::test]
async fn typing_narrows_a_filter_and_a_number_picks_among_what_is_left() {
    let mut app = team_issues(80, 24).await;

    // "in" leaves In Progress then In Review; Any and the rest are hidden.
    let effects = press(&mut app, "fsin2");
    settle(&mut app, effects).await;

    assert!(
        header(&app).ends_with("Filters State In Review"),
        "{}",
        header(&app)
    );
}

#[tokio::test]
async fn picking_the_team_already_shown_reloads_nothing() {
    let mut app = team_issues(80, 24).await;

    assert_eq!(press(&mut app, "t\n"), Vec::new());
    assert_eq!(n_of_m(&app), (50, 70));
}

#[tokio::test]
async fn an_answer_for_a_team_no_longer_shown_is_dropped() {
    let mut app = team_issues(80, 24).await;
    press(&mut app, "t2");
    let stale = FakeClient.team_issues(&eng(), None).await.unwrap();

    app.handle(Event::TeamIssuesLoaded {
        query: eng(),
        after: None,
        result: Ok(stale),
    })
    .unwrap();
    app.handle(Event::TeamIssuesCounted {
        query: eng(),
        result: Ok(70),
    })
    .unwrap();

    assert!(screen(&app)[3].contains("Loading issues"));
    assert!(header(&app).contains("0 of …"), "{}", header(&app));
}

/// Picks the first value after "Any" for the filter under `key`, and
/// returns the effects and the header once they have settled.
async fn filter_by_first(app: &mut App<TestBackend>, key: char) -> Vec<Effect> {
    let effects = press(app, &format!("f{key}2"));
    settle(app, effects.clone()).await;
    effects
}

#[tokio::test]
async fn each_filter_narrows_the_list_and_can_be_cleared() {
    for (key, field) in [
        ('s', "State"),
        ('a', "Assignee"),
        ('l', "Label"),
        ('c', "Cycle"),
        ('p', "Project"),
    ] {
        let mut app = team_issues(80, 24).await;
        let (_, unfiltered) = n_of_m(&app);

        let effects = filter_by_first(&mut app, key).await;

        let Some(Effect::LoadTeamIssues { query, after: None }) = effects.first() else {
            panic!("{field}: {effects:?}");
        };
        assert!(!query.filter.is_empty(), "{field}");
        let (_, filtered) = n_of_m(&app);
        assert!(filtered < unfiltered, "{field}: {filtered} of {unfiltered}");
        assert!(
            header(&app).contains(&format!("Filters {field} ")),
            "{}",
            header(&app)
        );

        let effects = press(&mut app, &format!("f{key}1"));
        assert_eq!(
            effects.first(),
            Some(&Effect::LoadTeamIssues {
                query: eng(),
                after: None
            }),
            "{field}"
        );
        settle(&mut app, effects).await;
        assert_eq!(n_of_m(&app).1, unfiltered, "{field}");
        assert!(header(&app).ends_with("Filters none"), "{}", header(&app));
    }
}

#[tokio::test]
async fn the_state_filter_lists_the_teams_workflow_states_after_any() {
    let mut app = team_issues(80, 24).await;

    press(&mut app, "fs");

    assert_snapshot!(app.backend());
}

#[tokio::test]
async fn filters_combine_and_show_in_the_header() {
    let mut app = team_issues(80, 24).await;

    // In Progress is the fifth entry, after Any.
    let effects = press(&mut app, "fs5");
    settle(&mut app, effects).await;
    let effects = press(&mut app, "flbackend\n");
    settle(&mut app, effects).await;

    assert!(
        header(&app).ends_with("Filters State In Progress, Label backend"),
        "{}",
        header(&app)
    );
    assert_snapshot!(app.backend());
}

#[tokio::test]
async fn filters_too_long_for_the_header_shrink_to_their_fields_then_a_count() {
    for (columns, filters) in [
        (80, "Filters State, Assignee, Cycle, Project"),
        (50, "Filters 4"),
    ] {
        let mut app = team_issues(columns, 24).await;
        for key in ['s', 'a', 'c', 'p'] {
            filter_by_first(&mut app, key).await;
        }

        assert!(header(&app).ends_with(filters), "{}", header(&app));
    }
}

#[tokio::test]
async fn a_filtered_list_on_a_wide_terminal() {
    let mut app = team_issues(200, 60).await;

    let effects = press(&mut app, "fs5");
    settle(&mut app, effects).await;

    assert_snapshot!(app.backend());
}

#[tokio::test]
async fn a_filter_nothing_matches_says_so() {
    let mut app = team_issues(80, 24).await;
    let effects = press(&mut app, "fs2");
    let Some(Effect::LoadTeamIssues { query, .. }) = effects.first().cloned() else {
        panic!("{effects:?}");
    };
    let nothing = TeamIssuesPage {
        issues: Vec::new(),
        cycles: Vec::new(),
        members: Vec::new(),
        next: None,
        as_of: demo::NOW,
        problems: Vec::new(),
    };

    app.handle(Event::TeamIssuesLoaded {
        query,
        after: None,
        result: Ok(nothing),
    })
    .unwrap();

    assert_eq!(n_of_m(&app), (0, 0));
    assert!(screen(&app)[3].contains("No issues match these filters"));
}

#[tokio::test]
async fn copy_and_open_act_on_the_focused_team_issue() {
    let mut app = team_issues(80, 24).await;
    press(&mut app, "j");
    let focused = FakeClient.team_issues(&eng(), None).await.unwrap().issues[1].clone();

    assert_eq!(
        press(&mut app, "yi"),
        vec![Effect::Copy(focused.identifier.clone())]
    );
    press(&mut app, "o");
    assert_eq!(press(&mut app, "\n"), vec![Effect::OpenUrl(focused.url)]);
}

#[tokio::test]
async fn enter_opens_the_detail_and_esc_steps_back_to_the_list() {
    let mut app = team_issues(80, 24).await;

    press(&mut app, "\n");
    assert!(screen(&app)[2].starts_with("┌ENG-"), "{}", screen(&app)[2]);

    press(&mut app, "\x1b");
    assert!(
        screen(&app)[2].starts_with("┌Team Issues"),
        "{}",
        screen(&app)[2]
    );
}

#[tokio::test]
async fn the_opened_detail_asks_for_its_comments_at_once_and_j_scrolls_it() {
    let mut app = team_issues(80, 12).await;
    let focused = FakeClient.team_issues(&eng(), None).await.unwrap().issues[0].clone();

    let effects = press(&mut app, "\n");
    let title = screen(&app)[3].clone();
    press(&mut app, "j");

    assert_eq!(
        effects,
        vec![Effect::LoadIssueDetail {
            issue_id: focused.id,
            urgency: Urgency::Urgent,
        }]
    );
    assert!(title.contains(&focused.title), "{title}");
    assert!(
        !screen(&app)[3].contains(&focused.title),
        "{}",
        screen(&app)[3]
    );
}

#[tokio::test]
async fn s_opens_the_state_picker_for_the_focused_team_issue() {
    let mut app = team_issues(80, 24).await;

    let effects = press(&mut app, "s");

    assert_eq!(
        effects,
        vec![Effect::LoadWorkflowStates {
            team_id: "team-eng".to_owned(),
        }]
    );
    assert!(screen(&app).iter().any(|line| line.contains(" of ENG-")));
}

#[tokio::test]
async fn a_first_page_that_fails_says_why_in_the_list() {
    let mut app = sized(80, 24);
    press(&mut app, "2");
    let teams = FakeClient.teams().await.unwrap();
    app.handle(Event::TeamsLoaded(Ok(teams))).unwrap();

    app.handle(Event::TeamIssuesLoaded {
        query: eng(),
        after: None,
        result: Err("could not load the team's issues: Linear did not answer in time".to_owned()),
    })
    .unwrap();

    assert!(
        screen(&app)[3].contains("could not load the team's issues: Linear did not answer"),
        "{}",
        screen(&app)[3]
    );
}

#[tokio::test]
async fn a_next_page_that_fails_keeps_the_list_and_says_why_in_the_notice() {
    let mut app = team_issues(80, 24).await;
    press(&mut app, "m");

    app.handle(Event::TeamIssuesLoaded {
        query: eng(),
        after: Some("50".to_owned()),
        result: Err("could not load the team's issues: Linear did not answer in time".to_owned()),
    })
    .unwrap();

    assert_eq!(n_of_m(&app), (50, 70));
    assert!(header(&app).contains("m more"), "it can be asked for again");
    assert!(screen(&app)[21].contains("could not load the team's issues"));
}

#[tokio::test]
async fn the_team_issues_keys_can_be_remapped_in_their_own_context() {
    let loaded = config::parse(
        "[keys.team-issues]\nswitch-team = \"T\"\nload-more = \"+\"\n",
        Path::new("config.toml"),
    );
    assert!(loaded.errors.is_empty(), "{:?}", loaded.errors);
    let state = State::demo().with_config(&loaded);
    let mut app = settled(App::new(TestBackend::new(80, 24), state).unwrap()).await;

    assert!(
        header(&app).starts_with("T ENG Engineering · 50 of 70 + more · f Filters"),
        "{}",
        header(&app)
    );
    assert_eq!(press(&mut app, "t"), Vec::new());
    assert_eq!(press(&mut app, "+").len(), 1);
}

#[tokio::test]
async fn a_later_page_asked_for_before_switching_away_and_back_is_dropped() {
    let mut app = team_issues(80, 24).await;
    press(&mut app, "m");
    press(&mut app, "t2");
    press(&mut app, "t1");
    let stale = FakeClient.team_issues(&eng(), Some("50")).await.unwrap();

    app.handle(Event::TeamIssuesLoaded {
        query: eng(),
        after: Some("50".to_owned()),
        result: Ok(stale),
    })
    .unwrap();

    assert!(
        screen(&app)[3].contains("Loading issues"),
        "{}",
        screen(&app)[3]
    );
    assert!(header(&app).contains("0 of …"), "{}", header(&app));
}
