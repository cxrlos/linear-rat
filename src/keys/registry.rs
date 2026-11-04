use crossterm::event::KeyCode;

use super::{Context, Key};
use crate::app::{Action, Copyable, Modal, Pane, Screen, State};
use crate::ui;

/// One action as the viewer sees it: what it is called, where it applies,
/// when it can run and which keys run it by default (the vim preset).
#[derive(Debug)]
pub struct ActionSpec {
    pub action: Action,
    /// The name used in `config.toml`, for example `move-down`.
    pub id: &'static str,
    /// What the cheatsheet and the palette say it does.
    pub description: &'static str,
    /// Where the action shows in the bottom bar, if it does.
    pub hint: Option<Hint>,
    pub contexts: &'static [Context],
    pub default_keys: &'static [&'static [Key]],
    /// Whether it can run now. Unavailable actions are still listed, muted.
    pub available: fn(&State) -> bool,
}

/// An action's place in the bottom bar: its row and the word after its key.
/// Actions in one row with the same word share an item.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Hint {
    pub row: HintRow,
    pub word: &'static str,
}

/// The two rows of the bottom bar, top first.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HintRow {
    Move,
    Do,
}

impl HintRow {
    pub const ALL: [HintRow; 2] = [HintRow::Move, HintRow::Do];

    /// The label at the start of the row.
    pub fn label(self) -> &'static str {
        match self {
            HintRow::Move => "Move",
            HintRow::Do => "Do",
        }
    }
}

const fn moves(word: &'static str) -> Option<Hint> {
    Some(Hint {
        row: HintRow::Move,
        word,
    })
}

const fn does(word: &'static str) -> Option<Hint> {
    Some(Hint {
        row: HintRow::Do,
        word,
    })
}

const LIST: &[Context] = &[Context::List];
const PANES: &[Context] = &[Context::List, Context::Detail];
const GLOBAL: &[Context] = &[Context::Global];
const OVERLAY: &[Context] = &[Context::Overlay];

