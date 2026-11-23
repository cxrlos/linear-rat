//! The Inbox screen: the viewer's issue notifications, grouped by issue,
//! the group of the newest one first. Marking read, snoozing and archiving
//! show at once, go to Linear as one mutation each and are undone if Linear
//! rejects them.

use jiff::civil::Weekday;
use jiff::tz::TimeZone;
use jiff::{SignedDuration, Timestamp};

use super::modal::{Choice, Purpose};
use super::picker::{Entry, Picker};
use super::{Action, Effect, Failure, Modal, Pane, Progress, State, Store, Update, WriteClass};
use crate::linear::NotificationsLoaded;
use crate::linear::model::Notification;
use crate::ui;

/// How far the inbox has loaded and which notification is focused. The
/// notifications themselves live in the store.
#[derive(Debug, Default)]
pub struct Inbox {
    load: Progress,
    /// A delta is in flight.
    polling: bool,
    /// The last delta failed; the error log already has it.
    failing: bool,
    /// The focused notification, and its place in the list, where the focus
    /// stays when the notification leaves.
    focused_id: Option<String>,
    focused_at: usize,
}

impl Inbox {
    /// How far the first load has come.
    pub fn progress(&self) -> &Progress {
        &self.load
    }

    /// The place of the focused notification in `shown`.
    pub fn focused_index(&self, shown: &[&Notification]) -> usize {
        self.focused_id
            .as_deref()
            .and_then(|id| shown.iter().position(|n| n.id == id))
            .unwrap_or(self.focused_at)
            .min(shown.len().saturating_sub(1))
    }
}

/// The notifications about one issue, newest first.
#[derive(Debug)]
pub struct InboxGroup<'a> {
    pub issue_id: &'a str,
    pub notifications: Vec<&'a Notification>,
}

/// Whether `notification` is in the inbox at `now`: not archived, and not
/// snoozed past `now`.
fn is_shown(notification: &Notification, now: Timestamp) -> bool {
    notification.archived_at.is_none()
        && notification
            .snoozed_until_at
            .is_none_or(|until| until <= now)
}

/// The notifications shown at `now`, grouped by issue, the group of the
/// newest notification first.
pub fn groups(store: &Store, now: Timestamp) -> Vec<InboxGroup<'_>> {
    let mut shown: Vec<&Notification> = store
        .notifications()
        .filter(|notification| is_shown(notification, now))
        .collect();
    shown.sort_by(|a, b| b.created_at.cmp(&a.created_at).then(a.id.cmp(&b.id)));
    let mut groups: Vec<InboxGroup<'_>> = Vec::new();
    for notification in shown {
        match groups
            .iter_mut()
            .find(|group| group.issue_id == notification.issue_id)
        {
            Some(group) => group.notifications.push(notification),
            None => groups.push(InboxGroup {
                issue_id: &notification.issue_id,
                notifications: vec![notification],
            }),
        }
    }
    groups
}

/// The notifications shown, in the order the list shows them.
pub fn shown(state: &State) -> Vec<&Notification> {
    groups(&state.store, state.now())
        .into_iter()
        .flat_map(|group| group.notifications)
        .collect()
}

/// How many notifications shown are unread, for the tab.
pub fn unread_count(state: &State) -> usize {
    shown(state)
        .iter()
        .filter(|notification| notification.read_at.is_none())
        .count()
}

/// The focused notification, if any are shown.
pub fn focused(state: &State) -> Option<&Notification> {
    let shown = shown(state);
    let index = state.inbox.focused_index(&shown);
    shown.get(index).copied()
}

/// Whether some notification shown is read, so archive all read has work.
pub fn has_read(state: &State) -> bool {
    shown(state)
        .iter()
        .any(|notification| notification.read_at.is_some())
}

/// Moves the focus for a list of `rows` rows. Returns whether it moved.
pub(super) fn perform(state: &mut State, action: Action, rows: usize) -> bool {
    let ids: Vec<String> = shown(state)
        .iter()
        .map(|notification| notification.id.clone())
        .collect();
    let Some(last) = ids.len().checked_sub(1) else {
        return false;
    };
    let focused = state.inbox.focused_index(&shown(state)).min(last);
    let page = (rows / 2).max(1);
    let next = match action {
        Action::MoveDown => focused.saturating_add(1),
        Action::MoveUp => focused.saturating_sub(1),
        Action::MoveToTop => 0,
        Action::MoveToBottom => last,
        Action::HalfPageDown => focused.saturating_add(page),
        Action::HalfPageUp => focused.saturating_sub(page),
        _ => focused,
    }
    .min(last);
    let moved = next != focused;
    state.inbox.focused_at = next;
    state.inbox.focused_id = ids.get(next).cloned();
    moved
}

