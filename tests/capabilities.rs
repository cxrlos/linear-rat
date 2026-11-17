//! What the API key may do in each team: learned from forbidden writes,
//! shown as muted actions with the reason, and checked before a key runs.

// clippy's allow-unwrap-in-tests does not cover helpers outside #[test] functions.
#![allow(clippy::unwrap_used)]

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use insta::assert_snapshot;
use linear_rat::app::{
    App, Edit, Effect, Event, Failure, Modal, NewComment, Palette, State, Write, WriteClass,
};
use linear_rat::linear::model::Priority;
use linear_rat::linear::{ClientError, ErrorKind, FakeClient, GraphQlError, LinearClient, demo};
use ratatui::backend::TestBackend;

/// The issue My Issues focuses first in the demo, in team DES.
const FIRST: &str = "DES-34";

/// HYPOTHETICAL: what Linear might answer a key that may read but not edit
/// issues in a team. No limited key's answer has been captured yet; replace
/// this with a real one when there is.
const HYPOTHETICAL_LIMITED_KEY_ERROR: &str = r#"{
    "message": "You don't have permission to update this issue",
    "extensions": { "type": "forbidden", "code": "FORBIDDEN", "userError": true }
}"#;

fn app() -> App<TestBackend> {
    App::new(TestBackend::new(80, 24), State::demo()).unwrap()
}

fn key(code: KeyCode) -> Event {
    Event::Key(KeyEvent::new(code, KeyModifiers::NONE))
}

