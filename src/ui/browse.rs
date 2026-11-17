//! The Cycles and Projects screens: one row per cycle or project, the
//! focused one highlighted, Enter listing its issues.

use jiff::Timestamp;
use jiff::civil::Date;
use jiff::tz::TimeZone;
use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::Stylize;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Wrap};

use super::my_issues::{self, Focus};
use super::style;
use crate::app::{Action, Progress, State, cycle_counts, cycle_title};
use crate::keys;
use crate::linear::model::{Cycle, ProjectOverview, ProjectStatusType, StateType};

/// How many rows the Cycles header takes above the list: one once a team
/// shows, none before.
pub fn cycles_header_rows(state: &State) -> u16 {
    u16::from(state.cycles.team_id().is_some())
}

pub fn render_cycles(state: &State, frame: &mut Frame, area: Rect) {
    let rows = cycles_header_rows(state);
    let [header_area, list_area] =
        Layout::vertical([Constraint::Length(rows), Constraint::Fill(1)]).areas(area);
    frame.render_widget(cycles_header(state), header_area);

    let cycles = &state.cycles;
    let title = opening_title(
        "Cycles",
        cycles.opening().and_then(|id| {
            cycles
                .cycles()
                .iter()
                .find(|cycle| cycle.id == id)
                .map(cycle_title)
        }),
    );
    let block = my_issues::pane_block(title, Focus::Alone, &state.theme);
    let inner = block.inner(list_area);
    frame.render_widget(block, list_area);

    let teams = state.team_issues.teams_progress();
    let empty = match (cycles.team_id(), cycles.progress()) {
        (None, _) => match teams {
            Progress::Failed(error) => Some(failed(state, error)),
            Progress::Done => Some(muted(state, "You are not a member of any team.")),
            _ => Some(muted(state, "Loading your teams…")),
        },
        (_, Progress::Failed(error)) => Some(failed(state, error)),
        (_, Progress::Done) if cycles.cycles().is_empty() => {
            Some(muted(state, "This team has no recent cycles."))
        }
        (_, Progress::Done) => None,
        _ => Some(muted(state, "Loading cycles…")),
    };
    if let Some(empty) = empty {
        frame.render_widget(empty, inner);
        return;
    }

    let now = state.store.as_of().unwrap_or_default();
    let counts: Vec<String> = cycles
        .cycles()
        .iter()
        .map(|cycle| {
            cycle_counts(cycle)
                .map(|(done, total)| format!("{done} of {total} done"))
                .unwrap_or_default()
        })
        .collect();
    let count_width = counts.iter().map(|count| count.len()).max().unwrap_or(0);
    let cells = Layout::horizontal([
        Constraint::Length(1),
        Constraint::Fill(1),
        Constraint::Length(15),
        Constraint::Length(6),
        Constraint::Length(u16::try_from(count_width).unwrap_or(u16::MAX)),
    ])
    .spacing(1);
    let offset = offset(cycles.focused_index(), inner);
    for (index, (line_area, (cycle, done))) in inner
        .rows()
        .zip(cycles.cycles().iter().zip(counts).skip(offset))
        .enumerate()
    {
        let is_focused = index + offset == cycles.focused_index();
        let [marker, name, dates, tag, count] = cells.areas(line_area);
        if is_focused {
            frame.render_widget("▸", marker);
        }
        frame.render_widget(Span::raw(cycle_title(cycle)), name);
        frame.render_widget(
            Span::raw(format!("{} – {}", day(cycle.starts_at), day(cycle.ends_at)))
                .fg(state.theme.muted),
            dates,
        );
        frame.render_widget(cycle_tag(state, cycle, now), tag);
        frame.render_widget(Line::from(done).alignment(Alignment::Right), count);
        if is_focused {
            frame
                .buffer_mut()
                .set_style(line_area, state.theme.selection);
        }
    }
}

