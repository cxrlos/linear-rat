//! Everything lrat needs from Linear goes through [`LinearClient`], so tests
//! and `--demo` swap the real client for [`FakeClient`].

pub mod demo;
mod error;
mod governor;
pub mod http;
pub mod model;
pub mod paging;
pub(crate) mod search_bucket;

use std::cell::RefCell;
use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet};
use std::future::Future;
use std::path::PathBuf;

use futures_util::Stream;
use jiff::civil::Date;
use jiff::{SignedDuration, Timestamp};

pub use error::{ClientError, ErrorKind, GraphQlError, Response};
pub use governor::{SystemTimer, Timer, Urgency};
pub use http::{ClientConfig, Dial, HttpClient, LIVE_ENDPOINT, LiveConfig, Pipe};
use model::Reaction;
use model::{
    Attachment, Comment, Cycle, Issue, IssueRelation, Label, Member, Notification, Priority,
    Project, ProjectMilestone, ProjectOverview, RelationType, StateSpan, StateType, Team, Viewer,
    WorkflowState, Workspace,
};

/// Queries, mutations and live events.
pub trait LinearClient {
    /// The viewer and the workspace the API key belongs to. Login and
    /// `auth status` use it to check a key.
    fn viewer(&self) -> impl Future<Output = Result<(Viewer, Workspace), ClientError>> + Send;

    /// Every open issue assigned to the viewer, with what My Issues shows
    /// about them.
    fn assigned_issues(&self) -> impl Future<Output = Result<AssignedIssues, ClientError>> + Send;

    /// The issues of team `team_id` updated after `since`, archived and
    /// trashed ones included, with what they refer to. With `since` `None`,
    /// only the team's most recently updated issue, whose `updatedAt` starts
    /// the team's watermark. A background request.
    fn changed_issues(
        &self,
        team_id: &str,
        since: Option<Timestamp>,
    ) -> impl Future<Output = Result<TeamChanges, ClientError>> + Send;

    /// Which of `ids` are still issues the viewer can see and not in the
    /// trash. A background request.
    fn existing_issues(
        &self,
        ids: &[String],
    ) -> impl Future<Output = Result<Vec<String>, ClientError>> + Send;

    /// Whether background requests are being slowed or held back for
    /// Linear's rate limit right now.
    fn throttled(&self) -> bool {
        false
    }

    /// Every workflow state of the team with id `team_id`.
    fn workflow_states(
        &self,
        team_id: &str,
    ) -> impl Future<Output = Result<Vec<WorkflowState>, ClientError>> + Send;

    /// Every active member of the team with id `team_id`.
    fn team_members(
        &self,
        team_id: &str,
    ) -> impl Future<Output = Result<Vec<Member>, ClientError>> + Send;

    /// Every label an issue of the team with id `team_id` can carry: the
    /// team's own and the workspace's, without label groups.
    fn team_labels(
        &self,
        team_id: &str,
    ) -> impl Future<Output = Result<Vec<Label>, ClientError>> + Send;

    /// Every milestone of the project with id `project_id`, in Linear's
    /// order.
    fn project_milestones(
        &self,
        project_id: &str,
    ) -> impl Future<Output = Result<Vec<ProjectMilestone>, ClientError>> + Send;

    /// Archives the issue with id `issue_id`.
    fn archive_issue(&self, issue_id: &str)
    -> impl Future<Output = Result<(), ClientError>> + Send;

    /// Brings the archived issue with id `issue_id` back.
    fn unarchive_issue(
        &self,
        issue_id: &str,
    ) -> impl Future<Output = Result<(), ClientError>> + Send;

    /// The viewer's issue notifications. With `since` `None`, the newest 50
    /// unarchived ones, as the viewer waits on them; otherwise every one
    /// updated after `since`, archived ones included, in the background.
    /// A client without an inbox, as the test doubles are, has none.
    fn notifications(
        &self,
        since: Option<Timestamp>,
    ) -> impl Future<Output = Result<NotificationsLoaded, ClientError>> + Send {
        let _ = since;
        async { Ok(NotificationsLoaded::default()) }
    }

    /// Sends one `notificationUpdate` marking the notification with id `id`
    /// read or unread, or snoozing it.
    fn update_notification(
        &self,
        id: &str,
        update: &NotificationUpdate,
    ) -> impl Future<Output = Result<(), ClientError>> + Send {
        let _ = (id, update);
        async { Err(no_inbox()) }
    }

    /// Sends one `notificationArchive` for the notification with id `id`.
    /// Archive all read sends them in the background, so the governor
    /// spaces them.
    fn archive_notification(
        &self,
        id: &str,
        urgency: Urgency,
    ) -> impl Future<Output = Result<(), ClientError>> + Send {
        let _ = (id, urgency);
        async { Err(no_inbox()) }
    }

    /// Sends one `notificationMarkReadAll` marking every notification about
    /// the issue with id `issue_id` read at `read_at`.
    fn mark_issue_notifications_read(
        &self,
        issue_id: &str,
        read_at: Timestamp,
    ) -> impl Future<Output = Result<(), ClientError>> + Send {
        let _ = (issue_id, read_at);
        async { Err(no_inbox()) }
    }

    /// Moves the issue with id `issue_id` and its sub-issues to the trash,
    /// where Linear keeps them for 30 days. There is no permanent delete.
    fn trash_issue(&self, issue_id: &str) -> impl Future<Output = Result<(), ClientError>> + Send;

    /// Subscribes the viewer to the notifications of the issue with id
    /// `issue_id`.
    fn subscribe_to_issue(
        &self,
        issue_id: &str,
    ) -> impl Future<Output = Result<(), ClientError>> + Send;

    /// Unsubscribes the viewer from the notifications of the issue with id
    /// `issue_id`.
    fn unsubscribe_from_issue(
        &self,
        issue_id: &str,
    ) -> impl Future<Output = Result<(), ClientError>> + Send;

    /// Sends one `issueUpdate` for the issue with id `issue_id` and returns
    /// the issue as Linear stored it.
    fn update_issue(
        &self,
        issue_id: &str,
        update: &IssueUpdate,
    ) -> impl Future<Output = Result<Issue, ClientError>> + Send;

    /// What the detail of the issue with id `issue_id` shows beyond what a
    /// list loads: its comments, its workflow state history, its parent,
    /// sub-issues and relations. `urgency` says whether the viewer opened
    /// the detail and waits on it.
    fn issue_detail(
        &self,
        issue_id: &str,
        urgency: Urgency,
    ) -> impl Future<Output = Result<IssueDetail, ClientError>> + Send;

    /// The issues that match `query`'s term, best match first, closed ones
    /// included: anywhere in the workspace, or only those of its team and
    /// filter. Search has its own budget of 30 a minute; a search beyond it
    /// waits its turn rather than failing.
    fn search_issues(
        &self,
        query: &SearchQuery,
    ) -> impl Future<Output = Result<SearchResults, ClientError>> + Send;

    /// The teams the viewer is a member of, in the order Linear lists them.
    fn teams(&self) -> impl Future<Output = Result<Vec<Team>, ClientError>> + Send;

    /// What the Team Issues filters offer for `team_id`: its workflow
    /// states, members, labels (its own and the workspace's), recent cycles
    /// and projects.
    fn team_options(
        &self,
        team_id: &str,
    ) -> impl Future<Output = Result<TeamOptions, ClientError>> + Send;

    /// One page of the issues of `query`, newest first by its order,
    /// starting after the cursor `after` (or at the top when `None`).
    fn team_issues(
        &self,
        query: &TeamQuery,
        after: Option<&str>,
    ) -> impl Future<Output = Result<TeamIssuesPage, ClientError>> + Send;

    /// How many issues match `query`, counted over every page.
    fn count_team_issues(
        &self,
        query: &TeamQuery,
    ) -> impl Future<Output = Result<usize, ClientError>> + Send;

    /// The cycles of team `team_id` that the Cycles screen lists: those
    /// that ended in the last twelve weeks, the active one and the next
    /// one, with their daily issue counts.
    fn recent_cycles(
        &self,
        team_id: &str,
    ) -> impl Future<Output = Result<Vec<Cycle>, ClientError>> + Send;

    /// The projects of the viewer's teams, most recently updated first,
    /// with their status, target date, lead, progress and milestones.
    fn viewer_projects(&self) -> impl Future<Output = Result<ProjectList, ClientError>> + Send;

    /// The issues of the cycle with id `cycle_id`, closed ones included, up
    /// to 200.
    fn cycle_issues(
        &self,
        cycle_id: &str,
    ) -> impl Future<Output = Result<SearchResults, ClientError>> + Send;

