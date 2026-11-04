//! Search: `/`, a query, and Enter to ask Linear for every issue in the
//! workspace that matches it. The results show as a list, and Enter opens
//! one in its detail.

use super::update::Update;
use super::{Action, Effect, Modal, State};
use crate::linear::SearchResults;

/// The query and how far its search is.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Search {
    pub query: String,
    pub phase: Phase,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Phase {
    /// The viewer is typing. `problem` says why the last search failed.
    Typing { problem: Option<String> },
    /// Linear has been asked for `term`.
    Searching { term: String },
    /// What Linear found for `term`, as issue ids best first, and the
    /// highlighted one.
    Found {
        term: String,
        ids: Vec<String>,
        selected: usize,
    },
}

impl Default for Phase {
    fn default() -> Self {
        Self::Typing { problem: None }
    }
}

impl Search {
    /// The query while the viewer can still edit it.
    pub(super) fn text_field(&mut self) -> Option<&mut String> {
        match self.phase {
            Phase::Typing { .. } => Some(&mut self.query),
            Phase::Searching { .. } | Phase::Found { .. } => None,
        }
    }
}

/// Asks Linear for the query, or opens the highlighted result in its
/// detail. An empty query asks for nothing.
pub(super) fn accept(state: &mut State) -> Update {
    let Some(Modal::Search(search)) = state.modals.top_mut() else {
        return Update::default();
    };
    match &search.phase {
        Phase::Typing { .. } => {
            let term = search.query.trim().to_owned();
            if term.is_empty() {
                return Update::default();
            }
            search.phase = Phase::Searching { term: term.clone() };
            Update {
                redraw: true,
                effects: vec![Effect::Search(term)],
            }
        }
        Phase::Found { ids, selected, .. } => match ids.get(*selected) {
            Some(id) => {
                let id = id.clone();
                state.modals.push(Modal::Issue { id });
                Update::redraw()
            }
            None => Update::default(),
        },
        Phase::Searching { .. } => Update::default(),
    }
}

/// Moves the highlight one result down or up, within the results.
pub(super) fn step(state: &mut State, action: Action) -> Update {
    let Some(Modal::Search(Search {
        phase: Phase::Found { ids, selected, .. },
        ..
    })) = state.modals.top_mut()
    else {
        return Update::default();
    };
    let before = *selected;
    *selected = match action {
        Action::ScrollDown => selected.saturating_add(1).min(ids.len().saturating_sub(1)),
        _ => selected.saturating_sub(1),
    };
    Update {
        redraw: *selected != before,
        effects: Vec::new(),
    }
}

/// Takes in what Linear answered for `term`. Found issues go into the store
/// whatever happened to the search since; the overlay shows them only if it
/// is still waiting on `term`, so a slow answer to an older search cannot
/// replace a newer one.
pub(super) fn finish(state: &mut State, term: &str, result: Result<SearchResults, String>) {
    if let Ok(found) = &result {
        if state.notice.is_none() {
            state.notice = super::state::summary(&found.problems);
        }
        state.store.merge_search(found);
        state.rebuild_my_issues();
    }
    let search = state.modals.iter_mut().rev().find_map(|modal| match modal {
        Modal::Search(search) => Some(search),
        _ => None,
    });
    let Some(search) = search else {
        return;
    };
    if !matches!(&search.phase, Phase::Searching { term: waiting } if waiting == term) {
        return;
    }
    search.phase = match result {
        Ok(found) => Phase::Found {
            term: term.to_owned(),
            ids: found.issues.into_iter().map(|issue| issue.id).collect(),
            selected: 0,
        },
        Err(problem) => Phase::Typing {
            problem: Some(problem),
        },
    };
}
