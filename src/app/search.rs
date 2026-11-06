//! Search: `/`, a query, and Enter to ask Linear for every issue in the
//! workspace that matches it. The results open in the results view.

use super::update::Update;
use super::{Effect, Modal, State, results};
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
    /// Linear found nothing for `term`. The query can still be edited and
    /// sent again.
    NothingFound { term: String },
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
            Phase::Typing { .. } | Phase::NothingFound { .. } => Some(&mut self.query),
            Phase::Searching { .. } => None,
        }
    }
}

/// Asks Linear for the query. An empty query asks for nothing.
pub(super) fn accept(state: &mut State) -> Update {
    let Some(Modal::Search(search)) = state.modals.top_mut() else {
        return Update::default();
    };
    if matches!(search.phase, Phase::Searching { .. }) {
        return Update::default();
    }
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

/// Takes in what Linear answered for `term`. Found issues go into the store
/// whatever happened to the search since; they open in the results view,
/// closing the overlay, only if it is still waiting on `term`, so a slow
/// answer to an older search cannot replace a newer one.
pub(super) fn finish(state: &mut State, term: &str, result: Result<SearchResults, String>) {
    if let Ok(found) = &result {
        if state.notice.is_none() {
            state.notice = super::state::summary(&found.problems);
        }
        state.store.merge_search(found);
        state.rebuild_my_issues();
    }
    let waiting = state.modals.iter_mut().rev().find_map(|modal| match modal {
        Modal::Search(search) if matches!(&search.phase, Phase::Searching { term: waiting } if waiting == term) => {
            Some(search)
        }
        _ => None,
    });
    let Some(search) = waiting else {
        return;
    };
    let ids: Vec<String> = match result {
        Ok(found) if found.issues.is_empty() => {
            search.phase = Phase::NothingFound {
                term: term.to_owned(),
            };
            return;
        }
        Ok(found) => found.issues.into_iter().map(|issue| issue.id).collect(),
        Err(problem) => {
            search.phase = Phase::Typing {
                problem: Some(problem),
            };
            return;
        }
    };
    state
        .modals
        .retain(|modal| !matches!(modal, Modal::Search(_)));
    results::show(state, format!("Search \"{term}\""), ids);
}