    /// The issues of the project with id `project_id`, closed ones
    /// included, up to 200, each naming its project milestone so the list
    /// can be grouped by milestone.
    fn project_issues(
        &self,
        project_id: &str,
    ) -> impl Future<Output = Result<SearchResults, ClientError>> + Send;

    /// What personal analytics needs for `window`: the issues assigned to
    /// the viewer that were created or completed in it, the viewer's teams
    /// and their cycles that overlap it. A window that reaches now also
    /// brings every in-progress issue with its state history.
    fn analytics_issues(
        &self,
        window: AnalyticsWindow,
    ) -> impl Future<Output = Result<AnalyticsIssues, ClientError>> + Send;

    /// Sends one `issueRelationCreate` and returns the relation as Linear
    /// stored it.
    fn create_relation(
        &self,
        input: &RelationCreate,
    ) -> impl Future<Output = Result<IssueRelation, ClientError>> + Send;

    /// Sends one `issueRelationDelete` for the relation with id `id`.
    fn delete_relation(&self, id: &str) -> impl Future<Output = Result<(), ClientError>> + Send;

    /// Sends one `commentCreate` adding `body` to the issue with id
    /// `issue_id`, as a reply to the comment `parent_id` when given, and
    /// returns the comment as Linear stored it.
    fn create_comment(
        &self,
        issue_id: &str,
        body: &str,
        parent_id: Option<&str>,
    ) -> impl Future<Output = Result<PostedComment, ClientError>> + Send;

    /// Sends one `commentUpdate` replacing the text of the comment `id`.
    fn update_comment(
        &self,
        id: &str,
        body: &str,
    ) -> impl Future<Output = Result<(), ClientError>> + Send;

    /// Sends one `commentDelete` for the comment `id`, which deletes its
    /// replies too.
    fn delete_comment(&self, id: &str) -> impl Future<Output = Result<(), ClientError>> + Send;

    /// Sends one `commentResolve`, or `commentUnresolve` when `resolved` is
    /// false, for the thread the comment `id` starts.
    fn resolve_comment(
        &self,
        id: &str,
        resolved: bool,
    ) -> impl Future<Output = Result<(), ClientError>> + Send;

    /// Sends one `reactionCreate` adding the viewer's `emoji` to `target`,
    /// and returns the reaction as Linear stored it.
    fn add_reaction(
        &self,
        target: &ReactionTarget,
        emoji: &str,
    ) -> impl Future<Output = Result<Reaction, ClientError>> + Send;

    /// Sends one `reactionDelete` for the reaction `id`.
    fn remove_reaction(&self, id: &str) -> impl Future<Output = Result<(), ClientError>> + Send;

    /// Sends one `issueCreate` and returns the issue as Linear stored it.
    fn create_issue(
        &self,
        input: &IssueCreate,
    ) -> impl Future<Output = Result<Issue, ClientError>> + Send;

    /// The ids of the teams the viewer can administer, from
    /// `Query.administrableTeams`: the teams whose per-person breakdown lrat
    /// shows (ADR 0001).
    fn administrable_teams(&self) -> impl Future<Output = Result<Vec<String>, ClientError>> + Send;

    /// What team analytics needs for team `team_id` over `window`: its issues
    /// created, completed or canceled in it. A window that reaches now also
    /// brings every open issue of the team, the in-progress ones with their
    /// state history, and the team's last six finished cycles and its active
    /// one with their history arrays.
    fn team_analytics(
        &self,
        team_id: &str,
        window: AnalyticsWindow,
    ) -> impl Future<Output = Result<TeamAnalyticsIssues, ClientError>> + Send;

    /// When the issue with id `issue_id` was last updated in Linear, read
    /// afresh, so a save can refuse to overwrite a newer edit.
    fn issue_updated_at(
        &self,
        issue_id: &str,
    ) -> impl Future<Output = Result<Timestamp, ClientError>> + Send;

    /// Live events from Linear's websocket for the issues of `team_ids` and
    /// every new comment, for as long as the stream is polled. It reconnects
    /// by itself after any disconnect and never ends. Without a live
    /// connection, as in the fake client, it never yields.
    fn live_events(&self, team_ids: Vec<String>) -> impl Stream<Item = LiveEvent> {
        let _ = team_ids;
        futures_util::stream::pending()
    }

    /// The issue whose key is `team_key`-`number`, archived ones included,
    /// matching the team key whatever its case, or `None` when the workspace
    /// has no such issue. `lrat open` uses it.
    fn issue_by_key(
        &self,
        team_key: &str,
        number: u32,
    ) -> impl Future<Output = Result<Option<IssueLink>, ClientError>> + Send;

    /// What `lrat doctor` reports about the key beyond its viewer: the
    /// viewer's role, Linear's `rateLimitStatus` and Linear's clock.
    fn diagnostics(&self) -> impl Future<Output = Result<Diagnostics, ClientError>> + Send;

    /// Uploads `file` to the URL Linear's `fileUpload` signs for it, with
    /// only the headers Linear returns, then attaches the uploaded file to
    /// the issue with id `issue_id`, titled with its name. `progress` hears
    /// how many bytes have gone out so far, as they go.
    fn attach_file(
        &self,
        issue_id: &str,
        file: &FileUpload,
        progress: impl FnMut(u64) + Send + Sync + 'static,
    ) -> impl Future<Output = Result<Attachment, ClientError>> + Send;

    /// Attaches `url` to the issue with id `issue_id`, titled `title`, or as
    /// Linear names it when `None`.
    fn attach_link(
        &self,
        issue_id: &str,
        url: &str,
        title: Option<&str>,
    ) -> impl Future<Output = Result<Attachment, ClientError>> + Send;
}

/// A file the viewer chose to attach, as lrat found it before uploading.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileUpload {
    pub path: PathBuf,
    /// The file's name, which titles the attachment.
    pub name: String,
    /// The MIME type guessed from the extension.
    pub content_type: String,
    pub size: u64,
}

/// The viewer's role, the rate limits and Linear's clock, as `lrat doctor`
/// reports them.
#[derive(Debug, Clone, PartialEq)]
pub struct Diagnostics {
    pub role: Role,
    pub rate_limits: Vec<RateLimit>,
    /// From the response's `Date` header, to the second; `None` when Linear
    /// sent none that could be read.
    pub server_time: Option<Timestamp>,
}

/// The viewer's workspace role, as Linear's `User.admin`, `owner` and
/// `guest`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Role {
    pub admin: bool,
    pub owner: bool,
    pub guest: bool,
}

/// One of Linear's rate limits, as `RateLimitResultPayload`.
#[derive(Debug, Clone, PartialEq)]
pub struct RateLimit {
    /// What it limits, as Linear names it, such as request count or query
    /// complexity.
    pub kind: String,
    pub allowed: f64,
    pub remaining: f64,
    /// When the limit is fully replenished.
    pub reset: Option<Timestamp>,
}

/// Enough of an issue to name it and open it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssueLink {
    /// The key, for example `ENG-123`.
    pub identifier: String,
    pub title: String,
    pub url: String,
}

/// Something Linear's websocket delivered.
#[derive(Debug, Clone, PartialEq)]
pub enum LiveEvent {
    /// A connection was acknowledged, after the first connect or a
    /// reconnect. Anything that changed while there was none is missed, so
    /// a delta poll catches up.
    Connected,
    /// An issue was created or updated. `newest` is `None`: a live event
    /// never moves a team's watermark, which only a poll may move.
    Issue(TeamChanges),
    /// A comment was added to an issue.
    Comment(PostedComment),
    /// An issue was archived. Like [`LiveEvent::Issue`], it moves no
    /// watermark.
    Archived(TeamChanges),
    /// An issue was brought back from the archive.
    Unarchived(TeamChanges),
    /// A comment on an issue was edited, resolved or reacted to.
    CommentUpdated(PostedComment),
    /// A comment on an issue was deleted.
    CommentDeleted {
        issue_id: String,
        comment_id: String,
    },
    /// An issue notification was created, read, snoozed or archived, for
    /// the person with id `recipient_id`, who may not be the viewer.
    Notification {
        recipient_id: String,
        loaded: NotificationsLoaded,
    },
}

/// Issue notifications Linear sent, with the issues they are about and the
/// people and teams those refer to.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct NotificationsLoaded {
    pub notifications: Vec<Notification>,
    pub issues: Vec<Issue>,
    pub cycles: Vec<Cycle>,
    /// Who did what the notifications are about, and the people of their
    /// issues.
    pub members: Vec<Member>,
    pub teams: Vec<Team>,
    /// How many notifications were about something other than an issue,
    /// and were left out.
    pub skipped: usize,
    pub as_of: Timestamp,
}

