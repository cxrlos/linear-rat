//! A picker: the filter typed so far, the entries it leaves with the number
//! that picks each of the first nine, and the keys that choose and close.

use ratatui::Frame;
use ratatui::layout::{Margin, Rect};
use ratatui::style::Stylize;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Padding, Paragraph};

use super::style;
use crate::app::{Action, Choice, CreateForm, Edit, EditPicker, Links, Note, Picker, State, Step};
use crate::keys::Context;

/// The width the list asks for at least, so short entries leave room for
/// the title and the keys at the bottom.
const MIN_WIDTH: usize = 34;

/// Draws an edit picker centered inside `area`'s border.
pub fn render_edit(state: &State, picker: &EditPicker, frame: &mut Frame, area: Rect) {
    render(
        state,
        &picker.title(),
        &picker.list,
        edit_glyph(state),
        frame,
        area,
    );
}

/// Draws the create form's step centered inside `area`'s border: the
/// picker of the field it is on, or the title being typed.
pub fn render_create(state: &State, form: &CreateForm, frame: &mut Frame, area: Rect) {
    let heading = form.heading();
    match &form.step {
        Step::Team(picker) => render(state, &heading, picker, |_| None, frame, area),
        Step::Field(_, picker) => render(state, &heading, picker, edit_glyph(state), frame, area),
        Step::Title => render_title(state, form, &heading, frame, area),
    }
}

/// The title step: what the form chose, and the title typed so far.
fn render_title(state: &State, form: &CreateForm, heading: &str, frame: &mut Frame, area: Rect) {
    let theme = &state.theme;
    let area = area.inner(Margin::new(1, 1));
    let mut chosen = Vec::new();
    if let Some(workflow_state) = &form.state {
        chosen.extend(edit_glyph(state)(&Edit::State(workflow_state.clone())));
        chosen.push(Span::raw(format!(" {}", workflow_state.name)));
    }
    if let Some(assignee) = &form.assignee {
        let name = assignee
            .as_deref()
            .and_then(|id| state.store.member(id))
            .map_or("Unassigned", |member| member.name.as_str());
        chosen.push(Span::styled(" · ", style::muted(theme)));
        chosen.push(Span::raw(name.to_owned()));
    }
    if let Some(priority) = form.priority {
        chosen.push(Span::styled(" · ", style::muted(theme)));
        chosen.extend(edit_glyph(state)(&Edit::Priority(priority)));
        chosen.push(Span::raw(format!(
            " {}",
            super::icons::priority_name(priority)
        )));
    }
    let title = if form.title.is_empty() {
        Line::from(vec![
            Span::styled("> ", style::accent(theme)),
            Span::styled("type the title", style::muted(theme)),
        ])
    } else {
        Line::from(vec![
            Span::styled("> ", style::accent(theme)),
            Span::raw(form.title.clone()),
        ])
    };
    let note = Line::styled(
        "The description is written in $EDITOR next.",
        style::muted(theme),
    );
    let lines = vec![Line::from(chosen), title, note];
    let bottom = keys_line(
        state,
        &[(&[Action::Accept], "describe"), (&[Action::Close], "close")],
    );
    let width = lines
        .iter()
        .map(Line::width)
        .chain([heading.chars().count() + 2, bottom.width(), MIN_WIDTH])
        .max()
        .unwrap_or(0)
        + 4;
    let width = u16::try_from(width).unwrap_or(u16::MAX);
    let popup = super::centered(area, width, 5);
    let block = super::popup_block(state)
        .title(format!(" {heading} "))
        .title_bottom(bottom);
    frame.render_widget(Clear, popup);
    frame.render_widget(
        Paragraph::new(lines).block(block.padding(Padding::horizontal(1))),
        popup,
    );
}

/// The glyph a picker shows before a state or priority.
fn edit_glyph(state: &State) -> impl Fn(&Edit) -> Option<Span<'static>> + '_ {
    |edit: &Edit| match edit {
        Edit::State(workflow_state) => {
            let state_type = workflow_state.state_type;
            Some(Span::raw(state.ui.icons.state(state_type)).fg(state.theme.state(state_type)))
        }
        Edit::Priority(priority) => {
            Some(Span::raw(state.ui.icons.priority(*priority)).fg(state.theme.priority(*priority)))
        }
        Edit::Assignee(_) | Edit::Labels(_) | Edit::Estimate(_) => None,
    }
}

/// Draws a picker of an issue's links centered inside `area`'s border.
pub fn render_links(state: &State, links: &Links, frame: &mut Frame, area: Rect) {
    render(state, &links.title(), &links.list, |_| None, frame, area);
}

/// Draws a team switcher or filter centered inside `area`'s border.
pub fn render_choice(state: &State, choice: &Choice, frame: &mut Frame, area: Rect) {
    render(state, &choice.title(), &choice.list, |_| None, frame, area);
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
            if picker.is_multi() {
                let mark = if picker.is_marked(*index) {
                    "[x] "
                } else {
                    "[ ] "
                };
                spans.push(Span::styled(mark, style::accent(theme)));
            }
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

    let bottom = bottom_keys(state, picker.is_multi());
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
/// ` Down/Up select  Enter choose  Esc close `, with the key that toggles an
/// entry in a multi-select picker. Keys the filter takes as text are left
/// out.
fn bottom_keys(state: &State, multi: bool) -> Line<'static> {
    let items: &[(&[Action], &str)] = if multi {
        &[
            (&[Action::ScrollDown, Action::ScrollUp], "select"),
            (&[Action::ToggleEntry], "toggle"),
            (&[Action::Accept], "save"),
            (&[Action::Close], "close"),
        ]
    } else {
        &[
            (&[Action::ScrollDown, Action::ScrollUp], "select"),
            (&[Action::Accept], "choose"),
            (&[Action::Close], "close"),
        ]
    };
    keys_line(state, items)
}

/// Each group of `items` as its keys and word, leaving out keys the text
/// field takes.
fn keys_line(state: &State, items: &[(&[Action], &str)]) -> Line<'static> {
    let mut spans = Vec::new();
    for &(actions, word) in items {
        let keys: Vec<String> = actions
            .iter()
            .filter_map(|&action| {
                state
                    .keymap
                    .keys_for(action, &[Context::Overlay])
                    .into_iter()
                    .find(|keys| keys.keys().first().is_none_or(|key| key.typed().is_none()))
                    .map(|keys| keys.to_string())
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
