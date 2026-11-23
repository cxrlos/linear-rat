use std::borrow::Cow;

use crossterm::event::KeyCode;

use super::{Context, Key};
use crate::analytics::WeekRange;
use crate::app::{
    self, Action, AnalyticsView, AttachStep, Copyable, EditPicker, Editable, FilterField, LinkSet,
    Modal, Pane, Phase, Screen, State, WriteClass, can_load_more,
};
use crate::ui;

/// One action as the viewer sees it: what it is called, where it applies,
/// when it can run and which keys run it by default (the vim preset).
#[derive(Debug)]
pub struct ActionSpec {
    pub action: Action,
    /// The name used in `config.toml`, for example `move-down`.
    pub id: &'static str,
    /// What the palette and the keymap reference say it does.
    pub description: &'static str,
    /// What the cheatsheet says it does, in a few words. Actions of one
    /// pane with the same label share a line, as `j/k  down / up`.
    pub label: &'static str,
    /// The cheatsheet pane it is listed in.
    pub group: Group,
    /// The kind of write it sends to the focused issue's team, so it is
    /// denied where the API key is known not to be allowed that.
    pub writes: Option<WriteClass>,
    /// Where the action shows in the bottom bar, if it does.
    pub hint: Option<Hint>,
    pub contexts: &'static [Context],
    /// The kinds of overlay it applies to, when it binds in
    /// [`Context::Overlay`]. The bottom bar leaves it out over any other
    /// kind, where it can never run; over these it is muted while it cannot
    /// run now.
    pub overlays: &'static [OverlayKind],
    pub default_keys: &'static [&'static [Key]],
    /// Whether it can run now, the API key aside. Unavailable actions are
    /// still listed, muted.
    pub available: fn(&State) -> bool,
}

impl ActionSpec {
    /// What the palette says it does on `state`: its description, but with
    /// the version named where it is about a release.
    pub fn description_in(&self, state: &State) -> Cow<'static, str> {
        match (self.action, state.release.notice()) {
            (Action::ShowReleaseNotes, Some(version)) => {
                Cow::Owned(format!("What's new in v{version}"))
            }
            _ => Cow::Borrowed(self.description),
        }
    }

    /// Where the bottom bar shows it on `state`: its hint, but with the word
    /// the open overlay uses for it, as Tab completing a path in the attach
    /// prompt.
    pub fn hint_in(&self, state: &State) -> Option<Hint> {
        match (self.action, state.modals.top()) {
            (Action::ToggleEntry, Some(Modal::Attach(_))) => self.hint.map(|hint| Hint {
                word: "complete",
                ..hint
            }),
            _ => self.hint,
        }
    }

    /// Whether it can run now: [`ActionSpec::available`], and then whether
    /// the API key may send what it [`writes`](ActionSpec::writes).
    pub fn availability(&self, state: &State) -> Availability {
        if !(self.available)(state) {
            return Availability::Unavailable;
        }
        match self.writes.and_then(|class| state.write_denied(class)) {
            Some(reason) => Availability::Denied(reason),
            None => Availability::Available,
        }
    }
}

/// Whether an action can run now, and why not when the API key is the
/// reason.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Availability {
    Available,
    /// It does not apply to what is on screen now.
    Unavailable,
    /// The API key is not allowed to do it; the reason says so, as `this
    /// API key cannot edit issues in ENG`.
    Denied(String),
}

impl Availability {
    pub fn can_run(&self) -> bool {
        *self == Availability::Available
    }

    pub fn reason(&self) -> Option<&str> {
        match self {
            Availability::Denied(reason) => Some(reason),
            Availability::Available | Availability::Unavailable => None,
        }
    }
}

/// The cheatsheet's panes by purpose, in the order they are laid out.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Group {
    Move,
    Edit,
    Open,
    App,
    /// Keys of one screen or of the overlays, as the Team Issues filters
    /// and the Analytics range. Its pane is named after their context.
    Screen,
}

impl Group {
    /// The pane's title, or `None` for [`Group::Screen`], whose title is
    /// its context's.
    pub fn title(self) -> Option<&'static str> {
        match self {
            Group::Move => Some("Move"),
            Group::Edit => Some("Edit the issue"),
            Group::Open => Some("Open & copy"),
            Group::App => Some("App"),
            Group::Screen => None,
        }
    }
}

/// An action's place in the bottom bar: its row and the word after its key.
/// Actions in one row with the same word share an item.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Hint {
    pub row: HintRow,
    pub word: &'static str,
}

/// The rows of the bottom bar. [`HintRow::LINES`] places them on its three
/// lines.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HintRow {
    Move,
    Do,
    /// The issue edits.
    Edit,
    /// A screen's own keys, which change what it shows.
    View,
}

impl HintRow {
    /// The bar's lines, top first, each with the rows it holds in order.
    /// A screen's own keys lead the third line, so the issue edits after
    /// them are the first to go when it runs out of room.
    pub const LINES: [&[HintRow]; 3] = [
        &[HintRow::Move],
        &[HintRow::Do],
        &[HintRow::View, HintRow::Edit],
    ];

    /// The label at the start of the row.
    pub fn label(self) -> &'static str {
        match self {
            HintRow::Move => "Move",
            HintRow::Do => "Do",
            HintRow::Edit => "Edit",
            HintRow::View => "View",
        }
    }
}

const fn hint(row: HintRow, word: &'static str) -> Option<Hint> {
    Some(Hint { row, word })
}

const fn moves(word: &'static str) -> Option<Hint> {
    hint(HintRow::Move, word)
}

const fn does(word: &'static str) -> Option<Hint> {
    hint(HintRow::Do, word)
}

const fn edits(word: &'static str) -> Option<Hint> {
    hint(HintRow::Edit, word)
}

const fn views(word: &'static str) -> Option<Hint> {
    hint(HintRow::View, word)
}

const LIST: &[Context] = &[Context::List];
const TEAM_ISSUES: &[Context] = &[Context::TeamIssues];
const PANES: &[Context] = &[Context::List, Context::Detail];
/// Moves also go through the Cycles and Projects lists and the Inbox.
const MOVES: &[Context] = &[
    Context::List,
    Context::Detail,
    Context::Browse,
    Context::Inbox,
];
const BROWSE: &[Context] = &[Context::Browse];
const CYCLES: &[Context] = &[Context::Cycles];
const DETAIL: &[Context] = &[Context::Detail];
const COMMENT: &[Context] = &[Context::Comment];
const REACT: &[Context] = &[Context::Detail, Context::Comment];
const INBOX: &[Context] = &[Context::Inbox];
const GLOBAL: &[Context] = &[Context::Global];
const OVERLAY: &[Context] = &[Context::Overlay];
const ANALYTICS: &[Context] = &[Context::Analytics];
/// The overlays and the Analytics screen both scroll by row.
const SCROLLS: &[Context] = &[Context::Overlay, Context::Analytics];

