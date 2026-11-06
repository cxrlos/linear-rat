//! Themes: the colors of every role lrat draws with.
//!
//! The default theme, `terminal`, uses the terminal's own palette and paints
//! no background. Every other built-in theme is a Base16 scheme (ported from
//! ratatoist) and paints its `base00` behind the whole screen, because its
//! colors are only legible against that background. A viewer's own theme is
//! a Base16 JSON file in `~/.config/lrat/themes/`, loaded by its file name.
//!
//! Base16 slots map to roles as follows: `base00` background, `base05`
//! text, `base03` muted for borders and inactive things (as in ratatoist,
//! but lifted toward `base05` until it reaches [`MUTED_MIN_CONTRAST`]),
//! `base0C` accent, `base08` error. State types: triage `base0E`, backlog and
//! canceled muted, unstarted text, started `base0A`, completed `base0B`.
//! Priorities: urgent `base08`, high `base09`, medium `base0A`, low
//! `base0D`, none muted.

use std::fmt;

use ratatui::style::{Color, Modifier, Style};
use serde::Deserialize;

use crate::linear::model::{Priority, StateType};

/// The name of the default theme.
pub const DEFAULT: &str = "terminal";

/// The least contrast ratio the muted role keeps against the background in
/// a Base16 theme, the WCAG minimum for large text and interface parts.
pub const MUTED_MIN_CONTRAST: f64 = 3.0;

/// The built-in Base16 schemes, by the name `[theme] name` takes.
const BASE16: [(&str, &str); 10] = [
    (
        "catppuccin-mocha",
        include_str!("themes/catppuccin-mocha.json"),
    ),
    ("dracula", include_str!("themes/dracula.json")),
    (
        "everforest-dark",
        include_str!("themes/everforest-dark.json"),
    ),
    ("gruvbox-dark", include_str!("themes/gruvbox-dark.json")),
    ("kanagawa-wave", include_str!("themes/kanagawa-wave.json")),
    ("nord", include_str!("themes/nord.json")),
    ("one-dark", include_str!("themes/one-dark.json")),
    ("rose-pine", include_str!("themes/rose-pine.json")),
    ("solarized-dark", include_str!("themes/solarized-dark.json")),
    ("tokyo-night", include_str!("themes/tokyo-night.json")),
];

/// The color of each role lrat draws with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Theme {
    /// Painted behind everything; [`Color::Reset`] keeps the terminal's own.
    pub background: Color,
    /// Everything that has no other role.
    pub text: Color,
    /// What is inactive or secondary: unavailable keys, the pane without
    /// focus, identifiers and field names.
    pub muted: Color,
    /// What is active: keys the viewer can use, the focused pane, the active
    /// cycle.
    pub accent: Color,
    /// Messages about something that went wrong.
    pub error: Color,
    /// Borders of a pane or popup that is neither focused nor unfocused.
    pub border: Color,
    /// The focused issue's row. Reversing keeps each cell's own contrast.
    pub selection: Style,
    /// The glyph of each workflow state type.
    pub states: StateColors,
    /// The glyph of each priority.
    pub priorities: PriorityColors,
}

/// The color of each workflow state type's glyph.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StateColors {
    pub triage: Color,
    pub backlog: Color,
    pub unstarted: Color,
    pub started: Color,
    pub completed: Color,
    pub canceled: Color,
}

/// The color of each priority's glyph.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PriorityColors {
    pub urgent: Color,
    pub high: Color,
    pub medium: Color,
    pub low: Color,
    pub none: Color,
}

impl Default for Theme {
    /// The `terminal` theme: the terminal's own palette, cyan accent and dark
    /// gray muted, on the terminal's background.
    fn default() -> Self {
        Self {
            background: Color::Reset,
            text: Color::Reset,
            muted: Color::DarkGray,
            accent: Color::Cyan,
            error: Color::Red,
            border: Color::Reset,
            selection: Style::new().add_modifier(Modifier::REVERSED),
            states: StateColors {
                triage: Color::Magenta,
                backlog: Color::DarkGray,
                unstarted: Color::Reset,
                started: Color::Yellow,
                completed: Color::Green,
                canceled: Color::DarkGray,
            },
            priorities: PriorityColors {
                urgent: Color::Red,
                high: Color::LightRed,
                medium: Color::Yellow,
                low: Color::Blue,
                none: Color::DarkGray,
            },
        }
    }
}

