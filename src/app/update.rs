use crossterm::event::KeyEventKind;
use ratatui::layout::Size;

use super::{Action, Copyable, Effect, Event, Load, Modal, Screen, State};
use crate::keys::{self, Key, Resolution};
use crate::ui;

/// What one event did: whether the screen must be drawn again, and what the
/// runtime must do next.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Update {
    pub redraw: bool,
    pub effects: Vec<Effect>,
}

impl Update {
    fn redraw() -> Self {
        Self {
            redraw: true,
            effects: Vec::new(),
        }
    }
}

pub fn update(state: &mut State, event: Event) -> Update {
    match event {
        Event::Start if state.load == Load::Loading => Update {
            redraw: false,
            effects: vec![Effect::LoadMyIssues],
        },
        Event::Start => Update::default(),
        Event::MyIssuesLoaded(Ok(loaded)) => {
            state.show_assigned(&loaded);
            Update::redraw()
        }
        Event::MyIssuesLoaded(Err(error)) => {
            state.load = Load::Failed(error);
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
            let cleared_notice = state.notice.take().is_some();
            let mut update = press(state, Key::from(key));
            update.redraw |= cleared_notice;
            update
        }
        Event::EffectFailed(error) => {
            state.notice = Some(error);
            Update::redraw()
        }
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
        Action::ShowCheatsheet => {
            state.modals.push(Modal::Cheatsheet { scroll: 0 });
            Update::redraw()
        }
        Action::Close if state.modals.pop().is_some() => Update::redraw(),
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
            Update {
                redraw: state.my_issues.perform(action, rows),
                effects: Vec::new(),
            }
        }
        Action::ScrollDown | Action::ScrollUp => scroll_cheatsheet(state, action),
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
        _ => Update::default(),
    }
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
