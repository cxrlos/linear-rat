//! An issue's detail: its title and state, the description as Markdown, the
//! comments, the workflow state history and the rest of what it refers to.
//! Built as lines already wrapped to the pane, so scrolling counts real rows.

use jiff::Timestamp;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use unicode_width::UnicodeWidthStr;

use super::theme::Theme;
use super::{icons, markdown, style};
use crate::app::{DetailLoad, State};
use crate::linear::model::{Estimation, Issue, StateSpan};

const FIELD_WIDTH: usize = 10;

type Pieces = Vec<(String, Style)>;

/// The issue whose detail is shown: the store's copy of the issue the
/// screen has focused, so it is the newest one loaded.
pub(super) fn issue(state: &State) -> Option<&Issue> {
    let focused = state.focused_issue()?;
    Some(state.store.issue(&focused.id).unwrap_or(focused))
}

/// Every line of `issue`'s detail, wrapped to `width` columns.
pub(super) fn lines(state: &State, issue: &Issue, width: u16) -> Vec<Line<'static>> {
    let theme = &state.theme;
    let store = &state.store;
    let now = store.as_of().unwrap_or(issue.updated_at);
    let bold = Style::new().add_modifier(Modifier::BOLD);
    let muted = style::muted(theme);

    let mut lines = markdown::wrap(&[(issue.title.clone(), bold)], width, &[], &[]);
    if let Some(error) = state.row_errors.get(&issue.id) {
        lines.extend(markdown::wrap(
            &[(format!("✘ Not saved: {error}"), style::error(theme))],
            width,
            &[],
            &[],
        ));
    }
    lines.push(Line::default());
    lines.extend(field(theme, "State", state_pieces(state, issue), width));
    let priority = issue.priority;
    lines.extend(field(
        theme,
        "Priority",
        vec![
            (
                state.ui.icons.priority(priority).to_owned(),
                Style::new().fg(theme.priority(priority)),
            ),
            (format!(" {}", icons::priority_name(priority)), Style::new()),
        ],
        width,
    ));
    let uses_estimates = store
        .team(&issue.team_id)
        .is_none_or(|team| team.estimation != Estimation::NotUsed);
    if uses_estimates {
        let estimate = issue
            .estimate
            .map_or("None".to_owned(), |points| points.to_string());
        lines.extend(field(theme, "Estimate", plain(estimate), width));
    }

    lines.push(Line::default());
    match issue.description.as_deref().map(str::trim) {
        Some(description) if !description.is_empty() => {
            lines.extend(markdown::render(description, width, theme));
        }
        _ => lines.push(Line::styled("No description.", muted)),
    }

    let load = state.detail.load(&issue.id);
    let comments = store.comments(&issue.id);
    lines.push(Line::default());
    let heading = match comments {
        Some(comments) => format!("Comments ({})", comments.len()),
        None => "Comments".to_owned(),
    };
    lines.push(Line::styled(heading, bold));
    lines.extend(load_note(theme, load, comments.is_some(), width));
    match comments {
        Some([]) => lines.push(Line::styled("No comments.", muted)),
        Some(comments) => {
            for (index, comment) in comments.iter().enumerate() {
                if index > 0 {
                    lines.push(Line::default());
                }
                let author = comment
                    .author_id
                    .as_deref()
                    .and_then(|id| store.member(id))
                    .map_or("Unknown", |member| member.name.as_str());
                lines.extend(markdown::wrap(
                    &[
                        (author.to_owned(), bold),
                        (format!(" · {}", ago(comment.created_at, now)), muted),
                    ],
                    width,
                    &[],
                    &[],
                ));
                let body = markdown::render(&comment.body, width.saturating_sub(2), theme);
                lines.extend(body.into_iter().map(|line| {
                    let mut spans = vec![Span::raw("  ")];
                    spans.extend(line.spans);
                    Line::from(spans)
                }));
            }
        }
        None => {}
    }

    let history = store.state_history(&issue.id);
    lines.push(Line::default());
    lines.push(Line::styled("History", bold));
    lines.extend(load_note(theme, load, history.is_some(), width));
    match history {
        Some([]) => lines.push(Line::styled("No state changes.", muted)),
        Some(spans) => lines.extend(history_lines(state, spans, now, width)),
        None => {}
    }

    lines.push(Line::default());
    lines.push(Line::styled("Details", bold));
    lines.extend(details(state, issue, now, width));
    lines
}