/// What an overlay is, as far as which keys can ever apply to it goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OverlayKind {
    /// The cheatsheet, which scrolls.
    Cheatsheet,
    /// A list to choose one entry from: a single-select edit picker, a team
    /// switcher or filter, the create form, an issue picker for one issue.
    List,
    /// A list to mark entries in: the label picker, an issue picker for
    /// several issues.
    MultiList,
    /// A picker of an issue's links, whose URLs can also be copied.
    Links,
    Palette,
    Search,
    /// The prompt a due date is typed into.
    Prompt,
    /// A question answered with yes or no.
    Confirm,
    /// The error log, which scrolls.
    Messages,
    /// The prompt a file path is typed into, which Tab completes.
    Path,
}

impl OverlayKind {
    pub const ALL: [OverlayKind; 10] = [
        OverlayKind::Cheatsheet,
        OverlayKind::List,
        OverlayKind::MultiList,
        OverlayKind::Links,
        OverlayKind::Palette,
        OverlayKind::Search,
        OverlayKind::Prompt,
        OverlayKind::Confirm,
        OverlayKind::Messages,
        OverlayKind::Path,
    ];

    /// The kind of `modal`, or `None` for the which-key popup, which only
    /// waits for the rest of a chord.
    pub fn of(modal: &Modal) -> Option<Self> {
        Some(match modal {
            Modal::Cheatsheet { .. } => OverlayKind::Cheatsheet,
            Modal::WhichKey { .. } => return None,
            Modal::Picker(picker) if picker.list.is_multi() => OverlayKind::MultiList,
            Modal::IssuePicker(picker) if picker.list.is_multi() => OverlayKind::MultiList,
            Modal::Choice(choice) if choice.list.is_multi() => OverlayKind::MultiList,
            Modal::Picker(_) | Modal::IssuePicker(_) | Modal::Choice(_) | Modal::Create(_) => {
                OverlayKind::List
            }
            Modal::Links(_) => OverlayKind::Links,
            Modal::Palette(_) => OverlayKind::Palette,
            Modal::Search(_) => OverlayKind::Search,
            Modal::DueDate(_) => OverlayKind::Prompt,
            Modal::Confirm(_) => OverlayKind::Confirm,
            Modal::Messages { .. } => OverlayKind::Messages,
            Modal::Attach(prompt) if prompt.step == AttachStep::File => OverlayKind::Path,
            Modal::Attach(_) => OverlayKind::Prompt,
        })
    }
}

/// The lists, which numbers pick from.
const LISTS: &[OverlayKind] = &[
    OverlayKind::List,
    OverlayKind::MultiList,
    OverlayKind::Links,
];
/// The overlays with something to move through.
const SCROLLING: &[OverlayKind] = &[
    OverlayKind::Cheatsheet,
    OverlayKind::List,
    OverlayKind::MultiList,
    OverlayKind::Links,
    OverlayKind::Palette,
    OverlayKind::Messages,
];
/// The overlays Enter chooses or sends in.
const ACCEPTING: &[OverlayKind] = &[
    OverlayKind::List,
    OverlayKind::MultiList,
    OverlayKind::Links,
    OverlayKind::Palette,
    OverlayKind::Search,
    OverlayKind::Prompt,
    OverlayKind::Confirm,
    OverlayKind::Path,
];

