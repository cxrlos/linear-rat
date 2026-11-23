//! Archiving, unarchiving and trashing an issue, and subscribing the viewer
//! to it or unsubscribing. Each shows at once, goes to Linear as one
//! mutation and is undone if Linear rejects it. Trash waits for the
//! viewer to confirm it in a [`Confirm`] modal; lrat never deletes an issue
//! for good.

use jiff::Timestamp;

use super::store::Trashed;
use super::{Effect, Failure, Modal, Screen, State, Update, WriteClass};

/// A change to whether an issue is archived, trashed or followed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lifecycle {
    Archive,
    Unarchive,
    /// Moves the issue and its sub-issues to Linear's trash for 30 days.
    Trash,
    Subscribe,
    Unsubscribe,
}

impl Lifecycle {
    /// How a message says it, as `Could not archive ENG-12`.
    /// The kind of write the change is, for what the API key may do.
    fn class(self) -> WriteClass {
        match self {
            Lifecycle::Archive | Lifecycle::Unarchive | Lifecycle::Trash => {
                WriteClass::ArchiveOrTrash
            }
            Lifecycle::Subscribe | Lifecycle::Unsubscribe => WriteClass::UpdateIssue,
        }
    }

    /// What the change does, as `archive` or `subscribe to`.
    pub fn verb(self) -> &'static str {
        match self {
            Lifecycle::Archive => "archive",
            Lifecycle::Unarchive => "unarchive",
            Lifecycle::Trash => "trash",
            Lifecycle::Subscribe => "subscribe to",
            Lifecycle::Unsubscribe => "unsubscribe from",
        }
    }
}

/// A lifecycle change on its way to Linear: the change already shown, and
/// what it replaced, to put back if Linear rejects it.
#[derive(Debug, Clone, PartialEq)]
pub struct LifecycleWrite {
    pub issue_id: String,
    pub identifier: String,
    /// The issue's team, where a refusal teaches what the API key may do.
    pub team_id: String,
    pub change: Lifecycle,
    undo: Undo,
}

#[derive(Debug, Clone, PartialEq, Default)]
struct Undo {
    /// When the issue was archived before the change.
    archived_at: Option<Timestamp>,
    /// The trashed issue with what the store kept about it.
    trashed: Option<Box<Trashed>>,
    /// Where Team Issues listed the issue before it left the list.
    team_position: Option<usize>,
    /// Where the results listed the issue before it left them.
    results_position: Option<usize>,
}

/// A question the viewer answers before something hard to undo runs: a
/// title, a body, and what confirming runs.
#[derive(Debug, Clone, PartialEq)]
pub struct Confirm {
    pub title: String,
    /// What the action applies to, on one line, cut with `…` when it is
    /// too long, so the body below always fits.
    pub subject: String,
    /// What to know before confirming, wrapped to the popup.
    pub body: Vec<String>,
    /// What the confirming key's word says, as `trash`.
    pub verb: &'static str,
    pub on_confirm: OnConfirm,
}

/// What confirming a [`Confirm`] runs. It names its issue, so a list that
/// moved under the modal cannot change what it runs on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OnConfirm {
    Trash {
        issue_id: String,
    },
    /// Delete the viewer's comment `comment_id` on the issue `issue_id`.
    DeleteComment {
        issue_id: String,
        comment_id: String,
    },
}

/// Whether the focused issue is archived, so it can be unarchived.
pub fn focused_is_archived(state: &State) -> bool {
    state
        .focused_issue()
        .is_some_and(|issue| issue.archived_at.is_some())
}

/// Archives the focused issue, unless it already is.
pub(super) fn archive(state: &mut State) -> Update {
    let Some(issue) = saved_focus(state) else {
        return Update::redraw();
    };
    if issue.archived_at.is_some() {
        return Update::default();
    }
    let now = state.now();
    let issue_id = issue.id.clone();
    state
        .store
        .change_issue(&issue_id, |issue| issue.archived_at = Some(now));
    let undo = Undo {
        team_position: state.team_issues.forget_at(&issue_id),
        ..Undo::default()
    };
    send(state, &issue_id, Lifecycle::Archive, undo)
}

/// Unarchives the focused issue, if it is archived.
pub(super) fn unarchive(state: &mut State) -> Update {
    let Some(issue) = saved_focus(state) else {
        return Update::redraw();
    };
    let Some(archived_at) = issue.archived_at else {
        return Update::default();
    };
    let issue_id = issue.id.clone();
    state
        .store
        .change_issue(&issue_id, |issue| issue.archived_at = None);
    let undo = Undo {
        archived_at: Some(archived_at),
        ..Undo::default()
    };
    send(state, &issue_id, Lifecycle::Unarchive, undo)
}

/// Subscribes the viewer to the focused issue, or unsubscribes them.
pub(super) fn toggle_subscription(state: &mut State) -> Update {
    let Some(issue) = saved_focus(state) else {
        return Update::redraw();
    };
    let issue_id = issue.id.clone();
    let subscribe = !issue.subscribed;
    state
        .store
        .change_issue(&issue_id, |issue| issue.subscribed = subscribe);
    let change = if subscribe {
        Lifecycle::Subscribe
    } else {
        Lifecycle::Unsubscribe
    };
    send(state, &issue_id, change, Undo::default())
}

