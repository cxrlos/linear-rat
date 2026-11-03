//! Rendering: a pure function of [`State`].

mod cheatsheet;
mod hints;
pub mod style;
mod which_key;

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph, Tabs};

use crate::app::{Action, Modal, Screen, State};
use crate::keys::Context;

const MAX_NOTICE_ROWS: usize = 3;

pub fn render(state: &State, frame: &mut Frame) {
    let notice = state
        .notice
        .as_deref()
        .map(|notice| notice_lines(notice, frame.area().width))
        .unwrap_or_default();
    let [tabs_area, body_area, notice_area, bar_area] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Fill(1),
        Constraint::Length(u16::try_from(notice.len()).unwrap_or(0)),
        Constraint::Length(1),
    ])
    .areas(frame.area());

    let selected = Screen::ALL
        .iter()
        .position(|&screen| screen == state.screen);
    let tabs = Tabs::new(Screen::ALL.map(Screen::title)).select(selected);
    frame.render_widget(tabs, tabs_area);
    frame.render_widget(Block::bordered().title(state.screen.title()), body_area);
    frame.render_widget(Paragraph::new(notice).style(style::error()), notice_area);
    hints::render(state, frame, bar_area);

    for modal in state.modals.iter() {
        match modal {
            Modal::Cheatsheet => cheatsheet::render(state, frame, body_area),
            Modal::WhichKey { prefix } => which_key::render(state, prefix, frame, body_area),
        }
    }
}

/// The first key that closes an overlay, as ` Esc close `, or nothing if
/// the viewer unbound them all.
fn close_hint(state: &State) -> Line<'static> {
    match state
        .keymap
        .keys_for(Action::Close, &[Context::Overlay])
        .first()
    {
        Some(key) => Line::from(vec![
            Span::raw(" "),
            Span::styled(key.to_string(), style::accent()),
            Span::raw(" close "),
        ]),
        None => Line::default(),
    }
}

/// A `width` by `height` box centered in `area`, shrunk to fit.
fn centered(area: Rect, width: u16, height: u16) -> Rect {
    let width = width.min(area.width);
    let height = height.min(area.height);
    Rect {
        x: area.x + (area.width - width) / 2,
        y: area.y + (area.height - height) / 2,
        width,
        height,
    }
}

/// `notice` cut into rows of `width` characters, at most [`MAX_NOTICE_ROWS`].
fn notice_lines(notice: &str, width: u16) -> Vec<Line<'static>> {
    let chars: Vec<char> = notice.chars().collect();
    chars
        .chunks(usize::from(width.max(1)))
        .take(MAX_NOTICE_ROWS)
        .map(|row| Line::from(row.iter().collect::<String>()))
        .collect()
}
