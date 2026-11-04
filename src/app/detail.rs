//! The focused issue's detail: how far it is scrolled, and which issues'
//! comments and state history have been asked for.

use std::collections::BTreeMap;

use super::{Action, Effect, Load, Pane, Screen, State, Update};
use crate::linear::{IssueDetail, Urgency};
use crate::ui;

/// The detail's scroll and loads. The scroll belongs to one issue, so it
/// survives the store updating that issue and resets for another one.
#[derive(Debug, Default)]
pub struct Detail {
    issue_id: Option<String>,
    scroll: usize,
    loads: BTreeMap<String, DetailLoad>,
}

/// How far the comments and state history of one issue are.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DetailLoad {
    Loading,
    Loaded,
    /// Why they did not load, with its context.
    Failed(String),
}

impl Detail {
    /// The first line on screen of `issue_id`'s detail.
    pub fn scroll(&self, issue_id: &str) -> usize {
        match &self.issue_id {
            Some(id) if id == issue_id => self.scroll,
            _ => 0,
        }
    }

    /// How far the comments and history of `issue_id` are, or `None` if they
    /// have not been asked for.
    pub fn load(&self, issue_id: &str) -> Option<&DetailLoad> {
        self.loads.get(issue_id)
    }
}

/// The issue whose detail is on screen: the focused one, when the detail
/// shows beside the list or has the screen to itself.
fn shown_issue_id(state: &State) -> Option<String> {
    let wide = state.viewport.width >= state.ui.wide_layout_columns;
    let shown = state.screen == Screen::MyIssues
        && state.load == Load::Loaded
        && (wide || state.my_issues.pane() == Pane::Detail);
    shown
        .then(|| {
            state
                .my_issues
                .focused_issue()
                .map(|issue| issue.id.clone())
        })
        .flatten()
}

/// Whether the detail pane has focus, so moves scroll it.
pub(super) fn has_focus(state: &State) -> bool {
    state.screen == Screen::MyIssues
        && state.load == Load::Loaded
        && state.my_issues.pane() == Pane::Detail
}

/// Asks for the comments and history of the issue on screen, unless they
/// have been asked for already. A detail that only follows the list's focus
/// loads in the background, so browsing the list never spends the budget
/// reserve.
pub(super) fn request(state: &mut State) -> Option<Effect> {
    let issue_id = shown_issue_id(state)?;
    if state.detail.loads.contains_key(&issue_id) {
        return None;
    }
    state
        .detail
        .loads
        .insert(issue_id.clone(), DetailLoad::Loading);
    let urgency = if has_focus(state) {
        Urgency::Urgent
    } else {
        Urgency::Background
    };
    Some(Effect::LoadIssueDetail { issue_id, urgency })
}

/// The viewer opened the focused issue's detail: what loaded before is
/// asked for again, so it is fresh. A load in flight is left to finish.
pub(super) fn reopen(state: &mut State) {
    let Some(issue) = state.my_issues.focused_issue() else {
        return;
    };
    let loads = &mut state.detail.loads;
    if loads.get(&issue.id) != Some(&DetailLoad::Loading) {
        loads.remove(&issue.id);
    }
}

/// Takes in the result of [`Effect::LoadIssueDetail`]. What Linear could not
/// give becomes the notice, unless one is already showing.
pub(super) fn loaded(state: &mut State, issue_id: String, result: Result<IssueDetail, String>) {
    let load = match result {
        Ok(detail) => {
            if state.notice.is_none() {
                state.notice = detail.problems.first().cloned();
            }
            state.store.merge_detail(&issue_id, &detail);
            DetailLoad::Loaded
        }
        Err(error) => DetailLoad::Failed(error),
    };
    state.detail.loads.insert(issue_id, load);
}

/// Scrolls the focused issue's detail for a move, no further than its last
/// line. Half a page is half the rows the detail shows.
pub(super) fn scroll(state: &mut State, action: Action) -> Update {
    let Some(issue_id) = state
        .my_issues
        .focused_issue()
        .map(|issue| issue.id.clone())
    else {
        return Update::default();
    };
    let (rows, lines) = ui::detail_rows(state, state.viewport);
    let max = lines.saturating_sub(rows);
    let page = (rows / 2).max(1);
    let before = state.detail.scroll(&issue_id).min(max);
    let after = match action {
        Action::MoveDown => before.saturating_add(1),
        Action::MoveUp => before.saturating_sub(1),
        Action::MoveToTop => 0,
        Action::MoveToBottom => max,
        Action::HalfPageDown => before.saturating_add(page),
        Action::HalfPageUp => before.saturating_sub(page),
        _ => before,
    }
    .min(max);
    state.detail.issue_id = Some(issue_id);
    state.detail.scroll = after;
    Update {
        redraw: after != before,
        effects: Vec::new(),
    }
}
