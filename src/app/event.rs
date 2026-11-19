use crossterm::event::KeyEvent;
use jiff::Timestamp;
use semver::Version;

use super::{
    Failure, LifecycleWrite, LoadError, NewComment, NewIssue, RelationWrite, SyncFailure, Write,
    Writing,
};
use crate::linear::model::{
    Cycle, Issue, IssueRelation, Label, Member, ProjectMilestone, Team, WorkflowState,
};
use crate::linear::{
    AnalyticsIssues, AnalyticsWindow, AssignedIssues, IssueDetail, LiveEvent, PostedComment,
    ProjectList, SearchQuery, SearchResults, TeamAnalyticsIssues, TeamChanges, TeamIssuesPage,
    TeamOptions, TeamQuery,
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
        result: Result<IssueDetail, Failure>,
    },
    /// The result of [`Effect::Search`](super::Effect::Search) for `query`,
    /// or why it failed.
    SearchDone {
        query: SearchQuery,
        result: Result<SearchResults, Failure>,
    },
    /// The pause asked for by
    /// [`Effect::SearchAfterPause`](super::Effect::SearchAfterPause) for this
    /// query has passed.
    TypingPaused(String),
    /// The result of [`Effect::LoadTeams`](super::Effect::LoadTeams). An
    /// error is a message with its context.
    TeamsLoaded(Result<Vec<Team>, Failure>),
    /// The result of
    /// [`Effect::LoadTeamOptions`](super::Effect::LoadTeamOptions).
    TeamOptionsLoaded(Result<TeamOptions, Failure>),
    /// The result of
    /// [`Effect::LoadTeamIssues`](super::Effect::LoadTeamIssues), with the
    /// request it answers.
    TeamIssuesLoaded {
        query: TeamQuery,
        after: Option<String>,
        result: Result<TeamIssuesPage, Failure>,
    },
    /// The result of
    /// [`Effect::CountTeamIssues`](super::Effect::CountTeamIssues), with the
    /// query it counted.
    TeamIssuesCounted {
        query: TeamQuery,
        result: Result<usize, Failure>,
    },
    /// The result of [`Effect::LoadAnalytics`](super::Effect::LoadAnalytics)
    /// for `window`; the error says why it failed, with its context.
    AnalyticsLoaded {
        window: AnalyticsWindow,
        result: Result<AnalyticsIssues, Failure>,
    },
    /// The result of
    /// [`Effect::LoadAdministrableTeams`](super::Effect::LoadAdministrableTeams):
    /// team ids, or why they did not load.
    AdministrableTeamsLoaded(Result<Vec<String>, Failure>),
    /// The result of
    /// [`Effect::LoadTeamAnalytics`](super::Effect::LoadTeamAnalytics) for
    /// `team_id` over `window`; the error says why it failed, with its
    /// context.
    TeamAnalyticsLoaded {
        team_id: String,
        window: AnalyticsWindow,
        result: Result<TeamAnalyticsIssues, Failure>,
    },
    /// The result of [`Effect::LoadCycles`](super::Effect::LoadCycles) for
    /// team `team_id`, or why it failed.
    CyclesLoaded {
        team_id: String,
        result: Result<Vec<Cycle>, Failure>,
    },
    /// The result of [`Effect::LoadProjects`](super::Effect::LoadProjects).
    ProjectsLoaded(Result<ProjectList, Failure>),
    /// The result of
    /// [`Effect::LoadCycleIssues`](super::Effect::LoadCycleIssues).
    CycleIssuesLoaded {
        cycle_id: String,
        result: Result<SearchResults, Failure>,
    },
    /// The result of
    /// [`Effect::LoadProjectIssues`](super::Effect::LoadProjectIssues).
    ProjectIssuesLoaded {
        project_id: String,
        result: Result<SearchResults, Failure>,
    },
    /// The runtime could not carry out an effect. The message, with its
    /// context, becomes the notice.
    EffectFailed(Failure),
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
    /// A delivery from the websocket of [`Effect::GoLive`](super::Effect::GoLive).
    Live(LiveEvent),
    /// The result of
    /// [`Effect::LoadWorkflowStates`](super::Effect::LoadWorkflowStates), or
    /// why it failed.
    WorkflowStatesLoaded {
        team_id: String,
        result: Result<Vec<WorkflowState>, Failure>,
    },
    /// The result of [`Effect::LoadTeamMembers`](super::Effect::LoadTeamMembers),
    /// or why it failed.
    TeamMembersLoaded {
        team_id: String,
        result: Result<Vec<Member>, Failure>,
    },
    /// The result of [`Effect::LoadTeamLabels`](super::Effect::LoadTeamLabels),
    /// or why it failed.
    TeamLabelsLoaded {
        team_id: String,
        result: Result<Vec<Label>, Failure>,
    },
    /// The result of
    /// [`Effect::LoadProjectMilestones`](super::Effect::LoadProjectMilestones),
    /// or why it failed.
    ProjectMilestonesLoaded {
        project_id: String,
        result: Result<Vec<ProjectMilestone>, Failure>,
    },

    /// Linear's answer to [`Effect::UpdateIssue`](super::Effect::UpdateIssue):
    /// the issue as it stored it, or why it rejected the edit.
    IssueUpdated {
        write: Write,
        result: Result<Box<Issue>, Failure>,
    },
    /// Linear's answer to
    /// [`Effect::ChangeLifecycle`](super::Effect::ChangeLifecycle): done, or
    /// why it was rejected.
    LifecycleChanged {
        write: LifecycleWrite,
        result: Result<(), Failure>,
    },
    /// Linear's answer to
    /// [`Effect::CreateRelation`](super::Effect::CreateRelation): the
    /// relation as it stored it, or why it was not created.
    RelationCreated {
        write: RelationWrite,
        result: Result<IssueRelation, Failure>,
    },
    /// Linear's answer to
    /// [`Effect::DeleteRelation`](super::Effect::DeleteRelation).
    RelationDeleted {
        write: RelationWrite,
        result: Result<(), Failure>,
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
        result: Result<PostedComment, Failure>,
    },
    /// Linear's answer to [`Effect::CreateIssue`](super::Effect::CreateIssue):
    /// the issue as it stored it, or why it was not created.
    IssueCreated {
        issue: NewIssue,
        result: Result<Box<Issue>, Failure>,
    },
    /// The result of [`Effect::CheckRelease`](super::Effect::CheckRelease):
    /// the latest release, or `None` when the check failed.
    ReleaseChecked(Option<Version>),
}
