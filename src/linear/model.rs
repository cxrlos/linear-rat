//! The Linear entities lrat works with, independent of how they were fetched.
//! The real client, the fake client, the app and the analytics calculator all
//! share these types.

use jiff::Timestamp;
use serde::{Deserialize, Serialize};

/// The person running lrat, identified by the API key in use.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Viewer {
    pub id: String,
    pub name: String,
}

/// The single Linear organization the API key belongs to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Workspace {
    pub id: String,
    pub name: String,
    /// The slug in `linear.app/<url_key>`, also the keyring account name.
    pub url_key: String,
}

/// Anyone in the workspace who can be an assignee, creator or comment author.
/// Linear calls this a `User`; the viewer is one of them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Member {
    pub id: String,
    pub name: String,
    pub display_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Team {
    pub id: String,
    /// The identifier prefix, for example `ENG`.
    pub key: String,
    pub name: String,
    pub estimation: Estimation,
}

/// A team's estimation scale. With [`Estimation::NotUsed`], estimates are
/// hidden and analytics count issues instead of points.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Estimation {
    NotUsed,
    Exponential,
    Fibonacci,
    Linear,
    TShirt,
}

/// A team's named status for an issue.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkflowState {
    pub id: String,
    pub team_id: String,
    pub name: String,
    pub state_type: StateType,
    /// Order within the team's states of the same type.
    pub position: f64,
}

/// Every workflow state belongs to exactly one of these. Declared in the order
/// work moves through them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum StateType {
    Triage,
    Backlog,
    Unstarted,
    Started,
    Completed,
    Canceled,
}

impl StateType {
    /// Completed and canceled issues are closed; the rest are open.
    pub fn is_open(self) -> bool {
        !matches!(self, StateType::Completed | StateType::Canceled)
    }
}

/// Linear's priority, which it stores as 0 (none) to 4 (low).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Priority {
    Urgent,
    High,
    Medium,
    Low,
    None,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Label {
    pub id: String,
    /// `None` for a workspace label.
    pub team_id: Option<String>,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Project {
    pub id: String,
    pub name: String,
}

/// A team's fixed time box of committed issues.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Cycle {
    pub id: String,
    pub team_id: String,
    pub number: u32,
    pub name: Option<String>,
    pub starts_at: Timestamp,
    pub ends_at: Timestamp,
    pub completed_at: Option<Timestamp>,
    /// One entry per day since the cycle started: total estimate points in
    /// scope, as Linear's `scopeHistory`.
    pub scope_history: Vec<f64>,
    /// One entry per day: estimate points completed, as `completedScopeHistory`.
    pub completed_scope_history: Vec<f64>,
    /// One entry per day: issues in scope, as `issueCountHistory`.
    pub issue_count_history: Vec<u32>,
    /// One entry per day: issues completed, as `completedIssueCountHistory`.
    pub completed_issue_count_history: Vec<u32>,
}

/// A unit of work in a team, identified by its key (for example `ENG-123`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Issue {
    pub id: String,
    /// The key, for example `ENG-123`.
    pub identifier: String,
    pub title: String,
    pub description: Option<String>,
    pub url: String,
    /// The Git branch name Linear suggests for the issue, as `branchName`.
    pub branch_name: String,
    pub team_id: String,
    pub state: WorkflowState,
    pub priority: Priority,
    pub estimate: Option<f64>,
    pub assignee_id: Option<String>,
    pub creator_id: Option<String>,
    pub cycle_id: Option<String>,
    pub project_id: Option<String>,
    pub label_ids: Vec<String>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
    pub started_at: Option<Timestamp>,
    pub completed_at: Option<Timestamp>,
    pub canceled_at: Option<Timestamp>,
    /// When the issue left triage, if it was ever in triage.
    pub triaged_at: Option<Timestamp>,
    /// Every workflow state the issue has been in, oldest first. Loaded only
    /// where needed (in-progress issues for analytics, the detail view).
    pub state_history: Vec<StateSpan>,
}

/// A stretch of time an issue spent in one workflow state, as Linear's
/// `IssueStateSpan`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StateSpan {
    pub state_id: String,
    pub state_type: StateType,
    pub started_at: Timestamp,
    /// `None` while the issue is still in this state.
    pub ended_at: Option<Timestamp>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Comment {
    pub id: String,
    pub issue_id: String,
    pub author_id: Option<String>,
    pub body: String,
    pub created_at: Timestamp,
}

/// A link Linear keeps on an issue, such as a GitHub pull request, a Slack
/// thread or a Figma file, as Linear's `Attachment`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Attachment {
    pub id: String,
    pub issue_id: String,
    /// The linked resource. Unique among one issue's attachments.
    pub url: String,
    pub title: String,
    pub subtitle: Option<String>,
    /// The integration that made it, as Linear's `sourceType`: `github`,
    /// `slack`, `figma` and so on, or `None` when Linear does not say.
    pub source_type: Option<String>,
    /// A pull request's state, such as `open` or `merged`, when the
    /// attachment's metadata carries one as `status`.
    pub status: Option<String>,
}

impl Attachment {
    /// Whether it links a GitHub pull request.
    pub fn is_pull_request(&self) -> bool {
        self.source_type
            .as_deref()
            .is_some_and(|source| source.eq_ignore_ascii_case("github"))
            && self.url.contains("/pull/")
    }
}
