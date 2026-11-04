//! A picker: the filter typed so far, the entries it leaves with the number
//! that picks each of the first nine, and the keys that choose and close.

use ratatui::Frame;
use ratatui::layout::{Margin, Rect};
use ratatui::style::Stylize;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Padding, Paragraph};

use super::style;
use crate::app::{Action, Edit, EditPicker, Note, Picker, State};
use crate::keys::Context;

/// The width the list asks for at least, so short entries leave room for
/// the title and the keys at the bottom.
const MIN_WIDTH: usize = 34;

/// Draws an edit picker centered inside `area`'s border.
pub fn render_edit(state: &State, picker: &EditPicker, frame: &mut Frame, area: Rect) {
    let glyph = |edit: &Edit| match edit {
        Edit::State(workflow_state) => {
            let state_type = workflow_state.state_type;
            Some(Span::raw(state.ui.icons.state(state_type)).fg(state.theme.state(state_type)))
        }
        Edit::Priority(priority) => {
            Some(Span::raw(state.ui.icons.priority(*priority)).fg(state.theme.priority(*priority)))
        }
        Edit::Assignee(_) => None,
    };
    render(state, &picker.title(), &picker.list, glyph, frame, area);
}

/// Draws `picker` titled `title`, each entry led by its number key and the
/// glyph `glyph` gives its value, if any.
fn render<T>(
    state: &State,
    title: &str,
    picker: &Picker<T>,
    glyph: impl Fn(&T) -> Option<Span<'static>>,
    frame: &mut Frame,
    area: Rect,
) {
    let theme = &state.theme;
    let area = area.inner(Margin::new(1, 1));
    let number_keys: Vec<String> = (1..=9)
        .map(|n| {
            state
                .keymap
                .keys_for(Action::PickNumber(n), &[Context::Overlay])
                .first()
                .map(ToString::to_string)
                .unwrap_or_default()
        })
        .collect();
    let key_width = number_keys
        .iter()
        .map(|key| key.chars().count())
        .max()
        .unwrap_or(0);

    let query = if picker.query().is_empty() {
        Line::from(vec![
            Span::styled("> ", style::accent(theme)),
            Span::styled("type to filter", style::muted(theme)),
        ])
    } else {
        Line::from(vec![
            Span::styled("> ", style::accent(theme)),
            Span::raw(picker.query().to_owned()),
        ])
    };

    let visible = picker.visible();
    let selected = picker.selected_position();
    let mut rows: Vec<Line> = visible
        .iter()
        .enumerate()
        .map(|(position, (index, entry))| {
            let key = number_keys.get(position).cloned().unwrap_or_default();
            let mut spans = vec![Span::styled(
                format!("{key:<key_width$} "),
                style::accent(theme),
            )];
            if let Some(glyph) = glyph(&entry.value) {
                spans.push(glyph);
                spans.push(Span::raw(" "));
            }
            spans.push(Span::raw(entry.label.clone()));
            if picker.is_current(*index) {
                spans.push(Span::styled(" ✓", style::accent(theme)));
            }
            let line = Line::from(spans);
            if selected == Some(position) {
                line.style(theme.selection)
            } else {
                line
            }
        })
        .collect();
    let note = match picker.note() {
        Some(Note::Loading) => Some(Line::styled("Loading…", style::muted(theme))),
        Some(Note::Failed(error)) => Some(Line::styled(
            format!("Could not load: {error}"),
            style::error(theme),
        )),
        None if visible.is_empty() => Some(Line::styled("No matches", style::muted(theme))),
        None => None,
    };

    let bottom = bottom_keys(state);
    let width = rows
        .iter()
        .chain(note.iter())
        .chain([&query])
        .map(Line::width)
        .chain([title.chars().count() + 2, bottom.width(), MIN_WIDTH])
        .max()
        .unwrap_or(0)
        + 4;
    let width = u16::try_from(width).unwrap_or(u16::MAX);
    let wanted = rows.len() + usize::from(note.is_some()) + 3;
    let popup = super::centered(area, width, u16::try_from(wanted).unwrap_or(u16::MAX));

    // Rows for entries once the border, the filter and any note are drawn.
    let room = usize::from(popup.height)
        .saturating_sub(3 + usize::from(note.is_some()))
        .max(1);
    let offset = selected.map_or(0, |position| (position + 1).saturating_sub(room));
    let mut lines = vec![query];
    lines.extend(rows.drain(..).skip(offset).take(room));
    lines.extend(note);

    let block = super::popup_block(state)
        .title(format!(" {title} "))
        .title_bottom(bottom);
    frame.render_widget(Clear, popup);
    frame.render_widget(
        Paragraph::new(lines).block(block.padding(Padding::horizontal(1))),
        popup,
    );
}

/// The keys that select, choose and close, as
/// ` Down/Up select  Enter choose  Esc close `.
fn bottom_keys(state: &State) -> Line<'static> {
    let items: [(&[Action], &str); 3] = [
        (&[Action::SelectNext, Action::SelectPrevious], "select"),
        (&[Action::Choose], "choose"),
        (&[Action::Close], "close"),
    ];
    let mut spans = Vec::new();
    for (actions, word) in items {
        let keys: Vec<String> = actions
            .iter()
            .filter_map(|&action| {
                state
                    .keymap
                    .keys_for(action, &[Context::Overlay])
                    .first()
                    .map(ToString::to_string)
            })
            .collect();
        if keys.is_empty() {
            continue;
        }
        spans.push(Span::raw(" "));
        spans.push(Span::styled(keys.join("/"), style::accent(&state.theme)));
        spans.push(Span::raw(format!(" {word} ")));
    }
    Line::from(spans)
}
