use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::{Effect, Event, State};

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
            None => on_key(key),
        },
    }
}

// The action registry (#3) replaces this hard-coded binding.
fn on_key(key: KeyEvent) -> Update {
    match (key.code, key.modifiers) {
        (KeyCode::Char('q'), KeyModifiers::NONE) => Update {
            redraw: false,
            effects: vec![Effect::Quit],
        },
        _ => Update::default(),
    }
}