/// The first load, once the viewer is confirmed, unless it has started.
pub(super) fn first_load(state: &mut State) -> Option<Effect> {
    match state.inbox.load {
        Progress::NotAsked => {
            state.inbox.load = Progress::Loading;
            Some(Effect::LoadNotifications { since: None })
        }
        _ => None,
    }
}

/// What a poll asks of the inbox: the first load again after it failed,
/// else a delta from the watermark, unless one is in flight.
pub(super) fn poll(state: &mut State) -> Option<Effect> {
    match state.inbox.load {
        Progress::NotAsked | Progress::Failed(_) => {
            state.inbox.load = Progress::Loading;
            Some(Effect::LoadNotifications { since: None })
        }
        Progress::Loading => None,
        Progress::Done if state.inbox.polling => None,
        Progress::Done => {
            state.inbox.polling = true;
            Some(Effect::LoadNotifications {
                since: state.store.notifications_watermark(),
            })
        }
    }
}

/// Takes in a load or delta. A failed first load says why on the screen; a
/// failed delta goes in the error log, once for a run of failures, as the
/// next poll tries again.
pub(super) fn loaded(
    state: &mut State,
    since: Option<Timestamp>,
    result: Result<NotificationsLoaded, Failure>,
) -> Update {
    state.inbox.polling = false;
    match result {
        Ok(loaded) => {
            let first = state.inbox.load != Progress::Done;
            state.inbox.load = Progress::Done;
            state.inbox.failing = false;
            let changed = state.store.merge_notifications(&loaded);
            if changed {
                state.rebuild_lists();
            }
            Update {
                redraw: changed || first,
                effects: Vec::new(),
            }
        }
        Err(failure) => {
            if since.is_none() || state.inbox.load != Progress::Done {
                state.inbox.load = Progress::Failed(failure.message.clone());
            }
            if !state.inbox.failing {
                state.record(failure);
            }
            state.inbox.failing = true;
            Update::redraw()
        }
    }
}

/// Whether a snoozed notification comes back between `before` and `now`,
/// so the tick redraws.
pub(super) fn wakes(state: &State, before: Option<Timestamp>, now: Timestamp) -> bool {
    state.store.notifications().any(|notification| {
        notification.archived_at.is_none()
            && notification
                .snoozed_until_at
                .is_some_and(|until| until <= now && before.is_none_or(|before| until > before))
    })
}

/// A change to notifications on its way to Linear, and the notifications
/// as they were, to put back if Linear rejects it.
#[derive(Debug, Clone, PartialEq)]
pub struct InboxWrite {
    /// The notification the change was made on.
    pub notification_id: String,
    pub issue_id: String,
    /// The issue's team, where a refusal teaches what the API key may do.
    pub team_id: String,
    /// The issue's key, for messages.
    pub identifier: String,
    pub change: InboxChange,
    undo: Vec<Notification>,
}

/// What an [`InboxWrite`] does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InboxChange {
    /// Marks the notification read at this time.
    Read(Timestamp),
    Unread,
    /// Marks every notification about the issue read at this time.
    ReadIssue(Timestamp),
    /// Hides the notification until this time.
    Snooze(Timestamp),
    Archive,
}

impl InboxChange {
    /// What it does, as `mark read`.
    pub fn verb(self) -> &'static str {
        match self {
            InboxChange::Read(_) | InboxChange::ReadIssue(_) => "mark read",
            InboxChange::Unread => "mark unread",
            InboxChange::Snooze(_) => "snooze",
            InboxChange::Archive => "archive",
        }
    }
}

/// `m`: marks the focused notification read, or unread when it is read.
pub(super) fn toggle_read(state: &mut State) -> Update {
    let Some(notification) = focused(state).cloned() else {
        return Update::default();
    };
    let now = state.now();
    let change = if notification.read_at.is_some() {
        InboxChange::Unread
    } else {
        InboxChange::Read(now)
    };
    let read_at = match change {
        InboxChange::Read(at) => Some(at),
        _ => None,
    };
    let write = change_one(state, &notification, change, |n| n.read_at = read_at);
    send(state, vec![write])
}

