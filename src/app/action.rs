use super::{Editable, FilterField, LinkSet, Screen};

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
    /// Open the palette, to find an action or a loaded issue.
    ShowPalette,
    /// Open the search across the workspace.
    ShowSearch,
    /// Run or open what an overlay has highlighted, or send its query.
    Accept,
    /// Show the next shorter or longer analytics range.
    ShorterRange,
    LongerRange,
    /// Scroll the cheatsheet, an issue overlay or the Analytics screen one
    /// row, or move an overlay's highlight.
    ScrollDown,
    ScrollUp,
    /// Copy something about the focused issue to the clipboard.
    Copy(Copyable),
    /// Open the picker of these links of the focused issue, to open one in
    /// the browser.
    OpenLinks(LinkSet),
    /// Poll Linear for changes now.
    Refresh,
    /// Open the picker that changes a field of the focused issue.
    Edit(Editable),
    /// Close a picker on the entry with this number, from 1 to 9, counted
    /// among the entries the filter leaves.
    PickNumber(u8),
    /// Mark or unmark the highlighted entry of a multi-select picker.
    ToggleEntry,
    /// Pick another of the viewer's teams for Team Issues.
    SwitchTeam,
    /// Pick the value Team Issues filters this field by.
    FilterBy(FilterField),
    /// Load the next page of Team Issues.
    LoadMore,
    /// Write a comment on the focused issue in `$EDITOR`.
    Comment,
    /// Open the form that creates an issue.
    CreateIssue,
    /// Switch the Analytics screen between its Personal and Team views.
    ToggleAnalyticsView,
    /// Pick another of the viewer's teams for the Team view of Analytics.
    SwitchAnalyticsTeam,
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
