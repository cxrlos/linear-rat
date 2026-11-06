//! The search overlay: the query, and how far its search is.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::Line;
use ratatui::widgets::{Paragraph, Wrap};

use super::{palette, style};
use crate::app::{Phase, Search, State};

pub fn render(state: &State, search: &Search, frame: &mut Frame, area: Rect) {
    let theme = &state.theme;
    let list = palette::finder(state, " Search Linear ", "/", &search.query, frame, area);
    let message = match &search.phase {
        Phase::Typing {
            problem: Some(problem),
        } => Line::styled(format!(" {problem}"), style::error(theme)),
        Phase::Typing { problem: None } => Line::styled(
            " Enter searches every issue in the workspace.",
            style::muted(theme),
        ),
        Phase::Searching { term } => {
            Line::styled(format!(" Searching for \"{term}\"..."), style::muted(theme))
        }
        Phase::NothingFound { term } => {
            Line::styled(format!(" Nothing matches \"{term}\"."), style::muted(theme))
        }
    };
    frame.render_widget(Paragraph::new(message).wrap(Wrap { trim: false }), list);
}