/// Every action lrat has, in the order hints and the cheatsheet list them.
/// This is the only place a key binding is written down.
pub static ACTIONS: &[ActionSpec] = &[
    ActionSpec {
        action: Action::MoveDown,
        id: "move-down",
        description: "Move down",
        label: "down / up",
        group: Group::Move,
        writes: None,
        hint: moves("down"),
        contexts: MOVES,
        overlays: &[],
        default_keys: &[&[Key::char('j')]],
        available: can_move_down,
    },
    ActionSpec {
        action: Action::MoveUp,
        id: "move-up",
        description: "Move up",
        label: "down / up",
        group: Group::Move,
        writes: None,
        hint: moves("up"),
        contexts: MOVES,
        overlays: &[],
        default_keys: &[&[Key::char('k')]],
        available: can_move_up,
    },
    ActionSpec {
        action: Action::MoveToTop,
        id: "move-to-top",
        description: "Move to the top",
        label: "top / bottom",
        group: Group::Move,
        writes: None,
        hint: moves("top"),
        contexts: MOVES,
        overlays: &[],
        default_keys: &[&[Key::char('g'), Key::char('g')]],
        available: can_move_up,
    },
    ActionSpec {
        action: Action::MoveToBottom,
        id: "move-to-bottom",
        description: "Move to the bottom",
        label: "top / bottom",
        group: Group::Move,
        writes: None,
        hint: moves("bottom"),
        contexts: MOVES,
        overlays: &[],
        default_keys: &[&[Key::char('G')]],
        available: can_move_down,
    },
    ActionSpec {
        action: Action::HalfPageDown,
        id: "half-page-down",
        description: "Move down half a page",
        label: "half page",
        group: Group::Move,
        writes: None,
        hint: moves("page"),
        contexts: MOVES,
        overlays: &[],
        default_keys: &[&[Key::ctrl('d')]],
        available: can_move_down,
    },
    ActionSpec {
        action: Action::HalfPageUp,
        id: "half-page-up",
        description: "Move up half a page",
        label: "half page",
        group: Group::Move,
        writes: None,
        hint: moves("page"),
        contexts: MOVES,
        overlays: &[],
        default_keys: &[&[Key::ctrl('u')]],
        available: can_move_up,
    },
    ActionSpec {
        action: Action::FocusLeft,
        id: "focus-left",
        description: "Focus the pane on the left",
        label: "pane left / right",
        group: Group::Move,
        writes: None,
        hint: moves("pane"),
        contexts: PANES,
        overlays: &[],
        default_keys: &[&[Key::char('h')]],
        available: always,
    },
    ActionSpec {
        action: Action::FocusRight,
        id: "focus-right",
        description: "Focus the pane on the right",
        label: "pane left / right",
        group: Group::Move,
        writes: None,
        hint: moves("pane"),
        contexts: PANES,
        overlays: &[],
        default_keys: &[&[Key::char('l')]],
        available: always,
    },
    ActionSpec {
        action: Action::Open,
        id: "open",
        description: "Open the focused issue",
        label: "open the issue",
        group: Group::Open,
        writes: None,
        hint: does("open"),
        contexts: LIST,
        overlays: &[],
        default_keys: &[&[Key::named(KeyCode::Enter)]],
        available: always,
    },
    ActionSpec {
        action: Action::ListIssues,
        id: "list-issues",
        description: "List the issues of the focused cycle or project",
        label: "list its issues",
        group: Group::Open,
        writes: None,
        hint: does("issues"),
        contexts: BROWSE,
        overlays: &[],
        default_keys: &[&[Key::named(KeyCode::Enter)]],
        available: |state| match state.screen {
            Screen::Cycles => state.cycles.focused().is_some(),
            Screen::Projects => state.projects.focused().is_some(),
            _ => false,
        },
    },
    ActionSpec {
        action: Action::Copy(Copyable::Identifier),
        id: "copy-identifier",
        description: "Copy the identifier",
        label: "copy key / URL",
        group: Group::Open,
        writes: None,
        hint: does("yank"),
        contexts: PANES,
        overlays: &[],
        default_keys: &[&[Key::char('y'), Key::char('i')]],
        available: has_focused_issue,
    },
    ActionSpec {
        action: Action::Copy(Copyable::Url),
        id: "copy-url",
        description: "Copy the URL",
        label: "copy key / URL",
        group: Group::Open,
        writes: None,
        hint: does("yank"),
        contexts: PANES,
        overlays: &[],
        default_keys: &[&[Key::char('y'), Key::char('u')]],
        available: has_focused_issue,
    },
    ActionSpec {
        action: Action::Copy(Copyable::BranchName),
        id: "copy-branch-name",
        description: "Copy the branch name",
        label: "copy branch / link",
        group: Group::Open,
        writes: None,
        hint: does("yank"),
        contexts: PANES,
        overlays: &[],
        default_keys: &[&[Key::char('y'), Key::char('b')]],
        available: has_focused_issue,
    },
    ActionSpec {
        action: Action::Copy(Copyable::MarkdownLink),
        id: "copy-markdown-link",
        description: "Copy a Markdown link",
        label: "copy branch / link",
        group: Group::Open,
        writes: None,
        hint: does("yank"),
        contexts: PANES,
        overlays: &[],
        default_keys: &[&[Key::char('y'), Key::char('m')]],
        available: has_focused_issue,
    },
    ActionSpec {
        action: Action::OpenLinks(LinkSet::Issue),
        id: "open-in-browser",
        description: "Open the issue or a pull request",
        label: "Linear page or PR",
        group: Group::Open,
        writes: None,
        hint: does("links"),
        contexts: PANES,
        overlays: &[],
        default_keys: &[&[Key::char('o')]],
        available: has_focused_issue,
    },
    ActionSpec {
        action: Action::OpenLinks(LinkSet::Other),
        id: "open-link",
        description: "Open another link of the issue",
        label: "other links",
        group: Group::Open,
        writes: None,
        hint: None,
        contexts: PANES,
        overlays: &[],
        default_keys: &[&[Key::char('O')]],
        available: has_focused_issue,
    },
    ActionSpec {
        action: Action::Edit(Editable::State),
        id: "edit-state",
        description: "Change the workflow state",
        label: "workflow state",
        group: Group::Edit,
        writes: Some(WriteClass::UpdateIssue),
        hint: edits("state"),
        contexts: PANES,
        overlays: &[],
        default_keys: &[&[Key::char('s')]],
        available: has_focused_issue,
    },
    ActionSpec {
        action: Action::Edit(Editable::Assignee),
        id: "edit-assignee",
        description: "Change the assignee",
        label: "assignee",
        group: Group::Edit,
        writes: Some(WriteClass::UpdateIssue),
        hint: edits("assignee"),
        contexts: PANES,
        overlays: &[],
        default_keys: &[&[Key::char('a')]],
        available: has_focused_issue,
    },
    ActionSpec {
        action: Action::Edit(Editable::Priority),
        id: "edit-priority",
        description: "Change the priority",
        label: "priority",
        group: Group::Edit,
        writes: Some(WriteClass::UpdateIssue),
        hint: edits("priority"),
        contexts: PANES,
        overlays: &[],
        default_keys: &[&[Key::char('p')]],
        available: has_focused_issue,
    },
    ActionSpec {
        action: Action::Edit(Editable::Labels),
        id: "edit-labels",
        description: "Add or remove labels",
        label: "labels",
        group: Group::Edit,
        writes: Some(WriteClass::UpdateIssue),
        hint: edits("labels"),
        contexts: PANES,
        overlays: &[],
        default_keys: &[&[Key::char('L')]],
        available: has_focused_issue,
    },
    ActionSpec {
        action: Action::Edit(Editable::Estimate),
        id: "edit-estimate",
        description: "Change the estimate",
        label: "estimate",
        group: Group::Edit,
        writes: Some(WriteClass::UpdateIssue),
        hint: edits("estimate"),
        contexts: PANES,
        overlays: &[],
        default_keys: &[&[Key::char('e')]],
        available: app::focused_team_estimates,
    },
    ActionSpec {
        action: Action::Edit(Editable::Cycle),
        id: "edit-cycle",
        description: "Change the cycle",
        label: "cycle / project",
        group: Group::Edit,
        writes: Some(WriteClass::UpdateIssue),
        hint: None,
        contexts: PANES,
        overlays: &[],
        default_keys: &[&[Key::char('C')]],
        available: has_saved_issue,
    },
    ActionSpec {
        action: Action::Edit(Editable::Project),
        id: "edit-project",
        description: "Change the project",
        label: "cycle / project",
        group: Group::Edit,
        writes: Some(WriteClass::UpdateIssue),
        hint: None,
        contexts: PANES,
        overlays: &[],
        default_keys: &[&[Key::char('P')]],
        available: has_saved_issue,
    },
    ActionSpec {
        action: Action::Edit(Editable::Milestone),
        id: "edit-milestone",
        description: "Change the project milestone",
        label: "milestone / due date",
        group: Group::Edit,
        writes: Some(WriteClass::UpdateIssue),
        hint: None,
        contexts: PANES,
        overlays: &[],
        default_keys: &[&[Key::char('M')]],
        available: app::focused_has_project,
    },
    ActionSpec {
        action: Action::Edit(Editable::DueDate),
        id: "edit-due-date",
        description: "Set or clear the due date",
        label: "milestone / due date",
        group: Group::Edit,
        writes: Some(WriteClass::UpdateIssue),
        hint: None,
        contexts: PANES,
        overlays: &[],
        default_keys: &[&[Key::char('d')]],
        available: has_saved_issue,
    },
    ActionSpec {
        action: Action::Edit(Editable::Text),
        id: "edit-text",
        description: "Edit the title and description",
        label: "title and description",
        group: Group::Edit,
        writes: Some(WriteClass::UpdateIssue),
        hint: edits("text"),
        contexts: PANES,
        overlays: &[],
        default_keys: &[&[Key::char('E')]],
        available: |state| {
            state
                .focused_issue()
                .is_some_and(|issue| !app::is_local(&issue.id))
        },
    },
    ActionSpec {
        action: Action::Comment,
        id: "comment",
        description: "Comment in $EDITOR",
        label: "comment",
        group: Group::Edit,
        writes: Some(WriteClass::CreateComment),
        hint: edits("comment"),
        contexts: PANES,
        overlays: &[],
        default_keys: &[&[Key::char('c')]],
        available: |state| {
            state
                .focused_issue()
                .is_some_and(|issue| !app::is_local(&issue.id))
        },
    },
    ActionSpec {
        action: Action::AttachFile,
        id: "attach-file",
        description: "Attach a file",
        label: "attach a file / link",
        group: Group::Edit,
        writes: Some(WriteClass::Attach),
        hint: None,
        contexts: PANES,
        overlays: &[],
        default_keys: &[&[Key::char('F')]],
        available: has_saved_issue,
    },
    ActionSpec {
        action: Action::AttachLink,
        id: "attach-link",
        description: "Attach a link",
        label: "attach a file / link",
        group: Group::Edit,
        writes: Some(WriteClass::Attach),
        hint: None,
        contexts: PANES,
        overlays: &[],
        default_keys: &[&[Key::char('W')]],
        available: has_saved_issue,
    },
    ActionSpec {
        action: Action::Archive,
        id: "archive",
        description: "Archive the issue",
        label: "archive / unarchive",
        group: Group::Edit,
        writes: Some(WriteClass::ArchiveOrTrash),
        hint: None,
        contexts: PANES,
        overlays: &[],
        default_keys: &[&[Key::char('A')]],
        available: |state| has_saved_issue(state) && !app::focused_is_archived(state),
    },
    ActionSpec {
        action: Action::Unarchive,
        id: "unarchive",
        description: "Unarchive the issue",
        label: "archive / unarchive",
        group: Group::Edit,
        writes: Some(WriteClass::ArchiveOrTrash),
        hint: None,
        contexts: PANES,
        overlays: &[],
        default_keys: &[&[Key::char('U')]],
        available: |state| has_saved_issue(state) && app::focused_is_archived(state),
    },
    ActionSpec {
        action: Action::Trash,
        id: "trash",
        description: "Move the issue to the trash",
        label: "trash / subscribe",
        group: Group::Edit,
        writes: Some(WriteClass::ArchiveOrTrash),
        hint: None,
        contexts: PANES,
        overlays: &[],
        default_keys: &[&[Key::char('X')]],
        available: has_saved_issue,
    },
    ActionSpec {
        action: Action::ToggleSubscription,
        id: "toggle-subscription",
        description: "Subscribe to or unsubscribe from the issue",
        label: "trash / subscribe",
        group: Group::Edit,
        writes: Some(WriteClass::UpdateIssue),
        hint: None,
        contexts: PANES,
        overlays: &[],
        default_keys: &[&[Key::char('S')]],
        available: has_saved_issue,
    },
    ActionSpec {
        action: Action::CreateIssue,
        id: "create-issue",
        description: "Create an issue",
        label: "new issue",
        group: Group::Edit,
        writes: None,
        hint: None,
        contexts: PANES,
        overlays: &[],
        default_keys: &[&[Key::char('n')]],
        available: |state| state.store.teams().next().is_some(),
    },
    ActionSpec {
        action: Action::Edit(Editable::Parent),
        id: "set-parent",
        description: "Set the parent issue",
        label: "parent / relations",
        group: Group::Edit,
        writes: Some(WriteClass::UpdateIssue),
        hint: None,
        contexts: PANES,
        overlays: &[],
        default_keys: &[&[Key::char('H')]],
        available: has_saved_issue,
    },
    ActionSpec {
        action: Action::ClearParent,
        id: "clear-parent",
        description: "Clear the parent issue",
        label: "parent / relations",
        group: Group::Edit,
        writes: Some(WriteClass::UpdateIssue),
        hint: None,
        contexts: PANES,
        overlays: &[],
        default_keys: &[&[Key::char('D'), Key::char('p')]],
        available: |state| {
            has_saved_issue(state)
                && state
                    .focused_issue()
                    .is_some_and(|issue| issue.parent_id.is_some())
        },
    },
    ActionSpec {
        action: Action::AddRelation,
        id: "add-relation",
        description: "Add a relation to other issues",
        label: "parent / relations",
        group: Group::Edit,
        writes: Some(WriteClass::Relations),
        hint: None,
        contexts: PANES,
        overlays: &[],
        default_keys: &[&[Key::char('R')]],
        available: has_saved_issue,
    },
    ActionSpec {
        action: Action::RemoveRelation,
        id: "remove-relation",
        description: "Remove relations to other issues",
        label: "parent / relations",
        group: Group::Edit,
        writes: Some(WriteClass::Relations),
        hint: None,
        contexts: PANES,
        overlays: &[],
        default_keys: &[&[Key::char('D'), Key::char('r')]],
        available: has_saved_issue,
    },
    ActionSpec {
        action: Action::GoToRelated,
        id: "go-to-related",
        description: "Go to a related issue",
        label: "related issue",
        group: Group::Open,
        writes: None,
        hint: None,
        contexts: PANES,
        overlays: &[],
        default_keys: &[&[Key::char('g'), Key::char('r')]],
        available: has_saved_issue,
    },
    ActionSpec {
        action: Action::NextComment,
        id: "next-comment",
        description: "Focus the next comment",
        label: "next / previous comment",
        group: Group::Move,
        writes: None,
        hint: moves("comment"),
        contexts: DETAIL,
        overlays: &[],
        default_keys: &[&[Key::char('}')]],
        available: app::can_focus_comments,
    },
    ActionSpec {
        action: Action::PreviousComment,
        id: "previous-comment",
        description: "Focus the previous comment",
        label: "next / previous comment",
        group: Group::Move,
        writes: None,
        hint: moves("comment"),
        contexts: DETAIL,
        overlays: &[],
        default_keys: &[&[Key::char('{')]],
        available: app::can_focus_comments,
    },
    ActionSpec {
        action: Action::ReplyToComment,
        id: "reply-to-comment",
        description: "Reply in the focused comment's thread",
        label: "reply",
        group: Group::Screen,
        writes: Some(WriteClass::CreateComment),
        hint: does("reply"),
        contexts: COMMENT,
        overlays: &[],
        default_keys: &[&[Key::char('r')]],
        available: has_focused_comment,
    },
    ActionSpec {
        action: Action::EditComment,
        id: "edit-comment",
        description: "Edit your focused comment",
        label: "edit yours",
        group: Group::Screen,
        writes: Some(WriteClass::ChangeComment),
        hint: None,
        contexts: COMMENT,
        overlays: &[],
        default_keys: &[&[Key::char('E')]],
        available: app::owns_focused_comment,
    },
    ActionSpec {
        action: Action::DeleteComment,
        id: "delete-comment",
        description: "Delete your focused comment",
        label: "delete yours",
        group: Group::Screen,
        writes: Some(WriteClass::ChangeComment),
        hint: None,
        contexts: COMMENT,
        overlays: &[],
        default_keys: &[&[Key::char('X')]],
        available: app::owns_focused_comment,
    },
    ActionSpec {
        action: Action::ToggleResolved,
        id: "toggle-resolved",
        description: "Resolve or unresolve the focused thread",
        label: "resolve / unresolve",
        group: Group::Screen,
        writes: Some(WriteClass::ChangeComment),
        hint: does("resolve"),
        contexts: COMMENT,
        overlays: &[],
        default_keys: &[&[Key::char('x')]],
        available: has_focused_comment,
    },
    ActionSpec {
        action: Action::React,
        id: "react",
        description: "React to the focused comment, or the issue",
        label: "react",
        group: Group::Screen,
        writes: Some(WriteClass::React),
        hint: does("react"),
        contexts: REACT,
        overlays: &[],
        default_keys: &[&[Key::char('+')]],
        available: app::can_react,
    },
    ActionSpec {
        action: Action::PickNumber(1),
        id: "pick-1",
        description: "Pick entry 1 of the list",
        label: "pick by number",
        group: Group::Screen,
        writes: None,
        hint: None,
        contexts: OVERLAY,
        overlays: LISTS,
        default_keys: &[&[Key::char('1')]],
        available: |state| picker_has(1, state),
    },
    ActionSpec {
        action: Action::PickNumber(2),
        id: "pick-2",
        description: "Pick entry 2 of the list",
        label: "pick by number",
        group: Group::Screen,
        writes: None,
        hint: None,
        contexts: OVERLAY,
        overlays: LISTS,
        default_keys: &[&[Key::char('2')]],
        available: |state| picker_has(2, state),
    },
    ActionSpec {
        action: Action::PickNumber(3),
        id: "pick-3",
        description: "Pick entry 3 of the list",
        label: "pick by number",
        group: Group::Screen,
        writes: None,
        hint: None,
        contexts: OVERLAY,
        overlays: LISTS,
        default_keys: &[&[Key::char('3')]],
        available: |state| picker_has(3, state),
    },
    ActionSpec {
        action: Action::PickNumber(4),
        id: "pick-4",
        description: "Pick entry 4 of the list",
        label: "pick by number",
        group: Group::Screen,
        writes: None,
        hint: None,
        contexts: OVERLAY,
        overlays: LISTS,
        default_keys: &[&[Key::char('4')]],
        available: |state| picker_has(4, state),
    },
    ActionSpec {
        action: Action::PickNumber(5),
        id: "pick-5",
        description: "Pick entry 5 of the list",
        label: "pick by number",
        group: Group::Screen,
        writes: None,
        hint: None,
        contexts: OVERLAY,
        overlays: LISTS,
        default_keys: &[&[Key::char('5')]],
        available: |state| picker_has(5, state),
    },
    ActionSpec {
        action: Action::PickNumber(6),
        id: "pick-6",
        description: "Pick entry 6 of the list",
        label: "pick by number",
        group: Group::Screen,
        writes: None,
        hint: None,
        contexts: OVERLAY,
        overlays: LISTS,
        default_keys: &[&[Key::char('6')]],
        available: |state| picker_has(6, state),
    },
    ActionSpec {
        action: Action::PickNumber(7),
        id: "pick-7",
        description: "Pick entry 7 of the list",
        label: "pick by number",
        group: Group::Screen,
        writes: None,
        hint: None,
        contexts: OVERLAY,
        overlays: LISTS,
        default_keys: &[&[Key::char('7')]],
        available: |state| picker_has(7, state),
    },
    ActionSpec {
        action: Action::PickNumber(8),
        id: "pick-8",
        description: "Pick entry 8 of the list",
        label: "pick by number",
        group: Group::Screen,
        writes: None,
        hint: None,
        contexts: OVERLAY,
        overlays: LISTS,
        default_keys: &[&[Key::char('8')]],
        available: |state| picker_has(8, state),
    },
    ActionSpec {
        action: Action::PickNumber(9),
        id: "pick-9",
        description: "Pick entry 9 of the list",
        label: "pick by number",
        group: Group::Screen,
        writes: None,
        hint: None,
        contexts: OVERLAY,
        overlays: LISTS,
        default_keys: &[&[Key::char('9')]],
        available: |state| picker_has(9, state),
    },
    ActionSpec {
        action: Action::OpenNotification,
        id: "open-notification",
        description: "Open the notification's issue and mark it read",
        label: "open the issue",
        group: Group::Screen,
        writes: None,
        hint: does("open"),
        contexts: INBOX,
        overlays: &[],
        default_keys: &[&[Key::named(KeyCode::Enter)]],
        available: has_notification,
    },
    ActionSpec {
        action: Action::ToggleRead,
        id: "toggle-read",
        description: "Mark the notification read or unread",
        label: "read / issue read",
        group: Group::Screen,
        writes: Some(WriteClass::Inbox),
        hint: does("read"),
        contexts: INBOX,
        overlays: &[],
        default_keys: &[&[Key::char('m')]],
        available: has_notification,
    },
    ActionSpec {
        action: Action::MarkIssueRead,
        id: "mark-issue-read",
        description: "Mark every notification about the issue read",
        label: "read / issue read",
        group: Group::Screen,
        writes: Some(WriteClass::Inbox),
        hint: None,
        contexts: INBOX,
        overlays: &[],
        default_keys: &[&[Key::char('M')]],
        available: has_notification,
    },
    ActionSpec {
        action: Action::ArchiveNotification,
        id: "archive-notification",
        description: "Archive the notification",
        label: "archive / snooze",
        group: Group::Screen,
        writes: Some(WriteClass::Inbox),
        hint: does("archive"),
        contexts: INBOX,
        overlays: &[],
        default_keys: &[&[Key::char('A')]],
        available: has_notification,
    },
    ActionSpec {
        action: Action::Snooze,
        id: "snooze",
        description: "Snooze the notification until a preset time",
        label: "archive / snooze",
        group: Group::Screen,
        writes: Some(WriteClass::Inbox),
        hint: does("snooze"),
        contexts: INBOX,
        overlays: &[],
        default_keys: &[&[Key::char('z')]],
        available: has_notification,
    },
    // Run from the palette, so it has no key.
    ActionSpec {
        action: Action::ArchiveAllRead,
        id: "archive-all-read",
        description: "Archive every read notification in the Inbox",
        label: "archive all read",
        group: Group::Screen,
        writes: Some(WriteClass::Inbox),
        hint: None,
        contexts: INBOX,
        overlays: &[],
        default_keys: &[],
        available: app::inbox_has_read,
    },
    ActionSpec {
        action: Action::SwitchTeam,
        id: "switch-team",
        description: "Switch the team of Team Issues",
        label: "switch team",
        group: Group::Screen,
        writes: None,
        hint: views("team"),
        contexts: TEAM_ISSUES,
        overlays: &[],
        default_keys: &[&[Key::char('t')]],
        available: |state| state.store.viewer_teams().next().is_some(),
    },
    ActionSpec {
        action: Action::SwitchCyclesTeam,
        id: "switch-cycles-team",
        description: "Switch the team of the Cycles screen",
        label: "switch team",
        group: Group::Screen,
        writes: None,
        hint: views("team"),
        contexts: CYCLES,
        overlays: &[],
        default_keys: &[&[Key::char('t')]],
        available: |state| state.store.viewer_teams().next().is_some(),
    },
    ActionSpec {
        action: Action::FilterBy(FilterField::State),
        id: "filter-by-state",
        description: "Filter by workflow state",
        label: "filter by state",
        group: Group::Screen,
        writes: None,
        hint: views("filter"),
        contexts: TEAM_ISSUES,
        overlays: &[],
        default_keys: &[&[Key::char('f'), Key::char('s')]],
        available: has_team,
    },
    ActionSpec {
        action: Action::FilterBy(FilterField::Assignee),
        id: "filter-by-assignee",
        description: "Filter by assignee",
        label: "filter by assignee",
        group: Group::Screen,
        writes: None,
        hint: views("filter"),
        contexts: TEAM_ISSUES,
        overlays: &[],
        default_keys: &[&[Key::char('f'), Key::char('a')]],
        available: has_team,
    },
    ActionSpec {
        action: Action::FilterBy(FilterField::Label),
        id: "filter-by-label",
        description: "Filter by label",
        label: "filter by label",
        group: Group::Screen,
        writes: None,
        hint: views("filter"),
        contexts: TEAM_ISSUES,
        overlays: &[],
        default_keys: &[&[Key::char('f'), Key::char('l')]],
        available: has_team,
    },
    ActionSpec {
        action: Action::FilterBy(FilterField::Cycle),
        id: "filter-by-cycle",
        description: "Filter by cycle",
        label: "filter by cycle",
        group: Group::Screen,
        writes: None,
        hint: views("filter"),
        contexts: TEAM_ISSUES,
        overlays: &[],
        default_keys: &[&[Key::char('f'), Key::char('c')]],
        available: has_team,
    },
    ActionSpec {
        action: Action::FilterBy(FilterField::Project),
        id: "filter-by-project",
        description: "Filter by project",
        label: "filter by project",
        group: Group::Screen,
        writes: None,
        hint: views("filter"),
        contexts: TEAM_ISSUES,
        overlays: &[],
        default_keys: &[&[Key::char('f'), Key::char('p')]],
        available: has_team,
    },
    ActionSpec {
        action: Action::ToggleBlocked,
        id: "filter-blocked",
        description: "Show only blocked issues, or all again",
        label: "blocked only",
        group: Group::Screen,
        writes: None,
        hint: views("filter"),
        contexts: TEAM_ISSUES,
        overlays: &[],
        default_keys: &[&[Key::char('f'), Key::char('b')]],
        available: has_team,
    },
    ActionSpec {
        action: Action::LoadMore,
        id: "load-more",
        description: "Load the next page of Team Issues",
        label: "load more",
        group: Group::Screen,
        writes: None,
        hint: views("more"),
        contexts: TEAM_ISSUES,
        overlays: &[],
        default_keys: &[&[Key::char('m')]],
        available: can_load_more,
    },
    ActionSpec {
        action: Action::GroupBy,
        id: "group-by",
        description: "Pick how the issue list is grouped",
        label: "group / sort",
        group: Group::Screen,
        writes: None,
        hint: None,
        contexts: LIST,
        overlays: &[],
        default_keys: &[&[Key::char('v'), Key::char('g')]],
        available: app::can_rearrange,
    },
    ActionSpec {
        action: Action::SortBy,
        id: "sort-by",
        description: "Pick how the issue list is sorted",
        label: "group / sort",
        group: Group::Screen,
        writes: None,
        hint: None,
        contexts: LIST,
        overlays: &[],
        default_keys: &[&[Key::char('v'), Key::char('s')]],
        available: app::can_rearrange,
    },
    ActionSpec {
        action: Action::ScrollDown,
        id: "scroll-down",
        description: "Scroll down",
        label: "down / up",
        group: Group::Move,
        writes: None,
        hint: moves("move"),
        contexts: SCROLLS,
        overlays: SCROLLING,
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
        label: "down / up",
        group: Group::Move,
        writes: None,
        hint: moves("move"),
        contexts: SCROLLS,
        overlays: SCROLLING,
        default_keys: &[
            &[Key::char('k')],
            &[Key::named(KeyCode::Up)],
            &[Key::ctrl('p')],
        ],
        available: can_scroll_up,
    },
    ActionSpec {
        action: Action::Accept,
        id: "accept",
        description: "Choose the highlighted entry",
        label: "choose",
        group: Group::Screen,
        writes: None,
        hint: does("choose"),
        contexts: OVERLAY,
        overlays: ACCEPTING,
        default_keys: &[&[Key::named(KeyCode::Enter)]],
        available: can_accept,
    },
    ActionSpec {
        action: Action::Confirm,
        id: "confirm",
        description: "Confirm the question asked",
        label: "confirm / cancel",
        group: Group::Screen,
        writes: None,
        hint: None,
        contexts: OVERLAY,
        overlays: &[OverlayKind::Confirm],
        default_keys: &[&[Key::char('y')]],
        available: is_confirming,
    },
    ActionSpec {
        action: Action::Decline,
        id: "decline",
        description: "Cancel the question asked",
        label: "confirm / cancel",
        group: Group::Screen,
        writes: None,
        hint: None,
        contexts: OVERLAY,
        overlays: &[OverlayKind::Confirm],
        default_keys: &[&[Key::char('n')]],
        available: is_confirming,
    },
    ActionSpec {
        action: Action::ToggleEntry,
        id: "toggle-entry",
        description: "Mark or unmark the highlighted entry, or search comments too",
        label: "mark or unmark",
        group: Group::Screen,
        writes: None,
        hint: does("toggle"),
        contexts: OVERLAY,
        overlays: &[
            OverlayKind::MultiList,
            OverlayKind::Search,
            OverlayKind::Path,
        ],
        default_keys: &[&[Key::named(KeyCode::Tab)]],
        available: |state| match state.modals.top() {
            Some(Modal::Search(search)) => search.editable(),
            Some(Modal::Picker(picker)) => {
                picker.list.is_multi() && picker.list.selected().is_some()
            }
            Some(Modal::IssuePicker(picker)) => {
                picker.list.is_multi() && picker.list.selected().is_some()
            }
            Some(Modal::Choice(choice)) => {
                choice.list.is_multi() && choice.list.selected().is_some()
            }
            Some(Modal::Attach(prompt)) => prompt.step == AttachStep::File,
            _ => false,
        },
    },
    ActionSpec {
        action: Action::ToggleSearchScope,
        id: "toggle-search-scope",
        description: "Search within the Team Issues team and filter, or the whole workspace",
        label: "team or workspace",
        group: Group::Screen,
        writes: None,
        hint: None,
        contexts: OVERLAY,
        overlays: &[OverlayKind::Search],
        default_keys: &[&[Key::ctrl('t')]],
        available: |state| matches!(state.modals.top(), Some(Modal::Search(search)) if search.scope.is_some() && search.editable()),
    },
    ActionSpec {
        action: Action::ShorterRange,
        id: "shorter-range",
        description: "Show a shorter range",
        label: "shorter / longer range",
        group: Group::Screen,
        writes: None,
        hint: views("range"),
        contexts: ANALYTICS,
        overlays: &[],
        default_keys: &[&[Key::char('[')]],
        available: |state| state.analytics_range() != WeekRange::Four,
    },
    ActionSpec {
        action: Action::LongerRange,
        id: "longer-range",
        description: "Show a longer range",
        label: "shorter / longer range",
        group: Group::Screen,
        writes: None,
        hint: views("range"),
        contexts: ANALYTICS,
        overlays: &[],
        default_keys: &[&[Key::char(']')]],
        available: |state| state.analytics_range() != WeekRange::TwentySix,
    },
    ActionSpec {
        action: Action::ToggleAnalyticsView,
        id: "toggle-analytics-view",
        description: "Show Personal or Team analytics",
        label: "Personal or Team",
        group: Group::Screen,
        writes: None,
        hint: views("view"),
        contexts: ANALYTICS,
        overlays: &[],
        default_keys: &[&[Key::char('v')]],
        available: always,
    },
    ActionSpec {
        action: Action::SwitchAnalyticsTeam,
        id: "switch-analytics-team",
        description: "Pick the team for Team analytics",
        label: "team",
        group: Group::Screen,
        writes: None,
        hint: views("team"),
        contexts: ANALYTICS,
        overlays: &[],
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
        label: "close or step back",
        group: Group::Move,
        writes: None,
        hint: does("close"),
        contexts: &[Context::List, Context::Detail, Context::Overlay],
        overlays: &OverlayKind::ALL,
        default_keys: &[&[Key::named(KeyCode::Esc)]],
        available: |state| {
            state.modals.top().is_some() || in_detail(state) || state.screen == Screen::Results
        },
    },
    // After `close`, so a narrow bottom bar drops this before Esc.
    ActionSpec {
        action: Action::CopyLink,
        id: "copy-link",
        description: "Copy the highlighted link's URL",
        label: "copy the link",
        group: Group::Screen,
        writes: None,
        hint: does("copy"),
        contexts: OVERLAY,
        overlays: &[OverlayKind::Links],
        default_keys: &[&[Key::ctrl('y')]],
        available: |state| matches!(state.modals.top(), Some(Modal::Links(links)) if links.list.selected().is_some()),
    },
    ActionSpec {
        action: Action::ShowScreen(Screen::MyIssues),
        id: "show-my-issues",
        description: "Show My Issues",
        label: "screens",
        group: Group::App,
        writes: None,
        hint: None,
        contexts: GLOBAL,
        overlays: &[],
        default_keys: &[&[Key::char('1')]],
        available: |state| state.screen != Screen::MyIssues,
    },
    ActionSpec {
        action: Action::ShowScreen(Screen::TeamIssues),
        id: "show-team-issues",
        description: "Show Team Issues",
        label: "screens",
        group: Group::App,
        writes: None,
        hint: None,
        contexts: GLOBAL,
        overlays: &[],
        default_keys: &[&[Key::char('2')]],
        available: |state| state.screen != Screen::TeamIssues,
    },
    ActionSpec {
        action: Action::ShowScreen(Screen::Analytics),
        id: "show-analytics",
        description: "Show Analytics",
        label: "screens",
        group: Group::App,
        writes: None,
        hint: None,
        contexts: GLOBAL,
        overlays: &[],
        default_keys: &[&[Key::char('3')]],
        available: |state| state.screen != Screen::Analytics,
    },
    ActionSpec {
        action: Action::ShowScreen(Screen::Cycles),
        id: "show-cycles",
        description: "Show Cycles",
        label: "cycles",
        group: Group::App,
        writes: None,
        hint: None,
        contexts: GLOBAL,
        overlays: &[],
        default_keys: &[&[Key::char('4')]],
        available: |state| state.screen != Screen::Cycles,
    },
    ActionSpec {
        action: Action::ShowScreen(Screen::Projects),
        id: "show-projects",
        description: "Show Projects",
        label: "projects",
        group: Group::App,
        writes: None,
        hint: None,
        contexts: GLOBAL,
        overlays: &[],
        default_keys: &[&[Key::char('5')]],
        available: |state| state.screen != Screen::Projects,
    },
    ActionSpec {
        action: Action::ShowScreen(Screen::Inbox),
        id: "show-inbox",
        description: "Show the Inbox",
        label: "inbox",
        group: Group::App,
        writes: None,
        hint: None,
        contexts: GLOBAL,
        overlays: &[],
        default_keys: &[&[Key::char('6')]],
        available: |state| state.screen != Screen::Inbox,
    },
    ActionSpec {
        action: Action::Refresh,
        id: "refresh",
        description: "Refresh now",
        label: "refresh",
        group: Group::App,
        writes: None,
        hint: None,
        contexts: GLOBAL,
        overlays: &[],
        default_keys: &[&[Key::char('r')]],
        available: |state| state.can_refresh(),
    },
    ActionSpec {
        action: Action::ShowCheatsheet,
        id: "show-cheatsheet",
        description: "Show every key for this context",
        label: "keys",
        group: Group::App,
        writes: None,
        hint: does("keys"),
        contexts: GLOBAL,
        overlays: &[],
        default_keys: &[&[Key::char('?')]],
        available: always,
    },
    ActionSpec {
        action: Action::ShowPalette,
        id: "show-palette",
        description: "Find an action or loaded issue",
        label: "palette",
        group: Group::App,
        writes: None,
        hint: does("palette"),
        contexts: GLOBAL,
        overlays: &[],
        default_keys: &[&[Key::char(':')], &[Key::ctrl('k')]],
        available: always,
    },
    ActionSpec {
        action: Action::ShowSearch,
        id: "show-search",
        description: "Search all issues in Linear",
        label: "search",
        group: Group::App,
        writes: None,
        hint: moves("search"),
        contexts: GLOBAL,
        overlays: &[],
        default_keys: &[&[Key::char('/')]],
        available: always,
    },
    ActionSpec {
        action: Action::ShowMessages,
        id: "messages",
        description: "Show recent errors",
        label: "messages",
        group: Group::App,
        writes: None,
        hint: None,
        contexts: GLOBAL,
        overlays: &[],
        default_keys: &[&[Key::char('!')]],
        available: always,
    },
    ActionSpec {
        action: Action::Quit,
        id: "quit",
        description: "Quit lrat",
        label: "quit",
        group: Group::App,
        writes: None,
        hint: does("quit"),
        contexts: GLOBAL,
        overlays: &[],
        default_keys: &[&[Key::char('q')]],
        available: always,
    },
    ActionSpec {
        action: Action::Suspend,
        id: "suspend",
        description: "Suspend lrat to the shell",
        label: "suspend",
        group: Group::App,
        writes: None,
        hint: None,
        contexts: GLOBAL,
        overlays: &[],
        default_keys: &[&[Key::ctrl('z')]],
        available: always,
    },
    // The release actions run from the palette, so they have no keys.
    ActionSpec {
        action: Action::SkipRelease,
        id: "skip-release",
        description: "Skip this release",
        label: "skip release",
        group: Group::App,
        writes: None,
        hint: None,
        contexts: GLOBAL,
        overlays: &[],
        default_keys: &[],
        available: knows_newer_release,
    },
    ActionSpec {
        action: Action::ShowReleaseNotes,
        id: "release-notes",
        description: "What's new in the latest release",
        label: "release notes",
        group: Group::App,
        writes: None,
        hint: None,
        contexts: GLOBAL,
        overlays: &[],
        default_keys: &[],
        available: knows_newer_release,
    },
];

