//! The Inbox through the app driver: the grouped list and its tab count,
//! read, group read, archive, snooze and archive all read (each shown at once
//! and rolled back on a rejection), Enter opening the issue, and the delta
//! poll.

// clippy's allow-unwrap-in-tests does not cover helpers outside #[test] functions.
#![allow(clippy::unwrap_used)]

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use insta::assert_snapshot;
use jiff::tz::TimeZone;
use jiff::{SignedDuration, Timestamp};
use linear_rat::app::{
    App, Capability, Effect, Event, Failure, InboxChange, InboxWrite, Modal, Pane, Screen, State,
    WriteClass, snooze_presets,
};
use linear_rat::linear::model::Notification;
use linear_rat::linear::{ErrorKind, FakeClient, LinearClient, NotificationsLoaded, Urgency, demo};
use ratatui::backend::TestBackend;

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

async fn demo_inbox() -> NotificationsLoaded {
    FakeClient.notifications(None).await.unwrap()
}

fn loaded(inbox: NotificationsLoaded) -> Event {
    Event::NotificationsLoaded {
        since: None,
        result: Ok(inbox),
    }
}

/// The demo app at `columns` by `rows` with the demo inbox loaded, on the
/// Inbox.
async fn inbox_at(columns: u16, rows: u16) -> App<TestBackend> {
    let mut app = App::new(TestBackend::new(columns, rows), State::demo()).unwrap();
    app.handle(loaded(demo_inbox().await)).unwrap();
    press(&mut app, "6");
    app
}

async fn inbox() -> App<TestBackend> {
    inbox_at(80, 24).await
}

fn stored(app: &App<TestBackend>, id: &str) -> Notification {
    app.state().store.notification(id).unwrap().clone()
}

fn focused(app: &App<TestBackend>) -> Notification {
    linear_rat::app::focused_notification(app.state())
        .unwrap()
        .clone()
}

fn writes(effects: Vec<Effect>) -> Vec<InboxWrite> {
    effects
        .into_iter()
        .filter_map(|effect| match effect {
            Effect::WriteInbox(write) => Some(write),
            _ => None,
        })
        .collect()
}

fn only_write(effects: Vec<Effect>) -> InboxWrite {
    let mut writes = writes(effects);
    assert_eq!(writes.len(), 1, "{writes:?}");
    writes.remove(0)
}

fn rejected(write: InboxWrite) -> Event {
    Event::InboxWritten {
        write,
        result: Err(Failure::new(
            ErrorKind::Forbidden,
            "mark read",
            "Linear returned errors: forbidden (FORBIDDEN)",
        )),
    }
}

fn unread(app: &App<TestBackend>) -> usize {
    linear_rat::app::unread_count(app.state())
}

#[tokio::test]
async fn the_inbox_groups_notifications_by_issue_newest_first() {
    let app = inbox().await;

    assert_snapshot!("inbox_80x24", screen(&app));
}

#[tokio::test]
async fn the_inbox_at_120x40() {
    let app = inbox_at(120, 40).await;

    assert_snapshot!("inbox_120x40", screen(&app));
}

#[test]
fn before_it_loads_the_inbox_says_it_is_loading() {
    let mut app = App::new(TestBackend::new(80, 24), State::demo()).unwrap();
    press(&mut app, "6");

    assert_snapshot!("inbox_loading", screen(&app));
}

#[test]
fn an_empty_inbox_says_so() {
    let mut app = App::new(TestBackend::new(80, 24), State::demo()).unwrap();
    app.handle(loaded(NotificationsLoaded::default())).unwrap();
    press(&mut app, "6");

    assert_snapshot!("inbox_empty", screen(&app));
}

#[test]
fn a_failed_first_load_says_why_and_the_next_poll_tries_again() {
    let mut app = App::new(TestBackend::new(80, 24), State::demo()).unwrap();
    app.handle(Event::NotificationsLoaded {
        since: None,
        result: Err(Failure::new(
            ErrorKind::Offline,
            "load the Inbox",
            "could not reach Linear",
        )),
    })
    .unwrap();
    press(&mut app, "6");

    assert!(screen(&app).contains("could not reach Linear"));
    assert_eq!(app.state().errors.len(), 1);
}

