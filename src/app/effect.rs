use super::{TeamPoll, Write};
use crate::linear::{TeamQuery, Urgency};

/// A request to the outside world, returned by [`update`](super::update) and
/// carried out by the runtime.
#[derive(Debug, Clone, PartialEq)]
pub enum Effect {
    Quit,
    /// Load the viewer's open assigned issues; the result comes back as
    /// [`Event::MyIssuesLoaded`](super::Event::MyIssuesLoaded).
    LoadMyIssues,
    /// Load the comments and state history of the issue with this id; the
    /// result comes back as
    /// [`Event::IssueDetailLoaded`](super::Event::IssueDetailLoaded). Urgent
    /// when the viewer opened the detail, background when it only follows
    /// the list's focus.
    LoadIssueDetail {
        issue_id: String,
        urgency: Urgency,
    },
    /// Search the workspace for this term; the result comes back as
    /// [`Event::SearchDone`](super::Event::SearchDone).
    Search(String),
    /// Load the viewer's teams; the result comes back as
    /// [`Event::TeamsLoaded`](super::Event::TeamsLoaded).
    LoadTeams,
    /// Load what the filters of this team offer; the result comes back as
    /// [`Event::TeamOptionsLoaded`](super::Event::TeamOptionsLoaded).
    LoadTeamOptions(String),
    /// Load the page of `query`'s issues after the cursor `after`, or the
    /// first page; the result comes back as
    /// [`Event::TeamIssuesLoaded`](super::Event::TeamIssuesLoaded).
    LoadTeamIssues {
        query: TeamQuery,
        after: Option<String>,
    },
    /// Count every issue of the query; the result comes back as
    /// [`Event::TeamIssuesCounted`](super::Event::TeamIssuesCounted).
    CountTeamIssues(TeamQuery),
    /// Put this text on the system clipboard.
    Copy(String),
    /// Open this URL in the viewer's browser.
    OpenUrl(String),
    /// Ask each team for its issues changed since its watermark, one team
    /// after another; the result comes back as
    /// [`Event::Polled`](super::Event::Polled).
    Poll(Vec<TeamPoll>),
    /// Ask which of these issue ids still exist; the result comes back as
    /// [`Event::Reconciled`](super::Event::Reconciled).
    Reconcile(Vec<String>),
    /// Write the store to the snapshot cache.
    SaveSnapshot,
    /// Load a team's workflow states; the result comes back as
    /// [`Event::WorkflowStatesLoaded`](super::Event::WorkflowStatesLoaded).
    LoadWorkflowStates {
        team_id: String,
    },
    /// Load a team's members; the result comes back as
    /// [`Event::TeamMembersLoaded`](super::Event::TeamMembersLoaded).
    LoadTeamMembers {
        team_id: String,
    },
    /// Send one `issueUpdate` making [`Write::edit`]; the result comes back
    /// as [`Event::IssueUpdated`](super::Event::IssueUpdated).
    UpdateIssue(Write),
}
