//! The search overlay: the query, then what Linear found for it.

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
        Phase::Found { term, ids, .. } if ids.is_empty() => {
            Line::styled(format!(" Nothing matches \"{term}\"."), style::muted(theme))
        }
        Phase::Found { ids, selected, .. } => {
            let width = usize::from(list.width);
            let issues: Vec<_> = ids.iter().filter_map(|id| state.store.issue(id)).collect();
            let id_width = palette::identifier_width(issues.iter().copied());
            let lines = issues
                .into_iter()
                .map(|issue| palette::issue_line(state, issue, id_width, width))
                .collect();
            palette::render_list(state, lines, *selected, frame, list);
            return;
        }
    };
    frame.render_widget(Paragraph::new(message).wrap(Wrap { trim: false }), list);
}
