//! The action registry and the keymap built from it. Key dispatch, chords,
//! the which-key popup, the bottom bar, the cheatsheet and the config all
//! read bindings from here; no key is bound anywhere else.

mod key;
mod keymap;
mod registry;

pub use key::{Key, KeySequence, ParseKeyError};
pub use keymap::{Change, Keymap, Override, Resolution, Target};
pub use registry::{ACTIONS, ActionSpec, by_id, spec};

use crate::app::{Modal, Pane, Screen, State};

/// Where a binding applies. Several contexts are active at once, most
/// specific first, and the first one that binds a key wins.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Context {
    /// Everywhere outside an overlay.
    Global,
    /// An issue list.
    List,
    /// An issue's detail.
    Detail,
    /// An overlay such as the cheatsheet, which takes every key.
    Overlay,
}

impl Context {
    pub const ALL: [Context; 4] = [
        Context::Global,
        Context::List,
        Context::Detail,
        Context::Overlay,
    ];

    /// The name of its table in `config.toml`, as in `[keys.list]`.
    pub fn name(self) -> &'static str {
        match self {
            Context::Global => "global",
            Context::List => "list",
            Context::Detail => "detail",
            Context::Overlay => "overlay",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Context::ALL.into_iter().find(|c| c.name() == name)
    }

    /// The heading the cheatsheet shows over its actions.
    pub fn title(self) -> &'static str {
        match self {
            Context::Global => "Everywhere",
            Context::List => "Issue list",
            Context::Detail => "Issue detail",
            Context::Overlay => "Overlays",
        }
    }
}

/// The contexts taking keys now, most specific first. An open which-key
/// popup is ignored: it only waits for the rest of a chord from below it.
pub fn active_contexts(state: &State) -> Vec<Context> {
    let overlay = state
        .modals
        .iter()
        .rev()
        .find(|modal| !matches!(modal, Modal::WhichKey { .. }));
    match overlay {
        Some(_) => vec![Context::Overlay],
        None => screen_contexts(state),
    }
}

/// The contexts of the screen under any overlay.
pub fn screen_contexts(state: &State) -> Vec<Context> {
    match state.screen {
        Screen::MyIssues if state.my_issues.pane() == Pane::Detail => {
            vec![Context::Detail, Context::Global]
        }
        Screen::MyIssues | Screen::TeamIssues => vec![Context::List, Context::Global],
        Screen::Analytics => vec![Context::Global],
    }
}
