//! Everything lrat needs from Linear goes through [`LinearClient`], so tests
//! and `--demo` swap the real client for [`FakeClient`].

pub mod demo;
mod error;
mod governor;
pub mod http;
pub mod model;
pub(crate) mod search_bucket;

use std::cell::RefCell;
use std::cmp::Reverse;
use std::collections::BTreeMap;
use std::future::Future;

use jiff::{SignedDuration, Timestamp};

pub use error::{ClientError, GraphQlError, Response};
pub use governor::{SystemTimer, Timer, Urgency};
pub use http::{ClientConfig, HttpClient};
use model::{
    Attachment, Comment, Cycle, Issue, Label, Member, Priority, Project, StateSpan, StateType,
    Team, Viewer, WorkflowState, Workspace,
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

    /// Sends one `issueUpdate` for the issue with id `issue_id` and returns
    /// the issue as Linear stored it.
    fn update_issue(
        &self,
        issue_id: &str,
        update: &IssueUpdate,
    ) -> impl Future<Output = Result<Issue, ClientError>> + Send;

    /// What the detail of the issue with id `issue_id` shows beyond what a
    /// list loads: its comments and its workflow state history. `urgency`
    /// says whether the viewer opened the detail and waits on it.
    fn issue_detail(
        &self,
        issue_id: &str,
        urgency: Urgency,
    ) -> impl Future<Output = Result<IssueDetail, ClientError>> + Send;

    /// The issues anywhere in the workspace that match `term`, best match
    /// first, closed ones included. Search has its own budget of 30 a
    /// minute; a search beyond it waits its turn rather than failing.
    fn search_issues(
        &self,
        term: &str,
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

    /// One page of the issues of `query`, most recently updated first,
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

    /// What personal analytics needs for `window`: the issues assigned to
    /// the viewer that were created or completed in it, the viewer's teams
    /// and their cycles that overlap it. A window that reaches now also
    /// brings every in-progress issue with its state history.
    fn analytics_issues(
        &self,
        window: AnalyticsWindow,
    ) -> impl Future<Output = Result<AnalyticsIssues, ClientError>> + Send;
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

/// What a search found: the issues, best match first, and the cycles,
/// labels, members and teams they refer to.
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

/// A team's issues, narrowed by a filter.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TeamQuery {
    pub team_id: String,
    pub filter: TeamFilter,
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
}

impl TeamFilter {
    /// Whether no field is set.
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }

    /// Whether `issue` matches every field that is set.
    pub fn matches(&self, issue: &Issue) -> bool {
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
    }
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

/// One issue's comments and workflow state history, and the people, states
/// and project they refer to.
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
    /// The first 50 attachments, oldest first; any more are left out.
    pub attachments: Vec<Attachment>,
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
        Self {
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
            attachments: data
                .attachments
                .into_iter()
                .filter(|attachment| attachment.issue_id == issue_id)
                .collect(),
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
    issue.updated_at += SignedDuration::from_secs(1);
    Ok(issue)
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
            .filter(|id| data.issues.iter().any(|issue| &issue.id == *id))
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

    async fn update_issue(
        &self,
        issue_id: &str,
        update: &IssueUpdate,
    ) -> Result<Issue, ClientError> {
        let data = demo::workspace();
        let stored = DEMO_EDITS
            .with_borrow(|edits| edits.get(issue_id).cloned())
            .or_else(|| {
                data.issues
                    .iter()
                    .find(|issue| issue.id == issue_id)
                    .cloned()
            })
            .ok_or_else(|| not_found("issue", issue_id))?;
        let updated = demo_update(&data, stored, update)?;
        DEMO_EDITS.with_borrow_mut(|edits| edits.insert(issue_id.to_owned(), updated.clone()));
        Ok(updated)
    }

    async fn issue_detail(&self, issue_id: &str, _: Urgency) -> Result<IssueDetail, ClientError> {
        Ok(IssueDetail::demo(issue_id))
    }

    /// Every demo issue whose identifier, title or description holds each
    /// word of `term`, ignoring case, in the demo's order.
    async fn search_issues(&self, term: &str) -> Result<SearchResults, ClientError> {
        let words: Vec<String> = term.split_whitespace().map(str::to_lowercase).collect();
        let data = demo::workspace();
        let issues = data
            .issues
            .into_iter()
            .filter(|issue| {
                let text = format!(
                    "{} {} {}",
                    issue.identifier,
                    issue.title,
                    issue.description.as_deref().unwrap_or_default()
                )
                .to_lowercase();
                words.iter().all(|word| text.contains(word.as_str()))
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
        let data = demo::workspace();
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
        Ok(demo::team_issues(&demo::workspace(), query).len())
    }

    async fn analytics_issues(
        &self,
        window: AnalyticsWindow,
    ) -> Result<AnalyticsIssues, ClientError> {
        Ok(AnalyticsIssues::demo(window))
    }
}

/// How many issues a page from [`FakeClient`] holds, as the real client asks
/// Linear for.
const FAKE_PAGE_SIZE: usize = 50;
