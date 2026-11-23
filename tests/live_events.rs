//! The live events beyond new and updated issues and new comments, through
//! the app driver: archives, comment edits and deletes, and notifications,
//! with the ones that do not concern the viewer dropped. Also measures what
//! the echo of the viewer's own edit does to the screen.

// clippy's allow-unwrap-in-tests does not cover helpers outside #[test] functions.
#![allow(clippy::unwrap_used)]

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use jiff::SignedDuration;
use linear_rat::app::{App, Effect, Event, State, Write};
use linear_rat::linear::model::{Comment, Issue, Notification};
use linear_rat::linear::{
    FakeClient, IssueDetail, LinearClient, LiveEvent, NotificationsLoaded, PostedComment,
    TeamChanges, demo,
};
use ratatui::backend::TestBackend;

fn app() -> App<TestBackend> {
    App::new(TestBackend::new(80, 24), State::demo()).unwrap()
}

fn key_event(c: char) -> Event {
    let code = match c {
        '\n' => KeyCode::Enter,
        '\x1b' => KeyCode::Esc,
        // Down moves in a picker, where `j` types into the filter.
        '↓' => KeyCode::Down,
        c => KeyCode::Char(c),
    };
    Event::Key(KeyEvent::new(code, KeyModifiers::NONE))
}

