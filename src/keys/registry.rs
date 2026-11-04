use crossterm::event::KeyCode;

use super::{Context, Key};
use crate::analytics::WeekRange;
use crate::app::{
    self, Action, AnalyticsView, Copyable, EditPicker, Editable, FilterField, LinkSet, Modal, Pane,
    Phase, Screen, State, can_load_more,
};
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
const TEAM_ISSUES: &[Context] = &[Context::TeamIssues];
const PANES: &[Context] = &[Context::List, Context::Detail];
const GLOBAL: &[Context] = &[Context::Global];
const OVERLAY: &[Context] = &[Context::Overlay];
const ANALYTICS: &[Context] = &[Context::Analytics];
/// The overlays and the Analytics screen both scroll by row.
const SCROLLS: &[Context] = &[Context::Overlay, Context::Analytics];

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
        action: Action::OpenLinks(LinkSet::Issue),
        id: "open-in-browser",
        description: "Open the issue or a pull request",
        hint: does("browser"),
        contexts: PANES,
        default_keys: &[&[Key::char('o')]],
        available: has_focused_issue,
    },
    ActionSpec {
        action: Action::OpenLinks(LinkSet::Other),
        id: "open-link",
        description: "Open another link of the issue",
        hint: None,
        contexts: PANES,
        default_keys: &[&[Key::char('O')]],
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
        action: Action::Edit(Editable::Labels),
        id: "edit-labels",
        description: "Add or remove labels",
        hint: None,
        contexts: PANES,
        default_keys: &[&[Key::char('L')]],
        available: has_focused_issue,
    },
    ActionSpec {
        action: Action::Edit(Editable::Estimate),
        id: "edit-estimate",
        description: "Change the estimate",
        hint: None,
        contexts: PANES,
        default_keys: &[&[Key::char('e')]],
        available: app::focused_team_estimates,
    },
    ActionSpec {
        action: Action::Comment,
        id: "comment",
        description: "Comment in $EDITOR",
        hint: None,
        contexts: PANES,
        default_keys: &[&[Key::char('c')]],
        available: |state| {
            state
                .focused_issue()
                .is_some_and(|issue| !app::is_local(&issue.id))
        },
    },
    ActionSpec {
        action: Action::CreateIssue,
        id: "create-issue",
        description: "Create an issue",
        hint: None,
        contexts: PANES,
        default_keys: &[&[Key::char('n')]],
        available: |state| state.store.teams().next().is_some(),
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
        action: Action::SwitchTeam,
        id: "switch-team",
        description: "Switch the team of Team Issues",
        hint: None,
        contexts: TEAM_ISSUES,
        default_keys: &[&[Key::char('t')]],
        available: |state| state.store.viewer_teams().next().is_some(),
    },
    ActionSpec {
        action: Action::FilterBy(FilterField::State),
        id: "filter-by-state",
        description: "Filter by workflow state",
        hint: None,
        contexts: TEAM_ISSUES,
        default_keys: &[&[Key::char('f'), Key::char('s')]],
        available: has_team,
    },
    ActionSpec {
        action: Action::FilterBy(FilterField::Assignee),
        id: "filter-by-assignee",
        description: "Filter by assignee",
        hint: None,
        contexts: TEAM_ISSUES,
        default_keys: &[&[Key::char('f'), Key::char('a')]],
        available: has_team,
    },
    ActionSpec {
        action: Action::FilterBy(FilterField::Label),
        id: "filter-by-label",
        description: "Filter by label",
        hint: None,
        contexts: TEAM_ISSUES,
        default_keys: &[&[Key::char('f'), Key::char('l')]],
        available: has_team,
    },
    ActionSpec {
        action: Action::FilterBy(FilterField::Cycle),
        id: "filter-by-cycle",
        description: "Filter by cycle",
        hint: None,
        contexts: TEAM_ISSUES,
        default_keys: &[&[Key::char('f'), Key::char('c')]],
        available: has_team,
    },
    ActionSpec {
        action: Action::FilterBy(FilterField::Project),
        id: "filter-by-project",
        description: "Filter by project",
        hint: None,
        contexts: TEAM_ISSUES,
        default_keys: &[&[Key::char('f'), Key::char('p')]],
        available: has_team,
    },
    ActionSpec {
        action: Action::LoadMore,
        id: "load-more",
        description: "Load the next page of Team Issues",
        hint: None,
        contexts: TEAM_ISSUES,
        default_keys: &[&[Key::char('m')]],
        available: can_load_more,
    },
    ActionSpec {
        action: Action::ScrollDown,
        id: "scroll-down",
        description: "Scroll down",
        hint: moves("move"),
        contexts: SCROLLS,
        default_keys: &[
            &[Key::char('j')],
            &[Key::named(KeyCode::Down)],
            &[Key::ctrl('n')],
        ],
        available: can_scroll_down,
    },
    ActionSpec {
        action: Action::ScrollUp,
        id: "scroll-up",
        description: "Scroll up",
        hint: moves("move"),
        contexts: SCROLLS,
        default_keys: &[
            &[Key::char('k')],
            &[Key::named(KeyCode::Up)],
            &[Key::ctrl('p')],
        ],
        available: can_scroll_up,
    },
    ActionSpec {
        action: Action::ToggleEntry,
        id: "toggle-entry",
        description: "Mark or unmark the highlighted entry",
        hint: None,
        contexts: OVERLAY,
        default_keys: &[&[Key::named(KeyCode::Tab)]],
        available: |state| {
            picker(state)
                .is_some_and(|picker| picker.list.is_multi() && picker.list.selected().is_some())
        },
    },
    ActionSpec {
        action: Action::Accept,
        id: "accept",
        description: "Choose the highlighted entry",
        hint: None,
        contexts: OVERLAY,
        default_keys: &[&[Key::named(KeyCode::Enter)]],
        available: can_accept,
    },
    ActionSpec {
        action: Action::ShorterRange,
        id: "shorter-range",
        description: "Show a shorter range",
        hint: does("range"),
        contexts: ANALYTICS,
        default_keys: &[&[Key::char('[')]],
        available: |state| state.analytics_range() != WeekRange::Four,
    },
    ActionSpec {
        action: Action::LongerRange,
        id: "longer-range",
        description: "Show a longer range",
        hint: does("range"),
        contexts: ANALYTICS,
        default_keys: &[&[Key::char(']')]],
        available: |state| state.analytics_range() != WeekRange::TwentySix,
    },
    ActionSpec {
        action: Action::ToggleAnalyticsView,
        id: "toggle-analytics-view",
        description: "Show Personal or Team analytics",
        hint: None,
        contexts: ANALYTICS,
        default_keys: &[&[Key::char('v')]],
        available: always,
    },
    ActionSpec {
        action: Action::SwitchAnalyticsTeam,
        id: "switch-analytics-team",
        description: "Pick the team for Team analytics",
        hint: None,
        contexts: ANALYTICS,
        default_keys: &[&[Key::char('t')]],
        available: |state| {
            state.analytics_screen.view() == AnalyticsView::Team
                && state.store.viewer_teams().next().is_some()
        },
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
        action: Action::ShowPalette,
        id: "show-palette",
        description: "Find an action or loaded issue",
        hint: None,
        contexts: GLOBAL,
        default_keys: &[&[Key::char(':')], &[Key::ctrl('k')]],
        available: always,
    },
    ActionSpec {
        action: Action::ShowSearch,
        id: "show-search",
        description: "Search all issues in Linear",
        hint: None,
        contexts: GLOBAL,
        default_keys: &[&[Key::char('/')]],
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

fn can_scroll_down(state: &State) -> bool {
    match state.modals.top() {
        Some(Modal::Choice(choice)) => choice.cursor + 1 < choice.entries.len(),
        Some(Modal::Picker(picker)) => picker
            .list
            .selected_position()
            .is_some_and(|position| position + 1 < picker.list.visible().len()),
        Some(Modal::Links(links)) => links
            .list
            .selected_position()
            .is_some_and(|position| position + 1 < links.list.visible().len()),
        Some(Modal::Create(form)) => form
            .list()
            .is_some_and(|(len, selected)| selected.is_some_and(|position| position + 1 < len)),
        Some(Modal::Issue { id }) => {
            let (scroll, max) = overlay_issue_scroll(state, id);
            scroll < max
        }
        Some(Modal::Cheatsheet { scroll }) => {
            *scroll < ui::cheatsheet_max_scroll(state, state.viewport)
        }
        Some(Modal::Palette(palette)) => {
            palette.selected + 1 < app::palette_matches(state, &palette.query).len()
        }
        Some(Modal::Search(search)) => match &search.phase {
            Phase::Found { ids, selected, .. } => selected + 1 < ids.len(),
            _ => false,
        },
        None if in_analytics(state) => {
            state.analytics_screen.scroll() < ui::analytics_max_scroll(state, state.viewport)
        }
        _ => false,
    }
}

fn can_scroll_up(state: &State) -> bool {
    match state.modals.top() {
        Some(Modal::Choice(choice)) => choice.cursor > 0,
        Some(Modal::Picker(picker)) => picker
            .list
            .selected_position()
            .is_some_and(|position| position > 0),
        Some(Modal::Links(links)) => links
            .list
            .selected_position()
            .is_some_and(|position| position > 0),
        Some(Modal::Create(form)) => form
            .list()
            .is_some_and(|(_, selected)| selected.is_some_and(|position| position > 0)),
        Some(Modal::Issue { id }) => overlay_issue_scroll(state, id).0 > 0,
        Some(Modal::Cheatsheet { scroll }) => *scroll > 0,
        Some(Modal::Palette(palette)) => palette.selected > 0,
        Some(Modal::Search(search)) => {
            matches!(search.phase, Phase::Found { selected, .. } if selected > 0)
        }
        None if in_analytics(state) => state.analytics_screen.scroll() > 0,
        _ => false,
    }
}

fn can_accept(state: &State) -> bool {
    match state.modals.top() {
        Some(Modal::Choice(choice)) => choice.cursor < choice.entries.len(),
        Some(Modal::Picker(picker)) => picker.list.is_multi() || picker.list.selected().is_some(),
        Some(Modal::Links(links)) => links.list.selected().is_some(),
        Some(Modal::Create(form)) => form.list().is_none_or(|(_, selected)| selected.is_some()),
        Some(Modal::Palette(palette)) => {
            palette.selected < app::palette_matches(state, &palette.query).len()
        }
        Some(Modal::Search(search)) => match &search.phase {
            Phase::Typing { .. } => !search.query.trim().is_empty(),
            Phase::Searching { .. } => false,
            Phase::Found { ids, .. } => !ids.is_empty(),
        },
        _ => false,
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
    if let Some(picker) = picker(state) {
        return picker.list.visible().len() >= count;
    }
    match state.modals.top() {
        Some(Modal::Links(links)) => links.list.visible().len() >= count,
        Some(Modal::Create(form)) => form.list().is_some_and(|(len, _)| len >= count),
        _ => false,
    }
}

fn has_team(state: &State) -> bool {
    state.team_issues.query().is_some()
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

/// How far the issue overlay showing `issue_id` is scrolled and how far it
/// can scroll.
fn overlay_issue_scroll(state: &State, issue_id: &str) -> (usize, usize) {
    let (rows, lines) = ui::issue_overlay_rows(state, state.viewport, issue_id);
    let max = lines.saturating_sub(rows);
    (state.detail.scroll(issue_id).min(max), max)
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

fn in_analytics(state: &State) -> bool {
    state.screen == Screen::Analytics && state.modals.top().is_none()
}

fn in_detail(state: &State) -> bool {
    match state.screen {
        Screen::MyIssues => state.my_issues.pane() == Pane::Detail,
        Screen::TeamIssues => state.team_issues.pane() == Pane::Detail,
        Screen::Analytics => false,
    }
}

/// The registry entry for `action`.
pub fn spec(action: Action) -> Option<&'static ActionSpec> {
    ACTIONS.iter().find(|spec| spec.action == action)
}

/// The registry entry whose config name is `id`.
pub fn by_id(id: &str) -> Option<&'static ActionSpec> {
    ACTIONS.iter().find(|spec| spec.id == id)
}
