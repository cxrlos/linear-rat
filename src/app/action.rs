use super::Screen;

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
}