/// Types `keys`, one character each, and returns every effect asked for.
fn press(app: &mut App<TestBackend>, keys: &str) -> Vec<Effect> {
    keys.chars()
        .flat_map(|c| app.handle(key(KeyCode::Char(c))).unwrap())
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

/// The failure the runtime would send for the hypothetical limited key's
/// answer to `action`.
fn forbidden(action: &str) -> Failure {
    let error: GraphQlError = serde_json::from_str(HYPOTHETICAL_LIMITED_KEY_ERROR).unwrap();
    Failure::from_client(action, &ClientError::GraphQl(vec![error]))
}

fn only_write(effects: Vec<Effect>) -> Write {
    match effects.as_slice() {
        [Effect::UpdateIssue(write)] => write.clone(),
        other => panic!("expected exactly one write, got {other:?}"),
    }
}

/// The app after Linear refused a priority edit of the focused issue.
fn refused() -> App<TestBackend> {
    let mut app = app();
    let write = only_write(press(&mut app, "p1"));
    app.handle(Event::IssueUpdated {
        write,
        result: Err(forbidden(&format!("edit {FIRST}"))),
    })
    .unwrap();
    app
}

/// Linear's answer to `write`, as the fake client gives it, on a thread of
/// its own so the fake starts from the demo workspace.
fn accepted(write: &Write) -> Event {
    let issue = std::thread::scope(|scope| {
        scope
            .spawn(|| {
                tokio::runtime::Builder::new_current_thread()
                    .build()
                    .unwrap()
                    .block_on(FakeClient.update_issue(&write.issue_id, &write.edit.update()))
            })
            .join()
            .unwrap()
    })
    .unwrap();
    Event::IssueUpdated {
        write: write.clone(),
        result: Ok(Box::new(issue)),
    }
}

#[test]
fn the_hypothetical_limited_key_error_is_forbidden() {
    assert_eq!(forbidden("edit").kind, ErrorKind::Forbidden);
}

#[test]
fn after_a_forbidden_edit_the_next_press_says_why_without_a_request() {
    let mut app = refused();

    let effects = press(&mut app, "s");

    assert_eq!(effects, Vec::new());
    assert!(app.state().modals.top().is_none());
    assert_eq!(
        app.state().notice.as_deref(),
        Some("this API key cannot edit issues in DES")
    );
    assert!(shows(&app, "this API key cannot edit issues in DES"));
}

#[test]
fn a_forbidden_edit_leaves_comments_and_other_teams_alone() {
    let mut app = refused();

    assert!(
        app.state()
            .write_denied(WriteClass::CreateComment)
            .is_none()
    );
    press(&mut app, "j");
    assert_eq!(app.state().focused_issue().unwrap().identifier, "ENG-51");
    assert!(!press(&mut app, "s").is_empty());
}

#[test]
fn a_later_success_allows_the_class_again() {
    let mut app = refused();
    // The fake client accepts any edit, as a key that regained access would.
    let write = Write {
        issue_id: app.state().focused_issue().unwrap().id.clone(),
        identifier: FIRST.to_owned(),
        edit: Edit::Priority(Priority::Low),
        previous: Edit::Priority(Priority::High),
    };

    app.handle(accepted(&write)).unwrap();

    assert!(!press(&mut app, "s").is_empty());
}

#[test]
fn the_palette_closes_on_a_denied_action_and_says_why() {
    let mut app = refused();
    press(&mut app, ":");
    for c in "workflow".chars() {
        app.handle(key(KeyCode::Char(c))).unwrap();
    }

    let effects = app.handle(key(KeyCode::Enter)).unwrap();

    assert_eq!(effects, Vec::new());
    assert!(!matches!(app.state().modals.top(), Some(Modal::Palette(_))));
    assert_eq!(
        app.state().notice.as_deref(),
        Some("this API key cannot edit issues in DES")
    );
}

#[test]
fn the_palette_mutes_denied_actions_with_the_reason() {
    let mut app = refused();
    app.handle(key(KeyCode::F(12))).unwrap();
    press(&mut app, ":");
    for c in "change".chars() {
        app.handle(key(KeyCode::Char(c))).unwrap();
    }

    assert!(matches!(
        app.state().modals.top(),
        Some(Modal::Palette(Palette { .. }))
    ));
    assert!(shows(&app, "· this API key cannot edit issues in DES"));
    assert_snapshot!(app.backend());
}

#[test]
fn the_cheatsheet_mutes_denied_actions_and_lists_the_reason() {
    let mut app = App::new(TestBackend::new(120, 40), State::demo()).unwrap();
    app.handle(Event::Start).unwrap();
    let write = only_write(press(&mut app, "p1"));
    app.handle(Event::IssueUpdated {
        write,
        result: Err(forbidden(&format!("edit {FIRST}"))),
    })
    .unwrap();

    press(&mut app, "?");

    assert!(shows(&app, "this API key cannot edit issues in DES"));
    assert_snapshot!(app.backend());
}

#[test]
fn other_failures_of_a_write_deny_nothing() {
    let mut app = app();
    let write = only_write(press(&mut app, "p1"));
    app.handle(Event::IssueUpdated {
        write,
        result: Err(Failure::new(
            ErrorKind::ServerError,
            "edit DES-34",
            "HTTP 503",
        )),
    })
    .unwrap();

    assert!(!press(&mut app, "s").is_empty());
}

#[test]
fn a_forbidden_comment_denies_comments_in_the_team() {
    let mut app = app();
    app.handle(Event::CommentCreated {
        comment: NewComment {
            issue_id: app.state().focused_issue().unwrap().id.clone(),
            identifier: FIRST.to_owned(),
            body: "Brie".to_owned(),
        },
        result: Err(forbidden("comment on DES-34")),
    })
    .unwrap();

    let effects = press(&mut app, "c");

    assert_eq!(effects, Vec::new());
    assert_eq!(
        app.state().notice.as_deref(),
        Some("this API key cannot comment on issues in DES")
    );
}

#[test]
fn a_team_the_key_cannot_reach_says_so_on_team_issues() {
    let mut app = app();
    let mut effects = press(&mut app, "2");
    let mut guard = 0;
    while let Some(effect) = effects.pop() {
        guard += 1;
        assert!(guard < 20, "loads never settled");
        let event = match effect {
            Effect::LoadTeams => Event::TeamsLoaded(Ok(demo::workspace().teams)),
            Effect::LoadTeamIssues { query, after } => Event::TeamIssuesLoaded {
                query,
                after,
                result: Err(forbidden("load the team's issues")),
            },
            _ => continue,
        };
        effects.extend(app.handle(event).unwrap());
    }

    assert!(shows(&app, "Not accessible with this key"));
    assert_snapshot!(app.backend());
}