fn press(app: &mut App<TestBackend>, keys: &str) -> Vec<Effect> {
    keys.chars()
        .flat_map(|c| app.handle(key_event(c)).unwrap())
        .collect()
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

fn focused(app: &App<TestBackend>) -> Issue {
    app.state().my_issues.focused_issue().unwrap().clone()
}

fn changes(issue: Issue) -> TeamChanges {
    TeamChanges {
        team_id: issue.team_id.clone(),
        issues: vec![issue],
        trashed: Vec::new(),
        cycles: Vec::new(),
        labels: Vec::new(),
        members: Vec::new(),
        teams: Vec::new(),
        newest: None,
        as_of: demo::NOW,
    }
}

/// `issue` as Linear has it a minute later, changed by `change`.
fn later(issue: &Issue, change: impl FnOnce(&mut Issue)) -> Issue {
    let mut issue = issue.clone();
    issue.updated_at += SignedDuration::from_mins(1);
    change(&mut issue);
    issue
}

fn in_my_issues(app: &App<TestBackend>, id: &str) -> bool {
    app.state()
        .my_issues
        .groups()
        .iter()
        .flat_map(|group| &group.issues)
        .any(|issue| issue.id == id)
}

#[test]
fn an_issue_archived_elsewhere_leaves_my_issues_and_comes_back_unarchived() {
    let mut app = app();
    let issue = focused(&app);

    let archived = later(&issue, |issue| issue.archived_at = Some(demo::NOW));
    app.handle(Event::Live(LiveEvent::Archived(changes(archived.clone()))))
        .unwrap();
    assert!(!in_my_issues(&app, &issue.id));
    assert!(
        app.state()
            .store
            .issue(&issue.id)
            .unwrap()
            .archived_at
            .is_some()
    );

    let unarchived = later(&archived, |issue| issue.archived_at = None);
    app.handle(Event::Live(LiveEvent::Unarchived(changes(unarchived))))
        .unwrap();
    assert!(in_my_issues(&app, &issue.id));
}

/// The demo app with the comments of the focused issue loaded, and that
/// issue's id and its comments.
fn with_comments() -> (App<TestBackend>, String, Vec<Comment>) {
    let mut app = app();
    let issue_id = app
        .state()
        .store
        .issues()
        .find(|issue| !IssueDetail::demo(&issue.id).comments.is_empty())
        .map(|issue| issue.id.clone())
        .unwrap();
    let detail = IssueDetail::demo(&issue_id);
    let comments = detail.comments.clone();
    app.handle(Event::IssueDetailLoaded {
        issue_id: issue_id.clone(),
        result: Ok(detail),
    })
    .unwrap();
    (app, issue_id, comments)
}

fn stored_comments(app: &App<TestBackend>, issue_id: &str) -> Vec<Comment> {
    app.state().store.comments(issue_id).unwrap().to_vec()
}

#[test]
fn an_edited_comment_replaces_the_loaded_one() {
    let (mut app, issue_id, comments) = with_comments();
    let mut edited = comments[0].clone();
    edited.body = "Edited on the web".to_owned();
    edited.edited_at = Some(demo::NOW);

    let update = app
        .handle(Event::Live(LiveEvent::CommentUpdated(PostedComment {
            comment: edited.clone(),
            author: None,
        })))
        .unwrap();

    assert!(update.contains(&Effect::SaveSnapshot));
    assert_eq!(stored_comments(&app, &issue_id)[0], edited);
    assert_eq!(stored_comments(&app, &issue_id).len(), comments.len());
}

#[test]
fn a_deleted_comment_leaves_and_takes_the_detail_focus_with_it() {
    let (mut app, issue_id, comments) = with_comments();
    let deleted = comments[0].id.clone();
    // Focusing a comment is the Inbox's way in, so go through the state.
    let mut state = State::demo();
    state.detail.focus_comment(&issue_id, Some(deleted.clone()));
    let mut focused = App::new(TestBackend::new(80, 24), state).unwrap();
    focused
        .handle(Event::IssueDetailLoaded {
            issue_id: issue_id.clone(),
            result: Ok(IssueDetail::demo(&issue_id)),
        })
        .unwrap();

    for app in [&mut app, &mut focused] {
        app.handle(Event::Live(LiveEvent::CommentDeleted {
            issue_id: issue_id.clone(),
            comment_id: deleted.clone(),
        }))
        .unwrap();
        assert!(
            !stored_comments(app, &issue_id)
                .iter()
                .any(|comment| comment.id == deleted)
        );
        assert_eq!(stored_comments(app, &issue_id).len(), comments.len() - 1);
    }
    assert_eq!(focused.state().detail.focused_comment(&issue_id), None);
}

#[test]
fn a_deleted_thread_takes_its_replies_and_the_focus_on_one_of_them() {
    let mut state = State::demo();
    let (issue_id, reply) = state
        .store
        .issues()
        .find_map(|issue| {
            let reply = IssueDetail::demo(&issue.id)
                .comments
                .into_iter()
                .find(|comment| comment.parent_id.is_some())?;
            Some((issue.id.clone(), reply))
        })
        .unwrap();
    let root = reply.parent_id.clone().unwrap();
    state
        .detail
        .focus_comment(&issue_id, Some(reply.id.clone()));
    let mut app = App::new(TestBackend::new(80, 24), state).unwrap();
    app.handle(Event::IssueDetailLoaded {
        issue_id: issue_id.clone(),
        result: Ok(IssueDetail::demo(&issue_id)),
    })
    .unwrap();

    app.handle(Event::Live(LiveEvent::CommentDeleted {
        issue_id: issue_id.clone(),
        comment_id: root.clone(),
    }))
    .unwrap();

    assert!(stored_comments(&app, &issue_id).iter().all(|comment| {
        comment.id != root && comment.parent_id.as_deref() != Some(root.as_str())
    }));
    assert_eq!(app.state().detail.focused_comment(&issue_id), None);
}

#[test]
fn an_edited_comment_brings_its_author_along() {
    let (mut app, _, comments) = with_comments();
    let mut author = app.state().store.members().next().unwrap().clone();
    author.id = "member-new-to-lrat".to_owned();
    let mut edited = comments[0].clone();
    edited.author_id = Some(author.id.clone());

    app.handle(Event::Live(LiveEvent::CommentUpdated(PostedComment {
        comment: edited,
        author: Some(author.clone()),
    })))
    .unwrap();

    assert_eq!(app.state().store.member(&author.id), Some(&author));
}

#[test]
fn comment_events_on_an_issue_lrat_does_not_hold_are_dropped() {
    let (mut app, issue_id, comments) = with_comments();
    let mut elsewhere = comments[0].clone();
    elsewhere.id = "comment-of-another-team".to_owned();
    elsewhere.issue_id = "issue-of-another-team".to_owned();
    let before = app.state().store.clone();

    for event in [
        LiveEvent::Comment(PostedComment {
            comment: elsewhere.clone(),
            author: None,
        }),
        LiveEvent::CommentUpdated(PostedComment {
            comment: elsewhere.clone(),
            author: None,
        }),
        LiveEvent::CommentDeleted {
            issue_id: elsewhere.issue_id.clone(),
            comment_id: elsewhere.id.clone(),
        },
    ] {
        let update = app.handle(Event::Live(event)).unwrap();
        assert!(!update.contains(&Effect::SaveSnapshot), "{update:?}");
    }

    assert_eq!(app.state().store, before);
    assert_eq!(stored_comments(&app, &issue_id), comments);
}

async fn one_notification() -> (Notification, NotificationsLoaded) {
    let mut loaded = FakeClient.notifications(None).await.unwrap();
    loaded.notifications.truncate(1);
    let mut notification = loaded.notifications[0].clone();
    notification.id = "notification-live".to_owned();
    notification.read_at = None;
    loaded.notifications = vec![notification.clone()];
    (notification, loaded)
}

#[tokio::test]
async fn a_notification_for_the_viewer_lands_in_the_inbox() {
    let mut app = app();
    let (notification, loaded) = one_notification().await;
    let viewer = app.state().store.viewer().unwrap().id.clone();

    app.handle(Event::Live(LiveEvent::Notification {
        recipient_id: viewer.clone(),
        loaded: loaded.clone(),
    }))
    .unwrap();

    assert!(app.state().store.notification(&notification.id).is_some());
    assert_eq!(linear_rat::app::unread_count(app.state()), 1);
    assert_eq!(
        app.state().store.notifications_watermark(),
        None,
        "a live event moves no watermark"
    );

    // Archived elsewhere, it leaves.
    let mut archived = notification.clone();
    archived.archived_at = Some(demo::NOW);
    app.handle(Event::Live(LiveEvent::Notification {
        recipient_id: viewer,
        loaded: NotificationsLoaded {
            notifications: vec![archived],
            ..loaded
        },
    }))
    .unwrap();
    assert!(app.state().store.notification(&notification.id).is_none());
}

#[tokio::test]
async fn a_notification_for_someone_else_is_dropped() {
    let mut app = app();
    let (notification, loaded) = one_notification().await;

    let effects = app
        .handle(Event::Live(LiveEvent::Notification {
            recipient_id: "member-someone-else".to_owned(),
            loaded,
        }))
        .unwrap();

    assert!(app.state().store.notification(&notification.id).is_none());
    assert!(!effects.contains(&Effect::SaveSnapshot));
}

/// Changes the focused issue's priority through the picker and returns the
/// write it sent.
fn edit_priority(app: &mut App<TestBackend>) -> Write {
    let effects = press(app, "p↓\n");
    effects
        .into_iter()
        .find_map(|effect| match effect {
            Effect::UpdateIssue(write) => Some(write),
            _ => None,
        })
        .unwrap()
}

/// The echo of the viewer's own edit: Linear's copy after the write, one
/// second newer than the copy before it.
fn echo_of(app: &App<TestBackend>, issue_id: &str) -> Issue {
    let mut issue = app.state().store.issue(issue_id).unwrap().clone();
    issue.updated_at += SignedDuration::from_secs(1);
    issue
}

/// Measures the echo: the live copy of the viewer's own edit, before or
/// after Linear's answer to the mutation, leaves the screen as the edit
/// drew it. The newer-wins merge makes it harmless, so nothing drops it.
#[test]
fn the_echo_of_the_viewers_own_edit_changes_nothing_on_screen() {
    let mut app = app();
    let id = focused(&app).id;
    let write = edit_priority(&mut app);
    let drawn = screen(&app);
    let shown = app.state().store.issue(&id).unwrap().priority;
    let echo = echo_of(&app, &id);

    app.handle(Event::Live(LiveEvent::Issue(changes(echo.clone()))))
        .unwrap();
    assert_eq!(screen(&app), drawn, "the echo before the answer");
    assert_eq!(app.state().store.issue(&id).unwrap().priority, shown);

    app.handle(Event::IssueUpdated {
        write,
        result: Ok(Box::new(echo.clone())),
    })
    .unwrap();
    app.handle(Event::Live(LiveEvent::Issue(changes(echo))))
        .unwrap();
    assert_eq!(screen(&app), drawn, "the echo after the answer");
}

/// With two edits of one issue in flight, the echo of the first carries
/// the field as the first edit left it, just as Linear's answer to the
/// first does: the echo leaves the store exactly as that answer leaves it,
/// so dropping echoes would not change what the viewer sees.
#[test]
fn the_echo_of_an_earlier_edit_does_what_its_answer_does() {
    let mut echoed = app();
    let id = focused(&echoed).id;
    let first = edit_priority(&mut echoed);
    let first_echo = echo_of(&echoed, &id);
    edit_priority(&mut echoed);
    let mut answered = app();
    edit_priority(&mut answered);
    edit_priority(&mut answered);

    echoed
        .handle(Event::Live(LiveEvent::Issue(changes(first_echo.clone()))))
        .unwrap();
    answered
        .handle(Event::IssueUpdated {
            write: first,
            result: Ok(Box::new(first_echo.clone())),
        })
        .unwrap();

    let after_echo = echoed.state().store.issue(&id).unwrap();
    assert_eq!(Some(after_echo), answered.state().store.issue(&id));
    assert_eq!(
        after_echo.priority, first_echo.priority,
        "both show the first edit until the second is answered"
    );
}
