//! The issue picker: choose one issue, or several, for a purpose. Typing
//! fuzzy-finds the issues in the store by key and title, as the palette
//! does; after a pause it also asks Linear's search and adds what that
//! finds. What is chosen goes back to the purpose, which decides what to do
//! with it.

use std::cmp::Reverse;
use std::time::Duration;

use nucleo_matcher::pattern::{AtomKind, CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Matcher, Utf32Str};

use super::edit::{self, Edit};
use super::picker::{Entry, Note, Picker};
use super::update::Update;
use super::{Effect, Modal, State, Store, palette, relations, results};
use crate::linear::SearchQuery;
use crate::linear::model::{Issue, RelationKind};

/// How long typing must pause before the query goes to Linear's search.
pub const SEARCH_PAUSE: Duration = Duration::from_millis(400);

/// The shortest query, in characters, worth sending to Linear's search.
const SEARCH_MIN_CHARS: usize = 2;

/// What the chosen issues are for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IssuePurpose {
    /// Show them: one opens like a palette jump, several open together in
    /// the results view.
    Show,
    /// Go to the parent, a sub-issue or a related issue of the issue with
    /// this id.
    GoToRelated { issue_id: String },
    /// Make the chosen issue the parent of the issue with this id.
    SetParent { issue_id: String },
    /// Relate the issue with this id to each chosen issue as `kind` says.
    AddRelation {
        issue_id: String,
        kind: RelationKind,
    },
}

impl IssuePurpose {
    /// What the picker's border says, naming the issue by its key.
    pub fn title(&self, store: &Store) -> String {
        let key = |id: &str| {
            store
                .issue(id)
                .map_or_else(|| id.to_owned(), |issue| issue.identifier.clone())
        };
        match self {
            IssuePurpose::Show => "Go to an issue".to_owned(),
            IssuePurpose::GoToRelated { issue_id } => format!("Go from {}", key(issue_id)),
            IssuePurpose::SetParent { issue_id } => format!("Parent of {}", key(issue_id)),
            IssuePurpose::AddRelation { issue_id, kind } => {
                let phrase = match kind {
                    RelationKind::Blocks => "blocks",
                    RelationKind::BlockedBy => "is blocked by",
                    RelationKind::Related => "is related to",
                    RelationKind::DuplicateOf => "is a duplicate of",
                    RelationKind::DuplicatedBy => "is duplicated by",
                };
                format!("{} {phrase}", key(issue_id))
            }
        }
    }

    /// What a row says before the issue `other_id`: how it relates to the
    /// purpose's issue, when the purpose lists related issues.
    fn prefix(&self, store: &Store, other_id: &str) -> Option<String> {
        let IssuePurpose::GoToRelated { issue_id } = self else {
            return None;
        };
        let mut names: Vec<&str> = Vec::new();
        let issue = store.issue(issue_id);
        if issue.and_then(|issue| issue.parent_id.as_deref()) == Some(other_id) {
            names.push("Parent");
        }
        if store
            .issue(other_id)
            .is_some_and(|other| other.parent_id.as_deref() == Some(issue_id.as_str()))
        {
            names.push("Sub-issue");
        }
        for (kind, _, other) in store.relations(issue_id) {
            if other == other_id && !names.contains(&kind.name()) {
                names.push(kind.name());
            }
        }
        (!names.is_empty()).then(|| names.join(", "))
    }
}

/// Whether the picker returns one issue or several.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    One,
    Several,
}

/// Which issues the picker may offer. The caller rules out what its purpose
/// cannot take, such as the issue itself.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PickerScope {
    /// Never offered.
    pub excluded: Vec<String>,
    /// When set, only these are offered, in this order, and Linear's search
    /// is not asked, since nothing else could be chosen.
    pub only: Option<Vec<String>>,
    /// When set, the sub-issues of this issue at any depth are never
    /// offered. Checked against the store's parents each time the list is
    /// ranked, so it also holds for issues Linear's search brings later.
    pub not_under: Option<String>,
}

impl PickerScope {
    fn allows(&self, store: &Store, id: &str) -> bool {
        !self.excluded.iter().any(|excluded| excluded == id)
            && self
                .not_under
                .as_ref()
                .is_none_or(|ancestor| !store.is_under(id, ancestor))
            && self
                .only
                .as_ref()
                .is_none_or(|only| only.iter().any(|allowed| allowed == id))
    }
}

/// An open issue picker.
#[derive(Debug, Clone, PartialEq)]
pub struct IssuePicker {
    pub purpose: IssuePurpose,
    pub mode: Mode,
    /// Issue ids, ranked for the query.
    pub list: Picker<String>,
    scope: PickerScope,
    /// The issue the purpose holds now, marked in the list.
    current: Option<String>,
    /// The ids Linear's search found for `searched`, best first.
    found: Vec<String>,
    /// The query sent to Linear's search, whose answer the list holds or
    /// waits on.
    searched: Option<String>,
    /// The marked issues in [`Mode::Several`], including those the query
    /// hides.
    marked: Vec<String>,
}