/// `M`: marks every unread notification about the focused one's issue read.
pub(super) fn mark_issue_read(state: &mut State) -> Update {
    let Some(focused) = focused(state).cloned() else {
        return Update::default();
    };
    let unread: Vec<Notification> = state
        .store
        .notifications()
        .filter(|n| n.issue_id == focused.issue_id && n.read_at.is_none())
        .cloned()
        .collect();
    if unread.is_empty() {
        return Update::default();
    }
    let now = state.now();
    for notification in &unread {
        state
            .store
            .change_notification(&notification.id, |n| n.read_at = Some(now));
    }
    let write = write_for(state, &focused, InboxChange::ReadIssue(now), unread);
    send(state, vec![write])
}

/// `A`: archives the focused notification.
pub(super) fn archive(state: &mut State) -> Update {
    let Some(notification) = focused(state).cloned() else {
        return Update::default();
    };
    let now = state.now();
    let write = change_one(state, &notification, InboxChange::Archive, |n| {
        n.archived_at = Some(now);
    });
    send(state, vec![write])
}

/// Archives every read notification shown, one mutation each, sent one
/// after another in the background so the governor spaces them.
pub(super) fn archive_all_read(state: &mut State) -> Update {
    let read: Vec<Notification> = shown(state)
        .into_iter()
        .filter(|notification| notification.read_at.is_some())
        .cloned()
        .collect();
    if read.is_empty() {
        return Update::default();
    }
    let now = state.now();
    let writes: Vec<InboxWrite> = read
        .iter()
        .map(|notification| {
            change_one(state, notification, InboxChange::Archive, |n| {
                n.archived_at = Some(now);
            })
        })
        .collect();
    state.notice = Some(format!("Archiving {} read notifications…", writes.len()));
    Update {
        redraw: true,
        effects: vec![Effect::ArchiveNotifications(writes)],
    }
}

/// The snooze presets at `now` in `time_zone`: in an hour, tomorrow at
/// 09:00 and next Monday at 09:00.
pub fn snooze_presets(now: Timestamp, time_zone: &TimeZone) -> Vec<(String, Timestamp)> {
    let today = now.to_zoned(time_zone.clone()).date();
    let at_nine = |date: jiff::civil::Date| {
        date.at(9, 0, 0, 0)
            .to_zoned(time_zone.clone())
            .ok()
            .map(|zoned| zoned.timestamp())
    };
    let mut presets = Vec::new();
    if let Ok(hour) = now.checked_add(SignedDuration::from_hours(1)) {
        presets.push(("In 1 hour".to_owned(), hour));
    }
    if let Some(tomorrow) = today.tomorrow().ok().and_then(at_nine) {
        presets.push(("Tomorrow 09:00".to_owned(), tomorrow));
    }
    if let Some(monday) = today.nth_weekday(1, Weekday::Monday).ok().and_then(at_nine) {
        presets.push(("Next Monday 09:00".to_owned(), monday));
    }
    presets
}

/// `z`: opens the picker of snooze presets for the focused notification.
pub(super) fn open_snooze(state: &mut State) -> Update {
    if focused(state).is_none() {
        return Update::default();
    }
    let entries = snooze_presets(state.now(), state.analytics_screen.time_zone())
        .into_iter()
        .map(|(label, until)| Entry {
            label,
            value: Some(until.to_string()),
        })
        .collect();
    state.modals.push(Modal::Choice(Choice {
        purpose: Purpose::Snooze,
        list: Picker::new(entries, None),
    }));
    Update::redraw()
}

/// Closes the snooze picker on its selected preset, or the `nth` visible
/// one, and snoozes the focused notification until then.
pub(super) fn snooze_chosen(state: &mut State, nth: Option<usize>) -> Update {
    let Some(Modal::Choice(choice)) = state.modals.top() else {
        return Update::default();
    };
    let Some(until) = choice
        .chosen(nth)
        .cloned()
        .flatten()
        .and_then(|until| until.parse::<Timestamp>().ok())
    else {
        return Update::default();
    };
    state.modals.pop();
    let Some(notification) = focused(state).cloned() else {
        return Update::redraw();
    };
    let write = change_one(state, &notification, InboxChange::Snooze(until), |n| {
        n.snoozed_until_at = Some(until);
    });
    send(state, vec![write])
}

