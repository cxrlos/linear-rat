//! Everything lrat needs from Linear goes through [`LinearClient`], so tests
//! and `--demo` swap the real client for [`FakeClient`].

pub mod demo;
mod error;
mod governor;
pub mod http;
pub mod model;

use std::future::Future;

use jiff::Timestamp;

pub use error::{ClientError, GraphQlError, Response};
pub use governor::{SystemTimer, Timer};
pub use http::{ClientConfig, HttpClient};
use model::{Cycle, Issue, Label, Member, Team, Viewer, Workspace};

/// Queries, mutations and live events.
pub trait LinearClient {
    /// The viewer and the workspace the API key belongs to. Login and
    /// `auth status` use it to check a key.
    fn viewer(&self) -> impl Future<Output = Result<(Viewer, Workspace), ClientError>> + Send;

    /// Every open issue assigned to the viewer, with what My Issues shows
    /// about them.
    fn assigned_issues(&self) -> impl Future<Output = Result<AssignedIssues, ClientError>> + Send;
}

/// The viewer's open assigned issues, and the cycles, labels, members and
/// teams they refer to.
#[derive(Debug, Clone, PartialEq)]
pub struct AssignedIssues {
    pub viewer: Viewer,
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

/// A client that never touches the network.
#[derive(Debug, Default, Clone, Copy)]
pub struct FakeClient;

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
}