impl IssuePicker {
    pub fn new(
        purpose: IssuePurpose,
        mode: Mode,
        scope: PickerScope,
        current: Option<String>,
    ) -> Self {
        Self {
            purpose,
            mode,
            list: Picker::ranked(mode == Mode::Several),
            scope,
            current,
            found: Vec::new(),
            searched: None,
            marked: Vec::new(),
        }
    }

    pub fn title(&self, store: &Store) -> String {
        self.purpose.title(store)
    }

    /// Which issues the picker may offer.
    pub fn scope(&self) -> &PickerScope {
        &self.scope
    }

    /// The marked issues, those the query hides included, in the order they
    /// were marked.
    pub fn marked(&self) -> Vec<String> {
        let listed: Vec<&String> = self.list.values().collect();
        let marked_here = self.list.marked();
        let mut marked: Vec<String> = self
            .marked
            .iter()
            .filter(|id| !listed.contains(id) || marked_here.contains(id))
            .cloned()
            .collect();
        for id in marked_here {
            if !marked.contains(id) {
                marked.push(id.clone());
            }
        }
        marked
    }

    /// Whether the query goes to Linear's search after a pause.
    fn searches(&self) -> bool {
        self.scope.only.is_none() && self.list.query().trim().chars().count() >= SEARCH_MIN_CHARS
    }

    /// Ranks the store's issues for the query and refills the list, keeping
    /// the selection and the marks on the same issues.
    fn fill(&mut self, state: &State) {
        self.marked = self.marked();
        let selected = self.list.selected().cloned();
        let ids = rank(state, &self.scope, self.list.query(), &self.found);
        let entries: Vec<Entry<String>> = ids
            .into_iter()
            .filter_map(|id| state.store.issue(&id))
            .map(|issue| {
                let label = format!("{} {}", issue.identifier, issue.title);
                Entry {
                    label: match self.purpose.prefix(&state.store, &issue.id) {
                        Some(prefix) => format!("{prefix} · {label}"),
                        None => label,
                    },
                    value: issue.id.clone(),
                }
            })
            .collect();
        let current = self
            .current
            .as_ref()
            .and_then(|current| entries.iter().position(|entry| entry.value == *current));
        let marked: Vec<usize> = entries
            .iter()
            .enumerate()
            .filter(|(_, entry)| self.marked.contains(&entry.value))
            .map(|(index, _)| index)
            .collect();
        self.list.set_entries(entries, current);
        self.list.set_marked(marked);
        if let Some(selected) = selected {
            self.list.select_value(&selected);
        }
    }
}

/// The ids of the issues `scope` allows, best match for `query` first: an
/// issue whose key is the whole query, then the fuzzy matches on key and
/// title by score, most recently updated first among equals. With nothing
/// typed, the most recently updated first, or `scope.only`'s order. What
/// Linear's search `found` and the store has follows, in its order.
fn rank(state: &State, scope: &PickerScope, query: &str, found: &[String]) -> Vec<String> {
    let allowed = |issue: &&Issue| scope.allows(&state.store, &issue.id);
    let mut issues: Vec<&Issue> = match &scope.only {
        Some(only) => only
            .iter()
            .filter_map(|id| state.store.issue(id))
            .filter(allowed)
            .collect(),
        None => {
            let mut issues: Vec<&Issue> = state.store.issues().filter(allowed).collect();
            issues.sort_by_key(|issue| Reverse(issue.updated_at));
            issues
        }
    };
    let query = query.trim();
    if !query.is_empty() {
        let pattern = Pattern::new(
            query,
            CaseMatching::Ignore,
            Normalization::Smart,
            AtomKind::Fuzzy,
        );
        let mut matcher = Matcher::new(Config::DEFAULT);
        let mut buffer = Vec::new();
        let mut scored: Vec<(bool, u32, &Issue)> = issues
            .into_iter()
            .filter_map(|issue| {
                let text = format!("{} {}", issue.identifier, issue.title);
                let score = pattern.score(Utf32Str::new(&text, &mut buffer), &mut matcher)?;
                let exact = issue.identifier.eq_ignore_ascii_case(query);
                Some((exact, score, issue))
            })
            .collect();
        scored.sort_by_key(|&(exact, score, _)| (Reverse(exact), Reverse(score)));
        issues = scored.into_iter().map(|(_, _, issue)| issue).collect();
    }
    let mut ids: Vec<String> = issues.into_iter().map(|issue| issue.id.clone()).collect();
    for id in found {
        if scope.allows(&state.store, id) && state.store.issue(id).is_some() && !ids.contains(id) {
            ids.push(id.clone());
        }
    }
    ids
}

/// Opens `picker` over the screen, filled from the store.
pub fn open(state: &mut State, mut picker: IssuePicker) -> Update {
    picker.fill(state);
    state.modals.push(Modal::IssuePicker(picker));
    Update::redraw()
}

