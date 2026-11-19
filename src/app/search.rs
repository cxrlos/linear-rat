//! Search: `/`, a query, and Enter to ask Linear for the issues that match
//! it, across the workspace or within the team and filter Team Issues
//! shows. The results open in the results view.

use super::update::Update;
use super::{Effect, Failure, FilterField, Modal, Screen, State, filter_label, results};
use crate::linear::{SearchQuery, SearchResults, TeamQuery};

/// What a search is called in the error log.
const SEARCH: &str = "search";

/// The query, its options and how far its search is.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Search {
    pub query: String,
    pub phase: Phase,
    /// Whether the term is looked for in comments too. Off when the prompt
    /// opens.
    pub include_comments: bool,
    /// The Team Issues team and filter the prompt was opened over, if it
    /// was; on when the prompt opens.
    pub scope: Option<Scope>,
}

/// The team and filter a search can keep to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Scope {
    pub within: TeamQuery,
    /// The team's key, as `ENG`.
    pub team_key: String,
    /// The filters in force, as `State Todo, Label bug`; empty when none is.
    pub filters: String,
    /// Whether the search keeps to it.
    pub on: bool,
}

impl Scope {
    /// What the prompt says it keeps to, as `In ENG · State Todo`.
    pub fn label(&self) -> String {
        if self.filters.is_empty() {
            format!("In {}", self.team_key)
        } else {
            format!("In {} · {}", self.team_key, self.filters)
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Phase {
    /// The viewer is typing. `problem` says why the last search failed.
    Typing { problem: Option<String> },
    /// Linear has been asked for `query`.
    Searching { query: SearchQuery },
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
        self.editable().then_some(&mut self.query)
    }

    /// Whether the query and its options can still change: no search is
    /// waiting on Linear.
    pub fn editable(&self) -> bool {
        !matches!(self.phase, Phase::Searching { .. })
    }

    /// What Enter sends for `term`.
    fn query_for(&self, term: String) -> SearchQuery {
        SearchQuery {
            term,
            include_comments: self.include_comments,
            within: self
                .scope
                .as_ref()
                .filter(|scope| scope.on)
                .map(|scope| scope.within.clone()),
        }
    }
}

/// Opens the search prompt with both options reset: comments off, and from
/// Team Issues, kept to its team and filter.
pub(super) fn open(state: &mut State) -> Update {
    let scope = (state.screen == Screen::TeamIssues)
        .then(|| state.team_issues.query().cloned())
        .flatten()
        .map(|within| {
            let team_key = state
                .store
                .team(&within.team_id)
                .map_or_else(|| within.team_id.clone(), |team| team.key.clone());
            let filters = FilterField::ALL
                .into_iter()
                .filter_map(|field| {
                    filter_label(state, field).map(|value| format!("{} {value}", field.name()))
                })
                .collect::<Vec<_>>()
                .join(", ");
            Scope {
                within,
                team_key,
                filters,
                on: true,
            }
        });
    state.modals.push(Modal::Search(Search {
        scope,
        ..Search::default()
    }));
    Update::redraw()
}

/// Asks Linear for the query. An empty query asks for nothing.
pub(super) fn accept(state: &mut State) -> Update {
    let Some(Modal::Search(search)) = state.modals.top_mut() else {
        return Update::default();
    };
    if !search.editable() {
        return Update::default();
    }
    let term = search.query.trim().to_owned();
    if term.is_empty() {
        return Update::default();
    }
    let query = search.query_for(term);
    search.phase = Phase::Searching {
        query: query.clone(),
    };
    Update {
        redraw: true,
        effects: vec![Effect::Search(query)],
    }
}

/// Turns looking in comments on or off.
pub(super) fn toggle_comments(state: &mut State) -> Update {
    match state.modals.top_mut() {
        Some(Modal::Search(search)) if search.editable() => {
            search.include_comments = !search.include_comments;
            Update::redraw()
        }
        _ => Update::default(),
    }
}

/// Turns keeping to the Team Issues team and filter off or back on.
pub(super) fn toggle_scope(state: &mut State) -> Update {
    let Some(Modal::Search(search)) = state.modals.top_mut() else {
        return Update::default();
    };
    let editable = search.editable();
    match &mut search.scope {
        Some(scope) if editable => {
            scope.on = !scope.on;
            Update::redraw()
        }
        _ => Update::default(),
    }
}

/// Takes in what Linear answered for `query`. Found issues go into the
/// store whatever happened to the search since; they open in the results
/// view, closing the overlay, only if it is still waiting on `query`, so a
/// slow answer to an older search cannot replace a newer one.
pub(super) fn finish(
    state: &mut State,
    query: &SearchQuery,
    result: Result<SearchResults, Failure>,
) {
    match &result {
        Ok(found) => {
            if state.notice.is_none() {
                state.notice = super::state::summary(&found.problems);
            }
            state.record_problems(SEARCH, &found.problems);
            state.store.merge_search(found);
            state.rebuild_lists();
        }
        Err(failure) => state.record(failure.clone()),
    }
    let waiting = state.modals.iter_mut().rev().find_map(|modal| match modal {
        Modal::Search(search) if matches!(&search.phase, Phase::Searching { query: waiting } if waiting == query) => {
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
                term: query.term.clone(),
            };
            return;
        }
        Ok(found) => found.issues.into_iter().map(|issue| issue.id).collect(),
        Err(failure) => {
            search.phase = Phase::Typing {
                problem: Some(failure.message),
            };
            return;
        }
    };
    let title = title(state, query);
    state
        .modals
        .retain(|modal| !matches!(modal, Modal::Search(_)));
    results::show(state, title, ids);
}

/// What the results are titled: what was searched, as
/// `Search "login" in ENG (with comments)`.
fn title(state: &State, query: &SearchQuery) -> String {
    let mut title = format!("Search \"{}\"", query.term);
    if let Some(within) = &query.within {
        let team = state
            .store
            .team(&within.team_id)
            .map_or(within.team_id.as_str(), |team| team.key.as_str());
        title.push_str(&format!(" in {team}"));
    }
    if query.include_comments {
        title.push_str(" (with comments)");
    }
    title
}
