//! The palette: one query that fuzzy-finds every action in the registry and
//! every issue in the store, and Enter to run the action or jump to the issue.

use std::cmp::Reverse;

use nucleo_matcher::pattern::{AtomKind, CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Matcher, Utf32Str};

use super::update::{Update, perform};
use super::{Action, Modal, Screen, State, results};
use crate::keys::{ACTIONS, ActionSpec, Context};
use crate::linear::model::Issue;
use crate::ui;

/// What the viewer has typed and which match is highlighted.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Palette {
    pub query: String,
    /// The highlighted match, counted from the best one.
    pub selected: usize,
}

/// One thing the palette can find.
#[derive(Debug, Clone, Copy)]
pub enum PaletteEntry<'a> {
    Action(&'static ActionSpec),
    Issue(&'a Issue),
}

/// Every action and issue matching `query`, best first. Each word of the
/// query must match fuzzily, in an action's description or id, or in an
/// issue's identifier or title. An issue whose identifier is the whole
/// query comes first. Ties keep actions in registry order before issues,
/// most recently updated first.
pub fn palette_matches<'a>(state: &'a State, query: &str) -> Vec<PaletteEntry<'a>> {
    let pattern = Pattern::new(
        query,
        CaseMatching::Ignore,
        Normalization::Smart,
        AtomKind::Fuzzy,
    );
    let mut matcher = Matcher::new(Config::DEFAULT);
    let mut buffer = Vec::new();
    let mut score = |text: &str| pattern.score(Utf32Str::new(text, &mut buffer), &mut matcher);

    let mut issues: Vec<&Issue> = state.store.issues().collect();
    issues.sort_by_key(|issue| Reverse(issue.updated_at));
    let actions = ACTIONS
        .iter()
        .filter(|spec| spec.contexts.iter().any(|&c| c != Context::Overlay))
        .filter_map(|spec| {
            let found = score(&format!("{} {}", spec.description, spec.id))?;
            Some((found, false, PaletteEntry::Action(spec)))
        });
    let mut found: Vec<(u32, bool, PaletteEntry)> = actions.collect();
    found.extend(issues.into_iter().filter_map(|issue| {
        let found = score(&format!("{} {}", issue.identifier, issue.title))?;
        let exact = issue.identifier.eq_ignore_ascii_case(query.trim());
        Some((found, exact, PaletteEntry::Issue(issue)))
    }));
    found.sort_by_key(|&(score, exact, _)| (Reverse(exact), Reverse(score)));
    found.into_iter().map(|(_, _, entry)| entry).collect()
}

/// Moves the highlight one match down or up, within the matches.
pub(super) fn step(state: &mut State, action: Action) -> Update {
    let Some(Modal::Palette(palette)) = state.modals.top() else {
        return Update::default();
    };
    let last = palette_matches(state, &palette.query)
        .len()
        .saturating_sub(1);
    let Some(Modal::Palette(palette)) = state.modals.top_mut() else {
        return Update::default();
    };
    let before = palette.selected;
    palette.selected = match action {
        Action::ScrollDown => palette.selected.saturating_add(1).min(last),
        _ => palette.selected.saturating_sub(1),
    };
    Update {
        redraw: palette.selected != before,
        effects: Vec::new(),
    }
}

/// Runs the highlighted action on what is under the palette, or jumps to
/// the highlighted issue. An action that cannot run there leaves the palette
/// open.
pub(super) fn accept(state: &mut State) -> Update {
    let Some(Modal::Palette(palette)) = state.modals.top() else {
        return Update::default();
    };
    let (spec, issue_id) = match palette_matches(state, &palette.query).get(palette.selected) {
        Some(PaletteEntry::Action(spec)) => (Some(*spec), None),
        Some(PaletteEntry::Issue(issue)) => (None, Some(issue.id.clone())),
        None => return Update::default(),
    };
    let Some(palette) = state.modals.pop() else {
        return Update::default();
    };
    if let Some(id) = issue_id {
        jump_to(state, id);
        return Update::redraw();
    }
    match spec {
        Some(spec) if (spec.available)(state) => {
            let mut update = perform(state, spec.action);
            update.redraw = true;
            update
        }
        _ => {
            state.modals.push(palette);
            Update::default()
        }
    }
}

/// Shows the issue `id`: focused in My Issues when it is there, or else
/// alone in the results view.
fn jump_to(state: &mut State, id: String) {
    if state.my_issues.focus_id(&id) {
        state.screen = Screen::MyIssues;
        let rows = ui::list_rows(state, state.viewport);
        state.my_issues.scroll_into_view(rows);
    } else {
        results::show(state, "Issue".to_owned(), vec![id]);
    }
}