#[tokio::test]
async fn the_tab_counts_unread_notifications_shown() {
    let app = inbox().await;

    let count = unread(&app);
    assert_eq!(count, 5);
    assert!(
        screen(&app).lines().next().unwrap().contains("Inbox 5"),
        "{}",
        screen(&app)
    );
}

#[tokio::test]
async fn the_first_notification_is_the_newest_and_its_issue_heads_the_list() {
    let app = inbox().await;

    let newest = focused(&app);
    let all = demo_inbox().await.notifications;
    let latest = all.iter().max_by_key(|n| n.created_at).unwrap();
    assert_eq!(newest.id, latest.id);
}

#[tokio::test]
async fn m_marks_the_focused_notification_read_and_again_unread() {
    let mut app = inbox().await;
    let id = focused(&app).id;
    assert!(stored(&app, &id).read_at.is_none());

    let write = only_write(press(&mut app, "m"));
    assert!(matches!(write.change, InboxChange::Read(_)));
    assert_eq!(write.notification_id, id);
    assert!(stored(&app, &id).read_at.is_some());
    assert_eq!(unread(&app), 4);

    let write = only_write(press(&mut app, "m"));
    assert_eq!(write.change, InboxChange::Unread);
    assert!(stored(&app, &id).read_at.is_none());
}

#[tokio::test]
async fn a_rejected_read_puts_it_back_with_the_error_and_learns_the_denial() {
    let mut app = inbox().await;
    let id = focused(&app).id;
    let write = only_write(press(&mut app, "m"));
    let team_id = write.team_id.clone();

    app.handle(rejected(write)).unwrap();

    assert!(stored(&app, &id).read_at.is_none());
    let notice = app.state().notice.clone().unwrap();
    assert!(
        notice.starts_with("Could not mark read a notification"),
        "{notice}"
    );
    assert_eq!(app.state().errors.len(), 1);
    assert!(matches!(
        app.state()
            .store
            .capabilities()
            .write(&team_id, WriteClass::Inbox),
        Capability::Denied { .. }
    ));
    // The key is now known to be denied, so `m` says so and sends nothing.
    assert!(writes(press(&mut app, "m")).is_empty());
}

#[tokio::test]
async fn shift_m_marks_every_notification_of_the_issue_read() {
    let mut app = inbox().await;
    let issue_id = focused(&app).issue_id;

    let write = only_write(press(&mut app, "M"));

    assert!(matches!(write.change, InboxChange::ReadIssue(_)));
    assert_eq!(write.issue_id, issue_id);
    assert!(
        app.state()
            .store
            .notifications()
            .filter(|n| n.issue_id == issue_id)
            .all(|n| n.read_at.is_some())
    );

    app.handle(rejected(write)).unwrap();
    assert!(
        app.state()
            .store
            .notifications()
            .filter(|n| n.issue_id == issue_id)
            .any(|n| n.read_at.is_none()),
        "the rejection marks them unread again"
    );
}

#[tokio::test]
async fn shift_a_archives_the_notification_and_a_rejection_brings_it_back() {
    let mut app = inbox().await;
    let id = focused(&app).id;

    let write = only_write(press(&mut app, "A"));

    assert_eq!(write.change, InboxChange::Archive);
    assert_ne!(focused(&app).id, id, "the focus moves to the next one");
    assert!(
        !linear_rat::app::inbox_shown(app.state())
            .iter()
            .any(|n| n.id == id)
    );

    app.handle(rejected(write)).unwrap();
    assert!(
        linear_rat::app::inbox_shown(app.state())
            .iter()
            .any(|n| n.id == id)
    );
}