/// The query of the picker on top changed: ranks again with the best match
/// selected, forgets what an older query's search found, and asks to be
/// told when typing pauses.
pub(super) fn typed(state: &mut State) -> Update {
    let Some(Modal::IssuePicker(mut picker)) = state.modals.pop() else {
        return Update::redraw();
    };
    picker.found.clear();
    picker.searched = None;
    picker.list.set_note(None);
    picker.fill(state);
    picker.list.select(0);
    let effects = if picker.searches() {
        vec![Effect::SearchAfterPause(
            picker.list.query().trim().to_owned(),
        )]
    } else {
        Vec::new()
    };
    state.modals.push(Modal::IssuePicker(picker));
    Update {
        redraw: true,
        effects,
    }
}

/// Typing paused on `query`: sends it to Linear's search if the picker on
/// top still holds it and has not sent it already.
pub(super) fn paused(state: &mut State, query: &str) -> Update {
    let Some(Modal::IssuePicker(picker)) = state.modals.top_mut() else {
        return Update::default();
    };
    if !picker.searches()
        || picker.list.query().trim() != query
        || picker.searched.as_deref() == Some(query)
    {
        return Update::default();
    }
    picker.searched = Some(query.to_owned());
    picker.list.set_note(Some(Note::Loading));
    Update {
        redraw: true,
        effects: vec![Effect::Search(SearchQuery::new(query))],
    }
}

/// Takes in what Linear's search answered for `term`: the ids it found, or
/// why it failed. Only a picker waiting on `term` uses it; a failure keeps
/// the issues already listed.
pub(super) fn searched(state: &mut State, term: &str, outcome: Result<Vec<String>, String>) {
    let mut modals = std::mem::take(&mut state.modals);
    for modal in modals.iter_mut() {
        let Modal::IssuePicker(picker) = modal else {
            continue;
        };
        if picker.searched.as_deref() != Some(term) || picker.list.note() != Some(&Note::Loading) {
            continue;
        }
        match &outcome {
            Ok(found) => {
                picker.found.clone_from(found);
                picker.list.set_note(None);
                picker.fill(state);
            }
            Err(error) => picker.list.set_note(Some(Note::Failed(error.clone()))),
        }
    }
    state.modals = modals;
}

/// Moves the selection of the picker on top by `step`.
pub(super) fn select(state: &mut State, step: isize) -> Update {
    match state.modals.top_mut() {
        Some(Modal::IssuePicker(picker)) => Update {
            redraw: picker.list.select_by(step),
            effects: Vec::new(),
        },
        _ => Update::default(),
    }
}

/// Marks or unmarks the selected issue, or the `nth` visible one when
/// given, in [`Mode::Several`].
pub(super) fn toggle(state: &mut State, nth: Option<usize>) -> Update {
    match state.modals.top_mut() {
        Some(Modal::IssuePicker(picker)) => Update {
            redraw: match nth {
                Some(n) => picker.list.toggle_nth(n),
                None => picker.list.toggle_selected(),
            },
            effects: Vec::new(),
        },
        _ => Update::default(),
    }
}

/// Closes the picker on what was chosen and hands it to the purpose. In
/// [`Mode::One`], the selected issue or the `nth` visible one. In
/// [`Mode::Several`], a number toggles that issue instead, and accepting
/// returns every marked issue, or the selected one when none is marked.
pub(super) fn choose(state: &mut State, nth: Option<usize>) -> Update {
    let Some(Modal::IssuePicker(picker)) = state.modals.top() else {
        return Update::default();
    };
    let chosen = match (picker.mode, nth) {
        (Mode::Several, Some(_)) => return toggle(state, nth),
        (Mode::Several, None) => {
            let marked = picker.marked();
            if marked.is_empty() {
                picker.list.selected().cloned().into_iter().collect()
            } else {
                marked
            }
        }
        (Mode::One, Some(n)) => picker.list.nth(n).cloned().into_iter().collect(),
        (Mode::One, None) => picker.list.selected().cloned().into_iter().collect(),
    };
    if chosen.is_empty() {
        return Update::default();
    }
    let purpose = picker.purpose.clone();
    state.modals.pop();
    carry_out(state, purpose, chosen)
}

/// Does what `purpose` does with the `chosen` issue ids.
fn carry_out(state: &mut State, purpose: IssuePurpose, chosen: Vec<String>) -> Update {
    match purpose {
        IssuePurpose::Show | IssuePurpose::GoToRelated { .. } => {
            match chosen.as_slice() {
                [one] => palette::jump_to(state, one.clone()),
                _ => results::show(state, "Issues".to_owned(), chosen),
            }
            Update::redraw()
        }
        IssuePurpose::SetParent { issue_id } => match chosen.into_iter().next() {
            Some(parent) => edit::write(state, &issue_id, Edit::Parent(Some(parent))),
            None => Update::redraw(),
        },
        IssuePurpose::AddRelation { issue_id, kind } => {
            relations::add(state, &issue_id, kind, &chosen)
        }
    }
}
