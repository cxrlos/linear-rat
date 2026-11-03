use crossterm::event::KeyCode;

use super::{Context, Key};
use crate::app::{Action, Screen, State};

/// One action as the viewer sees it: what it is called, where it applies,
/// when it can run and which keys run it by default (the vim preset).
#[derive(Debug)]
pub struct ActionSpec {
    pub action: Action,
    /// The name used in `config.toml`, for example `move-down`.
    pub id: &'static str,
    /// What the cheatsheet and the palette say it does.
    pub description: &'static str,
    /// The short word shown in the bottom bar, if it appears there.
    pub hint: Option<&'static str>,
    pub contexts: &'static [Context],
    pub default_keys: &'static [&'static [Key]],
    /// Whether it can run now. Unavailable actions are still listed, muted.
    pub available: fn(&State) -> bool,
}

const LIST: &[Context] = &[Context::List];
const PANES: &[Context] = &[Context::List, Context::Detail];
const GLOBAL: &[Context] = &[Context::Global];

/// Every action lrat has, in the order hints and the cheatsheet list them.
/// This is the only place a key binding is written down.
pub static ACTIONS: &[ActionSpec] = &[
    ActionSpec {
        action: Action::MoveDown,
        id: "move-down",
        description: "Move down",
        hint: Some("down"),
        contexts: PANES,
        default_keys: &[&[Key::char('j')]],
        available: always,
    },
    ActionSpec {
        action: Action::MoveUp,
        id: "move-up",
        description: "Move up",
        hint: Some("up"),
        contexts: PANES,
        default_keys: &[&[Key::char('k')]],
        available: always,
    },
    ActionSpec {
        action: Action::MoveToTop,
        id: "move-to-top",
        description: "Move to the top",
        hint: Some("top"),
        contexts: PANES,
        default_keys: &[&[Key::char('g'), Key::char('g')]],
        available: always,
    },
    ActionSpec {
        action: Action::MoveToBottom,
        id: "move-to-bottom",
        description: "Move to the bottom",
        hint: Some("bottom"),
        contexts: PANES,
        default_keys: &[&[Key::char('G')]],
        available: always,
    },
    ActionSpec {
        action: Action::HalfPageDown,
        id: "half-page-down",
        description: "Move down half a page",
        hint: None,
        contexts: PANES,
        default_keys: &[&[Key::ctrl('d')]],
        available: always,
    },
    ActionSpec {
        action: Action::HalfPageUp,
        id: "half-page-up",
        description: "Move up half a page",
        hint: None,
        contexts: PANES,
        default_keys: &[&[Key::ctrl('u')]],
        available: always,
    },
    ActionSpec {
        action: Action::FocusLeft,
        id: "focus-left",
        description: "Focus the pane on the left",
        hint: None,
        contexts: PANES,
        default_keys: &[&[Key::char('h')]],
        available: always,
    },
    ActionSpec {
        action: Action::FocusRight,
        id: "focus-right",
        description: "Focus the pane on the right",
        hint: None,
        contexts: PANES,
        default_keys: &[&[Key::char('l')]],
        available: always,
    },
    ActionSpec {
        action: Action::Open,
        id: "open",
        description: "Open the focused issue",
        hint: Some("open"),
        contexts: LIST,
        default_keys: &[&[Key::named(KeyCode::Enter)]],
        available: always,
    },
    ActionSpec {
        action: Action::Close,
        id: "close",
        description: "Close the top overlay or step back",
        hint: Some("close"),
        contexts: &[Context::List, Context::Detail, Context::Overlay],
        default_keys: &[&[Key::named(KeyCode::Esc)]],
        available: |state| state.modals.top().is_some(),
    },
    ActionSpec {
        action: Action::ShowScreen(Screen::MyIssues),
        id: "show-my-issues",
        description: "Show My Issues",
        hint: None,
        contexts: GLOBAL,
        default_keys: &[&[Key::char('1')]],
        available: |state| state.screen != Screen::MyIssues,
    },
    ActionSpec {
        action: Action::ShowScreen(Screen::TeamIssues),
        id: "show-team-issues",
        description: "Show Team Issues",
        hint: None,
        contexts: GLOBAL,
        default_keys: &[&[Key::char('2')]],
        available: |state| state.screen != Screen::TeamIssues,
    },
    ActionSpec {
        action: Action::ShowScreen(Screen::Analytics),
        id: "show-analytics",
        description: "Show Analytics",
        hint: None,
        contexts: GLOBAL,
        default_keys: &[&[Key::char('3')]],
        available: |state| state.screen != Screen::Analytics,
    },
    ActionSpec {
        action: Action::ShowCheatsheet,
        id: "show-cheatsheet",
        description: "Show every key for this context",
        hint: Some("keys"),
        contexts: GLOBAL,
        default_keys: &[&[Key::char('?')]],
        available: always,
    },
    ActionSpec {
        action: Action::Quit,
        id: "quit",
        description: "Quit lrat",
        hint: Some("quit"),
        contexts: GLOBAL,
        default_keys: &[&[Key::char('q')]],
        available: always,
    },
];

fn always(_: &State) -> bool {
    true
}

/// The registry entry for `action`.
pub fn spec(action: Action) -> Option<&'static ActionSpec> {
    ACTIONS.iter().find(|spec| spec.action == action)
}

/// The registry entry whose config name is `id`.
pub fn by_id(id: &str) -> Option<&'static ActionSpec> {
    ACTIONS.iter().find(|spec| spec.id == id)
}
