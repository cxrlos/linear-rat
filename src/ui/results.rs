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
    // An issue deleted on Linear since leaves the store, so rows keep their
    // place in the results to match the focus.
    let issues: Vec<_> = results
        .ids()
        .iter()
        .enumerate()
        .filter_map(|(index, id)| state.store.issue(id).map(|issue| (index, issue)))
        .collect();
    let title = format!("{} ({})", results.title(), issues.len());
    let block = my_issues::pane_block(title, focus, &state.theme);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let focused_line = issues
        .iter()
        .position(|(index, _)| *index == results.focused_index())
        .unwrap_or(0);
    let offset = my_issues::visible_offset(results.scroll(), focused_line, inner);
    let cells = my_issues::row_cells(issues.iter().map(|(_, issue)| *issue));
    for (line_area, (index, issue)) in inner.rows().zip(issues.iter().skip(offset)) {
        let is_focused = *index == results.focused_index();
        my_issues::render_row(view, frame, line_area, &cells, issue, is_focused, focus);
    }
}
