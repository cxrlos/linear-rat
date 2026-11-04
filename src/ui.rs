//! Rendering: a pure function of [`State`].

mod cheatsheet;
mod error;
mod hints;
pub mod icons;
mod my_issues;
pub mod style;
pub mod theme;
mod which_key;

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect, Size};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph, Tabs};

use crate::app::{Action, Load, Modal, Screen, State};
use crate::keys::{self, Context};

const MAX_NOTICE_ROWS: usize = 3;

pub fn render(state: &State, frame: &mut Frame) {
    let theme = &state.theme;
    let area = frame.area();
    frame.buffer_mut().set_style(area, style::base(theme));
    let notice = notice_rows(state, area.width);
    let [tabs_area, body_area, notice_area, bar_area] = areas(area, notice.len());

    let selected = Screen::ALL
        .iter()
        .position(|&screen| screen == state.screen);
    let titles = Screen::ALL.map(|screen| tab_title(state, screen));
    let tabs_width = titles_width(&titles);
    frame.render_widget(Tabs::new(titles).select(selected), tabs_area);
    render_sync_indicator(state, frame, tabs_area, tabs_width);
    match state.screen {
        Screen::MyIssues => match &state.load {
            Load::Loaded => my_issues::render(state, frame, body_area),
            Load::Loading => error::render_loading(&state.theme, frame, body_area),
            Load::Failed(error) => {
                error::render(
                    error,
                    state.key_source.as_ref(),
                    &state.theme,
                    frame,
                    body_area,
                );
            }
        },
        Screen::TeamIssues | Screen::Analytics => {
            let block = Block::bordered()
                .border_style(style::border(theme))
                .title(state.screen.title());
            frame.render_widget(block, body_area);
        }
    }
    frame.render_widget(
        Paragraph::new(notice).style(style::error(theme)),
        notice_area,
    );
    hints::render(state, frame, bar_area);

    for modal in state.modals.iter() {
        match modal {
            Modal::Cheatsheet { scroll } => cheatsheet::render(state, *scroll, frame, body_area),
            Modal::WhichKey { prefix } => which_key::render(state, prefix, frame, body_area),
        }
    }
}

/// The screen's title after the first key that shows it from here, as
/// `[1] My Issues`, or the bare title if no key reaches it (unbound, or
/// shadowed by a binding of the current screen).
fn tab_title(state: &State, screen: Screen) -> String {
    match state
        .keymap
        .keys_for(Action::ShowScreen(screen), &keys::screen_contexts(state))
        .first()
    {
        Some(key) => format!("[{key}] {}", screen.title()),
        None => screen.title().to_owned(),
    }
}

/// How wide [`Tabs`] draws `titles`: each padded by a space on both sides,
/// with a one-column divider between them.
fn titles_width(titles: &[String]) -> u16 {
    let padded: usize = titles
        .iter()
        .map(|title| Span::raw(title.as_str()).width() + 2)
        .sum();
    let dividers = titles.len().saturating_sub(1);
    u16::try_from(padded + dividers).unwrap_or(u16::MAX)
}

/// The sync indicator at the right end of the tab row, muted while sync is
/// fine and accented when it is failing or throttled. Left out when it
/// would touch the tabs.
fn render_sync_indicator(state: &State, frame: &mut Frame, area: Rect, tabs_width: u16) {
    let Some(indicator) = state.sync_status.indicator() else {
        return;
    };
    let text = Span::raw(format!("{} ", indicator.text));
    let width = u16::try_from(text.width()).unwrap_or(u16::MAX);
    if tabs_width.saturating_add(width).saturating_add(1) > area.width {
        return;
    }
    let style = if indicator.alert {
        style::accent(&state.theme)
    } else {
        style::muted(&state.theme)
    };
    let [_, right] =
        Layout::horizontal([Constraint::Fill(1), Constraint::Length(width)]).areas(area);
    frame.render_widget(text.style(style), right);
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
            Span::styled(key.to_string(), style::accent(&state.theme)),
            Span::raw(" close "),
        ]),
        None => Line::default(),
    }
}

/// The block of a popup: its border in the theme's border color, and the
/// theme's text and background under it, since `Clear` resets them.
fn popup_block(state: &State) -> Block<'static> {
    Block::bordered()
        .border_style(style::border(&state.theme))
        .style(style::base(&state.theme))
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

/// How many issue rows fit in the body of a `viewport`-sized terminal,
/// inside its border. Page moves and scrolling use it.
pub fn list_rows(state: &State, viewport: Size) -> usize {
    let area = Rect::from((ratatui::layout::Position::ORIGIN, viewport));
    let [_, body_area, _, _] = areas(area, notice_rows(state, viewport.width).len());
    usize::from(body_area.height.saturating_sub(2))
}

/// How far the cheatsheet can scroll in a `viewport`-sized terminal: how
/// many of its rows do not fit.
pub fn cheatsheet_max_scroll(state: &State, viewport: Size) -> u16 {
    let area = Rect::from((ratatui::layout::Position::ORIGIN, viewport));
    let [_, body_area, _, _] = areas(area, notice_rows(state, viewport.width).len());
    cheatsheet::max_scroll(state, body_area)
}

/// The tab row, the body, the notice and the bottom bar, top to bottom.
fn areas(area: Rect, notice_rows: usize) -> [Rect; 4] {
    Layout::vertical([
        Constraint::Length(1),
        Constraint::Fill(1),
        Constraint::Length(u16::try_from(notice_rows).unwrap_or(0)),
        Constraint::Length(hints::ROWS),
    ])
    .areas(area)
}

fn notice_rows(state: &State, width: u16) -> Vec<Line<'static>> {
    state
        .notice
        .as_deref()
        .map(|notice| notice_lines(notice, width))
        .unwrap_or_default()
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