fn always(_: &State) -> bool {
    true
}

fn knows_newer_release(state: &State) -> bool {
    state.release.notice().is_some()
}

fn can_scroll_down(state: &State) -> bool {
    match state.modals.top() {
        Some(Modal::Choice(choice)) => choice
            .list
            .selected_position()
            .is_some_and(|position| position + 1 < choice.list.visible().len()),
        Some(Modal::Picker(picker)) => picker
            .list
            .selected_position()
            .is_some_and(|position| position + 1 < picker.list.visible().len()),
        Some(Modal::Links(links)) => links
            .list
            .selected_position()
            .is_some_and(|position| position + 1 < links.list.visible().len()),
        Some(Modal::IssuePicker(picker)) => picker
            .list
            .selected_position()
            .is_some_and(|position| position + 1 < picker.list.visible().len()),
        Some(Modal::Create(form)) => form
            .list()
            .is_some_and(|(len, selected)| selected.is_some_and(|position| position + 1 < len)),
        Some(Modal::Cheatsheet { scroll }) => {
            *scroll < ui::cheatsheet_max_scroll(state, state.viewport)
        }
        Some(Modal::Messages { scroll }) => {
            *scroll < ui::messages_max_scroll(state, state.viewport)
        }
        Some(Modal::Palette(palette)) => {
            palette.selected + 1 < app::palette_matches(state, &palette.query).len()
        }
        None if in_analytics(state) => {
            state.analytics_screen.scroll() < ui::analytics_max_scroll(state, state.viewport)
        }
        _ => false,
    }
}

