//! My Issues: the grouped list and the focused issue's detail.

use std::collections::BTreeMap;

use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Paragraph};

use super::theme::Theme;
use super::{detail, icons, style};
use crate::app::{Arrangement, Group, GroupKey, IssueList, Names, Pane, Sort, State};
use crate::linear::model::{Issue, Priority, StateType, WorkflowState};

/// From `ui.wide-layout-columns` columns on, the detail shows beside the
/// list; below it the focused pane fills the body.
pub fn render(state: &State, frame: &mut Frame, area: Rect) {
    let my_issues = &state.my_issues;
    let view = View::new(state, my_issues.names());
    let (list, detail) = panes(state, area, my_issues.pane());
    if let Some((list_area, focus)) = list {
        render_list(&view, state, frame, list_area, focus);
    }
    if let Some((detail_area, focus)) = detail {
        render_detail(state, detail::issue(state), frame, detail_area, focus);
    }
}

/// Where the detail pane is in `area`, with the list's pane `pane`
/// focused, or `None` when only the list shows.
pub(super) fn detail_area(state: &State, area: Rect, pane: Pane) -> Option<Rect> {
    panes(state, area, pane).1.map(|(detail, _)| detail)
}

/// Where a pane goes and how it shows focus, or `None` when it does not show.
pub(super) type Placed = Option<(Rect, Focus)>;

/// Where the list and the detail go in `area` with `pane` focused, and how
/// each shows focus. On a narrow terminal only the focused one shows.
pub(super) fn panes(state: &State, area: Rect, pane: Pane) -> (Placed, Placed) {
    if area.width >= state.ui.wide_layout_columns {
        let [list_area, detail_area] =
            Layout::horizontal([Constraint::Fill(3), Constraint::Fill(2)]).areas(area);
        let focus = |of| {
            if pane == of {
                Focus::Focused
            } else {
                Focus::Unfocused
            }
        };
        (
            Some((list_area, focus(Pane::List))),
            Some((detail_area, focus(Pane::Detail))),
        )
    } else if pane == Pane::Detail {
        (None, Some((area, Focus::Alone)))
    } else {
        (Some((area, Focus::Alone)), None)
    }
}

/// What every issue row and detail draws from.
pub(super) struct View<'a> {
    names: &'a Names,
    theme: &'a Theme,
    icons: icons::IconSet,
    /// Why the last edit of an issue failed, by issue id.
    row_errors: &'a BTreeMap<String, String>,
}

impl<'a> View<'a> {
    pub(super) fn new(state: &'a State, names: &'a Names) -> Self {
        Self {
            names,
            theme: &state.theme,
            icons: state.ui.icons,
            row_errors: &state.row_errors,
        }
    }
}

/// How a pane shows focus: a thick accented border beside an unfocused pane,
/// a plain one when it is the only pane on screen.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Focus {
    Alone,
    Focused,
    Unfocused,
}

pub(super) fn pane_block(
    title: impl Into<Line<'static>>,
    focus: Focus,
    theme: &Theme,
) -> Block<'static> {
    let block = Block::bordered().title(title);
    match focus {
        Focus::Alone => block.border_style(style::border(theme)),
        Focus::Focused => block
            .border_type(BorderType::Thick)
            .border_style(theme.accent),
        Focus::Unfocused => block.border_style(theme.muted),
    }
}

