//! My Issues: the grouped list and the focused issue's detail.

use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Paragraph, Wrap};

use super::icons;
use crate::app::{Group, MyIssues, Pane};
use crate::linear::model::{Issue, Priority, StateType, WorkflowState};

/// From this many columns on, the detail shows beside the list; below it the
/// focused pane fills the body.
pub const WIDE_MIN_COLUMNS: u16 = 120;

const FIELD_WIDTH: usize = 10;

pub fn render(my_issues: &MyIssues, frame: &mut Frame, area: Rect) {
    let pane = my_issues.pane();
    if area.width >= WIDE_MIN_COLUMNS {
        let [list_area, detail_area] =
            Layout::horizontal([Constraint::Fill(3), Constraint::Fill(2)]).areas(area);
        let focus = |of| {
            if pane == of {
                Focus::Focused
            } else {
                Focus::Unfocused
            }
        };
        render_list(my_issues, frame, list_area, focus(Pane::List));
        render_detail(my_issues, frame, detail_area, focus(Pane::Detail));
    } else if pane == Pane::Detail {
        render_detail(my_issues, frame, area, Focus::Alone);
    } else {
        render_list(my_issues, frame, area, Focus::Alone);
    }
}

/// How a pane shows focus: a thick accented border beside an unfocused pane,
/// a plain one when it is the only pane on screen.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Focus {
    Alone,
    Focused,
    Unfocused,
}