fn can_scroll_up(state: &State) -> bool {
    match state.modals.top() {
        Some(Modal::Choice(choice)) => choice
            .list
            .selected_position()
            .is_some_and(|position| position > 0),
        Some(Modal::Picker(picker)) => picker
            .list
            .selected_position()
            .is_some_and(|position| position > 0),
        Some(Modal::Links(links)) => links
            .list
            .selected_position()
            .is_some_and(|position| position > 0),
        Some(Modal::IssuePicker(picker)) => picker
            .list
            .selected_position()
            .is_some_and(|position| position > 0),
        Some(Modal::Create(form)) => form
            .list()
            .is_some_and(|(_, selected)| selected.is_some_and(|position| position > 0)),
        Some(Modal::Cheatsheet { scroll } | Modal::Messages { scroll }) => *scroll > 0,
        Some(Modal::Palette(palette)) => palette.selected > 0,
        None if in_analytics(state) => state.analytics_screen.scroll() > 0,
        _ => false,
    }
}

fn can_accept(state: &State) -> bool {
    match state.modals.top() {
        Some(Modal::Choice(choice)) => choice.list.selected().is_some(),
        Some(Modal::Picker(picker)) => picker.list.is_multi() || picker.list.selected().is_some(),
        Some(Modal::Links(links)) => links.list.selected().is_some(),
        Some(Modal::IssuePicker(picker)) => picker.list.selected().is_some(),
        Some(Modal::Create(form)) => form.list().is_none_or(|(_, selected)| selected.is_some()),
        Some(Modal::Palette(palette)) => {
            palette.selected < app::palette_matches(state, &palette.query).len()
        }
        Some(Modal::Search(search)) => match &search.phase {
            Phase::Typing { .. } | Phase::NothingFound { .. } => !search.query.trim().is_empty(),
            Phase::Searching { .. } => false,
        },
        Some(Modal::Confirm(_) | Modal::DueDate(_) | Modal::Attach(_)) => true,
        _ => false,
    }
}

