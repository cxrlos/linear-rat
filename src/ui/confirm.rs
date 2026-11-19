//! The confirmation modal: a title, a body and the keys that confirm or
//! cancel, all from the keymap.

use super::text;
use ratatui::Frame;
use ratatui::layout::{Margin, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Padding, Paragraph, Wrap};

use super::style;
use crate::app::{Action, Confirm, State};
use crate::keys::Context;

/// Draws `confirm` centered inside `area`'s border.
pub fn render(state: &State, confirm: &Confirm, frame: &mut Frame, area: Rect) {
    let area = area.inner(Margin::new(1, 1));
    let lines: Vec<Line> = confirm
        .body
        .iter()
        .map(|line| Line::raw(line.clone()))
        .collect();
    let bottom = Line::from(
        [
            (&[Action::Accept, Action::Confirm][..], confirm.verb),
            (&[Action::Close, Action::Decline][..], "cancel"),
        ]
        .into_iter()
        .flat_map(|(actions, word)| keys(state, actions, word))
        .collect::<Vec<_>>(),
    );
    let title = format!(" {} ", confirm.title);
    let widest = lines
        .iter()
        .map(Line::width)
        .chain([text::width(&title), bottom.width()])
        .max()
        .unwrap_or(0);
    let width = u16::try_from(widest + 4).unwrap_or(u16::MAX);
    let inner_width = usize::from(width.min(area.width).saturating_sub(4)).max(1);
    let rows: usize = lines
        .iter()
        .map(|line| line.width().max(1).div_ceil(inner_width))
        .sum();
    let height = u16::try_from(rows + 2).unwrap_or(u16::MAX);
    let popup = super::centered(area, width, height);
    let block = super::popup_block(state)
        .title(title)
        .title_bottom(bottom)
        .padding(Padding::horizontal(1));
    frame.render_widget(Clear, popup);
    frame.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .block(block),
        popup,
    );
}

/// The first key of each of `actions` in the overlay, joined, then `word`.
fn keys(state: &State, actions: &[Action], word: &str) -> Vec<Span<'static>> {
    let keys: Vec<String> = actions
        .iter()
        .filter_map(|&action| {
            state
                .keymap
                .keys_for(action, &[Context::Overlay])
                .into_iter()
                .next()
                .map(|keys| keys.to_string())
        })
        .collect();
    if keys.is_empty() {
        return Vec::new();
    }
    vec![
        Span::raw(" "),
        Span::styled(keys.join("/"), style::accent(&state.theme)),
        Span::raw(format!(" {word} ")),
    ]
}