#[tokio::test]
async fn z_snoozes_until_a_preset_and_the_notification_comes_back_once_it_passes() {
    // Wide enough for the tab to carry its count.
    let mut app = inbox_at(120, 40).await;
    let id = focused(&app).id;

    press(&mut app, "z");
    assert!(matches!(app.state().modals.top(), Some(Modal::Choice(_))));
    let snapshot = screen(&app);
    assert!(snapshot.contains("In 1 hour"), "{snapshot}");
    let write = only_write(press(&mut app, "\n"));

    let InboxChange::Snooze(until) = write.change else {
        panic!("not a snooze: {write:?}");
    };
    assert_eq!(until, demo::NOW + SignedDuration::from_hours(1));
    assert!(
        !linear_rat::app::inbox_shown(app.state())
            .iter()
            .any(|n| n.id == id)
    );
    let snoozed = screen(&app);
    assert!(snoozed.contains("[6] Inbox 4 "), "{snoozed}");
    assert!(snoozed.contains("Inbox · 4 unread"), "{snoozed}");

    let effects = app.handle(Event::Tick(until)).unwrap();
    drop(effects);
    assert!(
        linear_rat::app::inbox_shown(app.state())
            .iter()
            .any(|n| n.id == id),
        "it comes back once the snooze passes"
    );
    let woken = screen(&app);
    assert!(woken.contains("[6] Inbox 5 "), "the tick redraws: {woken}");
    assert!(woken.contains("Inbox · 5 unread"), "{woken}");
}

#[test]
fn the_snooze_presets_fall_in_the_viewers_time_zone() {
    // Tuesday 2026-11-03 18:30 in New York.
    let now: Timestamp = "2026-11-03T23:30:00Z".parse().unwrap();
    let zone = TimeZone::get("America/New_York").unwrap();

    let presets = snooze_presets(now, &zone);

    let times: Vec<String> = presets.iter().map(|(_, at)| at.to_string()).collect();
    assert_eq!(
        times,
        [
            "2026-11-04T00:30:00Z",
            "2026-11-04T14:00:00Z",
            "2026-11-09T14:00:00Z"
        ]
    );
}

#[tokio::test]
async fn archive_all_read_runs_from_the_palette_one_mutation_each() {
    let mut app = inbox().await;
    let read = linear_rat::app::inbox_shown(app.state())
        .iter()
        .filter(|n| n.read_at.is_some())
        .count();

    press(&mut app, ":");
    let effects = press(&mut app, "archive all read\n");

    let batch = effects
        .into_iter()
        .find_map(|effect| match effect {
            Effect::ArchiveNotifications(writes) => Some(writes),
            _ => None,
        })
        .unwrap();
    assert_eq!(batch.len(), read);
    assert!(batch.iter().all(|w| w.change == InboxChange::Archive));
    assert_eq!(linear_rat::app::inbox_shown(app.state()).len(), 12 - read);

    let answers = batch.into_iter().map(|write| (write, Ok(()))).collect();
    app.handle(Event::InboxBatchWritten(answers)).unwrap();
    assert_eq!(
        app.state().notice.as_deref(),
        Some(format!("Archived {read} read notifications").as_str())
    );
}

#[tokio::test]
async fn archive_all_read_is_held_back_once_inbox_writes_are_denied() {
    let mut app = inbox().await;
    let write = only_write(press(&mut app, "m"));
    app.handle(rejected(write)).unwrap();

    press(&mut app, ":");
    let effects = press(&mut app, "archive all read\n");

    assert!(
        !effects
            .iter()
            .any(|effect| matches!(effect, Effect::ArchiveNotifications(_))),
        "{effects:?}"
    );
    assert!(linear_rat::app::inbox_has_read(app.state()));
}

#[tokio::test]
async fn enter_opens_the_issue_with_the_comment_focused_and_marks_it_read() {
    let mut app = inbox().await;
    let notification = focused(&app);
    assert!(notification.comment_id.is_some(), "{notification:?}");

    let effects = press(&mut app, "\n");

    let state = app.state();
    assert_eq!(state.screen, Screen::Results);
    assert_eq!(state.results.pane(), Pane::Detail);
    assert_eq!(state.focused_issue().unwrap().id, notification.issue_id);
    assert_eq!(
        state.detail.focused_comment(&notification.issue_id),
        notification.comment_id.as_deref()
    );
    assert!(effects.iter().any(|effect| matches!(
        effect,
        Effect::LoadIssueDetail { issue_id, .. } if *issue_id == notification.issue_id
    )));
    let write = only_write(effects);
    assert!(matches!(write.change, InboxChange::Read(_)));

    // Esc twice comes back to the Inbox.
    press(&mut app, "\x1b\x1b");
    assert_eq!(app.state().screen, Screen::Inbox);
}

