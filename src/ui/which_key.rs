//! The which-key popup: after a chord's first keys, what can follow.

use ratatui::Frame;
use ratatui::layout::{Margin, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, Paragraph};

use super::style;
use crate::app::State;
use crate::keys::{self, Key, KeySequence};

/// Draws in the bottom-right corner inside `area`'s border.
pub fn render(state: &State, prefix: &[Key], frame: &mut Frame, area: Rect) {
    let area = area.inner(Margin::new(1, 1));
    let contexts = keys::active_contexts(state);
    let lines: Vec<Line> = state
        .keymap
        .continuations(prefix, &contexts)
        .into_iter()
        .filter_map(|(rest, action)| {
            let spec = keys::spec(action)?;
            Some(Line::from(vec![
                Span::styled(format!(" {rest}  "), style::accent()),
                Span::raw(format!("{} ", spec.description)),
            ]))
        })
        .collect();

    let title = format!(" {} ", KeySequence::from(prefix));
    let close = super::close_hint(state);
    let width = lines
        .iter()
        .map(Line::width)
        .chain([title.chars().count(), close.width()])
        .max()
        .unwrap_or(0)
        + 2;
    let width = u16::try_from(width).unwrap_or(u16::MAX).min(area.width);
    let height = u16::try_from(lines.len() + 2)
        .unwrap_or(u16::MAX)
        .min(area.height);
    let popup = Rect {
        x: area.right().saturating_sub(width),
        y: area.bottom().saturating_sub(height),
        width,
        height,
    };
    let block = Block::bordered().title(title).title_bottom(close);
    frame.render_widget(Clear, popup);
    frame.render_widget(Paragraph::new(lines).block(block), popup);
}
