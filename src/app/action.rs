use super::{Editable, Screen};

/// Something the viewer can ask lrat to do. Keys, the palette and the
/// cheatsheet all name actions; [`perform`](super::perform) carries them out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Quit,
    ShowScreen(Screen),
    MoveDown,
    MoveUp,
    MoveToTop,
    MoveToBottom,
    HalfPageDown,
    HalfPageUp,
    FocusLeft,
    FocusRight,
    Open,
    /// Close the top overlay, or step back from a detail.
    Close,
    ShowCheatsheet,
    /// Scroll the cheatsheet one row.
    ScrollDown,
    ScrollUp,
    /// Copy something about the focused issue to the clipboard.
    Copy(Copyable),
    /// Open the focused issue's page in the browser.
    OpenInBrowser,
    /// Poll Linear for changes now.
    Refresh,
    /// Open the picker that changes a field of the focused issue.
    Edit(Editable),
    /// Move a picker's selection one entry.
    SelectNext,
    SelectPrevious,
    /// Close a picker on its selected entry.
    Choose,
    /// Close a picker on the entry with this number, from 1 to 9, counted
    /// among the entries the filter leaves.
    PickNumber(u8),
}

/// What [`Action::Copy`] puts on the clipboard.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Copyable {
    /// The key, as `ENG-12`.
    Identifier,
    /// The issue's page in Linear.
    Url,
    /// The Git branch name Linear suggests.
    BranchName,
    /// `[ENG-12 Title](url)`.
    MarkdownLink,
}
