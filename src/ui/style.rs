//! The few colors lrat uses, in one place until themes (#19) replace them.

use ratatui::style::{Color, Modifier, Style};

/// Keys and other things the viewer can act on now.
pub fn accent() -> Style {
    Style::new().fg(Color::Cyan).add_modifier(Modifier::BOLD)
}

/// Things that exist but cannot be used right now.
pub fn muted() -> Style {
    Style::new().fg(Color::DarkGray)
}

/// Messages about something that went wrong.
pub fn error() -> Style {
    Style::new().fg(Color::Red)
}
