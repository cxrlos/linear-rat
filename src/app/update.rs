use crossterm::event::{KeyCode, KeyEventKind};
use ratatui::layout::Size;

use super::edit::{self, Editable};
use super::{
    Action, Copyable, Effect, Event, Load, Modal, Palette, Pane, Screen, Search, State, detail,
    palette, search, sync,
};
use crate::keys::{self, Key, Resolution};
use crate::ui;

/// What one event did: whether the screen must be drawn again, and what the
/// runtime must do next.
#[derive(Debug, Default, PartialEq)]
pub struct Update {
    pub redraw: bool,
    pub effects: Vec<Effect>,
}

impl Update {
    pub(super) fn redraw() -> Self {
        Self {
            redraw: true,
            effects: Vec::new(),
        }
    }
}

/// Applies `event`, then asks for the comments and history of the issue
/// whose detail is on screen if they have not been asked for yet.
pub fn update(state: &mut State, event: Event) -> Update {
    let mut update = apply(state, event);
    update.effects.extend(detail::request(state));
    update
}

fn apply(state: &mut State, event: Event) -> Update {
    match event {
        Event::Start if state.load == Load::Loading || state.sync_status.is_refreshing() => {
            Update {
                redraw: false,
                effects: vec![Effect::LoadMyIssues],
            }
        }
        Event::Start => Update::default(),
        Event::MyIssuesLoaded(Ok(loaded)) => sync::loaded(state, &loaded),
        Event::MyIssuesLoaded(Err(error)) => sync::load_failed(state, error),
        Event::Tick(now) => sync::tick(state, now),
        Event::FocusGained => sync::focus(state, true),
        Event::FocusLost => sync::focus(state, false),
        Event::Polled { changes, failure } => sync::polled(state, &changes, failure),
        Event::Reconciled { checked, result } => sync::reconciled(state, &checked, result),
        Event::Throttled => sync::throttled(state),
        Event::SearchDone { term, result } => {
            search::finish(state, &term, result);
            Update::redraw()
        }
        Event::Resize { columns, rows } => {
            state.viewport = Size::new(columns, rows);
            let rows = ui::list_rows(state, state.viewport);
            state.my_issues.scroll_into_view(rows);
            Update::redraw()
        }
        Event::Key(key) if key.kind == KeyEventKind::Release => Update::default(),
        Event::Key(key) => {
            sync::activity(state);
            let cleared_notice = state.notice.take().is_some();
            let key = Key::from(key);
            let mut update = type_text(state, key).unwrap_or_else(|| press(state, key));
            update.redraw |= cleared_notice;
            update
        }
        Event::IssueDetailLoaded { issue_id, result } => {
            detail::loaded(state, issue_id, result);
            Update::redraw()
        }
        Event::EffectFailed(error) => {
            state.notice = Some(error);
            Update::redraw()
        }
        Event::WorkflowStatesLoaded { team_id, result } => {
            let failure = match result {
                Ok(states) => {
                    state.store.set_workflow_states(&team_id, &states);
                    None
                }
                Err(error) => Some(error),
            };
            edit::loaded(state, Editable::State, &team_id, failure)
        }
        Event::TeamMembersLoaded { team_id, result } => {
            let failure = match result {
                Ok(members) => {
                    state.store.set_team_members(&team_id, &members);
                    None
                }
                Err(error) => Some(error),
            };
            edit::loaded(state, Editable::Assignee, &team_id, failure)
        }
        Event::IssueUpdated { write, result } => edit::finished(state, &write, result),
    }
}

/// Text entry: when the top overlay has a text field, a printable key
/// without Ctrl or Alt types into it and Backspace deletes, before the
/// keymap, so `j`, `k` and `q` type rather than move or quit. The one
/// exception is a key bound to a number pick that can run now, so 1 to 9
/// pick in a picker while a number past its last entry still types.
fn type_text(state: &mut State, key: Key) -> Option<Update> {
    let typed = key.typed();
    if typed.is_none() && key != Key::named(KeyCode::Backspace) {
        return None;
    }
    if typed.is_some() && picks_by_number(state, key) {
        return None;
    }
    let field = state.modals.top_mut()?.text_field()?;
    match typed {
        Some(c) => field.push(c),
        None => {
            field.pop();
        }
    }
    Some(Update::redraw())
}

/// Whether `key` runs a number pick that can run now.
fn picks_by_number(state: &State, key: Key) -> bool {
    let contexts = keys::active_contexts(state);
    match state.keymap.resolve(&contexts, &[key]) {
        Resolution::Action(action @ Action::PickNumber(_)) => {
            keys::spec(action).is_some_and(|spec| (spec.available)(state))
        }
        _ => false,
    }
}

/// Runs the binding for `key` in the active contexts, or for the chord it
/// completes. A key that continues no chord just closes the which-key popup.
fn press(state: &mut State, key: Key) -> Update {
    let mut keys = Vec::new();
    let mut chord_ended = false;
    if let Some(Modal::WhichKey { prefix }) = state.modals.top() {
        keys.clone_from(prefix);
        state.modals.pop();
        chord_ended = true;
    }
    keys.push(key);

    let contexts = keys::active_contexts(state);
    let mut update = match state.keymap.resolve(&contexts, &keys) {
        Resolution::Action(action) => perform(state, action),
        Resolution::Prefix => {
            state.modals.push(Modal::WhichKey { prefix: keys });
            Update::redraw()
        }
        Resolution::Unbound => Update::default(),
    };
    update.redraw |= chord_ended;
    update
}

