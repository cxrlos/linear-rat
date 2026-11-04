use crossterm::event::KeyEvent;
use jiff::Timestamp;

use super::{LoadError, SyncFailure, Write};
use crate::linear::model::{Issue, Member, WorkflowState};
use crate::linear::{AssignedIssues, IssueDetail, TeamChanges};

/// Something that happened, fed to [`update`](super::update).
#[derive(Debug, Clone, PartialEq)]
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
    /// The result of
    /// [`Effect::LoadIssueDetail`](super::Effect::LoadIssueDetail): the
    /// detail, or why it did not load, with its context.
    IssueDetailLoaded {
        issue_id: String,
        result: Result<IssueDetail, String>,
    },
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
    /// The result of
    /// [`Effect::LoadWorkflowStates`](super::Effect::LoadWorkflowStates), or
    /// why it failed.
    WorkflowStatesLoaded {
        team_id: String,
        result: Result<Vec<WorkflowState>, String>,
    },
    /// The result of [`Effect::LoadTeamMembers`](super::Effect::LoadTeamMembers),
    /// or why it failed.
    TeamMembersLoaded {
        team_id: String,
        result: Result<Vec<Member>, String>,
    },
    /// Linear's answer to [`Effect::UpdateIssue`](super::Effect::UpdateIssue):
    /// the issue as it stored it, or why it rejected the edit.
    IssueUpdated {
        write: Write,
        result: Result<Box<Issue>, String>,
    },
}