/// Enter: opens the focused notification's issue in the results with its
/// detail shown and the comment it is about focused, and marks it read.
pub(super) fn open(state: &mut State) -> Update {
    let Some(notification) = focused(state).cloned() else {
        return Update::default();
    };
    let Some(identifier) = state
        .store
        .issue(&notification.issue_id)
        .map(|issue| issue.identifier.clone())
    else {
        state.notice = Some("The issue of this notification has not loaded".to_owned());
        return Update::redraw();
    };
    super::results::show(state, identifier, vec![notification.issue_id.clone()]);
    let rows = ui::list_rows(state, state.viewport);
    if state.results.pane() == Pane::List {
        state.results.perform(Action::Open, rows);
    }
    super::detail::reopen(state);
    state
        .detail
        .focus_comment(&notification.issue_id, notification.comment_id.clone());
    if notification.read_at.is_some() {
        return Update::redraw();
    }
    let now = state.now();
    let write = change_one(state, &notification, InboxChange::Read(now), |n| {
        n.read_at = Some(now);
    });
    send(state, vec![write])
}

/// Applies `apply` to `notification` in the store and describes the write.
fn change_one(
    state: &mut State,
    notification: &Notification,
    change: InboxChange,
    apply: impl FnOnce(&mut Notification),
) -> InboxWrite {
    state.store.change_notification(&notification.id, apply);
    write_for(state, notification, change, vec![notification.clone()])
}

fn write_for(
    state: &State,
    notification: &Notification,
    change: InboxChange,
    undo: Vec<Notification>,
) -> InboxWrite {
    let identifier = state
        .store
        .issue(&notification.issue_id)
        .map_or_else(String::new, |issue| issue.identifier.clone());
    InboxWrite {
        notification_id: notification.id.clone(),
        issue_id: notification.issue_id.clone(),
        team_id: notification.team_id.clone(),
        identifier,
        change,
        undo,
    }
}

fn send(state: &mut State, writes: Vec<InboxWrite>) -> Update {
    keep_focus_in_place(state);
    Update {
        redraw: true,
        effects: writes.into_iter().map(Effect::WriteInbox).collect(),
    }
}

/// When the focused notification left the list, the focus moves to the
/// one now at its place.
fn keep_focus_in_place(state: &mut State) {
    let shown = shown(state);
    let index = state.inbox.focused_index(&shown);
    let id = shown.get(index).map(|notification| notification.id.clone());
    state.inbox.focused_at = index;
    state.inbox.focused_id = id;
}

/// Takes in Linear's answer to `write`. A rejection puts back what the
/// change replaced, unless newer data has moved the notification on since.
pub(super) fn finished(
    state: &mut State,
    write: &InboxWrite,
    result: Result<(), Failure>,
) -> Update {
    if let Err(failure) = learn(state, write, result) {
        state.notice = Some(format!(
            "Could not {} a notification on {}: {failure}",
            write.change.verb(),
            write.identifier
        ));
        state.record(failure);
    }
    keep_focus_in_place(state);
    Update::redraw()
}

/// Takes in Linear's answers to archive all read, saying in one notice how
/// it went.
pub(super) fn batch_finished(
    state: &mut State,
    answers: Vec<(InboxWrite, Result<(), Failure>)>,
) -> Update {
    let total = answers.len();
    let mut first_failure = None;
    let mut failed = 0;
    for (write, result) in answers {
        if let Err(failure) = learn(state, &write, result) {
            failed += 1;
            first_failure.get_or_insert_with(|| failure.to_string());
            state.record(failure);
        }
    }
    state.notice = Some(match first_failure {
        None => format!("Archived {total} read notifications"),
        Some(failure) => format!("Could not archive {failed} of {total} notifications: {failure}"),
    });
    keep_focus_in_place(state);
    Update::redraw()
}

/// Learns what the API key may do from `result`, and rolls `write` back
/// when it failed.
fn learn(
    state: &mut State,
    write: &InboxWrite,
    result: Result<(), Failure>,
) -> Result<(), Failure> {
    let outcome = result.as_ref().map(|_| ()).map_err(|failure| &failure.kind);
    state.learn_write(&write.team_id, WriteClass::Inbox, outcome);
    if result.is_err() {
        for notification in &write.undo {
            state.store.restore_notification(notification.clone());
        }
    }
    result
}