enum Row<'a> {
    Header(&'a Group),
    Issue(usize, &'a Issue),
}

fn render_list(view: &View, state: &State, frame: &mut Frame, area: Rect, focus: Focus) {
    let my_issues = &state.my_issues;
    let title = list_title(
        "My Issues",
        my_issues.arrangement(),
        "active cycle, then priority",
    );
    let block = pane_block(title, focus, view.theme);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if my_issues.is_empty() {
        let empty = Paragraph::new("No open issues are assigned to you.").fg(view.theme.muted);
        frame.render_widget(empty, inner);
        return;
    }
    render_groups(view, frame, inner, my_issues, focus);
}

/// `My Issues · group: workflow state · sort: priority, urgent first`, with
/// `own` naming the list's own order.
pub(super) fn list_title(title: &str, arrangement: Arrangement, own: &str) -> String {
    let sort = arrangement.sort.map_or_else(|| own.to_owned(), Sort::label);
    format!(
        "{title} · group: {} · sort: {sort}",
        arrangement.group.label()
    )
}

/// The rows of `list` in `inner`: each group's header, when it has one,
/// over its issues, scrolled so the focused issue shows.
pub(super) fn render_groups(
    view: &View,
    frame: &mut Frame,
    inner: Rect,
    list: &IssueList,
    focus: Focus,
) {
    let mut rows = Vec::new();
    let mut index = 0;
    for group in list.groups() {
        if group.key.has_header() {
            rows.push(Row::Header(group));
        }
        for issue in &group.issues {
            rows.push(Row::Issue(index, issue));
            index += 1;
        }
    }
    let offset = visible_offset(list.scroll(), list.focused_line(), inner);
    let cells = row_cells(list.issues());

    for (line_area, row) in inner.rows().zip(rows.iter().skip(offset)) {
        match row {
            Row::Header(group) => {
                let mut header = group_name(view, &group.key);
                header.push_span(Span::raw(format!(" ({})", group.issues.len())));
                frame.render_widget(header, line_area);
            }
            Row::Issue(index, issue) => {
                let is_focused = *index == list.focused_index();
                render_row(view, frame, line_area, &cells, issue, is_focused, focus);
            }
        }
    }
}

/// What a group header calls its group, with the glyph of a state type or
/// priority.
fn group_name(view: &View, key: &GroupKey) -> Line<'static> {
    let names = view.names;
    let named = |id: &Option<String>, name: Option<&str>, none: &str| {
        let text = match (id, name) {
            (None, _) => none.to_owned(),
            (Some(_), Some(name)) => name.to_owned(),
            (Some(id), None) => id.clone(),
        };
        Line::from(Span::raw(text).bold())
    };
    match key {
        GroupKey::State(state_type) => Line::from(vec![
            state_glyph(view, *state_type),
            Span::raw(" "),
            Span::raw(icons::state_type_name(*state_type)).bold(),
        ]),
        GroupKey::Priority(priority) => Line::from(vec![
            Span::raw(view.icons.priority(*priority)).fg(view.theme.priority(*priority)),
            Span::raw(" "),
            Span::raw(icons::priority_name(*priority)).bold(),
        ]),
        GroupKey::Project(id) => named(
            id,
            id.as_deref().and_then(|id| names.project_name(id)),
            "No project",
        ),
        GroupKey::Cycle(id) => named(
            id,
            id.as_deref().and_then(|id| names.cycle_name(id)),
            "No cycle",
        ),
        GroupKey::Assignee(id) => named(
            id,
            id.as_deref().and_then(|id| names.member_name(id)),
            "Unassigned",
        ),
        GroupKey::All => Line::default(),
    }
}

/// The first list line to draw so the focused line shows in `inner`. The
/// kept scroll assumes the list height perform() was given; it is clamped so
/// the focused issue shows at any height.
pub(super) fn visible_offset(scroll: usize, focused_line: usize, inner: Rect) -> usize {
    let height = usize::from(inner.height);
    scroll
        .min(focused_line)
        .max((focused_line + 1).saturating_sub(height))
}

/// The columns of an issue row, with the identifier as wide as the widest
/// of `issues`.
pub(super) fn row_cells<'a>(issues: impl Iterator<Item = &'a Issue>) -> Layout {
    let id_width = issues
        .map(|issue| issue.identifier.chars().count())
        .max()
        .unwrap_or(0);
    Layout::horizontal([
        Constraint::Length(1),
        Constraint::Length(u16::try_from(id_width).unwrap_or(u16::MAX)),
        Constraint::Fill(1),
        Constraint::Length(13),
        Constraint::Length(10),
        Constraint::Length(2),
        Constraint::Length(8),
    ])
    .spacing(1)
}