/// Every action lrat has, in the order hints and the cheatsheet list them.
/// This is the only place a key binding is written down.
pub static ACTIONS: &[ActionSpec] = &[
    ActionSpec {
        action: Action::MoveDown,
        id: "move-down",
        description: "Move down",
        hint: moves("down"),
        contexts: PANES,
        default_keys: &[&[Key::char('j')]],
        available: not_in_detail,
    },
    ActionSpec {
        action: Action::MoveUp,
        id: "move-up",
        description: "Move up",
        hint: moves("up"),
        contexts: PANES,
        default_keys: &[&[Key::char('k')]],
        available: not_in_detail,
    },
    ActionSpec {
        action: Action::MoveToTop,
        id: "move-to-top",
        description: "Move to the top",
        hint: moves("top"),
        contexts: PANES,
        default_keys: &[&[Key::char('g'), Key::char('g')]],
        available: not_in_detail,
    },
    ActionSpec {
        action: Action::MoveToBottom,
        id: "move-to-bottom",
        description: "Move to the bottom",
        hint: moves("bottom"),
        contexts: PANES,
        default_keys: &[&[Key::char('G')]],
        available: not_in_detail,
    },
    ActionSpec {
        action: Action::HalfPageDown,
        id: "half-page-down",
        description: "Move down half a page",
        hint: moves("page"),
        contexts: PANES,
        default_keys: &[&[Key::ctrl('d')]],
        available: not_in_detail,
    },
    ActionSpec {
        action: Action::HalfPageUp,
        id: "half-page-up",
        description: "Move up half a page",
        hint: moves("page"),
        contexts: PANES,
        default_keys: &[&[Key::ctrl('u')]],
        available: not_in_detail,
    },
    ActionSpec {
        action: Action::FocusLeft,
        id: "focus-left",
        description: "Focus the pane on the left",
        hint: moves("pane"),
        contexts: PANES,
        default_keys: &[&[Key::char('h')]],
        available: always,
    },
    ActionSpec {
        action: Action::FocusRight,
        id: "focus-right",
        description: "Focus the pane on the right",
        hint: moves("pane"),
        contexts: PANES,
        default_keys: &[&[Key::char('l')]],
        available: always,
    },
    ActionSpec {
        action: Action::Open,
        id: "open",
        description: "Open the focused issue",
        hint: does("open"),
        contexts: LIST,
        default_keys: &[&[Key::named(KeyCode::Enter)]],
        available: always,
    },
    ActionSpec {
        action: Action::Copy(Copyable::Identifier),
        id: "copy-identifier",
        description: "Copy the identifier",
        hint: does("yank"),
        contexts: PANES,
        default_keys: &[&[Key::char('y'), Key::char('i')]],
        available: has_focused_issue,
    },
    ActionSpec {
        action: Action::Copy(Copyable::Url),
        id: "copy-url",
        description: "Copy the URL",
        hint: does("yank"),
        contexts: PANES,
        default_keys: &[&[Key::char('y'), Key::char('u')]],
        available: has_focused_issue,
    },
    ActionSpec {
        action: Action::Copy(Copyable::BranchName),
        id: "copy-branch-name",
        description: "Copy the branch name",
        hint: does("yank"),
        contexts: PANES,
        default_keys: &[&[Key::char('y'), Key::char('b')]],
        available: has_focused_issue,
    },
    ActionSpec {
        action: Action::Copy(Copyable::MarkdownLink),
        id: "copy-markdown-link",
        description: "Copy a Markdown link",
        hint: does("yank"),
        contexts: PANES,
        default_keys: &[&[Key::char('y'), Key::char('m')]],
        available: has_focused_issue,
    },
    ActionSpec {
        action: Action::OpenInBrowser,
        id: "open-in-browser",
        description: "Open in the browser",
        hint: does("browser"),
        contexts: PANES,
        default_keys: &[&[Key::char('o')]],
        available: has_focused_issue,
    },
    ActionSpec {
        action: Action::ScrollDown,
        id: "scroll-down",
        description: "Scroll down",
        hint: moves("down"),
        contexts: OVERLAY,
        default_keys: &[&[Key::char('j')]],
        available: |state| {
            cheatsheet_scroll(state)
                .is_some_and(|scroll| scroll < ui::cheatsheet_max_scroll(state, state.viewport))
        },
    },
    ActionSpec {
        action: Action::ScrollUp,
        id: "scroll-up",
        description: "Scroll up",
        hint: moves("up"),
        contexts: OVERLAY,
        default_keys: &[&[Key::char('k')]],
        available: |state| cheatsheet_scroll(state).is_some_and(|scroll| scroll > 0),
    },
    ActionSpec {
        action: Action::Close,
        id: "close",
        description: "Close the top overlay or step back",
        hint: does("close"),
        contexts: &[Context::List, Context::Detail, Context::Overlay],
        default_keys: &[&[Key::named(KeyCode::Esc)]],
        available: |state| state.modals.top().is_some() || in_detail(state),
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
        action: Action::Refresh,
        id: "refresh",
        description: "Refresh now",
        hint: None,
        contexts: GLOBAL,
        default_keys: &[&[Key::char('r')]],
        available: |state| state.can_refresh(),
    },
    ActionSpec {
        action: Action::ShowCheatsheet,
        id: "show-cheatsheet",
        description: "Show every key for this context",
        hint: does("keys"),
        contexts: GLOBAL,
        default_keys: &[&[Key::char('?')]],
        available: always,
    },
    ActionSpec {
        action: Action::Quit,
        id: "quit",
        description: "Quit lrat",
        hint: does("quit"),
        contexts: GLOBAL,
        default_keys: &[&[Key::char('q')]],
        available: always,
    },
];

fn always(_: &State) -> bool {
    true
}

fn cheatsheet_scroll(state: &State) -> Option<u16> {
    match state.modals.top() {
        Some(Modal::Cheatsheet { scroll }) => Some(*scroll),
        _ => None,
    }
}

fn has_focused_issue(state: &State) -> bool {
    state.focused_issue().is_some()
}

// The detail does not scroll yet, so moves do nothing there.
fn not_in_detail(state: &State) -> bool {
    !in_detail(state)
}

fn in_detail(state: &State) -> bool {
    state.screen == Screen::MyIssues && state.my_issues.pane() == Pane::Detail
}

/// The registry entry for `action`.
pub fn spec(action: Action) -> Option<&'static ActionSpec> {
    ACTIONS.iter().find(|spec| spec.action == action)
}

/// The registry entry whose config name is `id`.
pub fn by_id(id: &str) -> Option<&'static ActionSpec> {
    ACTIONS.iter().find(|spec| spec.id == id)
}
