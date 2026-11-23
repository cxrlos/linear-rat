//! The focused issue's detail: how far it is scrolled, and which issues'
//! comments and state history have been asked for.

use std::collections::BTreeMap;

use super::{Action, Effect, Failure, Load, Pane, Screen, State, Update};
use crate::linear::{IssueDetail, Urgency};
use crate::ui;

/// The detail's scroll, focused comment and loads. The scroll and the
/// focused comment belong to one issue, so they survive the store updating
/// that issue and reset for another one.
#[derive(Debug, Default)]
pub struct Detail {
    issue_id: Option<String>,
    scroll: usize,
    comment_id: Option<String>,
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

    /// The id of the focused comment in `issue_id`'s detail, if one is.
    pub fn focused_comment(&self, issue_id: &str) -> Option<&str> {
        match &self.issue_id {
            Some(id) if id == issue_id => self.comment_id.as_deref(),
            _ => None,
        }
    }

    /// Focuses the comment `comment_id` in `issue_id`'s detail, or no
    /// comment. Focusing in another issue's detail starts it at the top.
    pub fn focus_comment(&mut self, issue_id: &str, comment_id: Option<String>) {
        if self.issue_id.as_deref() != Some(issue_id) {
            self.issue_id = Some(issue_id.to_owned());
            self.scroll = 0;
        }
        self.comment_id = comment_id;
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
    let shown = screen_pane(state).is_some_and(|pane| wide || pane == Pane::Detail);
    shown
        .then(|| state.focused_issue().map(|issue| issue.id.clone()))
        .flatten()
}

/// The focused pane of the screen, when it is one with an issue list and a
/// detail and its list has loaded.
fn screen_pane(state: &State) -> Option<Pane> {
    match state.screen {
        Screen::MyIssues => (state.load == Load::Loaded).then(|| state.my_issues.pane()),
        Screen::TeamIssues => Some(state.team_issues.pane()),
        Screen::Results => Some(state.results.pane()),
        Screen::Analytics | Screen::Cycles | Screen::Projects => None,
    }
}

/// Whether the detail pane has focus, so moves scroll it.
pub(super) fn has_focus(state: &State) -> bool {
    screen_pane(state) == Some(Pane::Detail)
}

/// Asks for the comments and history of the issue on screen, unless they
/// have been asked for already. A detail that only follows the list's focus
/// loads in the background, so browsing the list never spends the budget
/// reserve.
pub(super) fn request(state: &mut State) -> Option<Effect> {
    let issue_id = shown_issue_id(state)?;
    if state.detail.loads.contains_key(&issue_id) || super::is_local(&issue_id) {
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

/// Asks for the detail of `issue_id` as a load the viewer waits on, unless
/// it has loaded or is loading. A failed load is tried again.
pub(super) fn load_now(state: &mut State, issue_id: &str) -> Option<Effect> {
    if matches!(
        state.detail.loads.get(issue_id),
        Some(DetailLoad::Loading | DetailLoad::Loaded)
    ) {
        return None;
    }
    state
        .detail
        .loads
        .insert(issue_id.to_owned(), DetailLoad::Loading);
    Some(Effect::LoadIssueDetail {
        issue_id: issue_id.to_owned(),
        urgency: Urgency::Urgent,
    })
}

/// The viewer opened the focused issue's detail: what loaded before is
/// asked for again, so it is fresh. A load in flight is left to finish.
pub(super) fn reopen(state: &mut State) {
    let Some(id) = state.focused_issue().map(|issue| issue.id.clone()) else {
        return;
    };
    let loads = &mut state.detail.loads;
    if loads.get(&id) != Some(&DetailLoad::Loading) {
        loads.remove(&id);
    }
}

/// Takes in the result of [`Effect::LoadIssueDetail`]. What Linear could not
/// give becomes the notice, unless one is already showing.
pub(super) fn loaded(state: &mut State, issue_id: String, result: Result<IssueDetail, Failure>) {
    let load = match result {
        Ok(detail) => {
            if state.notice.is_none() {
                state.notice = detail.problems.first().cloned();
            }
            state.record_problems("load the issue's comments and history", &detail.problems);
            state.store.merge_detail(&issue_id, &detail);
            super::relations::reapply_in_flight(state);
            // The detail can bring newer copies of related issues.
            state.rebuild_lists();
            DetailLoad::Loaded
        }
        Err(failure) => {
            let load = DetailLoad::Failed(failure.message.clone());
            state.record(failure);
            load
        }
    };
    state.detail.loads.insert(issue_id, load);
}

/// Scrolls the focused issue's detail for a move, no further than its last
/// line. Half a page is half the rows the detail shows.
pub(super) fn scroll(state: &mut State, action: Action) -> Update {
    let Some(issue_id) = state.focused_issue().map(|issue| issue.id.clone()) else {
        return Update::default();
    };
    let shown = ui::detail_rows(state, state.viewport);
    scroll_by(state, issue_id, shown, action)
}

/// Scrolls the detail of `issue_id`, which shows `rows` of its `lines`.
fn scroll_by(
    state: &mut State,
    issue_id: String,
    (rows, lines): (usize, usize),
    action: Action,
) -> Update {
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
    if state.detail.issue_id.as_deref() != Some(issue_id.as_str()) {
        state.detail.comment_id = None;
    }
    state.detail.issue_id = Some(issue_id);
    state.detail.scroll = after;
    Update {
        redraw: after != before,
        effects: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_focused_comment_belongs_to_its_issue() {
        let mut detail = Detail::default();
        detail.focus_comment("issue-1", Some("comment-1".to_owned()));

        assert_eq!(detail.focused_comment("issue-1"), Some("comment-1"));
        assert_eq!(detail.focused_comment("issue-2"), None);

        detail.focus_comment("issue-1", None);
        assert_eq!(detail.focused_comment("issue-1"), None);
    }

    #[test]
    fn focusing_in_another_issue_starts_its_detail_at_the_top() {
        let mut detail = Detail {
            issue_id: Some("issue-1".to_owned()),
            scroll: 7,
            ..Detail::default()
        };
        detail.focus_comment("issue-1", Some("comment-1".to_owned()));
        assert_eq!(detail.scroll("issue-1"), 7);

        detail.focus_comment("issue-2", Some("comment-9".to_owned()));
        assert_eq!(detail.scroll("issue-2"), 0);
        assert_eq!(detail.focused_comment("issue-1"), None);
    }
}