/// The fields one `notificationUpdate` changes.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NotificationUpdate {
    /// Marks it read at a time, or unread.
    pub read_at: Patch<Timestamp>,
    /// Hides it until this time.
    pub snoozed_until_at: Option<Timestamp>,
}

/// A comment Linear has stored, and its author.
#[derive(Debug, Clone, PartialEq)]
pub struct PostedComment {
    pub comment: Comment,
    pub author: Option<Member>,
}

/// What a reaction is on: an issue or a comment, by id.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum ReactionTarget {
    Issue(String),
    Comment(String),
}

/// Every field of a new issue that lrat's create form sets.
#[derive(Debug, Clone, PartialEq)]
pub struct IssueCreate {
    pub team_id: String,
    pub state_id: String,
    /// `None` leaves the issue unassigned.
    pub assignee_id: Option<String>,
    pub priority: Priority,
    pub title: String,
    /// Markdown.
    pub description: String,
}

/// A relation to create: the issue with id `issue_id` relates to the one
/// with id `related_issue_id` as `kind` says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelationCreate {
    pub issue_id: String,
    pub related_issue_id: String,
    pub kind: RelationType,
}

/// What changed in one team: its issues updated after a watermark, and the
/// cycles, labels, members and teams they refer to.
#[derive(Debug, Clone, PartialEq)]
pub struct TeamChanges {
    pub team_id: String,
    /// The changed issues that are not in the trash, archived ones included.
    pub issues: Vec<Issue>,
    /// The ids of changed issues that are in the trash.
    pub trashed: Vec<String>,
    pub cycles: Vec<Cycle>,
    pub labels: Vec<Label>,
    pub members: Vec<Member>,
    pub teams: Vec<Team>,
    /// The newest `updatedAt` among the changes, trashed ones included: the
    /// team's next watermark. `None` when nothing changed.
    pub newest: Option<Timestamp>,
    /// When the changes were loaded, which decides the active cycle.
    pub as_of: Timestamp,
}

/// The fields one `issueUpdate` changes. Every other field stays as it is.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct IssueUpdate {
    /// The workflow state to move to, or `None` to leave it.
    pub state_id: Option<String>,
    pub assignee_id: Patch<String>,
    /// The new priority, or `None` to leave it. [`Priority::None`] is a value
    /// like any other, not a clear.
    pub priority: Option<Priority>,
    /// The new estimate in points, as Linear's `Int`. T-shirt sizes are
    /// points too (XS 1, S 2, M 3, L 5, XL 8, XXL 13, XXXL 21).
    pub estimate: Patch<u32>,
    /// Labels to add, leaving the issue's other labels as they are.
    pub added_label_ids: Vec<String>,
    /// Labels to remove, leaving the issue's other labels as they are.
    pub removed_label_ids: Vec<String>,
    /// The new title, or `None` to leave it.
    pub title: Option<String>,
    /// The new Markdown description, or `None` to leave it. An empty one
    /// empties it.
    pub description: Option<String>,
    pub due_date: Patch<Date>,
    pub cycle_id: Patch<String>,
    /// The new project. Linear clears the project milestone when the project
    /// changes.
    pub project_id: Patch<String>,
    pub project_milestone_id: Patch<String>,
    pub parent_id: Patch<String>,
}

/// A change to a field that can be empty: leave it, empty it, or set it.
/// Linear tells the first two apart by whether the field is in the input at
/// all, so they must not collapse into one `Option`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum Patch<T> {
    #[default]
    Unchanged,
    Clear,
    Set(T),
}

/// What a search found, best match first, or the issues of a cycle or
/// project, and the cycles, labels, members and teams they refer to.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SearchResults {
    pub issues: Vec<Issue>,
    pub cycles: Vec<Cycle>,
    pub labels: Vec<Label>,
    pub members: Vec<Member>,
    pub teams: Vec<Team>,
    /// What Linear could not give, as in [`AssignedIssues::problems`].
    pub problems: Vec<String>,
}

/// What a search asks Linear for: the term, whether comments count as a
/// match, and the team and filter to keep to, if any.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SearchQuery {
    pub term: String,
    pub include_comments: bool,
    /// The team and Team Issues filter the results must match; `None`
    /// searches the whole workspace.
    pub within: Option<TeamQuery>,
}

impl SearchQuery {
    /// `term` across the whole workspace, comments left out.
    pub fn new(term: impl Into<String>) -> Self {
        Self {
            term: term.into(),
            ..Self::default()
        }
    }
}

/// A team's issues, narrowed by a filter, in the order Linear sends them.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TeamQuery {
    pub team_id: String,
    pub filter: TeamFilter,
    pub order: TeamOrder,
}

/// The order Linear sends a team's issues in, newest first, as its
/// `PaginationOrderBy`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum TeamOrder {
    #[default]
    Updated,
    Created,
}

impl TeamOrder {
    /// Linear's name for it.
    pub fn as_graphql(self) -> &'static str {
        match self {
            TeamOrder::Updated => "updatedAt",
            TeamOrder::Created => "createdAt",
        }
    }
}

/// One optional value per field; an issue matches when it matches every
/// field that is set. `None` leaves the field unfiltered.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TeamFilter {
    /// A workflow state id.
    pub state: Option<String>,
    /// A member id.
    pub assignee: Option<String>,
    /// A label id; the issue carries it among its labels.
    pub label: Option<String>,
    /// A cycle id.
    pub cycle: Option<String>,
    /// A project id.
    pub project: Option<String>,
    /// Only issues blocked by another issue, as Linear's
    /// `hasBlockedByRelations`, whatever that issue's state.
    pub blocked: bool,
}

impl TeamFilter {
    /// Whether no field is set.
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }

    /// Whether `issue` matches every field that is set. `blocked` says
    /// whether another issue blocks it, as far as the caller knows.
    pub fn matches(&self, issue: &Issue, blocked: bool) -> bool {
        let field = |wanted: &Option<String>, actual: Option<&String>| {
            wanted.as_ref().is_none_or(|wanted| actual == Some(wanted))
        };
        field(&self.state, Some(&issue.state.id))
            && field(&self.assignee, issue.assignee_id.as_ref())
            && field(&self.cycle, issue.cycle_id.as_ref())
            && field(&self.project, issue.project_id.as_ref())
            && self
                .label
                .as_ref()
                .is_none_or(|label| issue.label_ids.contains(label))
            && (!self.blocked || blocked)
    }
}

/// The projects the Projects screen lists.
#[derive(Debug, Clone, PartialEq)]
pub struct ProjectList {
    pub projects: Vec<ProjectOverview>,
    /// What Linear could not give, such as a list cut short.
    pub problems: Vec<String>,
}

/// What the filters of one team can pick from.
#[derive(Debug, Clone, PartialEq)]
pub struct TeamOptions {
    pub team_id: String,
    pub states: Vec<WorkflowState>,
    pub members: Vec<Member>,
    pub labels: Vec<Label>,
    pub cycles: Vec<Cycle>,
    pub projects: Vec<Project>,
    /// What Linear could not give, such as a list cut short.
    pub problems: Vec<String>,
}

/// One page of a team's issues, and the cycles and members they refer to.
#[derive(Debug, Clone, PartialEq)]
pub struct TeamIssuesPage {
    pub issues: Vec<Issue>,
    pub cycles: Vec<Cycle>,
    pub members: Vec<Member>,
    /// The cursor to ask for the next page with, `None` on the last page.
    pub next: Option<String>,
    /// When the page was loaded, which decides the active cycle.
    pub as_of: Timestamp,
    /// What Linear could not give: errors that came back with the data.
    pub problems: Vec<String>,
}

/// The stretch of time an analytics load covers, from `since` up to `until`
/// (excluded), or up to now when `until` is `None`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AnalyticsWindow {
    pub since: Timestamp,
    pub until: Option<Timestamp>,
}

impl AnalyticsWindow {
    pub fn contains(&self, at: Timestamp) -> bool {
        self.since <= at && self.until.is_none_or(|until| at < until)
    }

    /// Whether the window reaches now, so its load brings in-progress issues.
    pub fn reaches_now(&self) -> bool {
        self.until.is_none()
    }
}

/// The result of [`LinearClient::analytics_issues`].
#[derive(Debug, Clone, PartialEq)]
pub struct AnalyticsIssues {
    pub viewer: Viewer,
    pub issues: Vec<Issue>,
    /// The viewer's teams, which decide whether scope is in points.
    pub teams: Vec<Team>,
    /// The cycles of the viewer's teams that overlap the window, with their
    /// history arrays.
    pub cycles: Vec<Cycle>,
    pub labels: Vec<Label>,
    pub projects: Vec<Project>,
    pub members: Vec<Member>,
    /// When the data was loaded.
    pub as_of: Timestamp,
    /// What Linear could not give, as in [`AssignedIssues::problems`].
    pub problems: Vec<String>,
}

