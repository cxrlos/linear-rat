//! The few colors lrat uses, in one place until themes (#19) replace them.

use ratatui::style::{Color, Modifier, Style};

/// Marks what is active: keys the viewer can use, the focused pane, the
/// active cycle.
pub const ACCENT: Color = Color::Cyan;
/// Marks what is inactive: unavailable keys, the pane without focus.
pub const MUTED: Color = Color::DarkGray;

/// Keys and other things the viewer can act on now.
pub fn accent() -> Style {
    Style::new().fg(ACCENT).add_modifier(Modifier::BOLD)
}

/// Things that exist but cannot be used right now.
pub fn muted() -> Style {
    Style::new().fg(MUTED)
}

/// Messages about something that went wrong.
pub fn error() -> Style {
    Style::new().fg(Color::Red)
}