pub fn render_projects(state: &State, frame: &mut Frame, area: Rect) {
    let projects = &state.projects;
    let title = opening_title(
        "Projects",
        projects.opening().and_then(|id| {
            projects
                .projects()
                .iter()
                .find(|overview| overview.project.id == id)
                .map(|overview| overview.project.name.clone())
        }),
    );
    let block = my_issues::pane_block(title, Focus::Alone, &state.theme);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let empty = match projects.progress() {
        Progress::Failed(error) => Some(failed(state, error)),
        Progress::Done if projects.projects().is_empty() => {
            Some(muted(state, "Your teams have no projects."))
        }
        Progress::Done => None,
        _ => Some(muted(state, "Loading projects…")),
    };
    if let Some(empty) = empty {
        frame.render_widget(empty, inner);
        return;
    }

    let cells = Layout::horizontal([
        Constraint::Length(1),
        Constraint::Fill(1),
        Constraint::Length(14),
        Constraint::Length(10),
        Constraint::Length(14),
        Constraint::Length(4),
    ])
    .spacing(1);
    let offset = offset(projects.focused_index(), inner);
    for (index, (line_area, overview)) in inner
        .rows()
        .zip(projects.projects().iter().skip(offset))
        .enumerate()
    {
        let is_focused = index + offset == projects.focused_index();
        let [marker, name, status, target, lead, progress] = cells.areas(line_area);
        if is_focused {
            frame.render_widget("▸", marker);
        }
        frame.render_widget(Span::raw(overview.project.name.as_str()), name);
        frame.render_widget(status_line(state, overview), status);
        frame.render_widget(
            Span::raw(
                overview
                    .target_date
                    .map(|date| date.to_string())
                    .unwrap_or_default(),
            )
            .fg(state.theme.muted),
            target,
        );
        frame.render_widget(
            Span::raw(
                overview
                    .lead
                    .as_ref()
                    .map(|lead| lead.name.clone())
                    .unwrap_or_default(),
            ),
            lead,
        );
        frame.render_widget(
            Line::from(percent(overview.progress)).alignment(Alignment::Right),
            progress,
        );
        if is_focused {
            frame
                .buffer_mut()
                .set_style(line_area, state.theme.selection);
        }
    }
}

/// `Cycles`, or `Cycles · loading Cycle 12…` while a cycle's issues are on
/// their way.
fn opening_title(title: &str, opening: Option<String>) -> String {
    match opening {
        Some(name) => format!("{title} · loading {name}…"),
        None => title.to_owned(),
    }
}

/// `t ENG Engineering`: the key that switches the team, and the team shown.
fn cycles_header(state: &State) -> Line<'static> {
    let Some(team_id) = state.cycles.team_id() else {
        return Line::default();
    };
    let mut spans: Vec<Span<'static>> = state
        .keymap
        .keys_for(Action::SwitchCyclesTeam, &keys::screen_contexts(state))
        .first()
        .map(|key| {
            vec![
                Span::styled(key.to_string(), style::accent(&state.theme)),
                Span::raw(" "),
            ]
        })
        .unwrap_or_default();
    let team = state
        .store
        .team(team_id)
        .map_or(team_id.to_owned(), |team| {
            format!("{} {}", team.key, team.name)
        });
    spans.push(Span::raw(team).bold());
    Line::from(spans)
}

/// `active` for the cycle running at `now`, `next` for one still to come.
fn cycle_tag(state: &State, cycle: &Cycle, now: Timestamp) -> Span<'static> {
    if cycle.starts_at <= now && now < cycle.ends_at {
        Span::raw("active").fg(state.theme.accent).bold()
    } else if now < cycle.starts_at {
        Span::raw("next").fg(state.theme.muted)
    } else {
        Span::default()
    }
}

/// The project's status, with the glyph of the state type it is closest
/// to so a status reads without color.
fn status_line(state: &State, overview: &ProjectOverview) -> Line<'static> {
    let state_type = match overview.status.status_type {
        ProjectStatusType::Backlog => StateType::Backlog,
        ProjectStatusType::Planned | ProjectStatusType::Paused => StateType::Unstarted,
        ProjectStatusType::Started => StateType::Started,
        ProjectStatusType::Completed => StateType::Completed,
        ProjectStatusType::Canceled => StateType::Canceled,
    };
    Line::from(vec![
        Span::raw(state.ui.icons.state(state_type)).fg(state.theme.state(state_type)),
        Span::raw(" "),
        Span::raw(overview.status.name.clone()),
    ])
}

fn day(at: Timestamp) -> String {
    short_date(at.to_zoned(TimeZone::UTC).date())
}

/// `Nov 17`.
fn short_date(date: Date) -> String {
    date.strftime("%b %-d").to_string()
}

/// `40%`, from a progress of 0 to 1.
fn percent(progress: f64) -> String {
    format!("{:.0}%", (progress * 100.0).clamp(0.0, 100.0))
}

/// The first row to draw so the focused one shows in `inner`.
fn offset(focused: usize, inner: Rect) -> usize {
    (focused + 1).saturating_sub(usize::from(inner.height))
}

fn muted(state: &State, text: &str) -> Paragraph<'static> {
    Paragraph::new(text.to_owned())
        .wrap(Wrap { trim: true })
        .fg(state.theme.muted)
}

fn failed(state: &State, error: &str) -> Paragraph<'static> {
    Paragraph::new(error.to_owned())
        .wrap(Wrap { trim: true })
        .style(style::error(&state.theme))
}
