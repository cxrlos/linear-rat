//! The `?` overlay: every action of the contexts below it, with its keys.

use ratatui::Frame;
use ratatui::layout::{Margin, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph};

use super::style;
use crate::app::State;
use crate::keys::{self, ACTIONS, ActionSpec};

/// Draws inside `area`'s border, scrolled down by `scroll` rows (no further
/// than its last row).
pub fn render(state: &State, scroll: u16, frame: &mut Frame, area: Rect) {
    let lines = lines(state, |spec| (spec.available)(state));
    let popup = popup(area, &lines);
    let scroll = scroll.min(hidden_rows(popup, &lines));
    let block = super::popup_block(state)
        .title(" Keys ")
        .title_bottom(super::close_hint(state));
    frame.render_widget(Clear, popup);
    frame.render_widget(
        Paragraph::new(lines).block(block).scroll((scroll, 0)),
        popup,
    );
}

/// How many rows of the cheatsheet do not fit in `area`.
pub fn max_scroll(state: &State, area: Rect) -> u16 {
    // Availability only styles the lines, and the scroll actions' own
    // availability asks for this size, so it is left out here.
    let lines = lines(state, |_| true);
    hidden_rows(popup(area, &lines), &lines)
}

fn hidden_rows(popup: Rect, lines: &[Line]) -> u16 {
    u16::try_from(lines.len())
        .unwrap_or(u16::MAX)
        .saturating_sub(popup.height.saturating_sub(2))
}

/// The box `lines` need, centered inside `area`'s border and shrunk to fit.
fn popup(area: Rect, lines: &[Line]) -> Rect {
    let width = lines.iter().map(Line::width).max().unwrap_or(0) + 4;
    let height = lines.len() + 2;
    super::centered(
        area.inner(Margin::new(1, 1)),
        u16::try_from(width).unwrap_or(u16::MAX),
        u16::try_from(height).unwrap_or(u16::MAX),
    )
}

/// One heading per context, then its actions with their keys, muted where
/// `available` says they cannot run now.
fn lines(state: &State, available: impl Fn(&ActionSpec) -> bool) -> Vec<Line<'static>> {
    let theme = &state.theme;
    let contexts = keys::screen_contexts(state);
    let mut sections: Vec<(&str, Vec<(String, &ActionSpec)>)> = Vec::new();
    let mut listed = Vec::new();
    for &context in &contexts {
        let mut entries = Vec::new();
        for spec in ACTIONS.iter().filter(|s| s.contexts.contains(&context)) {
            if listed.contains(&spec.action) {
                continue;
            }
            listed.push(spec.action);
            let keys: Vec<String> = state
                .keymap
                .keys_for(spec.action, &contexts)
                .iter()
                .map(ToString::to_string)
                .collect();
            entries.push((keys.join(", "), spec));
        }
        if !entries.is_empty() {
            sections.push((context.title(), entries));
        }
    }

    let key_width = sections
        .iter()
        .flat_map(|(_, entries)| entries.iter().map(|(keys, _)| keys.chars().count()))
        .max()
        .unwrap_or(0);
    let mut lines: Vec<Line> = Vec::new();
    for (title, entries) in sections {
        if !lines.is_empty() {
            lines.push(Line::default());
        }
        lines.push(Line::styled(title, style::accent(theme)));
        for (keys, spec) in entries {
            let (key_style, text_style) = if available(spec) {
                (style::accent(theme), Style::new())
            } else {
                (style::muted(theme), style::muted(theme))
            };
            lines.push(Line::from(vec![
                Span::styled(format!("  {keys:<key_width$}  "), key_style),
                Span::styled(spec.description, text_style),
            ]));
        }
    }
    lines
}