impl AnalyticsIssues {
    /// The demo workspace's answer for `window`, as of [`demo::NOW`]. The
    /// state history is kept only on in-progress issues, as Linear's is.
    pub fn demo(window: AnalyticsWindow) -> Self {
        let data = demo::workspace();
        let viewer_id = data.viewer.id.clone();
        let assigned = |issue: &Issue| issue.assignee_id.as_ref() == Some(&viewer_id);
        let in_window = |issue: &Issue| {
            window.contains(issue.created_at)
                || issue.completed_at.is_some_and(|at| window.contains(at))
        };
        let in_progress =
            |issue: &Issue| window.reaches_now() && issue.state.state_type == StateType::Started;
        let issues = data
            .issues
            .into_iter()
            .filter(|issue| assigned(issue) && (in_window(issue) || in_progress(issue)))
            .map(|mut issue| {
                if issue.state.state_type != StateType::Started {
                    issue.state_history.clear();
                }
                issue
            })
            .collect();
        let cycles = data
            .cycles
            .into_iter()
            .filter(|cycle| {
                cycle.ends_at > window.since
                    && window.until.is_none_or(|until| cycle.starts_at < until)
            })
            .collect();
        Self {
            viewer: data.viewer,
            issues,
            // The viewer is a member of every demo team.
            teams: data.teams,
            cycles,
            labels: data.labels,
            projects: data.projects,
            members: data.members,
            as_of: demo::NOW,
            problems: Vec::new(),
        }
    }
}

/// The result of [`LinearClient::team_analytics`].
#[derive(Debug, Clone, PartialEq)]
pub struct TeamAnalyticsIssues {
    /// The team, whose estimate setting decides whether scope is in points.
    pub team: Team,
    pub issues: Vec<Issue>,
    /// The team's last six finished cycles and its active one, with their
    /// history arrays; empty for a window that does not reach now.
    pub cycles: Vec<Cycle>,
    pub labels: Vec<Label>,
    pub projects: Vec<Project>,
    pub members: Vec<Member>,
    /// When the data was loaded.
    pub as_of: Timestamp,
    /// What Linear could not give, as in [`AssignedIssues::problems`].
    pub problems: Vec<String>,
}

impl TeamAnalyticsIssues {
    /// The demo workspace's answer for team `team_id` over `window`, as of
    /// [`demo::NOW`]. The state history is kept only on in-progress issues,
    /// as Linear's is. An id the demo does not have is an error.
    pub fn demo(team_id: &str, window: AnalyticsWindow) -> Result<Self, ClientError> {
        let data = demo::workspace();
        let team = data
            .teams
            .iter()
            .find(|team| team.id == team_id)
            .cloned()
            .ok_or_else(|| not_found("team", team_id))?;
        let in_window = |at: Option<Timestamp>| at.is_some_and(|at| window.contains(at));
        let issues = data
            .issues
            .into_iter()
            .filter(|issue| issue.team_id == team_id)
            .filter(|issue| {
                in_window(Some(issue.created_at))
                    || in_window(issue.completed_at)
                    || in_window(issue.canceled_at)
                    || (window.reaches_now() && issue.state.state_type.is_open())
            })
            .map(|mut issue| {
                if issue.state.state_type != StateType::Started {
                    issue.state_history.clear();
                }
                issue
            })
            .collect();
        let mut cycles: Vec<Cycle> = data
            .cycles
            .into_iter()
            .filter(|cycle| window.reaches_now() && cycle.team_id == team_id)
            .filter(|cycle| cycle.starts_at <= demo::NOW)
            .collect();
        cycles.sort_by_key(|cycle| Reverse(cycle.number));
        cycles.truncate(7);
        Ok(Self {
            team,
            issues,
            cycles,
            labels: data.labels,
            projects: data.projects,
            members: data.members,
            as_of: demo::NOW,
            problems: Vec::new(),
        })
    }
}

/// The viewer's open assigned issues, and the cycles, labels, members and
/// teams they refer to.
#[derive(Debug, Clone, PartialEq)]
pub struct AssignedIssues {
    pub viewer: Viewer,
    /// The workspace the viewer belongs to.
    pub workspace: Workspace,
    pub issues: Vec<Issue>,
    pub cycles: Vec<Cycle>,
    pub labels: Vec<Label>,
    pub members: Vec<Member>,
    pub teams: Vec<Team>,
    /// When the data was loaded, which decides the active cycle.
    pub as_of: Timestamp,
    /// What Linear could not give, for the viewer: errors that came back
    /// with the data, or a lookup that failed. Empty when nothing is missing.
    pub problems: Vec<String>,
}

impl AssignedIssues {
    /// The demo workspace's open issues assigned to its viewer, as of
    /// [`demo::NOW`].
    pub fn demo() -> Self {
        let data = demo::workspace();
        let issues = data
            .issues
            .into_iter()
            .filter(|issue| issue.assignee_id.as_ref() == Some(&data.viewer.id))
            .filter(|issue| issue.state.state_type.is_open())
            .collect();
        Self {
            viewer: data.viewer,
            workspace: data.workspace,
            issues,
            cycles: data.cycles,
            labels: data.labels,
            members: data.members,
            teams: data.teams,
            as_of: demo::NOW,
            problems: Vec::new(),
        }
    }
}

/// One issue's comments, workflow state history, parent, sub-issues and
/// relations, and the people, states, project and issues they refer to.
#[derive(Debug, Clone, PartialEq)]
pub struct IssueDetail {
    /// Oldest first.
    pub comments: Vec<Comment>,
    /// Oldest first.
    pub state_history: Vec<StateSpan>,
    /// The workflow states the history passes through.
    pub workflow_states: Vec<WorkflowState>,
    /// The comment authors.
    pub members: Vec<Member>,
    pub project: Option<Project>,
    pub milestone: Option<ProjectMilestone>,
    /// The first 50 attachments, oldest first; any more are left out.
    pub attachments: Vec<Attachment>,
    /// The first 25 relations the issue is the source of and the first 25
    /// it is the target of.
    pub relations: Vec<IssueRelation>,
    /// The parent, the first 50 sub-issues and the issue at the other end of
    /// each relation.
    pub related_issues: Vec<Issue>,
    /// The teams and cycles of `related_issues`.
    pub teams: Vec<Team>,
    pub cycles: Vec<Cycle>,
    /// The issue's own reactions, oldest first.
    pub reactions: Vec<Reaction>,
    /// What Linear could not give, as in [`AssignedIssues::problems`].
    pub problems: Vec<String>,
}

impl IssueDetail {
    /// The detail of the demo issue with id `issue_id`; empty for an id the
    /// demo workspace does not have.
    pub fn demo(issue_id: &str) -> Self {
        let data = demo::workspace();
        let issue = data.issues.iter().find(|issue| issue.id == issue_id);
        let team_id = issue.map(|issue| issue.team_id.as_str());
        let project_id = issue.and_then(|issue| issue.project_id.as_deref());
        let milestone_id = issue.and_then(|issue| issue.project_milestone_id.as_deref());
        let relations: Vec<IssueRelation> = data
            .relations
            .iter()
            .filter(|relation| {
                relation.issue_id == issue_id || relation.related_issue_id == issue_id
            })
            .cloned()
            .collect();
        let parent_id = issue.and_then(|issue| issue.parent_id.as_deref());
        let related_issues: Vec<Issue> = data
            .issues
            .iter()
            .filter(|other| {
                Some(other.id.as_str()) == parent_id
                    || other.parent_id.as_deref() == Some(issue_id)
                    || relations.iter().any(|relation| {
                        relation.seen_from(issue_id).map(|(_, id)| id) == Some(other.id.as_str())
                    })
            })
            .cloned()
            .collect();
        Self {
            relations,
            related_issues,
            teams: data.teams.clone(),
            cycles: data.cycles.clone(),
            comments: data
                .comments
                .into_iter()
                .filter(|comment| comment.issue_id == issue_id)
                .collect(),
            state_history: issue
                .map(|issue| issue.state_history.clone())
                .unwrap_or_default(),
            workflow_states: data
                .workflow_states
                .into_iter()
                .filter(|state| Some(state.team_id.as_str()) == team_id)
                .collect(),
            members: data.members,
            project: data
                .projects
                .into_iter()
                .find(|project| Some(project.id.as_str()) == project_id),
            milestone: data
                .milestones
                .into_iter()
                .find(|milestone| Some(milestone.id.as_str()) == milestone_id),
            attachments: data
                .attachments
                .into_iter()
                .filter(|attachment| attachment.issue_id == issue_id)
                .collect(),
            reactions: data
                .issue_reactions
                .get(issue_id)
                .cloned()
                .unwrap_or_default(),
            problems: Vec::new(),
        }
    }
}

