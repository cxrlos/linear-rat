//! Results: what a search found, or the one issue a palette jump went to,
//! shown in the normal panes so every issue action works on the focused one.
//! Esc from the list goes back to the screen they were opened from.

use super::{Action, Names, Pane, Screen, State};

/// The issues shown, by id in the order they came, with where the focus is
/// and the screen to go back to. The issues themselves live in the store.
#[derive(Debug, Default)]
pub struct Results {
    title: String,
    ids: Vec<String>,
    focused: usize,
    scroll: usize,
    pane: Pane,
    names: Names,
    back: Screen,
}

impl Results {
    /// What the list pane is titled, as `Search "cheese"`.
    pub fn title(&self) -> &str {
        &self.title
    }

    /// The ids of the issues shown, in order.
    pub fn ids(&self) -> &[String] {
        &self.ids
    }

    /// Position of the focused issue in the list.
    pub fn focused_index(&self) -> usize {
        self.focused
    }

    /// The id of the issue the list cursor is on.
    pub fn focused_id(&self) -> Option<&str> {
        self.ids.get(self.focused).map(String::as_str)
    }

    /// The first list row on screen.
    pub fn scroll(&self) -> usize {
        self.scroll
    }

    pub fn pane(&self) -> Pane {
        self.pane
    }

    /// The names its rows and detail look up.
    pub fn names(&self) -> &Names {
        &self.names
    }

    /// The screen Esc from the list goes back to.
    pub fn back(&self) -> Screen {
        self.back
    }

    /// Carries out a navigation action on a list that shows `rows` rows at
    /// once. Returns whether anything changed.
    pub fn perform(&mut self, action: Action, rows: usize) -> bool {
        let page = (rows / 2).max(1);
        let last = self.ids.len().saturating_sub(1);
        let changed = match (self.pane, action) {
            (Pane::List, Action::MoveDown) => self.focus(self.focused.saturating_add(1)),
            (Pane::List, Action::MoveUp) => self.focus(self.focused.saturating_sub(1)),
            (Pane::List, Action::MoveToTop) => self.focus(0),
            (Pane::List, Action::MoveToBottom) => self.focus(last),
            (Pane::List, Action::HalfPageDown) => self.focus(self.focused.saturating_add(page)),
            (Pane::List, Action::HalfPageUp) => self.focus(self.focused.saturating_sub(page)),
            (Pane::List, Action::Open | Action::FocusRight) if !self.ids.is_empty() => {
                self.pane = Pane::Detail;
                true
            }
            (Pane::Detail, Action::Close | Action::FocusLeft) => {
                self.pane = Pane::List;
                true
            }
            _ => false,
        };
        self.scroll_into_view(rows) || changed
    }

    /// Scrolls the least that brings the focused issue on screen in a list
    /// of `rows` rows. Returns whether the scroll moved.
    pub fn scroll_into_view(&mut self, rows: usize) -> bool {
        if rows == 0 {
            return false;
        }
        let scroll = if self.focused < self.scroll {
            self.focused
        } else if self.focused >= self.scroll + rows {
            self.focused + 1 - rows
        } else {
            self.scroll
        };
        let changed = scroll != self.scroll;
        self.scroll = scroll;
        changed
    }

    fn focus(&mut self, index: usize) -> bool {
        let index = index.min(self.ids.len().saturating_sub(1));
        let changed = index != self.focused;
        self.focused = index;
        changed
    }
}

/// Shows `ids` under `title`, the first one focused. Opened from the results
/// themselves, Esc still goes back to where the first results were opened
/// from.
pub(super) fn show(state: &mut State, title: String, ids: Vec<String>) {
    let back = match state.screen {
        Screen::Results => state.results.back,
        screen => screen,
    };
    let store = &state.store;
    let names = Names::new(
        store.cycles(),
        store.labels(),
        store.members(),
        store.teams(),
        store.as_of().unwrap_or_default(),
    );
    state.results = Results {
        title,
        ids,
        names,
        back,
        ..Results::default()
    };
    state.screen = Screen::Results;
}

/// Esc from the list: back to the screen the results were opened from,
/// with its focus as it was.
pub(super) fn close(state: &mut State) {
    state.screen = state.results.back;
}
