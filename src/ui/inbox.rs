//! The Inbox screen: the viewer's issue notifications grouped under their
//! issue, unread ones accented and read ones muted, the focused one
//! highlighted.

use jiff::Timestamp;
use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::Stylize;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Wrap};

use super::my_issues::{self, Focus};
use super::{style, text};
use crate::app::{Progress, State, inbox_groups, unread_count};
use crate::linear::model::Notification;

/// One line of the list: an issue's heading, or one of its notifications.
enum Row<'a> {
    Issue(&'a str),
    Notification(usize, &'a Notification),
}

pub fn render(state: &State, frame: &mut Frame, area: Rect) {
    let unread = unread_count(state);
    let title = match unread {
        0 => "Inbox".to_owned(),
        n => format!("Inbox · {n} unread"),
    };
    let block = my_issues::pane_block(title, area, Focus::Alone, &state.theme);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let now = state.now();
    let groups = inbox_groups(&state.store, now);
    if groups.is_empty() {
        let empty = match state.inbox.progress() {
            Progress::Failed(error) => Paragraph::new(error.clone())
                .wrap(Wrap { trim: true })
                .style(style::error(&state.theme)),
            Progress::Done => muted(state, "Nothing in your inbox."),
            Progress::NotAsked | Progress::Loading => muted(state, "Loading your inbox…"),
        };
        frame.render_widget(empty, inner);
        return;
    }

    let mut rows = Vec::new();
    let mut index = 0;
    for group in &groups {
        rows.push(Row::Issue(group.issue_id));
        for notification in &group.notifications {
            rows.push(Row::Notification(index, notification));
            index += 1;
        }
    }
    let shown: Vec<&Notification> = groups
        .iter()
        .flat_map(|group| group.notifications.iter().copied())
        .collect();
    let focused = state.inbox.focused_index(&shown);
    let focused_line = rows
        .iter()
        .position(|row| matches!(row, Row::Notification(at, _) if *at == focused))
        .unwrap_or(0);
    let offset = (focused_line + 1).saturating_sub(usize::from(inner.height));

    let cells = Layout::horizontal([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(14),
        Constraint::Fill(1),
        Constraint::Length(4),
    ])
    .spacing(1);
    for (line_area, row) in inner.rows().zip(rows.iter().skip(offset)) {
        match row {
            Row::Issue(issue_id) => text::draw(frame, issue_heading(state, issue_id), line_area),
            Row::Notification(at, notification) => {
                let [marker, dot, kind, actor, age_area] = cells.areas(line_area);
                let is_focused = *at == focused;
                let unread = notification.read_at.is_none();
                let tone = if unread {
                    style::accent(&state.theme)
                } else {
                    style::muted(&state.theme)
                };
                if is_focused {
                    frame.render_widget("▸", marker);
                }
                let mark = if unread { "●" } else { "·" };
                frame.render_widget(Span::styled(mark, tone), dot);
                text::draw(frame, Span::styled(kind_label(notification), tone), kind);
                text::draw(
                    frame,
                    Span::styled(actor_name(state, notification), tone),
                    actor,
                );
                text::draw(
                    frame,
                    Line::from(Span::styled(age(now, notification.created_at), tone))
                        .alignment(Alignment::Right),
                    age_area,
                );
                if is_focused {
                    frame
                        .buffer_mut()
                        .set_style(line_area, state.theme.selection);
                }
            }
        }
    }
}

/// `ENG-12 Rind sensor drifts`, the key bold.
fn issue_heading(state: &State, issue_id: &str) -> Line<'static> {
    match state.store.issue(issue_id) {
        Some(issue) => Line::from(vec![
            Span::raw(issue.identifier.clone()).bold(),
            Span::raw(" "),
            Span::raw(issue.title.clone()),
        ]),
        None => Line::from(Span::raw("An issue not loaded").fg(state.theme.muted)),
    }
}

/// What happened, from Linear's `type`: comment, mention, assigned, state
/// change or reaction, with the emoji of a reaction.
fn kind_label(notification: &Notification) -> String {
    let kind = notification.kind.as_str();
    if kind.contains("Reaction") {
        return match &notification.reaction_emoji {
            Some(emoji) => format!("reaction {emoji}"),
            None => "reaction".to_owned(),
        };
    }
    let label = if kind.contains("Mention") {
        "mention"
    } else if kind.contains("Comment") {
        "comment"
    } else if kind.contains("Unassigned") {
        "unassigned"
    } else if kind.contains("Assigned") {
        "assigned"
    } else if kind.contains("StatusChanged") {
        "state change"
    } else {
        "update"
    };
    label.to_owned()
}

/// Who did it, by name, or `Linear` when no person did.
fn actor_name(state: &State, notification: &Notification) -> String {
    notification
        .actor_id
        .as_deref()
        .and_then(|id| state.store.member(id))
        .map_or_else(|| "Linear".to_owned(), |member| member.name.clone())
}

/// How long ago `then` was, as `12m`, `5h` or `2d`.
fn age(now: Timestamp, then: Timestamp) -> String {
    let seconds = now.duration_since(then).as_secs().max(0);
    match seconds {
        0..60 => "now".to_owned(),
        60..3600 => format!("{}m", seconds / 60),
        3600..86_400 => format!("{}h", seconds / 3600),
        _ => format!("{}d", seconds / 86_400),
    }
}

fn muted(state: &State, text: &str) -> Paragraph<'static> {
    Paragraph::new(text.to_owned())
        .wrap(Wrap { trim: true })
        .fg(state.theme.muted)
}
