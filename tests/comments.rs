//! Comment threads in the issue detail through the app driver: focusing a
//! comment, replying, editing and deleting the viewer's own, resolving and
//! unresolving a thread, each optimistic and undone when Linear refuses.

// clippy's allow-unwrap-in-tests does not cover helpers outside #[test] functions.
#![allow(clippy::unwrap_used)]

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use insta::assert_snapshot;
use linear_rat::app::{
    App, Capability, CommentChange, CommentWrite, Effect, Event, Failure, NewComment, State,
    WriteClass, Writing,
};
use linear_rat::linear::model::Comment;
use linear_rat::linear::{
    ClientError, ErrorKind, FakeClient, GraphQlError, IssueDetail, LinearClient,
};
use ratatui::backend::TestBackend;

/// The issue My Issues focuses first in the demo, with a resolved thread
/// and a thread with the viewer's reply.
const FIRST: &str = "DES-34";
const ISSUE: &str = "issue-des-34";
/// The resolved thread: Pia's question and the viewer's answer.
const RESOLVED: &str = "comment-des-34-rind";
const RESOLVED_REPLY: &str = "comment-des-34-rind-reply";
/// The open thread: Nell's comment and the viewer's edited reply.
const OPEN: &str = "comment-90";
const OPEN_REPLY: &str = "comment-91";

const FORBIDDEN: &str = r#"{
    "message": "You don't have permission to change this comment",
    "extensions": { "type": "forbidden", "code": "FORBIDDEN", "userError": true }
}"#;

fn app_sized(columns: u16, rows: u16) -> App<TestBackend> {
    let mut app = App::new(TestBackend::new(columns, rows), State::demo()).unwrap();
    open_detail(&mut app);
    app
}

/// The demo on an 80x24 terminal with DES-34's detail open full width.
fn app() -> App<TestBackend> {
    app_sized(80, 24)
}

fn key(code: KeyCode) -> Event {
    Event::Key(KeyEvent::new(code, KeyModifiers::NONE))
}

fn press(app: &mut App<TestBackend>, keys: &str) -> Vec<Effect> {
    keys.chars()
        .flat_map(|c| app.handle(key(KeyCode::Char(c))).unwrap())
        .collect()
}

fn open_detail(app: &mut App<TestBackend>) {
    for effect in app.handle(key(KeyCode::Enter)).unwrap() {
        if let Effect::LoadIssueDetail { issue_id, .. } = effect {
            let result = Ok(IssueDetail::demo(&issue_id));
            app.handle(Event::IssueDetailLoaded { issue_id, result })
                .unwrap();
        }
    }
}

fn focused(app: &App<TestBackend>) -> Option<String> {
    app.state().detail.focused_comment(ISSUE).map(str::to_owned)
}

/// Moves the focus with `}` from the first comment until it is on `id`.
fn focus(app: &mut App<TestBackend>, id: &str) {
    if focused(app).is_some() {
        app.handle(key(KeyCode::Esc)).unwrap();
    }
    for _ in 0..10 {
        if focused(app).as_deref() == Some(id) {
            return;
        }
        press(app, "}");
    }
    panic!("never focused {id}");
}