/// The line under a section that waits on the detail's load: why it did not
/// load, or that it is loading when nothing is there yet.
fn load_note(
    theme: &Theme,
    load: Option<&DetailLoad>,
    has_data: bool,
    width: u16,
) -> Vec<Line<'static>> {
    match load {
        Some(DetailLoad::Failed(error)) => markdown::wrap(
            &[(format!("Did not load: {error}"), style::error(theme))],
            width,
            &[],
            &[],
        ),
        _ if has_data => Vec::new(),
        _ => vec![Line::styled("Loading…", style::muted(theme))],
    }
}

fn history_lines(
    state: &State,
    spans: &[StateSpan],
    now: Timestamp,
    width: u16,
) -> Vec<Line<'static>> {
    let theme = &state.theme;
    let names: Vec<String> = spans
        .iter()
        .map(|span| {
            state.store.workflow_state(&span.state_id).map_or_else(
                || icons::state_type_name(span.state_type).to_owned(),
                |known| known.name.clone(),
            )
        })
        .collect();
    let name_width = names.iter().map(|name| name.width()).max().unwrap_or(0);
    let mut lines = Vec::new();
    for (span, name) in spans.iter().zip(names) {
        let ended = span.ended_at.unwrap_or(now);
        let lasted = duration(ended.as_second() - span.started_at.as_second());
        let lasted = if span.ended_at.is_some() {
            lasted
        } else {
            format!("{lasted} so far")
        };
        let padding = " ".repeat(name_width - name.width() + 2);
        let pieces = vec![
            (
                state.ui.icons.state(span.state_type).to_owned(),
                Style::new().fg(theme.state(span.state_type)),
            ),
            (format!(" {name}{padding}"), Style::new()),
            (lasted, style::muted(theme)),
        ];
        lines.extend(markdown::wrap(
            &pieces,
            width,
            &[],
            &[("  ".to_owned(), Style::new())],
        ));
    }
    lines
}

fn details(state: &State, issue: &Issue, now: Timestamp, width: u16) -> Vec<Line<'static>> {
    let theme = &state.theme;
    let store = &state.store;
    let muted = style::muted(theme);
    let name_of = |id: Option<&str>, none: &str| -> String {
        id.map_or(none.to_owned(), |id| {
            store
                .member(id)
                .map_or(id.to_owned(), |member| member.name.clone())
        })
    };

    let team = store
        .team(&issue.team_id)
        .map_or(issue.team_id.clone(), |team| team.name.clone());
    let project = match &issue.project_id {
        None => plain("None"),
        Some(id) => match (store.project(id), state.detail.load(&issue.id)) {
            (Some(project), _) => plain(project.name.clone()),
            (None, None | Some(DetailLoad::Loading)) => vec![("Loading…".to_owned(), muted)],
            (None, Some(_)) => plain(id.clone()),
        },
    };
    let cycle = match issue.cycle_id.as_deref() {
        None => plain("None"),
        Some(id) => match store.cycle(id) {
            Some(cycle) => {
                let name = cycle
                    .name
                    .clone()
                    .unwrap_or_else(|| format!("Cycle {}", cycle.number));
                if cycle.starts_at <= now && now < cycle.ends_at {
                    vec![
                        (name, Style::new().fg(theme.accent)),
                        (" (active)".to_owned(), Style::new()),
                    ]
                } else {
                    plain(name)
                }
            }
            None => plain(id),
        },
    };
    let labels: Vec<&str> = issue
        .label_ids
        .iter()
        .map(|id| {
            store
                .label(id)
                .map_or(id.as_str(), |label| label.name.as_str())
        })
        .collect();
    let labels = if labels.is_empty() {
        "None".to_owned()
    } else {
        labels.join(", ")
    };

    let mut lines = Vec::new();
    for (name, value) in [
        ("Team", plain(team)),
        ("Project", project),
        ("Cycle", cycle),
        ("Labels", plain(labels)),
        (
            "Assignee",
            plain(name_of(issue.assignee_id.as_deref(), "Unassigned")),
        ),
        (
            "Creator",
            plain(name_of(issue.creator_id.as_deref(), "Unknown")),
        ),
        ("Created", plain(when(issue.created_at, now))),
        ("Updated", plain(when(issue.updated_at, now))),
    ] {
        lines.extend(field(theme, name, value, width));
    }
    lines
}