/// A client that never touches the network. It serves the demo workspace and
/// remembers the edits it was sent, per thread: the runtime and each test
/// run on one.
#[derive(Debug, Default, Clone, Copy)]
pub struct FakeClient;

thread_local! {
    static DEMO_EDITS: RefCell<BTreeMap<String, Issue>> = const { RefCell::new(BTreeMap::new()) };
    static DEMO_COMMENTS: RefCell<Vec<Comment>> = const { RefCell::new(Vec::new()) };
    /// Comments edited, resolved and deleted through this client.
    static DEMO_COMMENT_CHANGES: RefCell<DemoCommentChanges> =
        RefCell::new(DemoCommentChanges::default());
    /// Reactions added through this client and on what, and the ids of
    /// those removed.
    static DEMO_REACTIONS: RefCell<DemoReactions> =
        const { RefCell::new((Vec::new(), Vec::new())) };
    static DEMO_CREATED: RefCell<u32> = const { RefCell::new(0) };
    static DEMO_TRASHED: RefCell<BTreeSet<String>> = const { RefCell::new(BTreeSet::new()) };
    /// Relations created through this client, and the ids of those deleted.
    static DEMO_RELATIONS: RefCell<(Vec<IssueRelation>, Vec<String>)> =
        const { RefCell::new((Vec::new(), Vec::new())) };
    static DEMO_ATTACHMENTS: RefCell<Vec<Attachment>> = const { RefCell::new(Vec::new()) };
    /// Notifications as writes through this client left them.
    static DEMO_NOTIFICATIONS: RefCell<BTreeMap<String, Notification>> =
        const { RefCell::new(BTreeMap::new()) };
}

/// Attaches `url` to the demo issue with id `issue_id` as Linear does: an
/// attachment with the same URL on the issue is replaced.
fn demo_attach(issue_id: &str, url: &str, title: &str) -> Result<Attachment, ClientError> {
    if !demo_has_issue(&demo::workspace(), issue_id) {
        return Err(not_found("issue", issue_id));
    }
    Ok(DEMO_ATTACHMENTS.with_borrow_mut(|attachments| {
        attachments.retain(|known| known.issue_id != issue_id || known.url != url);
        let attachment = Attachment {
            id: format!("demo-attachment-new-{}", attachments.len() + 1),
            issue_id: issue_id.to_owned(),
            url: url.to_owned(),
            title: title.to_owned(),
            subtitle: None,
            source_type: None,
            status: None,
        };
        attachments.push(attachment.clone());
        attachment
    }))
}

/// The comments changed through the fake client: their newest copies by
/// id, and the ids of those deleted.
#[derive(Default)]
struct DemoCommentChanges {
    changed: BTreeMap<String, Comment>,
    deleted: Vec<String>,
}

impl DemoCommentChanges {
    /// `comments` as the changes left them: changed ones replaced, deleted
    /// ones and their replies gone.
    fn apply(&self, comments: &mut Vec<Comment>) {
        comments.retain(|comment| {
            !self.deleted.contains(&comment.id)
                && !comment
                    .parent_id
                    .as_ref()
                    .is_some_and(|parent| self.deleted.contains(parent))
        });
        for comment in comments {
            if let Some(changed) = self.changed.get(&comment.id) {
                comment.clone_from(changed);
            }
        }
    }
}

/// The demo comment with id `id` as changes through the fake client left
/// it, unless it was deleted.
fn demo_comment(id: &str) -> Result<Comment, ClientError> {
    let mut comments = demo::workspace().comments;
    DEMO_COMMENTS.with_borrow(|created| comments.extend(created.iter().cloned()));
    DEMO_COMMENT_CHANGES.with_borrow(|changes| changes.apply(&mut comments));
    comments
        .into_iter()
        .find(|comment| comment.id == id)
        .ok_or_else(|| not_found("comment", id))
}

/// Applies `change` to the demo comment with id `id` and remembers it.
fn demo_change_comment(id: &str, change: impl FnOnce(&mut Comment)) -> Result<(), ClientError> {
    let mut comment = demo_comment(id)?;
    change(&mut comment);
    DEMO_COMMENT_CHANGES.with_borrow_mut(|changes| changes.changed.insert(id.to_owned(), comment));
    Ok(())
}

/// Reactions added through the fake client and what each is on, and the
/// ids of those removed.
type DemoReactions = (Vec<(ReactionTarget, Reaction)>, Vec<String>);

/// `detail`'s reactions, and those of its comments, as reactions added and
/// removed through the fake client left them.
fn apply_demo_reactions(issue_id: &str, detail: &mut IssueDetail) {
    DEMO_REACTIONS.with_borrow(|(added, removed)| {
        for (target, reaction) in added {
            let reactions = match target {
                ReactionTarget::Issue(id) if id == issue_id => &mut detail.reactions,
                ReactionTarget::Comment(id) => {
                    match detail.comments.iter_mut().find(|c| &c.id == id) {
                        Some(comment) => &mut comment.reactions,
                        None => continue,
                    }
                }
                ReactionTarget::Issue(_) => continue,
            };
            reactions.push(reaction.clone());
        }
        detail
            .reactions
            .retain(|reaction| !removed.contains(&reaction.id));
        for comment in &mut detail.comments {
            comment
                .reactions
                .retain(|reaction| !removed.contains(&reaction.id));
        }
    });
}

/// The demo inbox as writes through the fake client left it, archived
/// notifications included.
fn demo_notifications() -> Vec<Notification> {
    let mut notifications = demo::notifications(&demo::workspace());
    DEMO_NOTIFICATIONS.with_borrow(|edits| {
        for notification in &mut notifications {
            if let Some(edited) = edits.get(&notification.id) {
                notification.clone_from(edited);
            }
        }
    });
    notifications
}

/// Applies `change` to the demo notification with id `id` and remembers it,
/// one second newer.
fn demo_notification_change(
    id: &str,
    change: impl FnOnce(&mut Notification),
) -> Result<(), ClientError> {
    let mut notification = demo_notifications()
        .into_iter()
        .find(|notification| notification.id == id)
        .ok_or_else(|| not_found("notification", id))?;
    change(&mut notification);
    notification.updated_at += SignedDuration::from_secs(1);
    DEMO_NOTIFICATIONS.with_borrow_mut(|edits| edits.insert(id.to_owned(), notification));
    Ok(())
}

/// The demo issue with id `issue_id` as edits through the fake client left
/// it, unless it is in the trash.
fn demo_issue(data: &demo::DemoWorkspace, issue_id: &str) -> Result<Issue, ClientError> {
    if DEMO_TRASHED.with_borrow(|trashed| trashed.contains(issue_id)) {
        return Err(not_found("issue", issue_id));
    }
    DEMO_EDITS
        .with_borrow(|edits| edits.get(issue_id).cloned())
        .or_else(|| {
            data.issues
                .iter()
                .find(|issue| issue.id == issue_id)
                .cloned()
        })
        .ok_or_else(|| not_found("issue", issue_id))
}

/// Applies `change` to the demo issue with id `issue_id` and remembers it,
/// one second newer.
fn demo_change(issue_id: &str, change: impl FnOnce(&mut Issue)) -> Result<(), ClientError> {
    let mut issue = demo_issue(&demo::workspace(), issue_id)?;
    change(&mut issue);
    issue.updated_at += SignedDuration::from_secs(1);
    DEMO_EDITS.with_borrow_mut(|edits| edits.insert(issue_id.to_owned(), issue));
    Ok(())
}

/// The demo workspace with the relations created and deleted through the
/// fake client, so the Blocked filter sees them.
fn demo_workspace_with_relations() -> demo::DemoWorkspace {
    let mut data = demo::workspace();
    DEMO_RELATIONS.with_borrow(|(created, deleted)| {
        data.relations
            .retain(|relation| !deleted.contains(&relation.id));
        data.relations.extend(created.iter().cloned());
    });
    data
}

/// Whether the demo has an issue with id `id`, created ones included.
fn demo_has_issue(data: &demo::DemoWorkspace, id: &str) -> bool {
    data.issues.iter().any(|issue| issue.id == id)
        || DEMO_EDITS.with_borrow(|edits| edits.contains_key(id))
}

