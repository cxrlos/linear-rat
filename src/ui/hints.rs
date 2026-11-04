//! The bottom bar: the keys for the actions of the active contexts, in two
//! labelled rows (moves, then everything else).

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use super::style;
use crate::app::{Modal, State};
use crate::keys::{self, ACTIONS, HintRow, KeySequence};

/// How many rows the bar takes.
pub const ROWS: u16 = 2;

/// One item of a row: the keys that run it, its word, and whether any of
/// its actions can run now.
struct Item {
    keys: String,
    word: &'static str,
    available: bool,
}

impl Item {
    fn text(&self) -> String {
        format!("{} {}", self.keys, self.word)
    }
}

pub fn render(state: &State, frame: &mut Frame, area: Rect) {
    let rows: Vec<(HintRow, Vec<Item>)> = HintRow::ALL
        .into_iter()
        .map(|row| (row, items(state, row)))
        .collect();
    let label_width = HintRow::ALL
        .iter()
        .map(|row| row.label().len())
        .max()
        .unwrap_or(0);

    let columns = column_widths(&rows);
    let grid_width = 1 + label_width + columns.iter().map(|width| width + 3).sum::<usize>();
    let grid = grid_width <= usize::from(area.width);

    let lines: Vec<Line> = rows
        .iter()
        .map(|(row, items)| line(state, *row, items, label_width, grid.then_some(&columns)))
        .collect();
    frame.render_widget(Paragraph::new(lines), area);
}

/// The items of `row` for the active contexts, merged by word in registry
/// order. Actions without a live key are left out, and so are keys a text
/// field takes while it has focus.
fn items(state: &State, row: HintRow) -> Vec<Item> {
    let contexts = keys::active_contexts(state);
    let typing = state.modals.top().is_some_and(Modal::takes_text);
    let mut groups: Vec<(&'static str, Vec<KeySequence>, bool)> = Vec::new();
    let mut shown = Vec::new();
    for &context in &contexts {
        for spec in ACTIONS.iter().filter(|s| s.contexts.contains(&context)) {
            let Some(hint) = spec.hint.filter(|hint| hint.row == row) else {
                continue;
            };
            if shown.contains(&spec.action) {
                continue;
            }
            let Some(key) = state
                .keymap
                .keys_for(spec.action, &contexts)
                .into_iter()
                .find(|keys| {
                    !typing || keys.keys().first().is_none_or(|key| key.typed().is_none())
                })
            else {
                continue;
            };
            shown.push(spec.action);
            let available = (spec.available)(state);
            match groups.iter_mut().find(|(word, ..)| *word == hint.word) {
                Some((_, keys, any_available)) => {
                    keys.push(key);
                    *any_available |= available;
                }
                None => groups.push((hint.word, vec![key], available)),
            }
        }
    }
    groups
        .into_iter()
        .map(|(word, keys, available)| Item {
            keys: keys_label(&keys),
            word,
            available,
        })
        .collect()
}

/// The keys of a merged item: the chord prefix they all start with, as `y`
/// for `yi`, `yu`, `yb` and `ym`, or else each key joined with `/`.
fn keys_label(keys: &[KeySequence]) -> String {
    let first = keys.first().and_then(|keys| keys.keys().first());
    let shared_prefix = keys.len() > 1
        && keys
            .iter()
            .all(|sequence| sequence.keys().len() > 1 && sequence.keys().first() == first);
    match first {
        Some(prefix) if shared_prefix => prefix.to_string(),
        _ => keys
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("/"),
    }
}

/// The width of each column when the rows line up: the widest item at that
/// position in any row.
fn column_widths(rows: &[(HintRow, Vec<Item>)]) -> Vec<usize> {
    let mut widths: Vec<usize> = Vec::new();
    for (_, items) in rows {
        for (index, item) in items.iter().enumerate() {
            let width = item.text().chars().count();
            match widths.get_mut(index) {
                Some(widest) => *widest = (*widest).max(width),
                None => widths.push(width),
            }
        }
    }
    widths
}

/// One row: its label, a divider, then its items, either in `columns` with
/// a divider between each or two spaces apart.
fn line(
    state: &State,
    row: HintRow,
    items: &[Item],
    label_width: usize,
    columns: Option<&Vec<usize>>,
) -> Line<'static> {
    let theme = &state.theme;
    let divider = || Span::styled(" │ ", style::border(theme));
    if items.is_empty() {
        return Line::default();
    }
    let mut spans = vec![
        Span::raw(" "),
        Span::styled(
            format!("{:<label_width$}", row.label()),
            style::muted(theme),
        ),
        divider(),
    ];
    for (index, item) in items.iter().enumerate() {
        if index > 0 {
            spans.push(match columns {
                Some(_) => divider(),
                None => Span::raw("  "),
            });
        }
        let (key_style, word_style) = if item.available {
            (style::accent(theme), Style::new())
        } else {
            (style::muted(theme), style::muted(theme))
        };
        spans.push(Span::styled(item.keys.clone(), key_style));
        let padding = columns
            .and_then(|widths| widths.get(index))
            .map_or(0, |width| width.saturating_sub(item.text().chars().count()));
        spans.push(Span::styled(
            format!(" {}{}", item.word, " ".repeat(padding)),
            word_style,
        ));
    }
    Line::from(spans)
}
