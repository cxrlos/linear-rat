//! The messages overlay: the error log, newest first, each failure with
//! when it happened, its kind, the action it ended, what to do about it and
//! the full message.

use ratatui::Frame;
use ratatui::layout::{Margin, Rect};
use ratatui::style::Style;
use ratatui::text::Line;
use ratatui::widgets::{Clear, Paragraph};

use super::{markdown, style};
use crate::app::{LoggedFailure, State};

const MAX_WIDTH: u16 = 80;
/// Before each line of an entry after its first.
const INDENT: &str = "   ";

/// Draws inside `area`'s border, scrolled down by `scroll` rows (no further
/// than its last row).
pub fn render(state: &State, scroll: u16, frame: &mut Frame, area: Rect) {
    let popup = popup(area);
    let lines = lines(state, popup);
    let scroll = scroll.min(hidden_rows(popup, &lines));
    let block = super::popup_block(state)
        .title(" Messages ")
        .title_bottom(super::close_hint(state));
    frame.render_widget(Clear, popup);
    frame.render_widget(
        Paragraph::new(lines).block(block).scroll((scroll, 0)),
        popup,
    );
}

/// How many rows of the log do not fit in `area`.
pub fn max_scroll(state: &State, area: Rect) -> u16 {
    let popup = popup(area);
    hidden_rows(popup, &lines(state, popup))
}

fn hidden_rows(popup: Rect, lines: &[Line]) -> u16 {
    u16::try_from(lines.len())
        .unwrap_or(u16::MAX)
        .saturating_sub(popup.height.saturating_sub(2))
}

/// The popup: as tall as `area` inside its border, and up to
/// [`MAX_WIDTH`] wide, centered.
fn popup(area: Rect) -> Rect {
    let inside = area.inner(Margin::new(1, 1));
    super::centered(inside, MAX_WIDTH, inside.height)
}

fn lines(state: &State, popup: Rect) -> Vec<Line<'static>> {
    let theme = &state.theme;
    if state.errors.is_empty() {
        return vec![Line::styled(" No errors yet.", style::muted(theme))];
    }
    // The border and a space inside each side.
    let width = popup.width.saturating_sub(4);
    let mut lines = Vec::new();
    for (index, logged) in state.errors.newest_first().enumerate() {
        if index > 0 {
            lines.push(Line::default());
        }
        lines.extend(entry(state, logged, width));
    }
    lines
}

/// One failure: when, its kind and the action it ended on the first line,
/// then what went wrong with what to do about it, then the full message,
/// muted.
fn entry(state: &State, logged: &LoggedFailure, width: u16) -> Vec<Line<'static>> {
    let theme = &state.theme;
    let failure = &logged.failure;
    let time = logged.at.map_or_else(
        || "--:--:--".to_owned(),
        |at| {
            at.to_zoned(state.analytics_screen.time_zone().clone())
                .strftime("%H:%M:%S")
                .to_string()
        },
    );
    let indent = [(INDENT.to_owned(), Style::new())];
    let mut lines = markdown::wrap(
        &[
            (format!("{time}  "), style::muted(theme)),
            (failure.kind.name().to_owned(), style::error(theme)),
            (" · ".to_owned(), style::muted(theme)),
            (failure.action.clone(), Style::new()),
        ],
        width,
        &[(" ".to_owned(), Style::new())],
        &indent,
    );
    let advice = format!("{}. {}", failure.kind.summary(), failure.kind.fix());
    lines.extend(markdown::wrap(
        &[(advice, Style::new())],
        width,
        &indent,
        &indent,
    ));
    lines.extend(markdown::wrap(
        &[(failure.message.clone(), style::muted(theme))],
        width,
        &indent,
        &indent,
    ));
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_short_area_hides_the_rows_that_do_not_fit() {
        let popup = Rect::new(0, 0, 40, 5);
        let lines = vec![Line::default(); 7];

        assert_eq!(hidden_rows(popup, &lines), 4);
    }
}