/// Carries out `action` against `state`.
pub fn perform(state: &mut State, action: Action) -> Update {
    match action {
        Action::Quit => Update {
            redraw: false,
            effects: vec![Effect::Quit],
        },
        Action::ShowScreen(screen) if screen != state.screen => {
            state.screen = screen;
            Update::redraw()
        }
        Action::Refresh => sync::refresh(state),
        Action::ShowCheatsheet => {
            state.modals.push(Modal::Cheatsheet { scroll: 0 });
            Update::redraw()
        }
        Action::ShowPalette => {
            state.modals.push(Modal::Palette(Palette::default()));
            Update::redraw()
        }
        Action::ShowSearch => {
            state.modals.push(Modal::Search(Search::default()));
            Update::redraw()
        }
        Action::Accept => match state.modals.top() {
            Some(Modal::Picker(_)) => edit::choose(state, None),
            Some(Modal::Palette(_)) => palette::accept(state),
            Some(Modal::Search(_)) => search::accept(state),
            _ => Update::default(),
        },
        Action::Close if state.modals.pop().is_some() => Update::redraw(),
        Action::MoveDown
        | Action::MoveUp
        | Action::MoveToTop
        | Action::MoveToBottom
        | Action::HalfPageDown
        | Action::HalfPageUp
            if detail::has_focus(state) =>
        {
            detail::scroll(state, action)
        }
        Action::MoveDown
        | Action::MoveUp
        | Action::MoveToTop
        | Action::MoveToBottom
        | Action::HalfPageDown
        | Action::HalfPageUp
        | Action::FocusLeft
        | Action::FocusRight
        | Action::Open
        | Action::Close
            if state.screen == Screen::MyIssues =>
        {
            let rows = ui::list_rows(state, state.viewport);
            let pane = state.my_issues.pane();
            let redraw = state.my_issues.perform(action, rows);
            if pane == Pane::List && state.my_issues.pane() == Pane::Detail {
                detail::reopen(state);
            }
            Update {
                redraw,
                effects: Vec::new(),
            }
        }
        Action::ScrollDown | Action::ScrollUp => match state.modals.top() {
            Some(Modal::Picker(_)) => edit::select(state, step(action)),
            Some(Modal::Issue { id }) => {
                let id = id.clone();
                detail::scroll_overlay(state, &id, action)
            }
            Some(Modal::Palette(_)) => palette::step(state, action),
            Some(Modal::Search(_)) => search::step(state, action),
            _ => scroll_cheatsheet(state, action),
        },
        Action::Copy(copyable) => copy(state, copyable),
        Action::OpenInBrowser => match state.focused_issue() {
            Some(issue) => {
                let effects = vec![Effect::OpenUrl(issue.url.clone())];
                state.notice = Some(format!("Opening {} in the browser", issue.identifier));
                Update {
                    redraw: true,
                    effects,
                }
            }
            None => Update::default(),
        },
        Action::Edit(field) => edit::open(state, field),
        Action::PickNumber(n) => edit::choose(state, Some(usize::from(n))),
        _ => Update::default(),
    }
}

/// One entry down for [`Action::ScrollDown`], one up for the rest.
fn step(action: Action) -> isize {
    if action == Action::ScrollDown { 1 } else { -1 }
}

/// Moves the open cheatsheet one row, no further than its last row.
fn scroll_cheatsheet(state: &mut State, action: Action) -> Update {
    let max = ui::cheatsheet_max_scroll(state, state.viewport);
    let Some(Modal::Cheatsheet { scroll }) = state.modals.top_mut() else {
        return Update::default();
    };
    let before = *scroll;
    *scroll = match action {
        Action::ScrollDown => scroll.saturating_add(1).min(max),
        _ => scroll.saturating_sub(1),
    };
    Update {
        redraw: *scroll != before,
        effects: Vec::new(),
    }
}

/// Asks to copy `copyable` of the focused issue and says so in the notice.
fn copy(state: &mut State, copyable: Copyable) -> Update {
    let Some(issue) = state.focused_issue() else {
        return Update::default();
    };
    let (text, notice) = match copyable {
        Copyable::Identifier => (issue.identifier.clone(), issue.identifier.clone()),
        Copyable::Url => (
            issue.url.clone(),
            format!("the URL of {}", issue.identifier),
        ),
        Copyable::BranchName => (
            issue.branch_name.clone(),
            format!("the branch name of {}", issue.identifier),
        ),
        Copyable::MarkdownLink => (
            markdown_link(&format!("{} {}", issue.identifier, issue.title), &issue.url),
            format!("a Markdown link to {}", issue.identifier),
        ),
    };
    state.notice = Some(format!("Copied {notice}"));
    Update {
        redraw: true,
        effects: vec![Effect::Copy(text)],
    }
}

/// `[text](url)`, with the backslashes and brackets in `text` escaped so a
/// title like `Fix [beta]` cannot end the link text early.
fn markdown_link(text: &str, url: &str) -> String {
    let mut link = String::from("[");
    for c in text.chars() {
        if matches!(c, '\\' | '[' | ']') {
            link.push('\\');
        }
        link.push(c);
    }
    link.push_str("](");
    link.push_str(url);
    link.push(')');
    link
}
