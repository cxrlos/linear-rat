//! A list to pick one entry from, such as the team switcher or a filter's
//! values.

use ratatui::Frame;
use ratatui::layout::{Margin, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph};

use super::style;
use crate::app::{Action, Choice, State};
use crate::keys::Context;

pub fn render(state: &State, choice: &Choice, frame: &mut Frame, area: Rect) {
    let theme = &state.theme;
    let title = format!(" {} ", choice.title());
    let (pick, close) = (pick_hint(state), super::close_hint(state));
    // Room for both hints in the bottom border, a dash between them.
    let hints_width = pick.width() + close.width() + 3;
    let width = choice
        .entries
        .iter()
        .map(|entry| entry.label.chars().count() + 4)
        .chain([title.chars().count() + 2, hints_width])
        .max()
        .unwrap_or(0);
    let popup = super::centered(
        area.inner(Margin::new(1, 1)),
        u16::try_from(width).unwrap_or(u16::MAX),
        u16::try_from(choice.entries.len() + 2).unwrap_or(u16::MAX),
    );
    let height = usize::from(popup.height.saturating_sub(2));
    let offset = (choice.cursor + 1).saturating_sub(height);
    let lines: Vec<Line> = choice
        .entries
        .iter()
        .enumerate()
        .skip(offset)
        .take(height)
        .map(|(index, entry)| {
            let line = Line::from(format!(" {} ", entry.label));
            if index == choice.cursor {
                line.style(theme.selection)
            } else {
                line
            }
        })
        .collect();
    let block = super::popup_block(state)
        .title(title)
        .title_bottom(pick)
        .title_bottom(close);
    frame.render_widget(Clear, popup);
    frame.render_widget(Paragraph::new(lines).block(block), popup);
}

/// The first key that picks, as ` Enter pick `, or nothing if unbound.
fn pick_hint(state: &State) -> Line<'static> {
    match state
        .keymap
        .keys_for(Action::Accept, &[Context::Overlay])
        .first()
    {
        Some(key) => Line::from(vec![
            Span::raw(" "),
            Span::styled(key.to_string(), style::accent(&state.theme)),
            Span::raw(" pick "),
        ]),
        None => Line::default(),
    }
}
