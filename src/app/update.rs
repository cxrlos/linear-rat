use crossterm::event::KeyEventKind;
use ratatui::layout::Size;

use super::{Action, Effect, Event, Load, Modal, Screen, State};
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
            state.modals.push(Modal::Cheatsheet);
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
        _ => Update::default(),
    }
}
