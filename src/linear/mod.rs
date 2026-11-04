//! Everything lrat needs from Linear goes through [`LinearClient`], so tests
//! and `--demo` swap the real client for [`FakeClient`].

pub mod demo;
mod error;
mod governor;
pub mod http;
pub mod model;

use std::cell::RefCell;
use std::cmp::Reverse;
use std::collections::BTreeMap;
use std::future::Future;

use jiff::{SignedDuration, Timestamp};

pub use error::{ClientError, GraphQlError, Response};
pub use governor::{SystemTimer, Timer};
pub use http::{ClientConfig, HttpClient};
use model::{Cycle, Issue, Label, Member, Priority, Team, Viewer, WorkflowState, Workspace};

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

    /// Sends one `issueUpdate` for the issue with id `issue_id` and returns
    /// the issue as Linear stored it.
    fn update_issue(
        &self,
        issue_id: &str,
        update: &IssueUpdate,
    ) -> impl Future<Output = Result<Issue, ClientError>> + Send;
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
}