/// Asks the viewer to confirm trashing the focused issue.
pub(super) fn ask_to_trash(state: &mut State) -> Update {
    let Some(issue) = saved_focus(state) else {
        return Update::redraw();
    };
    let confirm = Confirm {
        title: format!("Trash {}?", issue.identifier),
        subject: format!("{} {}", issue.identifier, issue.title),
        body: vec![
            "Its sub-issues go to the trash too.".to_owned(),
            "It can be restored from Linear for 30 days.".to_owned(),
        ],
        verb: "trash",
        on_confirm: OnConfirm::Trash {
            issue_id: issue.id.clone(),
        },
    };
    state.modals.push(Modal::Confirm(confirm));
    Update::redraw()
}

/// Closes the confirmation on top and runs what it asked about.
pub(super) fn confirm(state: &mut State) -> Update {
    let Some(Modal::Confirm(confirm)) = state.modals.top() else {
        return Update::default();
    };
    let on_confirm = confirm.on_confirm.clone();
    state.modals.pop();
    match on_confirm {
        OnConfirm::Trash { issue_id } => trash(state, &issue_id),
        OnConfirm::DeleteComment {
            issue_id,
            comment_id,
        } => super::comments::delete(state, &issue_id, &comment_id),
    }
}

/// Closes the confirmation on top without running anything.
pub(super) fn decline(state: &mut State) -> Update {
    match state.modals.top() {
        Some(Modal::Confirm(_)) => {
            state.modals.pop();
            Update::redraw()
        }
        _ => Update::default(),
    }
}

/// Takes the issue out of the store and every list at once and asks Linear
/// to trash it.
fn trash(state: &mut State, issue_id: &str) -> Update {
    let Some(trashed) = state.store.take_issue(issue_id) else {
        return Update::redraw();
    };
    let undo = Undo {
        trashed: Some(Box::new(trashed)),
        team_position: state.team_issues.forget_at(issue_id),
        results_position: state.results.forget_at(issue_id),
        ..Undo::default()
    };
    send(state, issue_id, Lifecycle::Trash, undo)
}

/// The focused issue when Linear has confirmed it, else `None` with a
/// notice saying why it cannot change yet.
fn saved_focus(state: &mut State) -> Option<crate::linear::model::Issue> {
    let issue = state.focused_issue()?.clone();
    if super::is_local(&issue.id) {
        state.notice = Some(format!(
            "{} is not created yet; change it once Linear confirms it",
            issue.identifier
        ));
        return None;
    }
    Some(issue)
}

fn send(state: &mut State, issue_id: &str, change: Lifecycle, undo: Undo) -> Update {
    let (identifier, team_id) = undo
        .trashed
        .as_ref()
        .map(|trashed| &trashed.issue)
        .or_else(|| state.store.issue(issue_id))
        .map(|issue| (issue.identifier.clone(), issue.team_id.clone()))
        .unwrap_or_default();
    state.row_errors.remove(issue_id);
    state.rebuild_lists();
    Update {
        redraw: true,
        effects: vec![Effect::ChangeLifecycle(LifecycleWrite {
            issue_id: issue_id.to_owned(),
            identifier,
            team_id,
            change,
            undo,
        })],
    }
}

/// Takes in Linear's answer to `write`. A rejection puts back what the
/// change replaced, unless newer data has moved the issue on since, and
/// shows the error on the row and in the notice.
pub(super) fn finished(
    state: &mut State,
    write: &LifecycleWrite,
    result: Result<(), Failure>,
) -> Update {
    let outcome = result.as_ref().map(|_| ()).map_err(|failure| &failure.kind);
    state.learn_write(&write.team_id, write.change.class(), outcome);
    match result {
        Ok(()) => {
            state.notice = match write.change {
                Lifecycle::Archive => Some(format!("Archived {}", write.identifier)),
                Lifecycle::Unarchive => Some(format!("Unarchived {}", write.identifier)),
                Lifecycle::Trash => Some(format!(
                    "Trashed {}; it can be restored from Linear for 30 days",
                    write.identifier
                )),
                Lifecycle::Subscribe | Lifecycle::Unsubscribe => None,
            };
        }
        Err(failure) => {
            roll_back(state, write);
            state.notice = Some(format!(
                "Could not {} {}: {failure}",
                write.change.verb(),
                write.identifier
            ));
            state
                .row_errors
                .insert(write.issue_id.clone(), failure.message.clone());
            state.record(failure);
        }
    }
    state.rebuild_lists();
    Update::redraw()
}

fn roll_back(state: &mut State, write: &LifecycleWrite) {
    let id = write.issue_id.as_str();
    let undo = &write.undo;
    match write.change {
        Lifecycle::Archive => {
            state.store.change_issue(id, |issue| {
                if issue.archived_at.is_some() {
                    issue.archived_at = undo.archived_at;
                }
            });
        }
        Lifecycle::Unarchive => {
            state.store.change_issue(id, |issue| {
                if issue.archived_at.is_none() {
                    issue.archived_at = undo.archived_at;
                }
            });
        }
        Lifecycle::Subscribe | Lifecycle::Unsubscribe => {
            let subscribed = write.change == Lifecycle::Subscribe;
            state.store.change_issue(id, |issue| {
                if issue.subscribed == subscribed {
                    issue.subscribed = !subscribed;
                }
            });
        }
        Lifecycle::Trash => {
            if let Some(trashed) = &undo.trashed {
                state.store.put_back((**trashed).clone());
            }
            if let Some(position) = undo.results_position {
                state.results.put_back(id, position);
            }
        }
    }
    if let Some(position) = undo.team_position {
        state.team_issues.put_back(id, position);
    }
    if state.screen == Screen::Results {
        let rows = crate::ui::list_rows(state, state.viewport);
        state.results.scroll_into_view(rows);
    }
}