/// One issue row: marker, identifier, title, state, priority, estimate and
/// cycle, highlighted when it is the focused issue.
pub(super) fn render_row(
    view: &View,
    frame: &mut Frame,
    line_area: Rect,
    cells: &Layout,
    issue: &Issue,
    is_focused: bool,
    focus: Focus,
) {
    let theme = view.theme;
    let [marker, identifier, title, state, priority, estimate, cycle] = cells.areas(line_area);
    if is_focused {
        frame.render_widget("▸", marker);
    }
    frame.render_widget(Span::raw(&issue.identifier).fg(theme.muted), identifier);
    let title_line = if view.row_errors.contains_key(&issue.id) {
        Line::from(vec![
            Span::styled("✘ ", style::error(theme)),
            Span::styled(issue.title.as_str(), style::error(theme)),
        ])
    } else {
        Line::from(issue.title.as_str())
    };
    frame.render_widget(title_line, title);
    frame.render_widget(state_line(view, &issue.state), state);
    frame.render_widget(priority_line(view, issue.priority), priority);
    frame.render_widget(
        Line::from(
            issue
                .estimate
                .filter(|_| view.names.uses_estimates(issue))
                .map(|points| points.to_string())
                .unwrap_or_default(),
        )
        .alignment(Alignment::Right),
        estimate,
    );
    frame.render_widget(cycle_span(view, issue), cycle);
    if is_focused {
        let highlight = if focus == Focus::Unfocused {
            Style::new().add_modifier(Modifier::BOLD)
        } else {
            theme.selection
        };
        frame.buffer_mut().set_style(line_area, highlight);
    }
}

/// The detail of `issue`, or an empty pane when there is none, scrolled as
/// far as the viewer scrolled it.
pub(super) fn render_detail(
    state: &State,
    issue: Option<&Issue>,
    frame: &mut Frame,
    area: Rect,
    focus: Focus,
) {
    let theme = &state.theme;
    let Some(issue) = issue else {
        frame.render_widget(pane_block("Detail", focus, theme), area);
        return;
    };
    let block = pane_block(issue.identifier.clone(), focus, theme);
    let inner = block.inner(area);
    let lines = detail::lines(state, issue, inner.width);
    // The kept scroll may be past the end after the detail got shorter.
    let max = lines.len().saturating_sub(usize::from(inner.height));
    let scroll = state.detail.scroll(&issue.id).min(max);
    let shown: Vec<Line> = lines.into_iter().skip(scroll).collect();
    frame.render_widget(Paragraph::new(shown).block(block), area);
}

fn state_glyph(view: &View, state_type: StateType) -> Span<'static> {
    Span::raw(view.icons.state(state_type)).fg(view.theme.state(state_type))
}

fn state_line<'a>(view: &View, state: &'a WorkflowState) -> Line<'a> {
    Line::from(vec![
        state_glyph(view, state.state_type),
        Span::raw(" "),
        Span::raw(state.name.as_str()),
    ])
}

fn priority_line(view: &View, priority: Priority) -> Line<'static> {
    Line::from(vec![
        Span::raw(view.icons.priority(priority)).fg(view.theme.priority(priority)),
        Span::raw(" "),
        Span::raw(icons::priority_name(priority)),
    ])
}

fn cycle_span<'a>(view: &View<'a>, issue: &'a Issue) -> Span<'a> {
    let names = view.names;
    let Some(id) = &issue.cycle_id else {
        return Span::default();
    };
    let name = Span::raw(names.cycle_name(id).unwrap_or(id));
    if names.is_in_active_cycle(issue) {
        name.fg(view.theme.accent).bold()
    } else {
        name
    }
}
