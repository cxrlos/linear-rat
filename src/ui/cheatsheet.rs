//! The `?` overlay: every action of the contexts below it, with its keys.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, Paragraph};

use super::style;
use crate::app::State;
use crate::keys::{self, ACTIONS, ActionSpec};

pub fn render(state: &State, frame: &mut Frame, area: Rect) {
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
        lines.push(Line::styled(title, style::accent()));
        for (keys, spec) in entries {
            let (key_style, text_style) = if (spec.available)(state) {
                (style::accent(), Style::new())
            } else {
                (style::muted(), style::muted())
            };
            lines.push(Line::from(vec![
                Span::styled(format!("  {keys:<key_width$}  "), key_style),
                Span::styled(spec.description, text_style),
            ]));
        }
    }

    let width = lines.iter().map(Line::width).max().unwrap_or(0) + 4;
    let height = lines.len() + 2;
    let popup = super::centered(
        area,
        u16::try_from(width).unwrap_or(u16::MAX),
        u16::try_from(height).unwrap_or(u16::MAX),
    );
    let block = Block::bordered()
        .title(" Keys ")
        .title_bottom(super::close_hint(state));
    frame.render_widget(Clear, popup);
    frame.render_widget(Paragraph::new(lines).block(block), popup);
}
