//! A grouped issue list on screen: its groups, the focused issue, how far it
//! is scrolled and which pane has focus. My Issues, Team Issues and Results
//! all move through one.

use super::{Action, Group};
use crate::linear::model::Issue;

/// The two panes of an issue list screen. On a narrow terminal only the
/// focused one shows.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum Pane {
    #[default]
    List,
    Detail,
}

#[derive(Debug, Default)]
pub struct IssueList {
    groups: Vec<Group>,
    focused: usize,
    /// The first list line on screen, counting group headers.
    scroll: usize,
    pane: Pane,
}

impl IssueList {
    /// `groups` with the first issue focused.
    pub fn new(groups: Vec<Group>) -> Self {
        Self {
            groups,
            ..Self::default()
        }
    }

    /// Takes the focused issue, the pane and the scroll of `previous`, an
    /// earlier build of the list, and brings the focus on screen in a list
    /// of `rows` lines. When the focused issue is no longer listed, the
    /// focus stays at its position, on the issue that took its place.
    pub fn keep_place_of(&mut self, previous: &IssueList, rows: usize) {
        let index = previous
            .focused_issue()
            .and_then(|focused| self.issues().position(|issue| issue.id == focused.id))
            .unwrap_or(previous.focused);
        self.focused = index.min(self.len().saturating_sub(1));
        self.scroll = previous.scroll;
        self.pane = if self.is_empty() {
            Pane::List
        } else {
            previous.pane
        };
        self.scroll_into_view(rows);
    }

    /// The non-empty groups, in order.
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
    /// group that has one.
    pub fn focused_line(&self) -> usize {
        let mut headers = 0;
        let mut first = 0;
        for group in &self.groups {
            headers += usize::from(group.key.has_header());
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

    /// Every issue, top to bottom.
    pub fn issues(&self) -> impl Iterator<Item = &Issue> {
        self.groups.iter().flat_map(|group| &group.issues)
    }

    /// How many issues the list holds.
    pub fn len(&self) -> usize {
        self.groups.iter().map(|group| group.issues.len()).sum()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
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
        let top = if self.is_first_under_header() {
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

    /// Focuses the first issue and the list pane, scrolled to the top.
    pub fn reset(&mut self) {
        self.focused = 0;
        self.scroll = 0;
        self.pane = Pane::List;
    }

    fn navigate(&mut self, action: Action, page: usize) -> bool {
        let last = self.len().saturating_sub(1);
        match (self.pane, action) {
            (Pane::List, Action::MoveDown) => self.focus(self.focused.saturating_add(1)),
            (Pane::List, Action::MoveUp) => self.focus(self.focused.saturating_sub(1)),
            (Pane::List, Action::MoveToTop) => self.focus(0),
            (Pane::List, Action::MoveToBottom) => self.focus(last),
            (Pane::List, Action::HalfPageDown) => self.focus(self.focused.saturating_add(page)),
            (Pane::List, Action::HalfPageUp) => self.focus(self.focused.saturating_sub(page)),
            (Pane::List, Action::Open | Action::FocusRight) if !self.is_empty() => {
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

    fn focus(&mut self, index: usize) -> bool {
        let index = index.min(self.len().saturating_sub(1));
        let changed = index != self.focused;
        self.focused = index;
        changed
    }

    fn is_first_under_header(&self) -> bool {
        let mut first = 0;
        for group in &self.groups {
            if self.focused == first {
                return group.key.has_header();
            }
            first += group.issues.len();
        }
        false
    }
}