impl Theme {
    /// The names of the built-in themes, the default first.
    pub fn built_in_names() -> impl Iterator<Item = &'static str> {
        std::iter::once(DEFAULT).chain(BASE16.iter().map(|(name, _)| *name))
    }

    /// The built-in theme called `name`, if there is one.
    pub fn built_in(name: &str) -> Option<Self> {
        if name == DEFAULT {
            return Some(Self::default());
        }
        let (_, json) = BASE16.iter().find(|(built_in, _)| *built_in == name)?;
        Self::from_base16_json(json).ok()
    }

    /// A theme from a Base16 scheme in JSON, with `base00` to `base0F` as
    /// six-digit hex colors. Other fields, such as `name`, are ignored.
    pub fn from_base16_json(json: &str) -> Result<Self, Base16Error> {
        let scheme: Base16 = serde_json::from_str(json).map_err(|error| {
            let text = error.to_string();
            // serde_json appends the position to every message.
            let message = match text.rsplit_once(" at line ") {
                Some((message, _)) => message.to_owned(),
                None => text,
            };
            Base16Error {
                line: (error.line() > 0).then_some(error.line()),
                message,
            }
        })?;
        Ok(Self::from_base16(&scheme))
    }

    fn from_base16(s: &Base16) -> Self {
        let background = s.base00.0;
        let text = s.base05.0;
        let muted = legible(s.base03.0, text, background);
        Self {
            background,
            text,
            muted,
            accent: s.base0c.0,
            error: s.base08.0,
            border: muted,
            selection: Style::new().add_modifier(Modifier::REVERSED),
            states: StateColors {
                triage: s.base0e.0,
                backlog: muted,
                unstarted: text,
                started: s.base0a.0,
                completed: s.base0b.0,
                canceled: muted,
            },
            priorities: PriorityColors {
                urgent: s.base08.0,
                high: s.base09.0,
                medium: s.base0a.0,
                low: s.base0d.0,
                none: muted,
            },
        }
    }

    /// The color of a workflow state of `state_type`.
    pub fn state(&self, state_type: StateType) -> Color {
        let states = &self.states;
        match state_type {
            StateType::Triage => states.triage,
            StateType::Backlog => states.backlog,
            StateType::Unstarted => states.unstarted,
            StateType::Started => states.started,
            StateType::Completed => states.completed,
            StateType::Canceled => states.canceled,
        }
    }

    /// The color of `priority`.
    pub fn priority(&self, priority: Priority) -> Color {
        let priorities = &self.priorities;
        match priority {
            Priority::Urgent => priorities.urgent,
            Priority::High => priorities.high,
            Priority::Medium => priorities.medium,
            Priority::Low => priorities.low,
            Priority::None => priorities.none,
        }
    }
}

/// A Base16 scheme that could not be read, with the line where it broke.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Base16Error {
    pub line: Option<usize>,
    pub message: String,
}

impl fmt::Display for Base16Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for Base16Error {}

/// The WCAG 2 contrast ratio of two RGB colors, from 1 to 21. `None` for a
/// named or indexed color, whose value only the terminal knows.
pub fn contrast_ratio(a: Color, b: Color) -> Option<f64> {
    let (a, b) = (luminance(a)?, luminance(b)?);
    let (light, dark) = if a > b { (a, b) } else { (b, a) };
    Some((light + 0.05) / (dark + 0.05))
}

fn luminance(color: Color) -> Option<f64> {
    let Color::Rgb(r, g, b) = color else {
        return None;
    };
    let channel = |value: u8| {
        let c = f64::from(value) / 255.0;
        if c <= 0.04045 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    };
    Some(0.2126 * channel(r) + 0.7152 * channel(g) + 0.0722 * channel(b))
}

/// `color`, or the first mix of it toward `toward` that reaches
/// [`MUTED_MIN_CONTRAST`] against `background`. Most schemes put a `base03`
/// too dark to read on their own `base00`.
fn legible(color: Color, toward: Color, background: Color) -> Color {
    const STEPS: u16 = 16;
    (0..=STEPS)
        .map(|step| mix(color, toward, step, STEPS))
        .find(|&mixed| {
            contrast_ratio(mixed, background).is_some_and(|ratio| ratio >= MUTED_MIN_CONTRAST)
        })
        .unwrap_or(toward)
}

fn mix(from: Color, to: Color, step: u16, steps: u16) -> Color {
    let (Color::Rgb(r1, g1, b1), Color::Rgb(r2, g2, b2)) = (from, to) else {
        return from;
    };
    let channel = |a: u8, b: u8| {
        let (a, b) = (u16::from(a), u16::from(b));
        let mixed = (a * (steps - step) + b * step) / steps;
        u8::try_from(mixed).unwrap_or(u8::MAX)
    };
    Color::Rgb(channel(r1, r2), channel(g1, g2), channel(b1, b2))
}

#[derive(Deserialize)]
struct Base16 {
    base00: Hex,
    #[expect(dead_code, reason = "no role draws with it")]
    base01: Hex,
    #[expect(dead_code, reason = "no role draws with it")]
    base02: Hex,
    base03: Hex,
    #[expect(dead_code, reason = "no role draws with it")]
    base04: Hex,
    base05: Hex,
    #[expect(dead_code, reason = "no role draws with it")]
    base06: Hex,
    #[expect(dead_code, reason = "no role draws with it")]
    base07: Hex,
    base08: Hex,
    base09: Hex,
    #[serde(rename = "base0A")]
    base0a: Hex,
    #[serde(rename = "base0B")]
    base0b: Hex,
    #[serde(rename = "base0C")]
    base0c: Hex,
    #[serde(rename = "base0D")]
    base0d: Hex,
    #[serde(rename = "base0E")]
    base0e: Hex,
    #[expect(dead_code, reason = "no role draws with it")]
    #[serde(rename = "base0F")]
    base0f: Hex,
}

/// A six-digit hex color such as `"1e1e2e"`, with or without a leading `#`.
struct Hex(Color);

impl<'de> Deserialize<'de> for Hex {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        let digits = text.strip_prefix('#').unwrap_or(&text);
        let channel = |range| {
            digits
                .get(range)
                .and_then(|pair| u8::from_str_radix(pair, 16).ok())
        };
        let is_hex = digits.len() == 6 && digits.bytes().all(|byte| byte.is_ascii_hexdigit());
        match (is_hex, channel(0..2), channel(2..4), channel(4..6)) {
            (true, Some(r), Some(g), Some(b)) => Ok(Hex(Color::Rgb(r, g, b))),
            _ => Err(serde::de::Error::custom(format!(
                "`{text}` is not a hex color such as \"1e1e2e\""
            ))),
        }
    }
}