fn is_confirming(state: &State) -> bool {
    matches!(state.modals.top(), Some(Modal::Confirm(_)))
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
        // A multi-select choice lists issue keys, typed like `ENG-12`.
        Some(Modal::Choice(choice)) if choice.list.is_multi() => {
            !continues_word(choice.list.query()) && choice.list.visible().len() >= count
        }
        Some(Modal::Choice(choice)) => choice.list.visible().len() >= count,
        Some(Modal::Create(form)) => form.list().is_some_and(|(len, _)| len >= count),
        // A digit that continues a word, as in `ENG-12`, types into the
        // query instead.
        Some(Modal::IssuePicker(picker)) => {
            !continues_word(picker.list.query()) && picker.list.visible().len() >= count
        }
        _ => false,
    }
}

/// Whether a character typed after `query` continues its last word: the
/// query ends in a letter, a digit or `-`.
fn continues_word(query: &str) -> bool {
    query
        .chars()
        .last()
        .is_some_and(|c| c.is_alphanumeric() || c == '-')
}

/// Whether the Inbox is on screen with a notification focused.
fn has_notification(state: &State) -> bool {
    state.screen == Screen::Inbox && app::focused_notification(state).is_some()
}

fn has_team(state: &State) -> bool {
    state.team_issues.query().is_some()
}

fn has_focused_issue(state: &State) -> bool {
    state.focused_issue().is_some()
}

/// Whether an issue Linear has confirmed is focused, so it can be edited.
fn has_saved_issue(state: &State) -> bool {
    state
        .focused_issue()
        .is_some_and(|issue| !app::is_local(&issue.id))
}

fn has_focused_comment(state: &State) -> bool {
    app::focused_comment(state).is_some()
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

fn in_analytics(state: &State) -> bool {
    state.screen == Screen::Analytics && state.modals.top().is_none()
}

fn in_detail(state: &State) -> bool {
    match state.screen {
        Screen::MyIssues => state.my_issues.pane() == Pane::Detail,
        Screen::TeamIssues => state.team_issues.pane() == Pane::Detail,
        Screen::Results => state.results.pane() == Pane::Detail,
        Screen::Analytics | Screen::Cycles | Screen::Projects | Screen::Inbox => false,
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
