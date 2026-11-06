//! The palette overlay, and the query box and list it shares with search.

use ratatui::Frame;
use ratatui::layout::{Margin, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, Paragraph};

use super::style;
use crate::app::{self, Palette, PaletteEntry, State};
use crate::keys;
use crate::linear::model::Issue;

const MAX_WIDTH: u16 = 76;
const MAX_HEIGHT: u16 = 16;

pub fn render(state: &State, palette: &Palette, frame: &mut Frame, area: Rect) {
    let theme = &state.theme;
    let list = finder(state, " Palette ", ":", &palette.query, frame, area);
    let entries = app::palette_matches(state, &palette.query);
    if entries.is_empty() {
        let none = Line::styled(" Nothing matches.", style::muted(theme));
        frame.render_widget(none, list);
        return;
    }
    let width = usize::from(list.width);
    let id_width = identifier_width(entries.iter().filter_map(|entry| match entry {
        PaletteEntry::Issue(issue) => Some(*issue),
        PaletteEntry::Action(_) => None,
    }));
    let lines = entries.iter().map(|entry| match entry {
        PaletteEntry::Action(spec) => {
            let keys: Vec<String> = action_keys(state, spec)
                .iter()
                .map(ToString::to_string)
                .collect();
            let (key_style, text_style) = if (spec.available)(state) {
                (style::accent(theme), Style::new())
            } else {
                (style::muted(theme), style::muted(theme))
            };
            spread(
                vec![Span::styled(spec.description, text_style)],
                vec![Span::styled(keys.join(", "), key_style)],
                width,
            )
        }
        PaletteEntry::Issue(issue) => issue_line(state, issue, id_width, width),
    });
    render_list(state, lines.collect(), palette.selected, frame, list);
}

/// The keys that run `spec` on the screen under the overlay or, when none
/// do there, the keys it has where it applies.
fn action_keys(state: &State, spec: &keys::ActionSpec) -> Vec<keys::KeySequence> {
    let here = state
        .keymap
        .keys_for(spec.action, &keys::screen_contexts(state));
    if here.is_empty() {
        state.keymap.keys_for(spec.action, spec.contexts)
    } else {
        here
    }
}

/// Draws the popup with `title`, its query line after `prompt`, and a rule
/// under it, near the top of `area`. Returns the area left for the list.
pub(super) fn finder(
    state: &State,
    title: &'static str,
    prompt: &'static str,
    query: &str,
    frame: &mut Frame,
    area: Rect,
) -> Rect {
    let theme = &state.theme;
    let inside = area.inner(Margin::new(1, 1));
    let width = MAX_WIDTH.min(inside.width);
    let popup = Rect {
        x: inside.x + (inside.width - width) / 2,
        y: inside.y,
        width,
        height: MAX_HEIGHT.min(inside.height),
    };
    let block: Block = super::popup_block(state)
        .title(title)
        .title_bottom(super::close_hint(state));
    let inner = block.inner(popup);
    frame.render_widget(Clear, popup);
    frame.render_widget(block, popup);

    let query_line = Line::from(vec![
        Span::styled(format!(" {prompt} "), style::accent(theme)),
        Span::raw(query.to_owned()),
        Span::styled(" ", Style::new().add_modifier(Modifier::REVERSED)),
    ]);
    let rows: Vec<Rect> = inner.rows().collect();
    let [query_row, rule_row, ..] = rows[..] else {
        return Rect::default();
    };
    frame.render_widget(query_line, query_row);
    frame.render_widget(
        Line::styled(
            "─".repeat(usize::from(rule_row.width)),
            style::border(theme),
        ),
        rule_row,
    );
    Rect {
        y: rule_row.y + 1,
        height: inner.height.saturating_sub(2),
        ..inner
    }
}

/// Draws `lines` in `area`, scrolled so the `selected` one shows, and
/// highlights it.
pub(super) fn render_list(
    state: &State,
    lines: Vec<Line<'static>>,
    selected: usize,
    frame: &mut Frame,
    area: Rect,
) {
    let rows = usize::from(area.height);
    let offset = (selected + 1).saturating_sub(rows);
    for (row, (index, line)) in area.rows().zip(lines.into_iter().enumerate().skip(offset)) {
        frame.render_widget(Paragraph::new(line), row);
        if index == selected {
            frame.buffer_mut().set_style(row, state.theme.selection);
        }
    }
}

/// The widest identifier of `issues`, so their titles line up.
pub(super) fn identifier_width<'a>(issues: impl Iterator<Item = &'a Issue>) -> usize {
    issues
        .map(|issue| issue.identifier.chars().count())
        .max()
        .unwrap_or(0)
}

/// An issue as both lists show it: its identifier, padded to `id_width`,
/// and title on the left, its workflow state on the right.
pub(super) fn issue_line(
    state: &State,
    issue: &Issue,
    id_width: usize,
    width: usize,
) -> Line<'static> {
    let theme = &state.theme;
    let state_type = issue.state.state_type;
    spread(
        vec![
            Span::styled(
                format!("{:<id_width$}", issue.identifier),
                style::muted(theme),
            ),
            Span::raw(" "),
            Span::raw(issue.title.clone()),
        ],
        vec![
            Span::styled(state.ui.icons.state(state_type), theme.state(state_type)),
            Span::raw(" "),
            Span::raw(issue.state.name.clone()),
        ],
        width,
    )
}

/// `left` and `right` on one line `width` wide, a space in from each edge,
/// with `left` cut short when both do not fit.
fn spread(left: Vec<Span<'static>>, right: Vec<Span<'static>>, width: usize) -> Line<'static> {
    let right_width: usize = right.iter().map(Span::width).sum();
    let room = width.saturating_sub(right_width + 3);
    let mut spans = vec![Span::raw(" ")];
    let mut used = 0;
    for span in left {
        let left_here = room.saturating_sub(used);
        if span.width() <= left_here {
            used += span.width();
            spans.push(span);
        } else if left_here > 0 {
            let cut: String = span
                .content
                .chars()
                .take(left_here.saturating_sub(1))
                .collect();
            used += cut.chars().count() + 1;
            spans.push(Span::styled(format!("{cut}…"), span.style));
            break;
        }
    }
    spans.push(Span::raw(
        " ".repeat(width.saturating_sub(used + right_width + 2).max(1)),
    ));
    spans.extend(right);
    Line::from(spans)
}
