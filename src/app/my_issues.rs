use jiff::Timestamp;

use super::{Action, Names};
use crate::linear::model::{Cycle, Issue, Label, Member, StateType, Team, Viewer};

/// The state types My Issues shows, in the order its groups appear.
const GROUP_ORDER: [StateType; 4] = [
    StateType::Started,
    StateType::Unstarted,
    StateType::Backlog,
    StateType::Triage,
];

/// The viewer's open issues, grouped by state type, with the focused issue,
/// how far the list is scrolled and which pane has focus.
#[derive(Debug, Default)]
pub struct MyIssues {
    groups: Vec<Group>,
    focused: usize,
    /// The first list line on screen, counting group headers.
    scroll: usize,
    pane: Pane,
    names: Names,
}

/// The open issues of one state type, active-cycle issues first.
#[derive(Debug, Clone, PartialEq)]
pub struct Group {
    pub state_type: StateType,
    pub issues: Vec<Issue>,
}

/// The two panes of My Issues. On a narrow terminal only the focused one shows.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum Pane {
    #[default]
    List,
    Detail,
}

impl MyIssues {
    /// Picks the issues assigned to `viewer` that are still open and groups
    /// them. A cycle is active when `now` falls inside it.
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
        let groups = GROUP_ORDER
            .into_iter()
            .filter_map(|state_type| {
                let mut issues: Vec<Issue> = issues
                    .iter()
                    .filter(|issue| issue.assignee_id.as_ref() == Some(&viewer.id))
                    .filter(|issue| issue.state.state_type == state_type)
                    .cloned()
                    .collect();
                issues.sort_by_key(|issue| {
                    (
                        !names.is_in_active_cycle(issue),
                        issue.priority,
                        issue.created_at,
                    )
                });
                (!issues.is_empty()).then_some(Group { state_type, issues })
            })
            .collect();
        Self {
            groups,
            focused: 0,
            scroll: 0,
            pane: Pane::List,
            names,
        }
    }

    /// Takes the focused issue, the pane and the scroll of `previous`, an
    /// earlier build of the list, and brings the focus on screen in a list
    /// of `rows` lines. When the focused issue is no longer listed, the
    /// focus stays at its position, on the issue that took its place.
    pub fn keep_place_of(&mut self, previous: &MyIssues, rows: usize) {
        let index = previous
            .focused_issue()
            .and_then(|focused| self.issues().position(|issue| issue.id == focused.id))
            .unwrap_or(previous.focused);
        self.focused = index.min(self.len().saturating_sub(1));
        self.scroll = previous.scroll;
        self.pane = if self.len() == 0 {
            Pane::List
        } else {
            previous.pane
        };
        self.scroll_into_view(rows);
    }

    /// The non-empty groups: started, unstarted, backlog, then triage.
    pub fn groups(&self) -> &[Group] {
        &self.groups
    }

    /// Position of the focused issue across all groups, top to bottom.
    pub fn focused_index(&self) -> usize {
        self.focused
    }

    /// The issue the list cursor is on, `None` when the list is empty.
    pub fn focused_issue(&self) -> Option<&Issue> {
        self.issues().nth(self.focused)
    }

    /// The list line of the focused issue, counting the header above each
    /// group.
    pub fn focused_line(&self) -> usize {
        let mut headers = 0;
        let mut first = 0;
        for group in &self.groups {
            headers += 1;
            if self.focused < first + group.issues.len() {
                break;
            }
            first += group.issues.len();
        }
        self.focused + headers
    }

    /// Moves the focus to the issue `id`. Returns whether the list holds it.
    pub fn focus_id(&mut self, id: &str) -> bool {
        let found = self.issues().position(|issue| issue.id == id);
        if let Some(index) = found {
            self.focused = index;
        }
        found.is_some()
    }

    /// The first list line on screen, counting group headers.
    pub fn scroll(&self) -> usize {
        self.scroll
    }

    /// Which pane has focus.
    pub fn pane(&self) -> Pane {
        self.pane
    }

    /// The names its rows and detail look up.
    pub fn names(&self) -> &Names {
        &self.names
    }

    /// Whether `issue` belongs to a cycle running now.
    pub fn is_in_active_cycle(&self, issue: &Issue) -> bool {
        self.names.is_in_active_cycle(issue)
    }

    /// Carries out a navigation action on a list that shows `rows` lines at
    /// once. Half a page is half of `rows`. Returns whether anything changed.
    pub fn perform(&mut self, action: Action, rows: usize) -> bool {
        let changed = self.navigate(action, (rows / 2).max(1));
        self.scroll_into_view(rows) || changed
    }

    /// Scrolls the least that brings the focused issue on screen in a list
    /// of `rows` lines, showing its group header when it is the group's
    /// first issue. Returns whether the scroll moved.
    pub fn scroll_into_view(&mut self, rows: usize) -> bool {
        if rows == 0 {
            return false;
        }
        let line = self.focused_line();
        let top = if self.is_first_in_group() {
            line.saturating_sub(1)
        } else {
            line
        };
        let scroll = if top < self.scroll {
            top
        } else if line >= self.scroll + rows {
            line + 1 - rows
        } else {
            self.scroll
        };
        let changed = scroll != self.scroll;
        self.scroll = scroll;
        changed
    }

    fn navigate(&mut self, action: Action, page: usize) -> bool {
        let last = self.len().saturating_sub(1);
        match (self.pane, action) {
            (Pane::List, Action::MoveDown) => self.focus_issue(self.focused.saturating_add(1)),
            (Pane::List, Action::MoveUp) => self.focus_issue(self.focused.saturating_sub(1)),
            (Pane::List, Action::MoveToTop) => self.focus_issue(0),
            (Pane::List, Action::MoveToBottom) => self.focus_issue(last),
            (Pane::List, Action::HalfPageDown) => {
                self.focus_issue(self.focused.saturating_add(page))
            }
            (Pane::List, Action::HalfPageUp) => self.focus_issue(self.focused.saturating_sub(page)),
            (Pane::List, Action::Open | Action::FocusRight) if self.len() > 0 => {
                self.pane = Pane::Detail;
                true
            }
            (Pane::Detail, Action::Close | Action::FocusLeft) => {
                self.pane = Pane::List;
                true
            }
            _ => false,
        }
    }

    fn focus_issue(&mut self, index: usize) -> bool {
        let index = index.min(self.len().saturating_sub(1));
        let changed = index != self.focused;
        self.focused = index;
        changed
    }

    fn is_first_in_group(&self) -> bool {
        let mut first = 0;
        for group in &self.groups {
            if self.focused == first {
                return true;
            }
            first += group.issues.len();
        }
        false
    }

    fn issues(&self) -> impl Iterator<Item = &Issue> {
        self.groups.iter().flat_map(|group| &group.issues)
    }

    fn len(&self) -> usize {
        self.groups.iter().map(|group| group.issues.len()).sum()
    }
}
