use crossterm::event::KeyEvent;
use jiff::Timestamp;

use super::{LoadError, NewComment, NewIssue, SyncFailure, Write, Writing};
use crate::linear::model::{Issue, Label, Member, Team, WorkflowState};
use crate::linear::{
    AnalyticsIssues, AnalyticsWindow, AssignedIssues, IssueDetail, PostedComment, SearchResults,
    TeamChanges, TeamIssuesPage, TeamOptions, TeamQuery,
};

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
    /// The result of [`Effect::Search`](super::Effect::Search) for `term`,
    /// or why it failed.
    SearchDone {
        term: String,
        result: Result<SearchResults, String>,
    },
    /// The result of [`Effect::LoadTeams`](super::Effect::LoadTeams). An
    /// error is a message with its context.
    TeamsLoaded(Result<Vec<Team>, String>),
    /// The result of
    /// [`Effect::LoadTeamOptions`](super::Effect::LoadTeamOptions).
    TeamOptionsLoaded(Result<TeamOptions, String>),
    /// The result of
    /// [`Effect::LoadTeamIssues`](super::Effect::LoadTeamIssues), with the
    /// request it answers.
    TeamIssuesLoaded {
        query: TeamQuery,
        after: Option<String>,
        result: Result<TeamIssuesPage, String>,
    },
    /// The result of
    /// [`Effect::CountTeamIssues`](super::Effect::CountTeamIssues), with the
    /// query it counted.
    TeamIssuesCounted {
        query: TeamQuery,
        result: Result<usize, String>,
    },
    /// The result of [`Effect::LoadAnalytics`](super::Effect::LoadAnalytics)
    /// for `window`; the error says why it failed, with its context.
    AnalyticsLoaded {
        window: AnalyticsWindow,
        result: Result<AnalyticsIssues, String>,
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
    /// The result of [`Effect::LoadTeamLabels`](super::Effect::LoadTeamLabels),
    /// or why it failed.
    TeamLabelsLoaded {
        team_id: String,
        result: Result<Vec<Label>, String>,
    },
    /// Linear's answer to [`Effect::UpdateIssue`](super::Effect::UpdateIssue):
    /// the issue as it stored it, or why it rejected the edit.
    IssueUpdated {
        write: Write,
        result: Result<Box<Issue>, String>,
    },
    /// The editor of [`Effect::OpenEditor`](super::Effect::OpenEditor)
    /// closed: what the viewer saved, or why it could not run.
    EditorClosed {
        writing: Writing,
        result: Result<String, String>,
    },
    /// Linear's answer to
    /// [`Effect::CreateComment`](super::Effect::CreateComment).
    CommentCreated {
        comment: NewComment,
        result: Result<PostedComment, String>,
    },
    /// Linear's answer to [`Effect::CreateIssue`](super::Effect::CreateIssue):
    /// the issue as it stored it, or why it was not created.
    IssueCreated {
        issue: NewIssue,
        result: Result<Box<Issue>, String>,
    },
}
