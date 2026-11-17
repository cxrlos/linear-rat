//! Results: what a search found, the one issue a palette jump went to, or
//! the issues of a cycle or project, shown in the normal panes so every issue action works on the focused one.
//! Esc from the list goes back to the screen they were opened from.

use super::{Action, Arrangement, IssueList, Names, Pane, Screen, State, arrange};
use crate::linear::model::Issue;
use crate::ui;

/// The issues shown, by id in the order they came, as the list shows them,
/// and the screen to go back to. The issues themselves live in the store.
#[derive(Debug)]
pub struct Results {
    title: String,
    ids: Vec<String>,
    list: IssueList,
    arrangement: Arrangement,
    names: Names,
    back: Screen,
}

impl Default for Results {
    fn default() -> Self {
        Self {
            title: String::new(),
            ids: Vec::new(),
            list: IssueList::default(),
            arrangement: Arrangement::LINEAR_ORDER,
            names: Names::default(),
            back: Screen::default(),
        }
    }
}

impl Results {
    /// What the list pane is titled, as `Search "cheese"`.
    pub fn title(&self) -> &str {
        &self.title
    }

    /// The ids of the issues shown, in the order they came.
    pub fn ids(&self) -> &[String] {
        &self.ids
    }

    /// The issues as they show.
    pub fn list(&self) -> &IssueList {
        &self.list
    }

    /// The id of the issue the list cursor is on.
    pub fn focused_id(&self) -> Option<&str> {
        self.list.focused_issue().map(|issue| issue.id.as_str())
    }

    pub fn pane(&self) -> Pane {
        self.list.pane()
    }

    /// The names its rows and detail look up.
    pub fn names(&self) -> &Names {
        &self.names
    }

    /// How the list is grouped and sorted.
    pub fn arrangement(&self) -> Arrangement {
        self.arrangement
    }

    /// The screen Esc from the list goes back to.
    pub fn back(&self) -> Screen {
        self.back
    }

    /// Carries out a navigation action on a list that shows `rows` rows at
    /// once. Returns whether anything changed.
    pub fn perform(&mut self, action: Action, rows: usize) -> bool {
        self.list.perform(action, rows)
    }

    /// Scrolls the least that brings the focused issue on screen in a list
    /// of `rows` rows. Returns whether the scroll moved.
    pub fn scroll_into_view(&mut self, rows: usize) -> bool {
        self.list.scroll_into_view(rows)
    }

    /// Builds the list again from `store`, keeping the focused issue. An
    /// issue deleted on Linear since has left the store and the list.
    pub(super) fn rebuild(&mut self, store: &super::Store, rows: usize) {
        let issues: Vec<Issue> = self
            .ids
            .iter()
            .filter_map(|id| store.issue(id))
            .cloned()
            .collect();
        let mut list = IssueList::new(arrange(issues, self.arrangement, &self.names));
        list.keep_place_of(&self.list, rows);
        self.list = list;
    }
}

/// Shows `ids` under `title` in the order they came, the first one
/// focused. Opened from the results themselves, Esc still goes back to
/// where the first results were opened from.
pub(super) fn show(state: &mut State, title: String, ids: Vec<String>) {
    show_arranged(state, title, ids, Arrangement::LINEAR_ORDER);
}

/// Shows `ids` under `title` as `arrangement` says, as [`show`] does.
pub(super) fn show_arranged(
    state: &mut State,
    title: String,
    ids: Vec<String>,
    arrangement: Arrangement,
) {
    let back = match state.screen {
        Screen::Results => state.results.back,
        screen => screen,
    };
    state.results = Results {
        title,
        ids,
        names: Names::from_store(&state.store),
        back,
        arrangement,
        list: IssueList::default(),
    };
    state.screen = Screen::Results;
    rebuild(state);
}

/// Builds the list again from the store, keeping the focus.
pub(super) fn rebuild(state: &mut State) {
    let rows = ui::list_rows(state, state.viewport);
    state.results.rebuild(&state.store, rows);
}

/// Groups and sorts the results as `arrangement` says, keeping the focused
/// issue.
pub(super) fn rearrange(state: &mut State, arrangement: Arrangement) {
    state.results.arrangement = arrangement;
    rebuild(state);
}

/// Esc from the list: back to the screen the results were opened from,
/// with its focus as it was.
pub(super) fn close(state: &mut State) {
    state.screen = state.results.back;
}
