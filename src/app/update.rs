use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::{Action, Effect, Event, State};

/// What one event did: whether the screen must be drawn again, and what the
/// runtime must do next.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Update {
    pub redraw: bool,
    pub effects: Vec<Effect>,
}

pub fn update(state: &mut State, event: Event) -> Update {
    match event {
        Event::Resize { .. } => Update {
            redraw: true,
            effects: Vec::new(),
        },
        Event::Key(key) => match state.modals.top() {
            Some(modal) => match *modal {},
            None => match action_for(key) {
                Some(action) => perform(state, action),
                None => Update::default(),
            },
        },
    }
}

/// Carries out `action` against `state`.
pub fn perform(_state: &mut State, action: Action) -> Update {
    match action {
        Action::Quit => Update {
            redraw: false,
            effects: vec![Effect::Quit],
        },
        _ => Update::default(),
    }
}

// The action registry (#3) replaces this hard-coded binding.
fn action_for(key: KeyEvent) -> Option<Action> {
    match (key.code, key.modifiers) {
        (KeyCode::Char('q'), KeyModifiers::NONE) => Some(Action::Quit),
        _ => None,
    }
}
