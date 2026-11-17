//! The results view: what a search found, or the issue a palette jump went
//! to, as a list beside the focused one's detail, laid out like Team Issues.

use ratatui::Frame;
use ratatui::layout::Rect;

use super::detail;
use super::my_issues::{self, Focus, View};
use crate::app::State;

pub fn render(state: &State, frame: &mut Frame, area: Rect) {
    let results = &state.results;
    let view = View::new(state, results.names());
    let (list, detail) = my_issues::panes(state, area, results.pane());
    if let Some((list_area, focus)) = list {
        render_list(&view, state, frame, list_area, focus);
    }
    if let Some((detail_area, focus)) = detail {
        my_issues::render_detail(state, detail::issue(state), frame, detail_area, focus);
    }
}

fn render_list(view: &View, state: &State, frame: &mut Frame, area: Rect, focus: Focus) {
    let results = &state.results;
    let name = format!("{} ({})", results.title(), results.list().len());
    let title = my_issues::list_title(&name, results.arrangement(), "Linear's order");
    let block = my_issues::pane_block(title, focus, &state.theme);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    my_issues::render_groups(view, frame, inner, results.list(), focus);
}