/// The issue `input` creates in the demo workspace, numbered after the
/// team's last issue and the `created` issues before it.
fn demo_create(
    data: &demo::DemoWorkspace,
    input: &IssueCreate,
    created: u32,
) -> Result<Issue, ClientError> {
    let team = data
        .teams
        .iter()
        .find(|team| team.id == input.team_id)
        .ok_or_else(|| not_found("team", &input.team_id))?;
    let state = data
        .workflow_states
        .iter()
        .find(|state| state.id == input.state_id && state.team_id == team.id)
        .cloned()
        .ok_or_else(|| not_found("workflow state", &input.state_id))?;
    if let Some(id) = &input.assignee_id
        && !data.members.iter().any(|member| member.id == *id)
    {
        return Err(not_found("member", id));
    }
    let last = data
        .issues
        .iter()
        .filter(|issue| issue.team_id == team.id)
        .filter_map(|issue| issue.identifier.rsplit('-').next()?.parse::<u32>().ok())
        .max()
        .unwrap_or(0);
    let identifier = format!("{}-{}", team.key, last + created);
    let slug: String = input
        .title
        .to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    Ok(Issue {
        id: format!("demo-created-{created}"),
        url: format!(
            "https://linear.app/{}/issue/{identifier}",
            data.workspace.url_key
        ),
        branch_name: format!("{}-{slug}", identifier.to_lowercase()),
        identifier,
        title: input.title.clone(),
        description: Some(input.description.clone()),
        team_id: team.id.clone(),
        state,
        priority: input.priority,
        estimate: None,
        assignee_id: input.assignee_id.clone(),
        creator_id: Some(data.viewer.id.clone()),
        cycle_id: None,
        project_id: None,
        project_milestone_id: None,
        parent_id: None,
        due_date: None,
        archived_at: None,
        subscribed: true,
        label_ids: Vec::new(),
        created_at: demo::NOW,
        updated_at: demo::NOW + SignedDuration::from_secs(1),
        started_at: None,
        completed_at: None,
        canceled_at: None,
        triaged_at: None,
        state_history: Vec::new(),
    })
}

/// Applies `patch` to `field`, leaving it when unchanged.
fn apply_patch<T: Clone>(field: &mut Option<T>, patch: &Patch<T>) {
    match patch {
        Patch::Unchanged => {}
        Patch::Clear => *field = None,
        Patch::Set(value) => *field = Some(value.clone()),
    }
}

/// `issue` with `update` applied as Linear would apply it in the demo
/// workspace, one second newer.
fn demo_update(
    data: &demo::DemoWorkspace,
    mut issue: Issue,
    update: &IssueUpdate,
) -> Result<Issue, ClientError> {
    if let Some(state_id) = &update.state_id {
        issue.state = data
            .workflow_states
            .iter()
            .find(|state| state.id == *state_id && state.team_id == issue.team_id)
            .cloned()
            .ok_or_else(|| not_found("workflow state", state_id))?;
    }
    match &update.assignee_id {
        Patch::Unchanged => {}
        Patch::Clear => issue.assignee_id = None,
        Patch::Set(id) if data.members.iter().any(|member| member.id == *id) => {
            issue.assignee_id = Some(id.clone());
        }
        Patch::Set(id) => return Err(not_found("member", id)),
    }
    if let Some(priority) = update.priority {
        issue.priority = priority;
    }
    match update.estimate {
        Patch::Unchanged => {}
        Patch::Clear => issue.estimate = None,
        Patch::Set(points) => issue.estimate = Some(f64::from(points)),
    }
    apply_patch(&mut issue.due_date, &update.due_date);
    apply_patch(&mut issue.cycle_id, &update.cycle_id);
    if update.project_id != Patch::Unchanged {
        issue.project_milestone_id = None;
    }
    apply_patch(&mut issue.project_id, &update.project_id);
    apply_patch(
        &mut issue.project_milestone_id,
        &update.project_milestone_id,
    );
    apply_patch(&mut issue.parent_id, &update.parent_id);
    for id in update
        .added_label_ids
        .iter()
        .chain(&update.removed_label_ids)
    {
        let usable = data.labels.iter().any(|label| {
            label.id == *id
                && label
                    .team_id
                    .as_ref()
                    .is_none_or(|team_id| *team_id == issue.team_id)
        });
        if !usable {
            return Err(not_found("label", id));
        }
    }
    issue
        .label_ids
        .retain(|id| !update.removed_label_ids.contains(id));
    for id in &update.added_label_ids {
        if !issue.label_ids.contains(id) {
            issue.label_ids.push(id.clone());
        }
    }
    if let Some(title) = &update.title {
        issue.title.clone_from(title);
    }
    if let Some(description) = &update.description {
        issue.description = (!description.is_empty()).then(|| description.clone());
    }
    issue.updated_at += SignedDuration::from_secs(1);
    Ok(issue)
}

/// What a client without an inbox answers a notification write with.
fn no_inbox() -> ClientError {
    ClientError::Decode("this client has no inbox".to_owned())
}

fn not_found(what: &str, id: &str) -> ClientError {
    ClientError::GraphQl(vec![GraphQlError {
        message: format!("Entity not found: {what} {id}"),
        path: Vec::new(),
        extensions: None,
    }])
}

impl LinearClient for FakeClient {
    async fn viewer(&self) -> Result<(Viewer, Workspace), ClientError> {
        Ok((
            Viewer {
                id: "viewer-remy".to_owned(),
                name: "Remy Fromage".to_owned(),
            },
            Workspace {
                id: "workspace-cheddar".to_owned(),
                name: "Cheddar Works".to_owned(),
                url_key: "cheddar-works".to_owned(),
            },
        ))
    }

    async fn assigned_issues(&self) -> Result<AssignedIssues, ClientError> {
        Ok(AssignedIssues::demo())
    }

    /// The demo workspace never changes after [`demo::NOW`], so a delta
    /// holds only what the overlap reaches back to.
    async fn changed_issues(
        &self,
        team_id: &str,
        since: Option<Timestamp>,
    ) -> Result<TeamChanges, ClientError> {
        let data = demo::workspace();
        let mut issues: Vec<Issue> = data
            .issues
            .into_iter()
            .filter(|issue| issue.team_id == team_id)
            .filter(|issue| since.is_none_or(|since| issue.updated_at > since))
            .collect();
        issues.sort_by_key(|issue| Reverse(issue.updated_at));
        if since.is_none() {
            issues.truncate(1);
        }
        Ok(TeamChanges {
            team_id: team_id.to_owned(),
            newest: issues.iter().map(|issue| issue.updated_at).max(),
            issues,
            trashed: Vec::new(),
            cycles: data.cycles,
            labels: data.labels,
            members: data.members,
            teams: data.teams,
            as_of: demo::NOW,
        })
    }

    async fn existing_issues(&self, ids: &[String]) -> Result<Vec<String>, ClientError> {
        let data = demo::workspace();
        Ok(ids
            .iter()
            .filter(|id| {
                data.issues.iter().any(|issue| &issue.id == *id)
                    || DEMO_EDITS.with_borrow(|edits| edits.contains_key(id.as_str()))
            })
            .filter(|id| !DEMO_TRASHED.with_borrow(|trashed| trashed.contains(id.as_str())))
            .cloned()
            .collect())
    }

    async fn workflow_states(&self, team_id: &str) -> Result<Vec<WorkflowState>, ClientError> {
        Ok(demo::workspace()
            .workflow_states
            .into_iter()
            .filter(|state| state.team_id == team_id)
            .collect())
    }

    async fn team_members(&self, team_id: &str) -> Result<Vec<Member>, ClientError> {
        let ids = demo::team_member_ids(team_id);
        Ok(demo::workspace()
            .members
            .into_iter()
            .filter(|member| ids.contains(&member.id))
            .collect())
    }

    async fn team_labels(&self, team_id: &str) -> Result<Vec<Label>, ClientError> {
        Ok(demo::team_options(&demo::workspace(), team_id).labels)
    }

    async fn project_milestones(
        &self,
        project_id: &str,
    ) -> Result<Vec<ProjectMilestone>, ClientError> {
        Ok(demo::workspace()
            .milestones
            .into_iter()
            .filter(|milestone| milestone.project_id == project_id)
            .collect())
    }

    async fn archive_issue(&self, issue_id: &str) -> Result<(), ClientError> {
        demo_change(issue_id, |issue| issue.archived_at = Some(demo::NOW))
    }

    async fn unarchive_issue(&self, issue_id: &str) -> Result<(), ClientError> {
        demo_change(issue_id, |issue| issue.archived_at = None)
    }

    async fn notifications(
        &self,
        since: Option<Timestamp>,
    ) -> Result<NotificationsLoaded, ClientError> {
        let data = demo::workspace();
        let notifications: Vec<Notification> = match since {
            None => demo_notifications()
                .into_iter()
                .filter(|notification| notification.archived_at.is_none())
                .collect(),
            Some(since) => demo_notifications()
                .into_iter()
                .filter(|notification| notification.updated_at > since)
                .collect(),
        };
        let mut issues: Vec<Issue> = Vec::new();
        for notification in &notifications {
            if !issues.iter().any(|issue| issue.id == notification.issue_id) {
                issues.push(demo_issue(&data, &notification.issue_id)?);
            }
        }
        Ok(NotificationsLoaded {
            notifications,
            issues,
            cycles: data.cycles,
            members: data.members,
            teams: data.teams,
            skipped: 0,
            as_of: demo::NOW,
        })
    }

