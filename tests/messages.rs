//! The error log: every failure that reaches the notice is kept, and `!` or
//! `:messages` shows the most recent, newest first.

// clippy's allow-unwrap-in-tests does not cover helpers outside #[test] functions.
#![allow(clippy::unwrap_used)]

use std::time::Duration;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use insta::assert_snapshot;
use jiff::Timestamp;
use linear_rat::app::{
    App, Edit, Effect, Event, Failure, Modal, NewComment, NewIssue, State, SyncFailure, Write,
    Writing,
};
use linear_rat::linear::model::Priority;
use linear_rat::linear::{ErrorKind, IssueCreate, TeamQuery, demo};
use ratatui::backend::TestBackend;

/// 2026-11-03T09:00:00Z.
const START: Timestamp = Timestamp::constant(1_793_696_400, 0);

fn app() -> App<TestBackend> {
    App::new(TestBackend::new(80, 24), State::demo()).unwrap()
}

fn key(c: char) -> Event {
    Event::Key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE))
}

fn press(code: KeyCode) -> Event {
    Event::Key(KeyEvent::new(code, KeyModifiers::NONE))
}

fn at(seconds: i64) -> Event {
    Event::Tick(
        START
            .checked_add(jiff::SignedDuration::from_secs(seconds))
            .unwrap(),
    )
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

fn offline(action: &str, message: &str) -> Failure {
    Failure::new(ErrorKind::Offline, action, message)
}

fn write() -> Write {
    Write {
        issue_id: "issue-des-34".to_owned(),
        identifier: "DES-34".to_owned(),
        edit: Edit::Priority(Priority::Urgent),
        previous: Edit::Priority(Priority::High),
    }
}

/// One event for each way a failure reaches the notice.
fn failures() -> Vec<Event> {
    let failure = |action: &str| Failure::new(ErrorKind::ServerError, action, "HTTP 503");
    vec![
        Event::EffectFailed(failure("copy")),
        Event::IssueUpdated {
            write: write(),
            result: Err(failure("edit DES-34")),
        },
        Event::CommentCreated {
            comment: NewComment {
                issue_id: "issue-des-34".to_owned(),
                identifier: "DES-34".to_owned(),
                body: "Brie".to_owned(),
                parent_id: None,
                local_id: None,
            },
            result: Err(failure("comment on DES-34")),
        },
        Event::IssueCreated {
            issue: NewIssue {
                local_id: "local-1".to_owned(),
                input: IssueCreate {
                    team_id: "team-des".to_owned(),
                    state_id: "state-todo".to_owned(),
                    assignee_id: None,
                    priority: Priority::None,
                    title: "Gouda".to_owned(),
                    description: String::new(),
                },
            },
            result: Err(failure("create an issue")),
        },
        Event::EditorClosed {
            writing: Writing::Comment {
                issue_id: "issue-des-34".to_owned(),
                identifier: "DES-34".to_owned(),
            },
            result: Err("no editor found".to_owned()),
        },
        Event::WorkflowStatesLoaded {
            team_id: "team-des".to_owned(),
            result: Err(failure("load the team's workflow states")),
        },
        Event::TeamMembersLoaded {
            team_id: "team-des".to_owned(),
            result: Err(failure("load the team's members")),
        },
        Event::TeamLabelsLoaded {
            team_id: "team-des".to_owned(),
            result: Err(failure("load the team's labels")),
        },
        Event::TeamOptionsLoaded(Err(failure("load the team's filters"))),
        Event::AdministrableTeamsLoaded(Err(failure("load the teams you administer"))),
        Event::Polled {
            changes: Vec::new(),
            failure: Some(SyncFailure::Failed(failure("sync"))),
        },
    ]
}

#[test]
fn each_failure_that_sets_the_notice_also_goes_in_the_log() {
    let mut app = app();

    for event in failures() {
        let described = format!("{event:?}");
        // Any key clears the notice, so each failure starts without one.
        app.handle(press(KeyCode::F(12))).unwrap();
        let before = app.state().errors.len();

        app.handle(event).unwrap();

        assert!(app.state().notice.is_some(), "no notice for {described}");
        assert_eq!(
            app.state().errors.len(),
            before + 1,
            "not logged: {described}"
        );
    }
}

#[test]
fn the_log_keeps_the_action_kind_and_full_message() {
    let mut app = app();
    app.handle(at(0)).unwrap();

    app.handle(Event::EffectFailed(Failure::new(
        ErrorKind::Other,
        "open in the browser",
        "could not open the browser with xdg-open: not found",
    )))
    .unwrap();

    let newest = app.state().errors.newest_first().next().unwrap();
    assert_eq!(newest.at, Some(START));
    assert_eq!(newest.failure.action, "open in the browser");
    assert_eq!(newest.failure.kind, ErrorKind::Other);
    assert_eq!(
        newest.failure.message,
        "could not open the browser with xdg-open: not found"
    );
}

#[test]
fn failures_never_open_the_log_on_their_own() {
    let mut app = app();

    for event in failures() {
        app.handle(event).unwrap();
    }

    assert_eq!(app.state().modals.top(), None);
}

/// The log after three failures, a minute apart.
fn app_with_failures() -> App<TestBackend> {
    let mut app = app();
    app.handle(at(0)).unwrap();
    app.handle(Event::Polled {
        changes: Vec::new(),
        failure: Some(SyncFailure::Failed(offline(
            "sync",
            "timed out connecting to Linear",
        ))),
    })
    .unwrap();
    app.handle(at(60)).unwrap();
    app.handle(Event::IssueUpdated {
        write: write(),
        result: Err(Failure::new(
            ErrorKind::Validation {
                field: Some("priority".to_owned()),
            },
            "edit DES-34",
            "Linear returned errors: Argument Validation Error (INVALID_INPUT)",
        )),
    })
    .unwrap();
    app.handle(at(120)).unwrap();
    app.handle(Event::AdministrableTeamsLoaded(Err(Failure::new(
        ErrorKind::RateLimited {
            retry_after: Some(Duration::from_secs(40)),
        },
        "load the teams you administer",
        "could not load the teams you administer: Linear's rate limit is used up",
    ))))
    .unwrap();
    app
}

#[test]
fn exclamation_mark_shows_the_log_newest_first() {
    let mut app = app_with_failures();

    app.handle(key('!')).unwrap();

    assert_eq!(
        app.state().modals.top(),
        Some(&Modal::Messages { scroll: 0 })
    );
    assert_snapshot!(app.backend());
}

#[test]
fn a_rate_limit_in_the_log_says_until_when_so_it_does_not_go_stale() {
    let mut app = app_with_failures();
    app.handle(at(600)).unwrap();

    app.handle(key('!')).unwrap();

    let screen = screen(&app);
    assert!(
        screen.contains("Linear's rate limit is used up until 09:02:40"),
        "{screen}"
    );
}

#[test]
fn typing_messages_in_the_palette_opens_the_log() {
    let mut app = app_with_failures();

    app.handle(key(':')).unwrap();
    for c in "messages".chars() {
        app.handle(key(c)).unwrap();
    }
    app.handle(press(KeyCode::Enter)).unwrap();

    assert_eq!(
        app.state().modals.top(),
        Some(&Modal::Messages { scroll: 0 })
    );
}

#[test]
fn j_scrolls_the_log_and_esc_closes_it() {
    let mut app = app_with_failures();
    app.backend_mut().resize(80, 14);
    app.handle(Event::Resize {
        columns: 80,
        rows: 14,
    })
    .unwrap();
    app.handle(key('!')).unwrap();

    app.handle(key('j')).unwrap();
    assert_eq!(
        app.state().modals.top(),
        Some(&Modal::Messages { scroll: 1 })
    );
    app.handle(key('k')).unwrap();
    app.handle(key('k')).unwrap();
    assert_eq!(
        app.state().modals.top(),
        Some(&Modal::Messages { scroll: 0 })
    );

    app.handle(press(KeyCode::Esc)).unwrap();
    assert_eq!(app.state().modals.top(), None);
}

#[test]
fn an_empty_log_says_so() {
    let mut app = app();

    app.handle(key('!')).unwrap();

    assert!(screen(&app).contains("No errors yet."), "{}", screen(&app));
}

/// The app on Team Issues with the first team's load in flight, and the
/// query and count it asked for.
fn on_team_issues() -> (App<TestBackend>, TeamQuery) {
    let mut app = app();
    let mut effects = app.handle(key('2')).unwrap();
    let mut guard = 0;
    while let Some(effect) = effects.pop() {
        guard += 1;
        assert!(guard < 20, "loads never settled");
        match effect {
            Effect::LoadTeams => {
                effects.extend(
                    app.handle(Event::TeamsLoaded(Ok(demo::workspace().teams)))
                        .unwrap(),
                );
            }
            Effect::LoadTeamIssues { query, .. } => return (app, query),
            _ => {}
        }
    }
    panic!("Team Issues asked for no page");
}

#[test]
fn a_team_issues_first_page_failure_goes_in_the_log() {
    let (mut app, query) = on_team_issues();

    app.handle(Event::TeamIssuesLoaded {
        query,
        after: None,
        result: Err(Failure::new(
            ErrorKind::ServerError,
            "load the team's issues",
            "HTTP 503",
        )),
    })
    .unwrap();

    let newest = app.state().errors.newest_first().next().unwrap();
    assert_eq!(newest.failure.action, "load the team's issues");
    assert_eq!(newest.failure.kind, ErrorKind::ServerError);
}

#[test]
fn a_team_issues_count_failure_goes_in_the_log() {
    let (mut app, query) = on_team_issues();

    app.handle(Event::TeamIssuesCounted {
        query,
        result: Err(Failure::new(
            ErrorKind::ServerError,
            "count the team's issues",
            "HTTP 503",
        )),
    })
    .unwrap();

    let newest = app.state().errors.newest_first().next().unwrap();
    assert_eq!(newest.failure.action, "count the team's issues");
}
