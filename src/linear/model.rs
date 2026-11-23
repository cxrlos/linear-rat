//! The Linear entities lrat works with, independent of how they were fetched.
//! The real client, the fake client, the app and the analytics calculator all
//! share these types.

use jiff::Timestamp;
use jiff::civil::Date;
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

/// Where a project stands, as Linear's `ProjectStatus`: the workspace's
/// name for it and the type it belongs to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectStatus {
    pub name: String,
    pub status_type: ProjectStatusType,
}

/// Every project status belongs to one of these, as Linear's
/// `ProjectStatusType`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProjectStatusType {
    Backlog,
    Planned,
    Started,
    Paused,
    Completed,
    Canceled,
}

/// A project as the Projects screen shows it: its status, target date,
/// lead and progress, and its milestones in Linear's order.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProjectOverview {
    pub project: Project,
    pub status: ProjectStatus,
    pub target_date: Option<Date>,
    pub lead: Option<Member>,
    /// From 0 to 1, as Linear's `Project.progress`.
    pub progress: f64,
    pub milestones: Vec<ProjectMilestone>,
    /// Whether `milestones` holds every milestone, not a first page.
    pub milestones_complete: bool,
}

/// A stage inside a project that issues can be grouped under.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectMilestone {
    pub id: String,
    pub name: String,
    pub project_id: String,
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
    #[serde(default)]
    pub project_milestone_id: Option<String>,
    /// The issue this one is a sub-issue of.
    #[serde(default)]
    pub parent_id: Option<String>,
    /// The day the issue is due, with no time of day or time zone.
    #[serde(default)]
    pub due_date: Option<Date>,
    /// When the issue was archived, or `None` while it is not.
    #[serde(default)]
    pub archived_at: Option<Timestamp>,
    /// Whether the viewer is subscribed to the issue's notifications.
    #[serde(default)]
    pub subscribed: bool,
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
    /// The comment this one replies to; `None` starts a thread.
    #[serde(default)]
    pub parent_id: Option<String>,
    /// When its thread was resolved. Linear keeps it on the comment that
    /// starts the thread.
    #[serde(default)]
    pub resolved_at: Option<Timestamp>,
    /// When its author last changed the body.
    #[serde(default)]
    pub edited_at: Option<Timestamp>,
    #[serde(default)]
    pub reactions: Vec<Reaction>,
}

/// One emoji someone left on an issue or a comment.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Reaction {
    pub id: String,
    /// As Linear stores it: the emoji itself, or a shortcode such as
    /// `+1` for a workspace's own emoji.
    pub emoji: String,
    pub user_id: Option<String>,
    pub created_at: Timestamp,
}

/// Something Linear put in the viewer's inbox about an issue, as Linear's
/// `IssueNotification`. Other kinds of notification are not kept.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Notification {
    pub id: String,
    /// What happened, as Linear's `type`: `issueComment`, `issueMention`,
    /// `issueAssignedToYou`, `issueStatusChanged`, `issueReaction` and so on.
    pub kind: String,
    pub issue_id: String,
    pub team_id: String,
    /// The comment it is about, when it is about one.
    pub comment_id: Option<String>,
    /// Who did it, when a person did.
    pub actor_id: Option<String>,
    /// The emoji, for a reaction.
    pub reaction_emoji: Option<String>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
    pub read_at: Option<Timestamp>,
    pub snoozed_until_at: Option<Timestamp>,
    pub archived_at: Option<Timestamp>,
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

/// A link between two issues, as Linear's `IssueRelation`: the issue with id
/// `issue_id` relates to the one with id `related_issue_id` as `kind` says,
/// for example it blocks it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IssueRelation {
    pub id: String,
    pub kind: RelationType,
    pub issue_id: String,
    pub related_issue_id: String,
}

/// Linear's `IssueRelationType`, read from the source issue.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RelationType {
    Blocks,
    /// The source issue is a duplicate of the related one.
    Duplicate,
    Related,
    /// Suggested by Linear as alike; lrat does not show it.
    Similar,
}

/// How a relation reads from one of its two issues.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum RelationKind {
    Blocks,
    BlockedBy,
    Related,
    DuplicateOf,
    DuplicatedBy,
}

impl RelationKind {
    /// In the order the detail lists them.
    pub const ALL: [RelationKind; 5] = [
        RelationKind::Blocks,
        RelationKind::BlockedBy,
        RelationKind::Related,
        RelationKind::DuplicateOf,
        RelationKind::DuplicatedBy,
    ];

    /// What the detail and the pickers call it.
    pub fn name(self) -> &'static str {
        match self {
            RelationKind::Blocks => "Blocks",
            RelationKind::BlockedBy => "Blocked by",
            RelationKind::Related => "Related",
            RelationKind::DuplicateOf => "Duplicate of",
            RelationKind::DuplicatedBy => "Duplicated by",
        }
    }
}

impl IssueRelation {
    /// How this relation reads from the issue with id `issue_id`, and the
    /// id of the issue at its other end. `None` when `issue_id` is at
    /// neither end, or for a relation lrat does not show.
    pub fn seen_from(&self, issue_id: &str) -> Option<(RelationKind, &str)> {
        let outgoing = if self.issue_id == issue_id {
            true
        } else if self.related_issue_id == issue_id {
            false
        } else {
            return None;
        };
        let other = if outgoing {
            &self.related_issue_id
        } else {
            &self.issue_id
        };
        let kind = match (self.kind, outgoing) {
            (RelationType::Blocks, true) => RelationKind::Blocks,
            (RelationType::Blocks, false) => RelationKind::BlockedBy,
            (RelationType::Related, _) => RelationKind::Related,
            (RelationType::Duplicate, true) => RelationKind::DuplicateOf,
            (RelationType::Duplicate, false) => RelationKind::DuplicatedBy,
            (RelationType::Similar, _) => return None,
        };
        Some((kind, other))
    }
}
