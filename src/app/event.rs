use crossterm::event::KeyEvent;
use jiff::Timestamp;

use super::{LoadError, SyncFailure};
use crate::linear::{AssignedIssues, TeamChanges};

/// Something that happened, fed to [`update`](super::update).
#[derive(Debug, Clone, PartialEq)]
#[expect(
    clippy::large_enum_variant,
    reason = "a load comes once per session, so boxing it saves nothing"
)]
pub enum Event {
    /// The runtime is running; sent once, before any input.
    Start,
    Key(KeyEvent),
    Resize {
        columns: u16,
        rows: u16,
    },
    /// The result of [`Effect::LoadMyIssues`](super::Effect::LoadMyIssues).
    MyIssuesLoaded(Result<AssignedIssues, LoadError>),
    /// The runtime could not carry out an effect. The message, with its
    /// context, becomes the notice.
    EffectFailed(String),
    /// The time now, about once a second. Polls fall due and the sync
    /// indicator ages on ticks.
    Tick(Timestamp),
    /// The terminal reported that it gained focus.
    FocusGained,
    /// The terminal reported that it lost focus.
    FocusLost,
    /// The result of [`Effect::Poll`](super::Effect::Poll): the teams that
    /// answered, in order, and why the rest did not.
    Polled {
        changes: Vec<TeamChanges>,
        failure: Option<SyncFailure>,
    },
    /// The result of [`Effect::Reconcile`](super::Effect::Reconcile): the ids
    /// it checked and those of them that still exist.
    Reconciled {
        checked: Vec<String>,
        result: Result<Vec<String>, SyncFailure>,
    },
    /// A poll is starting while Linear's rate limit holds requests back.
    Throttled,
}