    async fn update_notification(
        &self,
        id: &str,
        update: &NotificationUpdate,
    ) -> Result<(), ClientError> {
        demo_notification_change(id, |notification| {
            match &update.read_at {
                Patch::Unchanged => {}
                Patch::Clear => notification.read_at = None,
                Patch::Set(at) => notification.read_at = Some(*at),
            }
            if let Some(until) = update.snoozed_until_at {
                notification.snoozed_until_at = Some(until);
            }
        })
    }

    async fn archive_notification(&self, id: &str, _: Urgency) -> Result<(), ClientError> {
        demo_notification_change(id, |notification| {
            notification.archived_at = Some(demo::NOW);
        })
    }

    async fn mark_issue_notifications_read(
        &self,
        issue_id: &str,
        read_at: Timestamp,
    ) -> Result<(), ClientError> {
        let ids: Vec<String> = demo_notifications()
            .into_iter()
            .filter(|notification| notification.issue_id == issue_id)
            .filter(|notification| notification.read_at.is_none())
            .map(|notification| notification.id)
            .collect();
        for id in ids {
            demo_notification_change(&id, |notification| {
                notification.read_at = Some(read_at);
            })?;
        }
        Ok(())
    }

    /// Remembers the issue as trashed: later edits and existence checks no
    /// longer find it.
    async fn trash_issue(&self, issue_id: &str) -> Result<(), ClientError> {
        demo_issue(&demo::workspace(), issue_id)?;
        DEMO_TRASHED.with_borrow_mut(|trashed| trashed.insert(issue_id.to_owned()));
        Ok(())
    }

    async fn subscribe_to_issue(&self, issue_id: &str) -> Result<(), ClientError> {
        demo_change(issue_id, |issue| issue.subscribed = true)
    }

    async fn unsubscribe_from_issue(&self, issue_id: &str) -> Result<(), ClientError> {
        demo_change(issue_id, |issue| issue.subscribed = false)
    }

    async fn update_issue(
        &self,
        issue_id: &str,
        update: &IssueUpdate,
    ) -> Result<Issue, ClientError> {
        let data = demo::workspace();
        let stored = demo_issue(&data, issue_id)?;
        let updated = demo_update(&data, stored, update)?;
        DEMO_EDITS.with_borrow_mut(|edits| edits.insert(issue_id.to_owned(), updated.clone()));
        Ok(updated)
    }

    /// The demo detail, with the relations created and deleted through
    /// this client and the issues as edits left them.
    async fn issue_detail(&self, issue_id: &str, _: Urgency) -> Result<IssueDetail, ClientError> {
        let mut detail = IssueDetail::demo(issue_id);
        DEMO_RELATIONS.with_borrow(|(created, deleted)| {
            detail
                .relations
                .retain(|relation| !deleted.contains(&relation.id));
            detail.relations.extend(
                created
                    .iter()
                    .filter(|relation| relation.seen_from(issue_id).is_some())
                    .cloned(),
            );
        });
        DEMO_EDITS.with_borrow(|edits| {
            for issue in &mut detail.related_issues {
                if let Some(edited) = edits.get(&issue.id) {
                    issue.clone_from(edited);
                }
            }
        });
        DEMO_COMMENTS.with_borrow(|comments| {
            detail.comments.extend(
                comments
                    .iter()
                    .filter(|comment| comment.issue_id == issue_id)
                    .cloned(),
            );
        });
        DEMO_ATTACHMENTS.with_borrow(|attachments| {
            for attachment in attachments.iter().filter(|a| a.issue_id == issue_id) {
                detail
                    .attachments
                    .retain(|known| known.url != attachment.url);
                detail.attachments.push(attachment.clone());
            }
        });
        DEMO_COMMENT_CHANGES.with_borrow(|changes| changes.apply(&mut detail.comments));
        apply_demo_reactions(issue_id, &mut detail);
        Ok(detail)
    }

    /// Every demo issue whose identifier, title or description, or with
    /// `include_comments` one of its comments, holds each word of the term,
    /// ignoring case, in the demo's order; with `within`, only those of
    /// that team that match its filter.
    async fn search_issues(&self, query: &SearchQuery) -> Result<SearchResults, ClientError> {
        let words: Vec<String> = query
            .term
            .split_whitespace()
            .map(str::to_lowercase)
            .collect();
        let data = demo_workspace_with_relations();
        let holds_every_word = |text: &str| {
            let text = text.to_lowercase();
            words.iter().all(|word| text.contains(word.as_str()))
        };
        let issues = data
            .issues
            .into_iter()
            .filter(|issue| {
                query.within.as_ref().is_none_or(|within| {
                    let blocked = data.relations.iter().any(|relation| {
                        relation.kind == RelationType::Blocks
                            && relation.related_issue_id == issue.id
                    });
                    issue.team_id == within.team_id && within.filter.matches(issue, blocked)
                })
            })
            .filter(|issue| {
                let text = format!(
                    "{} {} {}",
                    issue.identifier,
                    issue.title,
                    issue.description.as_deref().unwrap_or_default()
                );
                holds_every_word(&text)
                    || query.include_comments
                        && data
                            .comments
                            .iter()
                            .filter(|comment| comment.issue_id == issue.id)
                            .any(|comment| holds_every_word(&comment.body))
            })
            .collect();
        Ok(SearchResults {
            issues,
            cycles: data.cycles,
            labels: data.labels,
            members: data.members,
            teams: data.teams,
            problems: Vec::new(),
        })
    }

    async fn teams(&self) -> Result<Vec<Team>, ClientError> {
        Ok(demo::workspace().teams)
    }

    async fn team_options(&self, team_id: &str) -> Result<TeamOptions, ClientError> {
        Ok(demo::team_options(&demo::workspace(), team_id))
    }

    async fn team_issues(
        &self,
        query: &TeamQuery,
        after: Option<&str>,
    ) -> Result<TeamIssuesPage, ClientError> {
        let data = demo_workspace_with_relations();
        let matching = demo::team_issues(&data, query);
        let start = after.and_then(|cursor| cursor.parse().ok()).unwrap_or(0);
        let end = (start + FAKE_PAGE_SIZE).min(matching.len());
        let issues: Vec<Issue> = matching.get(start..end).unwrap_or_default().to_vec();
        let cycles = data
            .cycles
            .into_iter()
            .filter(|cycle| {
                issues
                    .iter()
                    .any(|issue| issue.cycle_id.as_ref() == Some(&cycle.id))
            })
            .collect();
        let members = data
            .members
            .into_iter()
            .filter(|member| {
                issues.iter().any(|issue| {
                    issue.assignee_id.as_ref() == Some(&member.id)
                        || issue.creator_id.as_ref() == Some(&member.id)
                })
            })
            .collect();
        Ok(TeamIssuesPage {
            issues,
            cycles,
            members,
            next: (end < matching.len()).then(|| end.to_string()),
            as_of: demo::NOW,
            problems: Vec::new(),
        })
    }

    async fn count_team_issues(&self, query: &TeamQuery) -> Result<usize, ClientError> {
        Ok(demo::team_issues(&demo_workspace_with_relations(), query).len())
    }

    async fn recent_cycles(&self, team_id: &str) -> Result<Vec<Cycle>, ClientError> {
        let since = demo::NOW - SignedDuration::from_hours(12 * 7 * 24);
        Ok(demo::workspace()
            .cycles
            .into_iter()
            .filter(|cycle| cycle.team_id == team_id && cycle.ends_at > since)
            .collect())
    }

    async fn viewer_projects(&self) -> Result<ProjectList, ClientError> {
        Ok(ProjectList {
            projects: demo::project_overviews(&demo::workspace()),
            problems: Vec::new(),
        })
    }

    async fn cycle_issues(&self, cycle_id: &str) -> Result<SearchResults, ClientError> {
        Ok(demo_listed(|issue| {
            issue.cycle_id.as_deref() == Some(cycle_id)
        }))
    }

    async fn project_issues(&self, project_id: &str) -> Result<SearchResults, ClientError> {
        Ok(demo_listed(|issue| {
            issue.project_id.as_deref() == Some(project_id)
        }))
    }

    async fn analytics_issues(
        &self,
        window: AnalyticsWindow,
    ) -> Result<AnalyticsIssues, ClientError> {
        Ok(AnalyticsIssues::demo(window))
    }

