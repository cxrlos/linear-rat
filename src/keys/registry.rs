use crossterm::event::KeyCode;

use super::{Context, Key};
use crate::app::{Action, Copyable, EditPicker, Editable, Modal, Pane, Screen, State};
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
        available: can_move_down,
    },
    ActionSpec {
        action: Action::MoveUp,
        id: "move-up",
        description: "Move up",
        hint: moves("up"),
        contexts: PANES,
        default_keys: &[&[Key::char('k')]],
        available: can_move_up,
    },
    ActionSpec {
        action: Action::MoveToTop,
        id: "move-to-top",
        description: "Move to the top",
        hint: moves("top"),
        contexts: PANES,
        default_keys: &[&[Key::char('g'), Key::char('g')]],
        available: can_move_up,
    },
    ActionSpec {
        action: Action::MoveToBottom,
        id: "move-to-bottom",
        description: "Move to the bottom",
        hint: moves("bottom"),
        contexts: PANES,
        default_keys: &[&[Key::char('G')]],
        available: can_move_down,
    },
    ActionSpec {
        action: Action::HalfPageDown,
        id: "half-page-down",
        description: "Move down half a page",
        hint: moves("page"),
        contexts: PANES,
        default_keys: &[&[Key::ctrl('d')]],
        available: can_move_down,
    },
    ActionSpec {
        action: Action::HalfPageUp,
        id: "half-page-up",
        description: "Move up half a page",
        hint: moves("page"),
        contexts: PANES,
        default_keys: &[&[Key::ctrl('u')]],
        available: can_move_up,
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
        action: Action::Edit(Editable::State),
        id: "edit-state",
        description: "Change the workflow state",
        hint: None,
        contexts: PANES,
        default_keys: &[&[Key::char('s')]],
        available: has_focused_issue,
    },
    ActionSpec {
        action: Action::Edit(Editable::Assignee),
        id: "edit-assignee",
        description: "Change the assignee",
        hint: None,
        contexts: PANES,
        default_keys: &[&[Key::char('a')]],
        available: has_focused_issue,
    },
    ActionSpec {
        action: Action::Edit(Editable::Priority),
        id: "edit-priority",
        description: "Change the priority",
        hint: None,
        contexts: PANES,
        default_keys: &[&[Key::char('p')]],
        available: has_focused_issue,
    },
    ActionSpec {
        action: Action::SelectNext,
        id: "select-next",
        description: "Select the next entry",
        hint: None,
        contexts: OVERLAY,
        default_keys: &[&[Key::named(KeyCode::Down)], &[Key::ctrl('n')]],
        available: |state| picker_has(1, state),
    },
    ActionSpec {
        action: Action::SelectPrevious,
        id: "select-previous",
        description: "Select the previous entry",
        hint: None,
        contexts: OVERLAY,
        default_keys: &[&[Key::named(KeyCode::Up)], &[Key::ctrl('p')]],
        available: |state| picker_has(1, state),
    },
    ActionSpec {
        action: Action::Choose,
        id: "choose",
        description: "Choose the selected entry",
        hint: None,
        contexts: OVERLAY,
        default_keys: &[&[Key::named(KeyCode::Enter)]],
        available: |state| picker_has(1, state),
    },
    ActionSpec {
        action: Action::PickNumber(1),
        id: "pick-1",
        description: "Pick entry 1 of the list",
        hint: None,
        contexts: OVERLAY,
        default_keys: &[&[Key::char('1')]],
        available: |state| picker_has(1, state),
    },
    ActionSpec {
        action: Action::PickNumber(2),
        id: "pick-2",
        description: "Pick entry 2 of the list",
        hint: None,
        contexts: OVERLAY,
        default_keys: &[&[Key::char('2')]],
        available: |state| picker_has(2, state),
    },
    ActionSpec {
        action: Action::PickNumber(3),
        id: "pick-3",
        description: "Pick entry 3 of the list",
        hint: None,
        contexts: OVERLAY,
        default_keys: &[&[Key::char('3')]],
        available: |state| picker_has(3, state),
    },
    ActionSpec {
        action: Action::PickNumber(4),
        id: "pick-4",
        description: "Pick entry 4 of the list",
        hint: None,
        contexts: OVERLAY,
        default_keys: &[&[Key::char('4')]],
        available: |state| picker_has(4, state),
    },
    ActionSpec {
        action: Action::PickNumber(5),
        id: "pick-5",
        description: "Pick entry 5 of the list",
        hint: None,
        contexts: OVERLAY,
        default_keys: &[&[Key::char('5')]],
        available: |state| picker_has(5, state),
    },
    ActionSpec {
        action: Action::PickNumber(6),
        id: "pick-6",
        description: "Pick entry 6 of the list",
        hint: None,
        contexts: OVERLAY,
        default_keys: &[&[Key::char('6')]],
        available: |state| picker_has(6, state),
    },
    ActionSpec {
        action: Action::PickNumber(7),
        id: "pick-7",
        description: "Pick entry 7 of the list",
        hint: None,
        contexts: OVERLAY,
        default_keys: &[&[Key::char('7')]],
        available: |state| picker_has(7, state),
    },
    ActionSpec {
        action: Action::PickNumber(8),
        id: "pick-8",
        description: "Pick entry 8 of the list",
        hint: None,
        contexts: OVERLAY,
        default_keys: &[&[Key::char('8')]],
        available: |state| picker_has(8, state),
    },
    ActionSpec {
        action: Action::PickNumber(9),
        id: "pick-9",
        description: "Pick entry 9 of the list",
        hint: None,
        contexts: OVERLAY,
        default_keys: &[&[Key::char('9')]],
        available: |state| picker_has(9, state),
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

fn picker(state: &State) -> Option<&EditPicker> {
    match state.modals.top() {
        Some(Modal::Picker(picker)) => Some(picker),
        _ => None,
    }
}

/// Whether a picker is open with at least `count` entries left by its
/// filter.
fn picker_has(count: usize, state: &State) -> bool {
    picker(state).is_some_and(|picker| picker.list.visible().len() >= count)
}

fn has_focused_issue(state: &State) -> bool {
    state.focused_issue().is_some()
}

// In the detail, moves scroll it, so they are available while it has more
// to show that way.
fn can_move_down(state: &State) -> bool {
    !in_detail(state) || {
        let (scroll, max) = detail_scroll(state);
        scroll < max
    }
}

fn can_move_up(state: &State) -> bool {
    !in_detail(state) || detail_scroll(state).0 > 0
}

/// How far the detail is scrolled and how far it can scroll.
fn detail_scroll(state: &State) -> (usize, usize) {
    let (rows, lines) = ui::detail_rows(state, state.viewport);
    let max = lines.saturating_sub(rows);
    let scroll = state
        .focused_issue()
        .map_or(0, |issue| state.detail.scroll(&issue.id));
    (scroll.min(max), max)
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
