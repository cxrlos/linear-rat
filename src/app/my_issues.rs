use std::ops::{Deref, DerefMut};

use jiff::Timestamp;

use super::{Arrangement, IssueList, Names, arrange};
use crate::linear::model::{Cycle, Issue, Label, Member, Team, Viewer};

/// The viewer's open issues, grouped and sorted as the viewer chose (by
/// state type, active-cycle issues first, unless they chose otherwise), and
/// the list on screen.
#[derive(Debug)]
pub struct MyIssues {
    list: IssueList,
    pub(super) arrangement: Arrangement,
    names: Names,
}

impl Default for MyIssues {
    fn default() -> Self {
        Self {
            list: IssueList::default(),
            arrangement: Arrangement::MY_ISSUES,
            names: Names::default(),
        }
    }
}

impl Deref for MyIssues {
    type Target = IssueList;

    fn deref(&self) -> &IssueList {
        &self.list
    }
}

impl DerefMut for MyIssues {
    fn deref_mut(&mut self) -> &mut IssueList {
        &mut self.list
    }
}

impl MyIssues {
    /// Picks the issues assigned to `viewer` that are still open and groups
    /// them by state type. A cycle is active when `now` falls inside it.
    pub fn new(
        viewer: &Viewer,
        issues: &[Issue],
        cycles: &[Cycle],
        labels: &[Label],
        members: &[Member],
        teams: &[Team],
        now: Timestamp,
    ) -> Self {
        let names = Names::new(cycles, labels, members, teams, now);
        Self::arranged(viewer, issues, names, Arrangement::MY_ISSUES)
    }

    /// The issues assigned to `viewer` that are still open, arranged as
    /// `arrangement` says.
    pub fn arranged(
        viewer: &Viewer,
        issues: &[Issue],
        names: Names,
        arrangement: Arrangement,
    ) -> Self {
        let mut mine: Vec<Issue> = issues
            .iter()
            .filter(|issue| issue.assignee_id.as_ref() == Some(&viewer.id))
            .filter(|issue| issue.state.state_type.is_open())
            .cloned()
            .collect();
        mine.sort_by_key(|issue| {
            (
                !names.is_in_active_cycle(issue),
                issue.priority,
                issue.created_at,
            )
        });
        Self {
            list: IssueList::new(arrange(mine, arrangement, &names)),
            arrangement,
            names,
        }
    }

    /// Takes the focused issue, the pane and the scroll of `previous`, as
    /// [`IssueList::keep_place_of`] does.
    pub fn keep_place_of(&mut self, previous: &MyIssues, rows: usize) {
        self.list.keep_place_of(&previous.list, rows);
    }

    /// How the list is grouped and sorted.
    pub fn arrangement(&self) -> Arrangement {
        self.arrangement
    }

    /// The names its rows and detail look up.
    pub fn names(&self) -> &Names {
        &self.names
    }

    /// Whether `issue` belongs to a cycle running now.
    pub fn is_in_active_cycle(&self, issue: &Issue) -> bool {
        self.names.is_in_active_cycle(issue)
    }
}
