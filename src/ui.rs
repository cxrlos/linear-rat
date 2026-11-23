//! Rendering: a pure function of [`State`].

mod analytics;
mod browse;
mod cheatsheet;
mod confirm;
mod detail;
mod error;
mod hints;
pub mod icons;
pub(crate) mod markdown;
mod messages;
mod my_issues;
mod palette;
mod picker;
mod results;
mod search;
pub mod style;
mod team_issues;
mod text;
pub mod theme;
mod which_key;

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect, Size};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph, Tabs};

use crate::app::{Action, Indicator, Load, Modal, Screen, State};
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
    let titles = tab_titles(state, tabs_area.width);
    let tabs_width = titles_width(&titles);
    frame.render_widget(Tabs::new(titles).select(selected), tabs_area);
    let indicator_width = render_sync_indicator(state, frame, tabs_area, tabs_width);
    render_release_notice(state, frame, tabs_area, tabs_width, indicator_width);
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
        Screen::TeamIssues => team_issues::render(state, frame, body_area),
        Screen::Analytics => analytics::render(state, frame, body_area),
        Screen::Cycles => browse::render_cycles(state, frame, body_area),
        Screen::Projects => browse::render_projects(state, frame, body_area),
        Screen::Results => results::render(state, frame, body_area),
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
            Modal::Picker(edit) => picker::render_edit(state, edit, frame, body_area),
            Modal::Links(links) => picker::render_links(state, links, frame, body_area),
            Modal::IssuePicker(issues) => picker::render_issues(state, issues, frame, body_area),
            Modal::Palette(palette) => palette::render(state, palette, frame, body_area),
            Modal::Search(search) => search::render(state, search, frame, body_area),
            Modal::Choice(choice) => picker::render_choice(state, choice, frame, body_area),
            Modal::Create(form) => picker::render_create(state, form, frame, body_area),
            Modal::DueDate(prompt) => picker::render_due_date(state, prompt, frame, body_area),
            Modal::Confirm(confirm) => confirm::render(state, confirm, frame, body_area),
            Modal::Messages { scroll } => messages::render(state, *scroll, frame, body_area),
            Modal::Attach(prompt) => picker::render_attach(state, prompt, frame, body_area),
        }
    }
}

/// Every tab's title, as [`tab_title`] has it. When they do not fit in
/// `width` beside the sync indicator, the tabs of the other screens show
/// only their key, as `[2]`.
fn tab_titles(state: &State, width: u16) -> [String; Screen::ALL.len()] {
    let full = Screen::ALL.map(|screen| tab_title(state, screen, false));
    let indicator = indicator(state).map_or(0, |indicator| text::width(&indicator.text) + 2);
    let room = usize::from(width).saturating_sub(indicator);
    if usize::from(titles_width(&full)) <= room {
        return full;
    }
    Screen::ALL.map(|screen| tab_title(state, screen, screen != state.screen))
}

/// The screen's title after the first key that shows it from here, as
/// `[1] My Issues`, or only the key when `short`, as `[1]`. The bare title
/// if no key reaches it (unbound, or shadowed by a binding of the current
/// screen).
fn tab_title(state: &State, screen: Screen, short: bool) -> String {
    match state
        .keymap
        .keys_for(Action::ShowScreen(screen), &keys::screen_contexts(state))
        .first()
    {
        Some(key) if short => format!("[{key}]"),
        Some(key) => format!("[{key}] {}", screen.title()),
        None => screen.title().to_owned(),
    }
}

/// How wide [`Tabs`] draws `titles`: each padded by a space on both sides,
/// with a one-column divider between them.
fn titles_width(titles: &[String]) -> u16 {
    let padded: usize = titles.iter().map(|title| text::width(title) + 2).sum();
    let dividers = titles.len().saturating_sub(1);
    u16::try_from(padded + dividers).unwrap_or(u16::MAX)
}

/// The sync indicator at the right end of the tab row, muted while sync is
/// fine and accented when it is failing or throttled. Left out when it
/// would touch the tabs. Returns how many columns it took.
fn render_sync_indicator(state: &State, frame: &mut Frame, area: Rect, tabs_width: u16) -> u16 {
    let Some(indicator) = indicator(state) else {
        return 0;
    };
    let text = format!("{} ", indicator.text);
    let width = u16::try_from(text::width(&text)).unwrap_or(u16::MAX);
    if tabs_width.saturating_add(width).saturating_add(1) > area.width {
        return 0;
    }
    let style = if indicator.alert {
        style::accent(&state.theme)
    } else {
        style::muted(&state.theme)
    };
    let [_, right] =
        Layout::horizontal([Constraint::Fill(1), Constraint::Length(width)]).areas(area);
    frame.render_widget(Span::styled(text, style), right);
    width
}

