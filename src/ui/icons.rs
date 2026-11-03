//! Glyphs and colors for workflow state types and priorities, kept together so
//! themes and Nerd Font icons (#19) replace them in one place. Every glyph
//! differs in shape, so nothing relies on color alone.

use ratatui::style::Color;

use crate::linear::model::{Priority, StateType};

pub fn state_glyph(state_type: StateType) -> &'static str {
    match state_type {
        StateType::Triage => "◇",
        StateType::Backlog => "◌",
        StateType::Unstarted => "○",
        StateType::Started => "◐",
        StateType::Completed => "●",
        StateType::Canceled => "⊘",
    }
}

pub fn state_color(state_type: StateType) -> Color {
    match state_type {
        StateType::Triage => Color::Magenta,
        StateType::Backlog | StateType::Canceled => Color::DarkGray,
        StateType::Unstarted => Color::Reset,
        StateType::Started => Color::Yellow,
        StateType::Completed => Color::Green,
    }
}

pub fn state_type_name(state_type: StateType) -> &'static str {
    match state_type {
        StateType::Triage => "Triage",
        StateType::Backlog => "Backlog",
        StateType::Unstarted => "Unstarted",
        StateType::Started => "Started",
        StateType::Completed => "Completed",
        StateType::Canceled => "Canceled",
    }
}

pub fn priority_glyph(priority: Priority) -> &'static str {
    match priority {
        Priority::Urgent => "!!!",
        Priority::High => "▮▮▮",
        Priority::Medium => "▮▮▯",
        Priority::Low => "▮▯▯",
        Priority::None => "···",
    }
}

pub fn priority_color(priority: Priority) -> Color {
    match priority {
        Priority::Urgent => Color::Red,
        Priority::High => Color::LightRed,
        Priority::Medium => Color::Yellow,
        Priority::Low => Color::Blue,
        Priority::None => Color::DarkGray,
    }
}

pub fn priority_name(priority: Priority) -> &'static str {
    match priority {
        Priority::Urgent => "Urgent",
        Priority::High => "High",
        Priority::Medium => "Medium",
        Priority::Low => "Low",
        Priority::None => "None",
    }
}

/// Marks what is active: the focused pane, the active cycle.
pub const ACCENT: Color = Color::Cyan;
/// Marks what is inactive, such as the pane without focus.
pub const MUTED: Color = Color::DarkGray;
