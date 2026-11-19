use super::{Editable, FilterField, LinkSet, Screen};

/// Something the viewer can ask lrat to do. Keys, the palette and the
/// cheatsheet all name actions; [`perform`](super::perform) carries them out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Quit,
    /// Hide the release notice until a newer release ships.
    SkipRelease,
    /// Open the GitHub page of the release the notice shows.
    ShowReleaseNotes,
    /// Hand the terminal back to the shell and stop lrat until `fg`.
    Suspend,
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
    /// Open the log of recent errors.
    ShowMessages,
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
    /// Copy the URL of the link highlighted in a links picker and close it.
    CopyLink,
    /// Poll Linear for changes now.
    Refresh,
    /// Open the picker that changes a field of the focused issue.
    Edit(Editable),
    /// Close a picker on the entry with this number, from 1 to 9, counted
    /// among the entries the filter leaves.
    PickNumber(u8),
    /// Mark or unmark the highlighted entry of a multi-select picker, or
    /// turn the search of comments on or off.
    ToggleEntry,
    /// Turn keeping a search to the Team Issues team and filter off or
    /// back on.
    ToggleSearchScope,
    /// Pick another of the viewer's teams for Team Issues.
    SwitchTeam,
    /// Pick the value Team Issues filters this field by.
    FilterBy(FilterField),
    /// Load the next page of Team Issues.
    LoadMore,
    /// Archive the focused issue.
    Archive,
    /// Bring the focused archived issue back.
    Unarchive,
    /// Ask to confirm moving the focused issue to Linear's trash.
    Trash,
    /// Subscribe the viewer to the focused issue, or unsubscribe them.
    ToggleSubscription,
    /// Run what the confirmation on top asks about.
    Confirm,
    /// Close the confirmation on top without running anything.
    Decline,
    /// Pick how the issue list on screen is grouped.
    GroupBy,
    /// Pick how the issue list on screen is sorted.
    SortBy,
    /// Write a comment on the focused issue in `$EDITOR`.
    Comment,
    /// Open the form that creates an issue.
    CreateIssue,
    /// Choose a kind of relation, then the issues to relate the focused
    /// issue to.
    AddRelation,
    /// Choose related issues to remove the focused issue's relations with.
    RemoveRelation,
    /// Make the focused issue a top-level issue again.
    ClearParent,
    /// Go to the focused issue's parent, a sub-issue or a related issue.
    GoToRelated,
    /// Show only Team Issues blocked by another issue, or all again.
    ToggleBlocked,
    /// Switch the Analytics screen between its Personal and Team views.
    ToggleAnalyticsView,
    /// Pick another of the viewer's teams for the Team view of Analytics.
    SwitchAnalyticsTeam,
    /// Pick another of the viewer's teams for the Cycles screen.
    SwitchCyclesTeam,
    /// List the issues of the focused cycle or project.
    ListIssues,
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
