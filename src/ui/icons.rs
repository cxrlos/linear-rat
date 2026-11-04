//! Glyphs for workflow state types and priorities, from the icon set the
//! viewer picks with `ui.icons`. Within a set every glyph differs in shape,
//! so nothing relies on color alone; the theme colors them.

use crate::linear::model::{Priority, StateType};

/// Which glyphs lrat draws with.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum IconSet {
    /// Plain Unicode that any terminal font has.
    #[default]
    Unicode,
    /// Font Awesome glyphs from a patched Nerd Font.
    Nerd,
}

impl IconSet {
    /// Every icon set, the default first.
    pub const ALL: [IconSet; 2] = [IconSet::Unicode, IconSet::Nerd];

    /// Its value for `ui.icons` in `config.toml`.
    pub fn name(self) -> &'static str {
        match self {
            IconSet::Unicode => "unicode",
            IconSet::Nerd => "nerd",
        }
    }

    /// The icon set whose `ui.icons` value is `name`.
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|set| set.name() == name)
    }

    /// The glyph for a workflow state of `state_type`.
    pub fn state(self, state_type: StateType) -> &'static str {
        match (self, state_type) {
            (IconSet::Unicode, StateType::Triage) => "◇",
            (IconSet::Unicode, StateType::Backlog) => "◌",
            (IconSet::Unicode, StateType::Unstarted) => "○",
            (IconSet::Unicode, StateType::Started) => "◐",
            (IconSet::Unicode, StateType::Completed) => "●",
            (IconSet::Unicode, StateType::Canceled) => "⊘",
            // nf-fa-inbox, circle_o_notch, circle_o, adjust, check_circle,
            // times_circle
            (IconSet::Nerd, StateType::Triage) => "\u{f01c}",
            (IconSet::Nerd, StateType::Backlog) => "\u{f1ce}",
            (IconSet::Nerd, StateType::Unstarted) => "\u{f10c}",
            (IconSet::Nerd, StateType::Started) => "\u{f042}",
            (IconSet::Nerd, StateType::Completed) => "\u{f058}",
            (IconSet::Nerd, StateType::Canceled) => "\u{f057}",
        }
    }

    /// The glyph for `priority`.
    pub fn priority(self, priority: Priority) -> &'static str {
        match (self, priority) {
            (IconSet::Unicode, Priority::Urgent) => "!!!",
            (IconSet::Unicode, Priority::High) => "▮▮▮",
            (IconSet::Unicode, Priority::Medium) => "▮▮▯",
            (IconSet::Unicode, Priority::Low) => "▮▯▯",
            (IconSet::Unicode, Priority::None) => "···",
            // nf-fa-exclamation_circle, angle_double_up, angle_up,
            // angle_down, minus
            (IconSet::Nerd, Priority::Urgent) => "\u{f06a}",
            (IconSet::Nerd, Priority::High) => "\u{f102}",
            (IconSet::Nerd, Priority::Medium) => "\u{f106}",
            (IconSet::Nerd, Priority::Low) => "\u{f107}",
            (IconSet::Nerd, Priority::None) => "\u{f068}",
        }
    }
}

/// How a state type is named in a group header.
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

/// How a priority is named beside its glyph.
pub fn priority_name(priority: Priority) -> &'static str {
    match priority {
        Priority::Urgent => "Urgent",
        Priority::High => "High",
        Priority::Medium => "Medium",
        Priority::Low => "Low",
        Priority::None => "None",
    }
}