fn pane_block(title: impl Into<Line<'static>>, focus: Focus) -> Block<'static> {
    let block = Block::bordered().title(title);
    match focus {
        Focus::Alone => block,
        Focus::Focused => block
            .border_type(BorderType::Thick)
            .border_style(icons::ACCENT),
        Focus::Unfocused => block.border_style(icons::MUTED),
    }
}

enum Row<'a> {
    Header(&'a Group),
    Issue(usize, &'a Issue),
}

fn render_list(my_issues: &MyIssues, frame: &mut Frame, area: Rect, focus: Focus) {
    let block = pane_block("My Issues", focus);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if my_issues.groups().is_empty() {
        let empty = Paragraph::new("No open issues are assigned to you.").fg(icons::MUTED);
        frame.render_widget(empty, inner);
        return;
    }

    let mut rows = Vec::new();
    let mut index = 0;
    for group in my_issues.groups() {
        rows.push(Row::Header(group));
        for issue in &group.issues {
            rows.push(Row::Issue(index, issue));
            index += 1;
        }
    }
    // The kept scroll assumes the list height perform() was given; clamp it so
    // the focused issue shows at any height.
    let focused_line = my_issues.focused_line();
    let height = usize::from(inner.height);
    let offset = my_issues
        .scroll()
        .min(focused_line)
        .max((focused_line + 1).saturating_sub(height));

    let id_width = my_issues
        .groups()
        .iter()
        .flat_map(|group| &group.issues)
        .map(|issue| issue.identifier.chars().count())
        .max()
        .unwrap_or(0);
    let cells = Layout::horizontal([
        Constraint::Length(1),
        Constraint::Length(u16::try_from(id_width).unwrap_or(u16::MAX)),
        Constraint::Fill(1),
        Constraint::Length(13),
        Constraint::Length(10),
        Constraint::Length(2),
        Constraint::Length(8),
    ])
    .spacing(1);

    for (line_area, row) in inner.rows().zip(rows.iter().skip(offset)) {
        match row {
            Row::Header(group) => {
                let header = Line::from(vec![
                    state_glyph(group.state_type),
                    Span::raw(" "),
                    Span::raw(icons::state_type_name(group.state_type)).bold(),
                    Span::raw(format!(" ({})", group.issues.len())),
                ]);
                frame.render_widget(header, line_area);
            }
            Row::Issue(index, issue) => {
                let is_focused = *index == my_issues.focused_index();
                let [marker, identifier, title, state, priority, estimate, cycle] =
                    cells.areas(line_area);
                if is_focused {
                    frame.render_widget("▸", marker);
                }
                frame.render_widget(Span::raw(&issue.identifier).fg(icons::MUTED), identifier);
                frame.render_widget(issue.title.as_str(), title);
                frame.render_widget(state_line(&issue.state), state);
                frame.render_widget(priority_line(issue.priority), priority);
                frame.render_widget(
                    Line::from(
                        issue
                            .estimate
                            .map(|points| points.to_string())
                            .unwrap_or_default(),
                    )
                    .alignment(Alignment::Right),
                    estimate,
                );
                frame.render_widget(cycle_span(my_issues, issue), cycle);
                if is_focused {
                    let highlight = if focus == Focus::Unfocused {
                        Style::new().add_modifier(Modifier::BOLD)
                    } else {
                        Style::new().add_modifier(Modifier::REVERSED)
                    };
                    frame.buffer_mut().set_style(line_area, highlight);
                }
            }
        }
    }
}

fn render_detail(my_issues: &MyIssues, frame: &mut Frame, area: Rect, focus: Focus) {
    let Some(issue) = my_issues.focused_issue() else {
        frame.render_widget(pane_block("Detail", focus), area);
        return;
    };
    let block = pane_block(issue.identifier.clone(), focus);

    let cycle = match &issue.cycle_id {
        Some(id) => {
            let name = my_issues.cycle_name(id).unwrap_or(id).to_owned();
            if my_issues.is_in_active_cycle(issue) {
                format!("{name} (active)")
            } else {
                name
            }
        }
        None => "None".to_owned(),
    };
    let labels: Vec<&str> = issue
        .label_ids
        .iter()
        .map(|id| my_issues.label_name(id).unwrap_or(id))
        .collect();
    let assignee = issue
        .assignee_id
        .as_deref()
        .map(|id| my_issues.member_name(id).unwrap_or(id))
        .unwrap_or("Unassigned");

    let mut lines = vec![
        Line::from(issue.title.as_str()).bold(),
        Line::default(),
        field("State", state_line(&issue.state)),
        field("Priority", priority_line(issue.priority)),
    ];
    if my_issues.uses_estimates(issue) {
        let estimate = issue
            .estimate
            .map_or("None".to_owned(), |points| points.to_string());
        lines.push(field("Estimate", Line::from(estimate)));
    }
    lines.extend([
        field("Cycle", Line::from(cycle)),
        field(
            "Labels",
            Line::from(if labels.is_empty() {
                "None".to_owned()
            } else {
                labels.join(", ")
            }),
        ),
        field("Assignee", Line::from(assignee)),
        Line::default(),
    ]);
    match &issue.description {
        Some(description) => lines.extend(description.lines().map(Line::from)),
        None => lines.push(Line::from("No description.").fg(icons::MUTED)),
    }
    let detail = Paragraph::new(lines)
        .wrap(Wrap { trim: false })
        .block(block);
    frame.render_widget(detail, area);
}

fn field<'a>(name: &'a str, value: Line<'a>) -> Line<'a> {
    let mut spans = vec![Span::raw(format!("{name:FIELD_WIDTH$}")).fg(icons::MUTED)];
    spans.extend(value.spans);
    Line::from(spans)
}

fn state_glyph(state_type: StateType) -> Span<'static> {
    Span::raw(icons::state_glyph(state_type)).fg(icons::state_color(state_type))
}

fn state_line(state: &WorkflowState) -> Line<'_> {
    Line::from(vec![
        state_glyph(state.state_type),
        Span::raw(" "),
        Span::raw(state.name.as_str()),
    ])
}

fn priority_line(priority: Priority) -> Line<'static> {
    Line::from(vec![
        Span::raw(icons::priority_glyph(priority)).fg(icons::priority_color(priority)),
        Span::raw(" "),
        Span::raw(icons::priority_name(priority)),
    ])
}

fn cycle_span<'a>(my_issues: &'a MyIssues, issue: &'a Issue) -> Span<'a> {
    let Some(id) = &issue.cycle_id else {
        return Span::default();
    };
    let name = Span::raw(my_issues.cycle_name(id).unwrap_or(id));
    if my_issues.is_in_active_cycle(issue) {
        name.fg(icons::ACCENT).bold()
    } else {
        name
    }
}