fn state_pieces(state: &State, issue: &Issue) -> Pieces {
    let state_type = issue.state.state_type;
    vec![
        (
            state.ui.icons.state(state_type).to_owned(),
            Style::new().fg(state.theme.state(state_type)),
        ),
        (format!(" {}", issue.state.name), Style::new()),
    ]
}

fn plain(text: impl Into<String>) -> Pieces {
    vec![(text.into(), Style::new())]
}

/// `name` in the muted field column, then `value`, wrapped under itself.
fn field(theme: &Theme, name: &str, value: Pieces, width: u16) -> Vec<Line<'static>> {
    markdown::wrap(
        &value,
        width,
        &[(format!("{name:FIELD_WIDTH$}"), style::muted(theme))],
        &[(" ".repeat(FIELD_WIDTH), Style::new())],
    )
}

/// `at` in UTC and how long before `now` it was, as
/// `2026-10-12 14:03 UTC (3 weeks ago)`.
fn when(at: Timestamp, now: Timestamp) -> String {
    format!("{} ({})", at.strftime("%Y-%m-%d %H:%M UTC"), ago(at, now))
}

/// How long before `now` `then` was, in the largest whole unit.
fn ago(then: Timestamp, now: Timestamp) -> String {
    const MINUTE: i64 = 60;
    const HOUR: i64 = 60 * MINUTE;
    const DAY: i64 = 24 * HOUR;
    let seconds = now.as_second() - then.as_second();
    let (count, unit) = match seconds {
        _ if seconds < MINUTE => return "just now".to_owned(),
        _ if seconds < HOUR => (seconds / MINUTE, "minute"),
        _ if seconds < DAY => (seconds / HOUR, "hour"),
        _ if seconds < 14 * DAY => (seconds / DAY, "day"),
        _ if seconds < 60 * DAY => (seconds / (7 * DAY), "week"),
        _ if seconds < 365 * DAY => (seconds / (30 * DAY), "month"),
        _ => (seconds / (365 * DAY), "year"),
    };
    let plural = if count == 1 { "" } else { "s" };
    format!("{count} {unit}{plural} ago")
}

/// `seconds` in its two largest units, as `2d 4h`, `5h 12m` or `12m`.
fn duration(seconds: i64) -> String {
    let minutes = seconds.max(0) / 60;
    let (days, hours, minutes) = (minutes / (24 * 60), minutes / 60 % 24, minutes % 60);
    match (days, hours, minutes) {
        (0, 0, 0) => "<1m".to_owned(),
        (0, 0, m) => format!("{m}m"),
        (0, h, 0) => format!("{h}h"),
        (0, h, m) => format!("{h}h {m}m"),
        (d, 0, _) => format!("{d}d"),
        (d, h, _) => format!("{d}d {h}h"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: Timestamp = Timestamp::constant(1_793_631_600, 0);

    fn before(seconds: i64) -> Timestamp {
        Timestamp::constant(1_793_631_600 - seconds, 0)
    }

    #[test]
    fn ago_names_the_largest_whole_unit() {
        let cases = [
            (30, "just now"),
            (60, "1 minute ago"),
            (5 * 60, "5 minutes ago"),
            (3 * 3600, "3 hours ago"),
            (86_400, "1 day ago"),
            (20 * 86_400, "2 weeks ago"),
            (90 * 86_400, "3 months ago"),
            (800 * 86_400, "2 years ago"),
        ];
        for (seconds, expected) in cases {
            assert_eq!(ago(before(seconds), NOW), expected, "{seconds}s");
        }
    }

    #[test]
    fn a_time_after_now_is_just_now() {
        assert_eq!(ago(before(-600), NOW), "just now");
    }

    #[test]
    fn durations_keep_the_two_largest_units() {
        assert_eq!(duration(20), "<1m");
        assert_eq!(duration(12 * 60), "12m");
        assert_eq!(duration(3 * 3600), "3h");
        assert_eq!(duration(5 * 3600 + 12 * 60), "5h 12m");
        assert_eq!(duration(2 * 86_400 + 4 * 3600 + 59), "2d 4h");
        assert_eq!(duration(3 * 86_400 + 30 * 60), "3d");
    }
}