fn comment(app: &App<TestBackend>, id: &str) -> Option<Comment> {
    app.state()
        .store
        .comments(ISSUE)
        .unwrap()
        .iter()
        .find(|comment| comment.id == id)
        .cloned()
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

fn only_editor(effects: Vec<Effect>) -> Writing {
    match effects.as_slice() {
        [Effect::OpenEditor { writing, .. }] => writing.clone(),
        other => panic!("expected the editor to open, got {other:?}"),
    }
}

fn only_change(effects: Vec<Effect>) -> CommentWrite {
    match effects.as_slice() {
        [Effect::ChangeComment(write)] => write.clone(),
        other => panic!("expected one comment change, got {other:?}"),
    }
}

fn saved(writing: Writing, text: &str) -> Event {
    Event::EditorClosed {
        writing,
        result: Ok(text.to_owned()),
    }
}

fn forbidden(action: &str) -> Failure {
    let error: GraphQlError = serde_json::from_str(FORBIDDEN).unwrap();
    Failure::from_client(action, &ClientError::GraphQl(vec![error]))
}

fn offline() -> Failure {
    Failure::new(ErrorKind::Offline, "test", "could not reach Linear")
}

/// What the editor opens on when `keys` are pressed.
fn editor_text(app: &mut App<TestBackend>, keys: &str) -> String {
    let effects = press(app, keys);
    let [Effect::OpenEditor { text, .. }] = effects.as_slice() else {
        panic!("expected the editor to open, got {effects:?}");
    };
    text.clone()
}

fn capability(app: &App<TestBackend>, class: WriteClass) -> Capability {
    let team_id = &app.state().store.issue(ISSUE).unwrap().team_id;
    app.state().store.capabilities().write(team_id, class)
}

/// Runs `request` on the fake client on a thread of its own, so what it
/// remembers starts from the demo workspace.
fn on_fake<T: Send>(request: impl AsyncFnOnce(FakeClient) -> T + Send) -> T {
    std::thread::scope(|scope| {
        scope
            .spawn(|| {
                tokio::runtime::Builder::new_current_thread()
                    .build()
                    .unwrap()
                    .block_on(request(FakeClient))
            })
            .join()
            .unwrap()
    })
}

#[test]
fn braces_move_the_focus_through_comments_and_replies_and_esc_clears_it() {
    let mut app = app();

    press(&mut app, "}");
    assert_eq!(focused(&app).as_deref(), Some(RESOLVED));
    press(&mut app, "}");
    assert_eq!(focused(&app).as_deref(), Some(RESOLVED_REPLY));
    press(&mut app, "}");
    assert_eq!(focused(&app).as_deref(), Some(OPEN));
    press(&mut app, "}");
    assert_eq!(focused(&app).as_deref(), Some(OPEN_REPLY));
    press(&mut app, "}");
    assert_eq!(
        focused(&app).as_deref(),
        Some(OPEN_REPLY),
        "stays at the end"
    );
    press(&mut app, "{");
    assert_eq!(focused(&app).as_deref(), Some(OPEN));
    // The resolved thread collapsed once the focus left it.
    press(&mut app, "{");
    assert_eq!(focused(&app).as_deref(), Some(RESOLVED));

    app.handle(key(KeyCode::Esc)).unwrap();
    assert_eq!(focused(&app), None);
    assert!(shows(&app, "Comments"), "Esc keeps the detail open");
}

#[test]
fn threads_render_nested_with_the_resolved_one_collapsed_on_a_narrow_terminal() {
    let mut app = app();
    focus(&mut app, OPEN);

    assert!(shows(&app, "▸ resolved · 1 reply"));
    assert!(shows(&app, "· edited"));
    // A wide emoji leaves the old text in the cell it covers in the test
    // backend; a full redraw shows the screen as a terminal would.
    app.redraw_all().unwrap();
    assert_snapshot!(app.backend());
}

#[test]
fn threads_render_nested_with_the_resolved_one_collapsed_on_a_wide_terminal() {
    let mut app = app_sized(120, 40);
    press(&mut app, "l");
    focus(&mut app, OPEN);

    assert!(shows(&app, "▸ resolved · 1 reply"));
    // A wide emoji leaves the old text in the cell it covers in the test
    // backend; a full redraw shows the screen as a terminal would.
    app.redraw_all().unwrap();
    assert_snapshot!(app.backend());
}

#[test]
fn a_focused_resolved_thread_opens_with_its_replies() {
    let mut app = app();
    focus(&mut app, RESOLVED);

    assert!(!shows(&app, "▸ resolved"));
    assert!(shows(&app, "It follows the theme now"));
    // A wide emoji leaves the old text in the cell it covers in the test
    // backend; a full redraw shows the screen as a terminal would.
    app.redraw_all().unwrap();
    assert_snapshot!(app.backend());
}

#[test]
fn r_replies_in_the_thread_and_the_reply_shows_until_linear_stores_it() {
    let mut app = app();
    focus(&mut app, OPEN_REPLY);

    let writing = only_editor(press(&mut app, "r"));
    let Writing::Reply { parent_id, .. } = &writing else {
        panic!("{writing:?}");
    };
    assert_eq!(parent_id, OPEN, "a reply to a reply joins its thread");

    let effects = app.handle(saved(writing, "Agreed\n")).unwrap();
    let [Effect::CreateComment(reply)] = effects.as_slice() else {
        panic!("{effects:?}");
    };
    assert_eq!(reply.parent_id.as_deref(), Some(OPEN));
    press(&mut app, "}");
    assert!(shows(&app, "sending…"), "{:#?}", screen(&app));

    let posted = on_fake(async |client| {
        client
            .create_comment(&reply.issue_id, &reply.body, reply.parent_id.as_deref())
            .await
            .unwrap()
    });
    let stored_id = posted.comment.id.clone();
    app.handle(Event::CommentCreated {
        comment: reply.clone(),
        result: Ok(posted),
    })
    .unwrap();

    let stored = comment(&app, &stored_id).unwrap();
    assert_eq!(stored.parent_id.as_deref(), Some(OPEN));
    assert!(!shows(&app, "sending…"));
    assert_eq!(
        app.state().notice.as_deref(),
        Some(format!("Replied on {FIRST}").as_str())
    );
}

#[test]
fn a_reply_linear_refuses_goes_and_its_text_starts_the_next_reply() {
    let mut app = app();
    focus(&mut app, OPEN);
    let writing = only_editor(press(&mut app, "r"));
    let effects = app.handle(saved(writing, "Agreed")).unwrap();
    let [Effect::CreateComment(reply)] = effects.as_slice() else {
        panic!("{effects:?}");
    };
    let before = app.state().errors.len();

    app.handle(Event::CommentCreated {
        comment: reply.clone(),
        result: Err(forbidden(&format!("comment on {FIRST}"))),
    })
    .unwrap();

    assert_eq!(app.state().store.comments(ISSUE).unwrap().len(), 4);
    assert_eq!(app.state().errors.len(), before + 1);
    assert!(capability(&app, WriteClass::CreateComment).is_denied());
    assert!(
        app.state()
            .notice
            .as_deref()
            .unwrap()
            .starts_with(&format!("Could not reply on {FIRST}"))
    );
}

#[test]
fn a_reply_that_fails_keeps_its_text_for_the_next_reply_in_the_thread() {
    let mut app = app();
    focus(&mut app, OPEN);
    let writing = only_editor(press(&mut app, "r"));
    let effects = app.handle(saved(writing, "Agreed")).unwrap();
    let [Effect::CreateComment(reply)] = effects.as_slice() else {
        panic!("{effects:?}");
    };

    app.handle(Event::CommentCreated {
        comment: reply.clone(),
        result: Err(offline()),
    })
    .unwrap();

    focus(&mut app, OPEN_REPLY);
    assert_eq!(editor_text(&mut app, "r"), "Agreed", "same thread");
    focus(&mut app, RESOLVED);
    assert_eq!(
        editor_text(&mut app, "r"),
        "",
        "another thread starts empty"
    );
}

#[test]
fn e_edits_the_viewers_comment_from_its_text_and_shows_the_change_at_once() {
    let mut app = app();
    focus(&mut app, OPEN_REPLY);
    let body = comment(&app, OPEN_REPLY).unwrap().body;

    let effects = press(&mut app, "E");
    let [Effect::OpenEditor { writing, text }] = effects.as_slice() else {
        panic!("{effects:?}");
    };
    assert_eq!(text, &body);

    let write = only_change(app.handle(saved(writing.clone(), "Picked up")).unwrap());
    assert_eq!(write.change, CommentChange::Edit("Picked up".to_owned()));
    assert_eq!(write.comment_id, OPEN_REPLY);
    assert_eq!(comment(&app, OPEN_REPLY).unwrap().body, "Picked up");

    app.handle(Event::CommentChanged {
        write,
        result: Ok(()),
    })
    .unwrap();
    assert_eq!(comment(&app, OPEN_REPLY).unwrap().body, "Picked up");
}

#[test]
fn an_edit_linear_refuses_is_undone_and_its_text_starts_the_next_edit() {
    let mut app = app();
    focus(&mut app, OPEN_REPLY);
    let body = comment(&app, OPEN_REPLY).unwrap().body;
    let writing = only_editor(press(&mut app, "E"));
    let write = only_change(app.handle(saved(writing, "Picked up")).unwrap());
    let before = app.state().errors.len();

    app.handle(Event::CommentChanged {
        write,
        result: Err(forbidden(&format!("edit a comment on {FIRST}"))),
    })
    .unwrap();

    assert_eq!(comment(&app, OPEN_REPLY).unwrap().body, body);
    assert_eq!(app.state().errors.len(), before + 1);
    assert!(capability(&app, WriteClass::ChangeComment).is_denied());
    assert!(
        app.state()
            .notice
            .as_deref()
            .unwrap()
            .starts_with(&format!("Could not edit the comment on {FIRST}"))
    );
}

/// The emoji of the reactions on the comment `id`.
fn reaction_emoji(app: &App<TestBackend>, id: &str) -> Vec<String> {
    comment(app, id)
        .unwrap()
        .reactions
        .iter()
        .map(|reaction| reaction.emoji.clone())
        .collect()
}

/// Reacts with the picker's `nth` emoji on the focused comment.
fn react(app: &mut App<TestBackend>, nth: usize) {
    press(app, "+");
    let effects = press(app, &nth.to_string());
    assert!(
        matches!(effects.as_slice(), [Effect::React(_)]),
        "{effects:?}"
    );
}

#[test]
fn an_edit_linear_refuses_undoes_only_the_text() {
    let mut app = app();
    focus(&mut app, OPEN_REPLY);
    let body = comment(&app, OPEN_REPLY).unwrap().body;
    let writing = only_editor(press(&mut app, "E"));
    let write = only_change(app.handle(saved(writing, "Picked up")).unwrap());
    react(&mut app, 7);
    let reacted = reaction_emoji(&app, OPEN_REPLY);
    assert!(reacted.contains(&"🚀".to_owned()), "{reacted:?}");

    app.handle(Event::CommentChanged {
        write,
        result: Err(forbidden(&format!("edit a comment on {FIRST}"))),
    })
    .unwrap();

    assert_eq!(comment(&app, OPEN_REPLY).unwrap().body, body);
    assert_eq!(reaction_emoji(&app, OPEN_REPLY), reacted);
}

#[test]
fn an_edit_that_fails_keeps_its_text_for_the_next_edit_until_one_is_taken() {
    let mut app = app();
    focus(&mut app, OPEN_REPLY);
    let writing = only_editor(press(&mut app, "E"));
    let write = only_change(app.handle(saved(writing, "Picked up")).unwrap());

    app.handle(Event::CommentChanged {
        write,
        result: Err(offline()),
    })
    .unwrap();

    let effects = press(&mut app, "E");
    let [Effect::OpenEditor { writing, text }] = effects.as_slice() else {
        panic!("{effects:?}");
    };
    assert_eq!(text, "Picked up");
    let write = only_change(
        app.handle(saved(writing.clone(), "Picked up today"))
            .unwrap(),
    );
    app.handle(Event::CommentChanged {
        write,
        result: Ok(()),
    })
    .unwrap();
    assert_eq!(
        editor_text(&mut app, "E"),
        "Picked up today",
        "once one is taken, the next edit starts from the comment"
    );
}

#[test]
fn edit_and_delete_are_refused_on_another_persons_comment_with_why() {
    let mut app = app();
    focus(&mut app, OPEN);

    assert_eq!(press(&mut app, "E"), Vec::new());
    assert_eq!(
        app.state().notice.as_deref(),
        Some("Only its author can edit a comment; this one is Nell Havarti's")
    );
    assert_eq!(press(&mut app, "X"), Vec::new());
    assert_eq!(
        app.state().notice.as_deref(),
        Some("Only its author can delete a comment; this one is Nell Havarti's")
    );
    assert!(app.state().modals.top().is_none());
}

#[test]
fn x_upper_deletes_the_viewers_comment_after_confirming() {
    let mut app = app();
    focus(&mut app, RESOLVED_REPLY);

    assert_eq!(press(&mut app, "X"), Vec::new());
    assert!(shows(&app, "Delete your comment?"));
    let write = only_change(press(&mut app, "y"));

    assert_eq!(write.change, CommentChange::Delete);
    assert_eq!(write.comment_id, RESOLVED_REPLY);
    assert_eq!(comment(&app, RESOLVED_REPLY), None);
    assert_eq!(focused(&app).as_deref(), Some(OPEN));
}

#[test]
fn a_delete_linear_refuses_puts_the_comment_back() {
    let mut app = app();
    focus(&mut app, RESOLVED_REPLY);
    press(&mut app, "X");
    let write = only_change(press(&mut app, "y"));

    app.handle(Event::CommentChanged {
        write,
        result: Err(forbidden(&format!("delete a comment on {FIRST}"))),
    })
    .unwrap();

    let ids: Vec<String> = app
        .state()
        .store
        .comments(ISSUE)
        .unwrap()
        .iter()
        .map(|comment| comment.id.clone())
        .collect();
    assert_eq!(ids, [RESOLVED, RESOLVED_REPLY, OPEN, OPEN_REPLY]);
    assert!(capability(&app, WriteClass::ChangeComment).is_denied());
}

#[test]
fn x_resolves_an_open_thread_from_a_reply_and_unresolves_a_resolved_one() {
    let mut app = app();
    focus(&mut app, OPEN_REPLY);

    let write = only_change(press(&mut app, "x"));
    assert_eq!(write.change, CommentChange::Resolve);
    assert_eq!(write.comment_id, OPEN, "the thread's first comment");
    assert!(comment(&app, OPEN).unwrap().resolved_at.is_some());

    focus(&mut app, RESOLVED);
    let write = only_change(press(&mut app, "x"));
    assert_eq!(write.change, CommentChange::Unresolve);
    assert!(comment(&app, RESOLVED).unwrap().resolved_at.is_none());
}

#[test]
fn a_resolve_linear_refuses_opens_the_thread_again() {
    let mut app = app();
    focus(&mut app, OPEN);
    let write = only_change(press(&mut app, "x"));
    let before = app.state().errors.len();

    app.handle(Event::CommentChanged {
        write,
        result: Err(forbidden(&format!("resolve a thread on {FIRST}"))),
    })
    .unwrap();

    assert!(comment(&app, OPEN).unwrap().resolved_at.is_none());
    assert_eq!(app.state().errors.len(), before + 1);
    assert!(
        app.state()
            .notice
            .as_deref()
            .unwrap()
            .starts_with(&format!("Could not resolve the thread on {FIRST}"))
    );
}

#[test]
fn a_resolve_linear_refuses_undoes_only_the_resolve() {
    let mut app = app();
    focus(&mut app, OPEN);
    let write = only_change(press(&mut app, "x"));
    react(&mut app, 7);
    let reacted = reaction_emoji(&app, OPEN);
    assert!(reacted.contains(&"🚀".to_owned()), "{reacted:?}");

    app.handle(Event::CommentChanged {
        write,
        result: Err(forbidden(&format!("resolve a thread on {FIRST}"))),
    })
    .unwrap();

    assert!(comment(&app, OPEN).unwrap().resolved_at.is_none());
    assert_eq!(reaction_emoji(&app, OPEN), reacted);
}

#[test]
fn a_detail_loaded_while_a_change_is_in_flight_keeps_the_change() {
    let mut app = app();
    focus(&mut app, OPEN);
    let write = only_change(press(&mut app, "x"));

    app.handle(Event::IssueDetailLoaded {
        issue_id: ISSUE.to_owned(),
        result: Ok(IssueDetail::demo(ISSUE)),
    })
    .unwrap();
    assert!(comment(&app, OPEN).unwrap().resolved_at.is_some());

    app.handle(Event::CommentChanged {
        write,
        result: Ok(()),
    })
    .unwrap();
    assert!(comment(&app, OPEN).unwrap().resolved_at.is_some());
}

/// Loads DES-34's detail again, as Linear would answer before it has
/// taken the changes in flight.
fn reload(app: &mut App<TestBackend>) {
    app.handle(Event::IssueDetailLoaded {
        issue_id: ISSUE.to_owned(),
        result: Ok(IssueDetail::demo(ISSUE)),
    })
    .unwrap();
}

#[test]
fn a_detail_loaded_while_a_delete_is_in_flight_keeps_the_comment_gone() {
    let mut app = app();
    focus(&mut app, RESOLVED_REPLY);
    press(&mut app, "X");
    let write = only_change(press(&mut app, "y"));

    reload(&mut app);
    assert_eq!(comment(&app, RESOLVED_REPLY), None);

    app.handle(Event::CommentChanged {
        write,
        result: Ok(()),
    })
    .unwrap();
    assert_eq!(comment(&app, RESOLVED_REPLY), None);
}

#[test]
fn a_detail_loaded_while_an_edit_is_in_flight_keeps_the_new_text() {
    let mut app = app();
    focus(&mut app, OPEN_REPLY);
    let writing = only_editor(press(&mut app, "E"));
    only_change(app.handle(saved(writing, "Picked up")).unwrap());

    reload(&mut app);

    let edited = comment(&app, OPEN_REPLY).unwrap();
    assert_eq!(edited.body, "Picked up");
    assert!(edited.edited_at.is_some());
}

#[test]
fn a_detail_loaded_while_an_unresolve_is_in_flight_keeps_the_thread_open() {
    let mut app = app();
    focus(&mut app, RESOLVED);
    let write = only_change(press(&mut app, "x"));
    assert_eq!(write.change, CommentChange::Unresolve);

    reload(&mut app);

    assert!(comment(&app, RESOLVED).unwrap().resolved_at.is_none());
}

#[test]
fn deleting_the_viewers_thread_takes_its_replies_and_a_refusal_brings_them_back() {
    let mut app = app();
    let viewer = app.state().store.viewer().unwrap().id.clone();
    let mut detail = IssueDetail::demo(ISSUE);
    for comment in &mut detail.comments {
        if comment.id == OPEN {
            comment.author_id = Some(viewer.clone());
        }
    }
    app.handle(Event::IssueDetailLoaded {
        issue_id: ISSUE.to_owned(),
        result: Ok(detail),
    })
    .unwrap();
    focus(&mut app, OPEN);

    press(&mut app, "X");
    assert!(
        shows(&app, "Its reply is deleted too."),
        "{:#?}",
        screen(&app)
    );
    let write = only_change(press(&mut app, "y"));

    assert_eq!(comment(&app, OPEN), None);
    assert_eq!(comment(&app, OPEN_REPLY), None, "its reply went with it");

    app.handle(Event::CommentChanged {
        write,
        result: Err(offline()),
    })
    .unwrap();
    let ids: Vec<String> = app
        .state()
        .store
        .comments(ISSUE)
        .unwrap()
        .iter()
        .map(|comment| comment.id.clone())
        .collect();
    assert_eq!(ids, [RESOLVED, RESOLVED_REPLY, OPEN, OPEN_REPLY]);
}

#[test]
fn r_and_e_keep_their_detail_meaning_without_a_focused_comment() {
    let mut app = app();

    let effects = press(&mut app, "E");

    assert!(
        matches!(
            effects.as_slice(),
            [Effect::OpenEditor {
                writing: Writing::Text(_),
                ..
            }]
        ),
        "{effects:?}"
    );
}

#[test]
fn the_fake_client_keeps_comment_changes_for_the_session() {
    let detail = on_fake(async |client| {
        client
            .update_comment(OPEN_REPLY, "Picked up")
            .await
            .unwrap();
        client.resolve_comment(OPEN, true).await.unwrap();
        client.delete_comment(RESOLVED).await.unwrap();
        client
            .issue_detail(ISSUE, linear_rat::linear::Urgency::Urgent)
            .await
            .unwrap()
    });

    let ids: Vec<&str> = detail.comments.iter().map(|c| c.id.as_str()).collect();
    assert_eq!(ids, [OPEN, OPEN_REPLY], "the thread went with its first");
    assert!(detail.comments[0].resolved_at.is_some());
    assert_eq!(detail.comments[1].body, "Picked up");
    assert!(detail.comments[1].edited_at.is_some());
}

#[test]
fn a_reply_is_sent_with_its_parent() {
    let reply = NewComment {
        issue_id: ISSUE.to_owned(),
        identifier: FIRST.to_owned(),
        body: "Agreed".to_owned(),
        parent_id: Some(OPEN.to_owned()),
        local_id: None,
    };
    let posted = on_fake(async |client| {
        client
            .create_comment(&reply.issue_id, &reply.body, reply.parent_id.as_deref())
            .await
            .unwrap()
    });
    assert_eq!(posted.comment.parent_id.as_deref(), Some(OPEN));
}