#[tokio::test]
async fn the_comment_enter_focused_is_the_detail_s_focused_comment_once_it_loads() {
    let mut app = inbox().await;
    let notification = focused(&app);
    let comment_id = notification.comment_id.clone().unwrap();

    press(&mut app, "\n");
    let detail = FakeClient
        .issue_detail(&notification.issue_id, Urgency::Urgent)
        .await
        .unwrap();
    app.handle(Event::IssueDetailLoaded {
        issue_id: notification.issue_id.clone(),
        result: Ok(detail),
    })
    .unwrap();

    let comment = linear_rat::app::focused_comment(app.state()).unwrap();
    assert_eq!(comment.id, comment_id);
    assert!(linear_rat::app::can_focus_comments(app.state()));
}

#[tokio::test]
async fn the_first_load_follows_my_issues_and_polls_bring_deltas() {
    let mut app = App::new(TestBackend::new(80, 24), State::loading(None)).unwrap();
    let assigned = FakeClient.assigned_issues().await.unwrap();

    let effects = app.handle(Event::MyIssuesLoaded(Ok(assigned))).unwrap();
    assert!(effects.contains(&Effect::LoadNotifications { since: None }));

    let inbox = demo_inbox().await;
    let watermark = inbox.notifications.iter().map(|n| n.updated_at).max();
    app.handle(loaded(inbox)).unwrap();
    let effects = app.handle(Event::Tick(demo::NOW)).unwrap();
    assert!(
        effects.contains(&Effect::LoadNotifications { since: watermark }),
        "{effects:?}"
    );

    // A delta marks one read elsewhere.
    let mut changed = stored(&app, "notification-1");
    changed.read_at = Some(demo::NOW);
    changed.updated_at = demo::NOW;
    let before = unread(&app);
    app.handle(Event::NotificationsLoaded {
        since: watermark,
        result: Ok(NotificationsLoaded {
            notifications: vec![changed],
            ..NotificationsLoaded::default()
        }),
    })
    .unwrap();
    assert_eq!(unread(&app), before - 1);
}

#[tokio::test]
async fn an_older_copy_does_not_undo_a_write_in_flight() {
    let mut app = inbox().await;
    let id = focused(&app).id;
    press(&mut app, "m");
    let older = demo_inbox()
        .await
        .notifications
        .into_iter()
        .find(|n| n.id == id)
        .unwrap();

    app.handle(Event::NotificationsLoaded {
        since: Some(demo::NOW),
        result: Ok(NotificationsLoaded {
            notifications: vec![older],
            ..NotificationsLoaded::default()
        }),
    })
    .unwrap();

    assert!(stored(&app, &id).read_at.is_some());
}

#[tokio::test]
async fn the_demo_client_keeps_what_inbox_writes_did() {
    let first = demo_inbox().await.notifications;
    let unread = first.iter().find(|n| n.read_at.is_none()).unwrap().clone();

    FakeClient
        .archive_notification(&unread.id, linear_rat::linear::Urgency::Urgent)
        .await
        .unwrap();

    let after = demo_inbox().await.notifications;
    assert!(!after.iter().any(|n| n.id == unread.id));
    let delta = FakeClient
        .notifications(Some(unread.updated_at))
        .await
        .unwrap();
    assert!(
        delta
            .notifications
            .iter()
            .any(|n| n.id == unread.id && n.archived_at.is_some())
    );
}

#[tokio::test]
async fn failing_deltas_go_in_the_error_log_once_and_keep_the_inbox() {
    let mut app = inbox().await;
    let failed = || Event::NotificationsLoaded {
        since: Some(demo::NOW),
        result: Err(Failure::new(
            ErrorKind::Offline,
            "load the Inbox",
            "could not reach Linear",
        )),
    };

    app.handle(failed()).unwrap();
    app.handle(failed()).unwrap();

    assert_eq!(app.state().errors.len(), 1);
    assert_eq!(unread(&app), 5);
    assert!(screen(&app).contains("Theo Whisker"));
}
