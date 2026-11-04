//! The bottom bar: the keys for the actions of the active contexts.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};

use super::style;
use crate::app::State;
use crate::keys::{self, ACTIONS};

pub fn render(state: &State, frame: &mut Frame, area: Rect) {
    let contexts = keys::active_contexts(state);
    let mut spans = Vec::new();
    let mut shown = Vec::new();
    for &context in &contexts {
        for spec in ACTIONS.iter().filter(|s| s.contexts.contains(&context)) {
            let Some(hint) = spec.hint else { continue };
            if shown.contains(&spec.action) {
                continue;
            }
            let Some(key) = state
                .keymap
                .keys_for(spec.action, &contexts)
                .into_iter()
                .next()
            else {
                continue;
            };
            shown.push(spec.action);
            let (key_style, hint_style) = if (spec.available)(state) {
                (style::accent(&state.theme), Style::new())
            } else {
                (style::muted(&state.theme), style::muted(&state.theme))
            };
            spans.push(Span::raw(" "));
            spans.push(Span::styled(key.to_string(), key_style));
            spans.push(Span::styled(format!(" {hint} "), hint_style));
        }
    }
    frame.render_widget(Line::from(spans), area);
}