/// The longest file name the upload indicator shows before cutting it.
const UPLOAD_NAME_WIDTH: usize = 24;

/// What the indicator shows: the uploads in flight while there are any, as
/// `uploading rind.log 42%`, else how sync is doing.
fn indicator(state: &State) -> Option<Indicator> {
    let uploads = state.uploads.active();
    let text = match uploads {
        [] => return state.sync_status.indicator(),
        [upload] => format!(
            "uploading {} {}%",
            text::fit(&upload.name, UPLOAD_NAME_WIDTH),
            upload.percent()
        ),
        several => {
            let size: u64 = several.iter().map(|upload| upload.size).sum();
            let sent: u64 = several
                .iter()
                .map(|upload| upload.sent.min(upload.size))
                .sum();
            let percent = (sent * 100).checked_div(size).unwrap_or(100);
            format!("uploading {} files {percent}%", several.len())
        }
    };
    Some(Indicator { text, alert: false })
}

/// The release notice, muted, between the tabs and the sync indicator,
/// `tabs_width` and `indicator_width` wide. The first thing left out when
/// the row is too narrow.
fn render_release_notice(
    state: &State,
    frame: &mut Frame,
    area: Rect,
    tabs_width: u16,
    indicator_width: u16,
) {
    let Some(version) = state.release.notice() else {
        return;
    };
    let text = format!("v{version} available  ");
    let width = u16::try_from(text::width(&text)).unwrap_or(u16::MAX);
    let taken = tabs_width.saturating_add(indicator_width);
    if taken.saturating_add(width).saturating_add(1) > area.width {
        return;
    }
    let left = area.width - indicator_width - width;
    let notice = Rect {
        x: area.x.saturating_add(left),
        width,
        ..area
    };
    frame.render_widget(Span::styled(text, style::muted(&state.theme)), notice);
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

/// How many rows the focused issue's detail shows in a `viewport`-sized
/// terminal, inside its border, and how many lines it has. Both are 0 when
/// no detail is on screen.
pub fn detail_rows(state: &State, viewport: Size) -> (usize, usize) {
    let area = Rect::from((ratatui::layout::Position::ORIGIN, viewport));
    let [_, body_area, _, _] = areas(area, notice_rows(state, viewport.width).len());
    let pane = match state.screen {
        Screen::MyIssues => my_issues::detail_area(state, body_area, state.my_issues.pane()),
        Screen::TeamIssues => team_issues::detail_area(state, body_area),
        Screen::Results => my_issues::detail_area(state, body_area, state.results.pane()),
        Screen::Analytics | Screen::Cycles | Screen::Projects => None,
    };
    let Some(pane) = pane else {
        return (0, 0);
    };
    let inner = Block::bordered().inner(pane);
    let lines =
        detail::issue(state).map_or(0, |issue| detail::lines(state, issue, inner.width).len());
    (usize::from(inner.height), lines)
}

/// How many issue rows fit in the Team Issues list of a `viewport`-sized
/// terminal, under its header.
pub fn team_list_rows(state: &State, viewport: Size) -> usize {
    list_rows(state, viewport).saturating_sub(usize::from(team_issues::header_rows(state)))
}

/// How many rows fit in the list of the Cycles or Projects screen of a
/// `viewport`-sized terminal, under any header.
pub fn browse_rows(state: &State, viewport: Size) -> usize {
    let header = match state.screen {
        Screen::Cycles => browse::cycles_header_rows(state),
        _ => 0,
    };
    list_rows(state, viewport).saturating_sub(usize::from(header))
}

/// How far the cheatsheet can scroll in a `viewport`-sized terminal: how
/// many of its rows do not fit.
pub fn cheatsheet_max_scroll(state: &State, viewport: Size) -> u16 {
    let area = Rect::from((ratatui::layout::Position::ORIGIN, viewport));
    let [_, body_area, _, _] = areas(area, notice_rows(state, viewport.width).len());
    cheatsheet::max_scroll(state, body_area)
}

/// How far the error log can scroll in a `viewport`-sized terminal: how
/// many of its rows do not fit.
pub fn messages_max_scroll(state: &State, viewport: Size) -> u16 {
    let area = Rect::from((ratatui::layout::Position::ORIGIN, viewport));
    let [_, body_area, _, _] = areas(area, notice_rows(state, viewport.width).len());
    messages::max_scroll(state, body_area)
}

/// How far the Analytics screen can scroll in a `viewport`-sized terminal:
/// how many of its rows do not fit.
pub fn analytics_max_scroll(state: &State, viewport: Size) -> u16 {
    let area = Rect::from((ratatui::layout::Position::ORIGIN, viewport));
    let [_, body_area, _, _] = areas(area, notice_rows(state, viewport.width).len());
    analytics::max_scroll(state, body_area)
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