    /// The comment by the demo viewer, at [`demo::NOW`], remembered for the
    /// detail of its issue.
    async fn create_comment(
        &self,
        issue_id: &str,
        body: &str,
        parent_id: Option<&str>,
    ) -> Result<PostedComment, ClientError> {
        let data = demo::workspace();
        let known = data.issues.iter().any(|issue| issue.id == issue_id)
            || DEMO_EDITS.with_borrow(|edits| edits.contains_key(issue_id));
        if !known {
            return Err(not_found("issue", issue_id));
        }
        let author = data
            .members
            .iter()
            .find(|member| member.id == data.viewer.id)
            .cloned();
        let comment = DEMO_COMMENTS.with_borrow_mut(|comments| {
            let comment = Comment {
                id: format!("demo-comment-new-{}", comments.len() + 1),
                issue_id: issue_id.to_owned(),
                author_id: Some(data.viewer.id.clone()),
                body: body.to_owned(),
                created_at: demo::NOW,
                parent_id: parent_id.map(str::to_owned),
                resolved_at: None,
                edited_at: None,
                reactions: Vec::new(),
            };
            comments.push(comment.clone());
            comment
        });
        Ok(PostedComment { comment, author })
    }

    async fn update_comment(&self, id: &str, body: &str) -> Result<(), ClientError> {
        demo_change_comment(id, |comment| {
            comment.body = body.to_owned();
            comment.edited_at = Some(demo::NOW);
        })
    }

    async fn delete_comment(&self, id: &str) -> Result<(), ClientError> {
        demo_comment(id)?;
        DEMO_COMMENT_CHANGES.with_borrow_mut(|changes| {
            changes.deleted.push(id.to_owned());
        });
        Ok(())
    }

    async fn resolve_comment(&self, id: &str, resolved: bool) -> Result<(), ClientError> {
        demo_change_comment(id, |comment| {
            comment.resolved_at = resolved.then_some(demo::NOW);
        })
    }

    async fn add_reaction(
        &self,
        target: &ReactionTarget,
        emoji: &str,
    ) -> Result<Reaction, ClientError> {
        match target {
            ReactionTarget::Issue(id) => {
                demo_issue(&demo::workspace(), id)?;
            }
            ReactionTarget::Comment(id) => {
                demo_comment(id)?;
            }
        }
        let viewer_id = demo::workspace().viewer.id;
        DEMO_REACTIONS.with_borrow_mut(|(added, _)| {
            let reaction = Reaction {
                id: format!("reaction-new-{}", added.len() + 1),
                emoji: emoji.to_owned(),
                user_id: Some(viewer_id),
                created_at: demo::NOW,
            };
            added.push((target.clone(), reaction.clone()));
            Ok(reaction)
        })
    }

    async fn remove_reaction(&self, id: &str) -> Result<(), ClientError> {
        let data = demo::workspace();
        let known = data
            .issue_reactions
            .values()
            .flatten()
            .chain(data.comments.iter().flat_map(|comment| &comment.reactions))
            .any(|reaction| reaction.id == id)
            || DEMO_REACTIONS.with_borrow(|(added, _)| added.iter().any(|(_, r)| r.id == id));
        if !known {
            return Err(not_found("reaction", id));
        }
        DEMO_REACTIONS.with_borrow_mut(|(_, removed)| removed.push(id.to_owned()));
        Ok(())
    }

    async fn create_issue(&self, input: &IssueCreate) -> Result<Issue, ClientError> {
        let data = demo::workspace();
        let created = DEMO_CREATED.with_borrow(|created| *created) + 1;
        let issue = demo_create(&data, input, created)?;
        DEMO_CREATED.with_borrow_mut(|count| *count = created);
        DEMO_EDITS.with_borrow_mut(|edits| edits.insert(issue.id.clone(), issue.clone()));
        Ok(issue)
    }

    async fn administrable_teams(&self) -> Result<Vec<String>, ClientError> {
        Ok(demo::administrable_team_ids())
    }

    async fn create_relation(&self, input: &RelationCreate) -> Result<IssueRelation, ClientError> {
        let data = demo::workspace();
        for id in [&input.issue_id, &input.related_issue_id] {
            if !demo_has_issue(&data, id) {
                return Err(not_found("issue", id));
            }
        }
        Ok(DEMO_RELATIONS.with_borrow_mut(|(created, _)| {
            let relation = IssueRelation {
                id: format!("demo-relation-new-{}", created.len() + 1),
                kind: input.kind,
                issue_id: input.issue_id.clone(),
                related_issue_id: input.related_issue_id.clone(),
            };
            created.push(relation.clone());
            relation
        }))
    }

    async fn delete_relation(&self, id: &str) -> Result<(), ClientError> {
        let known = demo::workspace()
            .relations
            .iter()
            .any(|relation| relation.id == id)
            || DEMO_RELATIONS.with_borrow(|(created, _)| created.iter().any(|r| r.id == id));
        if !known {
            return Err(not_found("issue relation", id));
        }
        DEMO_RELATIONS.with_borrow_mut(|(created, deleted)| {
            created.retain(|relation| relation.id != id);
            deleted.push(id.to_owned());
        });
        Ok(())
    }

    async fn team_analytics(
        &self,
        team_id: &str,
        window: AnalyticsWindow,
    ) -> Result<TeamAnalyticsIssues, ClientError> {
        TeamAnalyticsIssues::demo(team_id, window)
    }

    async fn issue_by_key(
        &self,
        team_key: &str,
        number: u32,
    ) -> Result<Option<IssueLink>, ClientError> {
        let identifier = format!("{team_key}-{number}");
        Ok(demo::workspace()
            .issues
            .into_iter()
            .find(|issue| issue.identifier.eq_ignore_ascii_case(&identifier))
            .map(|issue| IssueLink {
                identifier: issue.identifier,
                title: issue.title,
                url: issue.url,
            }))
    }

    async fn diagnostics(&self) -> Result<Diagnostics, ClientError> {
        Ok(Diagnostics {
            role: Role::default(),
            rate_limits: vec![
                RateLimit {
                    kind: "requests".to_owned(),
                    allowed: 5_000.0,
                    remaining: 4_990.0,
                    reset: demo::NOW.checked_add(SignedDuration::from_mins(40)).ok(),
                },
                RateLimit {
                    kind: "complexity".to_owned(),
                    allowed: 3_000_000.0,
                    remaining: 2_996_000.0,
                    reset: demo::NOW.checked_add(SignedDuration::from_mins(40)).ok(),
                },
            ],
            server_time: Some(demo::NOW),
        })
    }

    /// The file attached under a made-up upload URL, its bytes reported
    /// sent at once and never read.
    async fn attach_file(
        &self,
        issue_id: &str,
        file: &FileUpload,
        mut progress: impl FnMut(u64) + Send + Sync + 'static,
    ) -> Result<Attachment, ClientError> {
        let url = format!("https://uploads.linear.app/demo/{}/{}", issue_id, file.name);
        let attachment = demo_attach(issue_id, &url, &file.name)?;
        progress(file.size);
        Ok(attachment)
    }

    /// The link attached, titled with its host when no title is given, as
    /// Linear names a link.
    async fn attach_link(
        &self,
        issue_id: &str,
        url: &str,
        title: Option<&str>,
    ) -> Result<Attachment, ClientError> {
        let host = url
            .split_once("://")
            .map_or(url, |(_, rest)| rest)
            .split(['/', '?', '#'])
            .next()
            .unwrap_or(url);
        demo_attach(issue_id, url, title.unwrap_or(host))
    }

    /// The `updatedAt` of the issue as edits through this client left it.
    async fn issue_updated_at(&self, issue_id: &str) -> Result<Timestamp, ClientError> {
        DEMO_EDITS
            .with_borrow(|edits| edits.get(issue_id).map(|issue| issue.updated_at))
            .or_else(|| {
                demo::workspace()
                    .issues
                    .into_iter()
                    .find(|issue| issue.id == issue_id)
                    .map(|issue| issue.updated_at)
            })
            .ok_or_else(|| not_found("issue", issue_id))
    }
}

/// The demo issues `keep` picks, most recently updated first, with what
/// they refer to.
fn demo_listed(keep: impl Fn(&Issue) -> bool) -> SearchResults {
    let data = demo::workspace();
    let mut issues: Vec<Issue> = data
        .issues
        .into_iter()
        .filter(|issue| keep(issue))
        .collect();
    issues.sort_by_key(|issue| Reverse(issue.updated_at));
    SearchResults {
        issues,
        cycles: data.cycles,
        labels: data.labels,
        members: data.members,
        teams: data.teams,
        problems: Vec::new(),
    }
}

/// How many issues a page from [`FakeClient`] holds, as the real client asks
/// Linear for.
const FAKE_PAGE_SIZE: usize = 50;
