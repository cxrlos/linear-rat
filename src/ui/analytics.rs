//! The Analytics screen: the Personal view, how the viewer's own work flowed
//! over the picked range, and the Team view, how one of the viewer's teams
//! is doing. Every section is plain lines, so the view scrolls as one and
//! nothing depends on a widget's own layout.

use jiff::SignedDuration;
use jiff::civil::Date;
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph};

use super::icons::{self, IconSet};
use super::style;
use super::theme::Theme;
use crate::analytics::{
    BurnUp, DayCount, IssueAge, PerPersonBreakdown, Percentiles, PersonalAnalytics, Share,
    TeamAnalytics, Unit, WeekFlow, WeekRange, personal_analytics, team_analytics,
};
use crate::app::{Action, AnalyticsView, Load, State, Store};
use crate::keys;
use crate::linear::model::StateType;

const TITLE: &str = "Analytics";
const RANGES: [WeekRange; 3] = [WeekRange::Four, WeekRange::Twelve, WeekRange::TwentySix];
/// The flow table's first column, wide enough for `Completed`.
const LABEL_WIDTH: usize = 11;
const WEEK_WIDTH: usize = 4;
const TOTAL_WIDTH: usize = 7;
/// The heatmap's day labels and its cells, two columns each.
const DAY_LABEL_WIDTH: usize = 5;
const DAY_WIDTH: usize = 2;
const SHARE_NAME_WIDTH: usize = 18;
const SHARE_BAR_WIDTH: usize = 12;
/// Each split lists this many keys, then how many more there are.
const SHARES_SHOWN: usize = 6;
/// The Team view's issue lists show this many issues, then how many more.
const TEAM_ISSUES_SHOWN: usize = 8;
/// The per-person breakdown's number columns.
const PERSON_COLUMN_WIDTH: usize = 9;
/// The gap between the two columns of the wide layout.
const GAP: u16 = 2;

pub fn render(state: &State, frame: &mut Frame, area: Rect) {
    let theme = &state.theme;
    let block = Block::bordered()
        .border_style(style::border(theme))
        .title(TITLE);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let [header_area, status_area, body] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Fill(1),
    ])
    .areas(inner);
    frame.render_widget(
        Paragraph::new(header(state, usize::from(header_area.width))),
        header_area,
    );
    frame.render_widget(Paragraph::new(status(state)), status_area);

    let columns = columns(state, body.width);
    let hidden = hidden_rows(&columns, body.height);
    let scroll = (state.analytics_screen.scroll().min(hidden), 0);
    match columns {
        Columns::One(lines) => frame.render_widget(Paragraph::new(lines).scroll(scroll), body),
        Columns::Two { left, right } => {
            let [left_area, _, right_area] = two_columns(body);
            frame.render_widget(Paragraph::new(left).scroll(scroll), left_area);
            frame.render_widget(Paragraph::new(right).scroll(scroll), right_area);
        }
    }
}

/// How many rows of the view do not fit in `area`, the body below the tabs.
pub fn max_scroll(state: &State, area: Rect) -> u16 {
    let inner_width = area.width.saturating_sub(2);
    let height = area.height.saturating_sub(4);
    hidden_rows(&columns(state, inner_width), height)
}

/// How many rows of `columns` do not fit in a body `height` rows tall.
fn hidden_rows(columns: &Columns, height: u16) -> u16 {
    let rows = match columns {
        Columns::One(lines) => lines.len(),
        Columns::Two { left, right } => left.len().max(right.len()),
    };
    u16::try_from(rows)
        .unwrap_or(u16::MAX)
        .saturating_sub(height)
}

enum Columns {
    One(Vec<Line<'static>>),
    Two {
        left: Vec<Line<'static>>,
        right: Vec<Line<'static>>,
    },
}

fn two_columns(area: Rect) -> [Rect; 3] {
    Layout::horizontal([
        Constraint::Fill(5),
        Constraint::Length(GAP),
        Constraint::Fill(3),
    ])
    .areas(area)
}

/// The view's sections for a body `width` columns wide: one column below
/// `ui.wide-layout-columns`, the charts beside the lists from there on.
fn columns(state: &State, width: u16) -> Columns {
    let theme = &state.theme;
    let Some(view) = View::new(state) else {
        let message = match state.load {
            Load::Loading => "Loading your issues from Linear…",
            Load::Loaded | Load::Failed(_) if state.store.viewer().is_none() => {
                "Nothing to show until lrat loads from Linear."
            }
            Load::Loaded | Load::Failed(_) if state.analytics_screen.is_loading() => {
                "Loading your teams from Linear…"
            }
            Load::Loaded | Load::Failed(_) if state.analytics_period().is_some() => {
                "You are not a member of any team."
            }
            Load::Loaded | Load::Failed(_) => "Nothing to show until lrat loads from Linear.",
        };
        return Columns::One(vec![Line::styled(message, style::muted(theme))]);
    };
    // The border is not part of `width`.
    if width + 2 >= state.ui.wide_layout_columns {
        let [left, _, right] = two_columns(Rect::new(0, 0, width, 1));
        Columns::Two {
            left: view.charts(usize::from(left.width)),
            right: view.lists(usize::from(right.width)),
        }
    } else {
        let width = usize::from(width);
        let mut lines = view.charts(width);
        lines.push(Line::default());
        lines.extend(view.lists(width));
        Columns::One(lines)
    }
}

/// The key that toggles the view and the two views, the team the Team
/// view shows with the key that switches it, and the range picker. The
/// picked view and range are in brackets. The team name is cut to what
/// `width` leaves, so the range picker stays in view.
fn header(state: &State, width: usize) -> Line<'static> {
    let theme = &state.theme;
    let view = state.analytics_screen.view();
    let mut spans = key(state, Action::ToggleAnalyticsView);
    for (label, shown) in [
        ("Personal", view == AnalyticsView::Personal),
        ("Team", view == AnalyticsView::Team),
    ] {
        spans.push(if shown {
            Span::styled(format!("[{label}]"), style::accent(theme))
        } else {
            Span::styled(format!(" {label} "), style::muted(theme))
        });
    }
    let mut range_spans = vec![Span::styled("   Range ", style::muted(theme))];
    let picked = state.analytics_range();
    for range in RANGES {
        let label = format!("{} weeks", range.weeks());
        range_spans.push(if range == picked {
            Span::styled(format!("[{label}]"), style::accent(theme))
        } else {
            Span::styled(format!(" {label} "), style::muted(theme))
        });
    }
    if view == AnalyticsView::Team
        && let Some(team) = state.analytics_team()
    {
        spans.push(Span::raw("  "));
        spans.extend(key(state, Action::SwitchAnalyticsTeam));
        let used: usize = spans.iter().chain(&range_spans).map(Span::width).sum();
        // The team key stays whole; the name takes what is left.
        let name_width = width.saturating_sub(used + team.key.chars().count() + 1);
        let label = if name_width == 0 {
            team.key.clone()
        } else {
            format!("{} {}", team.key, fit(&team.name, name_width))
        };
        spans.push(Span::styled(
            label,
            Style::new().add_modifier(Modifier::BOLD),
        ));
    }
    spans.extend(range_spans);
    Line::from(spans)
}

/// The first key that runs `action` on the screen, as accented text and a
/// space.
fn key(state: &State, action: Action) -> Vec<Span<'static>> {
    state
        .keymap
        .keys_for(action, &keys::screen_contexts(state))
        .first()
        .map(|key| {
            vec![
                Span::styled(key.to_string(), style::accent(&state.theme)),
                Span::raw(" "),
            ]
        })
        .unwrap_or_default()
}

/// Whether a load is in flight or why the last one failed; blank otherwise.
fn status(state: &State) -> Line<'static> {
    let theme = &state.theme;
    let team_error = match state.analytics_screen.view() {
        AnalyticsView::Personal => None,
        AnalyticsView::Team => state
            .analytics_team()
            .and_then(|team| state.analytics_screen.team_error(&team.id)),
    };
    if let Some(error) = state.analytics_screen.error().or(team_error) {
        Line::styled(
            format!("Could not load analytics: {error}"),
            style::error(theme),
        )
    } else if state.analytics_screen.is_loading() {
        Line::styled("Loading analytics from Linear…", style::muted(theme))
    } else {
        Line::default()
    }
}

/// Everything the sections draw from.
struct View<'a> {
    /// What the sections both views share draw from; the Team view fills
    /// it from its team analytics, with no completion days.
    analytics: PersonalAnalytics,
    /// The rest of the Team view; `None` in the Personal view.
    team: Option<TeamView>,
    names: Names<'a>,
    theme: &'a Theme,
    icons: IconSet,
    today: Date,
    /// Whether the viewer's teams have loaded, so the unit is known.
    teams_known: bool,
}

struct TeamView {
    analytics: TeamAnalytics,
    /// The stale threshold, from `[analytics] stale-days`.
    stale_days: u32,
}

impl<'a> View<'a> {
    fn new(state: &'a State) -> Option<Self> {
        let viewer = state.store.viewer()?;
        let period = state.analytics_period()?;
        let teams: Vec<_> = state.store.viewer_teams().cloned().collect();
        let issues: Vec<_> = state.store.issues().cloned().collect();
        let today = period.now().to_zoned(period.time_zone().clone()).date();
        let (analytics, team) = match state.analytics_screen.view() {
            AnalyticsView::Personal => (personal_analytics(viewer, &teams, &issues, &period), None),
            AnalyticsView::Team => {
                let team = state.analytics_team()?;
                let cycles: Vec<_> = state.store.cycles().cloned().collect();
                let threshold = state.analytics.stale_threshold;
                let analytics = team_analytics(
                    team,
                    &state.store.administrable_teams().unwrap_or_default(),
                    &issues,
                    &cycles,
                    threshold,
                    &period,
                );
                let shared = PersonalAnalytics {
                    unit: analytics.unit,
                    weeks: analytics.weeks.clone(),
                    completion_days: Vec::new(),
                    cycle_time: analytics.cycle_time,
                    lead_time: analytics.lead_time,
                    in_progress: analytics.in_progress.clone(),
                    splits: analytics.splits.clone(),
                };
                let team = TeamView {
                    analytics,
                    stale_days: threshold.in_days(),
                };
                (shared, Some(team))
            }
        };
        Some(Self {
            analytics,
            team,
            names: Names(&state.store),
            theme: &state.theme,
            icons: state.ui.icons,
            today,
            teams_known: !teams.is_empty(),
        })
    }

    fn charts(&self, width: usize) -> Vec<Line<'static>> {
        match &self.team {
            None => self.personal_charts(width),
            Some(team) => self.team_charts(team, width),
        }
    }

    fn lists(&self, width: usize) -> Vec<Line<'static>> {
        match &self.team {
            None => self.personal_lists(width),
            Some(team) => self.team_lists(team, width),
        }
    }

    /// Created vs completed, throughput, the heatmap and the timings.
    fn personal_charts(&self, width: usize) -> Vec<Line<'static>> {
        let mut lines = self.flow(width, None);
        lines.push(Line::default());
        lines.extend(self.throughput());
        lines.push(Line::default());
        lines.extend(self.heatmap());
        lines.push(Line::default());
        lines.extend(self.timing());
        lines
    }

    /// The in-progress issues and the splits of completed work.
    fn personal_lists(&self, width: usize) -> Vec<Line<'static>> {
        let mut lines = self.in_progress(width, usize::MAX);
        let splits = &self.analytics.splits;
        let by_project = self.shares(
            "Completed by project",
            &splits.by_project,
            |key| match key {
                Some(id) => self.names.project(id),
                None => "No project".to_owned(),
            },
        );
        let by_label = self.shares("Completed by label", &splits.by_label, |key| match key {
            Some(id) => self.names.label(id),
            None => "No label".to_owned(),
        });
        let by_priority = self.shares("Completed by priority", &splits.by_priority, |key| {
            format!(
                "{} {}",
                self.icons.priority(*key),
                icons::priority_name(*key)
            )
        });
        for section in [by_project, by_label, by_priority] {
            lines.push(Line::default());
            lines.extend(section);
        }
        lines
    }

    fn title(&self, text: &str, note: Option<String>) -> Line<'static> {
        let mut spans = vec![Span::styled(
            text.to_owned(),
            Style::new().add_modifier(Modifier::BOLD),
        )];
        if let Some(note) = note {
            spans.push(Span::styled(format!("  {note}"), style::muted(self.theme)));
        }
        Line::from(spans)
    }

    /// One column per week, oldest first, then the totals. Weeks that do not
    /// fit are left out from the oldest. With `open`, the issues open at the
    /// end of each week follow, and open now in the total column.
    fn flow(&self, width: usize, open: Option<&[u32]>) -> Vec<Line<'static>> {
        let weeks = &self.analytics.weeks;
        let fit = width.saturating_sub(LABEL_WIDTH + TOTAL_WIDTH) / WEEK_WIDTH;
        let shown = &weeks[weeks.len().saturating_sub(fit)..];
        let note = (shown.len() < weeks.len())
            .then(|| format!("last {} of {} weeks", shown.len(), weeks.len()));
        let mut lines = vec![self.title("Created vs completed", note)];

        let row = |label: &str, cells: Vec<String>, total: String, style: Style| {
            let mut text = format!("{label:<LABEL_WIDTH$}");
            for cell in cells {
                text.push_str(&format!("{cell:>WEEK_WIDTH$}"));
            }
            text.push_str(&format!("{total:>TOTAL_WIDTH$}"));
            Line::styled(text, style)
        };
        let muted = style::muted(self.theme);
        let sum = |count: fn(&WeekFlow) -> i64| weeks.iter().map(count).sum::<i64>();
        lines.push(row(
            "Week",
            shown
                .iter()
                .map(|week| format!("W{:02}", week.week_start.iso_week_date().week()))
                .collect(),
            "Total".to_owned(),
            muted,
        ));
        lines.push(row(
            "Created",
            shown.iter().map(|week| week.created.to_string()).collect(),
            sum(|week| i64::from(week.created)).to_string(),
            Style::new(),
        ));
        lines.push(row(
            "Completed",
            shown
                .iter()
                .map(|week| week.completed.to_string())
                .collect(),
            sum(|week| i64::from(week.completed)).to_string(),
            Style::new(),
        ));
        lines.push(row(
            "Net flow",
            shown.iter().map(|week| signed(week.net_flow())).collect(),
            signed(sum(WeekFlow::net_flow)),
            Style::new(),
        ));
        if let Some(open) = open {
            let shown_open = &open[open.len().saturating_sub(shown.len())..];
            lines.push(row(
                "Open",
                shown_open.iter().map(u32::to_string).collect(),
                open.last().map_or_else(String::new, u32::to_string),
                Style::new(),
            ));
        }
        if self.analytics.unit == Unit::Points {
            let total: f64 = weeks.iter().map(|week| week.completed_scope).sum();
            lines.push(row(
                "Points",
                shown
                    .iter()
                    .map(|week| scope(week.completed_scope))
                    .collect(),
                scope(total),
                Style::new(),
            ));
        }
        lines
    }

    /// Issues completed per week as a sparkline, and estimate points per
    /// week when the viewer's teams use estimates.
    fn throughput(&self) -> Vec<Line<'static>> {
        let weeks = &self.analytics.weeks;
        let count = weeks.len().max(1) as f64;
        let completed: Vec<f64> = weeks.iter().map(|week| f64::from(week.completed)).collect();
        let mut lines = vec![self.sparkline(
            "Throughput",
            &completed,
            format!(
                "{} issues a week",
                one_decimal(completed.iter().sum::<f64>() / count)
            ),
        )];
        if self.analytics.unit == Unit::Points {
            let points: Vec<f64> = weeks.iter().map(|week| week.completed_scope).collect();
            lines.push(self.sparkline(
                "Points",
                &points,
                format!(
                    "{} points a week",
                    one_decimal(points.iter().sum::<f64>() / count)
                ),
            ));
        } else if self.team.is_some() {
            lines.push(Line::styled(
                "This team does not use estimates, so scope is counted in issues.",
                style::muted(self.theme),
            ));
        } else if self.teams_known {
            lines.push(Line::styled(
                "No team of yours uses estimates, so scope is counted in issues.",
                style::muted(self.theme),
            ));
        }
        lines
    }

    /// One bar per value, scaled to the largest; a zero is a muted dot so
    /// the weeks still line up.
    fn sparkline(&self, label: &str, values: &[f64], average: String) -> Line<'static> {
        const BARS: [char; 8] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
        let max = values.iter().copied().fold(0.0, f64::max);
        let mut spans = vec![Span::raw(format!("{label:<LABEL_WIDTH$}"))];
        for &value in values {
            spans.push(if value <= 0.0 || max <= 0.0 {
                Span::styled("·", style::muted(self.theme))
            } else {
                let level = (value / max * 8.0).ceil() as usize;
                Span::styled(
                    BARS[level.clamp(1, 8) - 1].to_string(),
                    style::accent(self.theme),
                )
            });
        }
        spans.push(Span::styled(
            format!("  {average}"),
            style::muted(self.theme),
        ));
        Line::from(spans)
    }

    /// Completion days: a row per weekday and a column per week, each cell
    /// shaded by how many issues were completed that day.
    fn heatmap(&self) -> Vec<Line<'static>> {
        let days = &self.analytics.completion_days;
        let weeks = days.len().div_ceil(7);
        let mut lines = vec![self.title("Completion days", None)];

        let mut months = vec![' '; DAY_LABEL_WIDTH + weeks * DAY_WIDTH];
        let mut previous_month = None;
        for (week, day) in days.iter().step_by(7).enumerate() {
            let month = day.day.month();
            let at = DAY_LABEL_WIDTH + week * DAY_WIDTH;
            if previous_month != Some(month) && at + 3 <= months.len() {
                for (offset, c) in month_name(month).chars().enumerate() {
                    months[at + offset] = c;
                }
            }
            previous_month = Some(month);
        }
        lines.push(Line::styled(
            months.into_iter().collect::<String>().trim_end().to_owned(),
            style::muted(self.theme),
        ));

        let done = Style::new().fg(self.theme.state(StateType::Completed));
        for (weekday, label) in WEEKDAYS.iter().enumerate() {
            let mut spans = vec![Span::styled(
                format!("{:<DAY_LABEL_WIDTH$}", label),
                style::muted(self.theme),
            )];
            for week in 0..weeks {
                let cell = days.get(week * 7 + weekday).filter(|d| d.day <= self.today);
                spans.push(match cell {
                    Some(DayCount { completed: 0, .. }) => {
                        Span::styled(format!("{:<DAY_WIDTH$}", '·'), style::muted(self.theme))
                    }
                    Some(day) => {
                        Span::styled(format!("{:<DAY_WIDTH$}", shade(day.completed)), done)
                    }
                    None => Span::raw(" ".repeat(DAY_WIDTH)),
                });
            }
            lines.push(Line::from(spans));
        }
        let mut legend = vec![Span::styled(
            format!("{:<DAY_LABEL_WIDTH$}· none", ""),
            style::muted(self.theme),
        )];
        for (count, label) in [(1, "1"), (2, "2"), (3, "3"), (4, "4+")] {
            legend.push(Span::raw("  "));
            legend.push(Span::styled(shade(count).to_string(), done));
            legend.push(Span::styled(format!(" {label}"), style::muted(self.theme)));
        }
        lines.push(Line::from(legend));
        lines
    }

    /// Cycle time and lead time at the median and the 85th percentile.
    fn timing(&self) -> Vec<Line<'static>> {
        let mut lines = vec![self.title("Time to complete", None)];
        for (label, percentiles) in [
            ("Cycle time", self.analytics.cycle_time),
            ("Lead time", self.analytics.lead_time),
        ] {
            lines.push(match percentiles {
                Some(Percentiles { p50, p85, samples }) => Line::from(vec![
                    Span::raw(format!("{label:<LABEL_WIDTH$}")),
                    Span::styled("p50 ", style::muted(self.theme)),
                    Span::raw(format!("{:<7}", duration(p50))),
                    Span::styled("p85 ", style::muted(self.theme)),
                    Span::raw(format!("{:<7}", duration(p85))),
                    Span::styled(
                        format!("{samples} {}", issues(samples)),
                        style::muted(self.theme),
                    ),
                ]),
                None => Line::from(vec![
                    Span::raw(format!("{label:<LABEL_WIDTH$}")),
                    Span::styled("no issues completed in range", style::muted(self.theme)),
                ]),
            });
        }
        lines
    }

    /// The in-progress issues with their issue age, oldest first, at most
    /// `shown` of them.
    fn in_progress(&self, width: usize, shown: usize) -> Vec<Line<'static>> {
        let ages = &self.analytics.in_progress;
        let mut lines = vec![self.title("In progress", Some(format!("{}", ages.len())))];
        if ages.is_empty() {
            lines.push(Line::styled(
                "Nothing in progress.",
                style::muted(self.theme),
            ));
            return lines;
        }
        let rows: Vec<(&str, SignedDuration)> = ages
            .iter()
            .map(|age: &IssueAge| (age.issue_id.as_str(), age.age))
            .collect();
        lines.extend(self.issue_rows(&rows, width, shown));
        lines
    }

    /// One row per issue: its identifier, title and a duration, at most
    /// `shown` rows, then how many more there are.
    fn issue_rows(
        &self,
        issues: &[(&str, SignedDuration)],
        width: usize,
        shown: usize,
    ) -> Vec<Line<'static>> {
        let rows: Vec<(String, String, SignedDuration)> = issues
            .iter()
            .take(shown)
            .map(|&(id, age)| {
                let (identifier, title) = self.names.issue(id);
                (identifier, title, age)
            })
            .collect();
        let id_width = rows
            .iter()
            .map(|(identifier, _, _)| identifier.chars().count())
            .max()
            .unwrap_or(0);
        let age_width = 6;
        let title_width = width.saturating_sub(id_width + age_width + 2);
        let mut lines: Vec<Line<'static>> = rows
            .into_iter()
            .map(|(identifier, title, age)| {
                Line::from(vec![
                    Span::styled(
                        format!("{identifier:<id_width$} "),
                        style::muted(self.theme),
                    ),
                    Span::raw(format!("{:<title_width$}", fit(&title, title_width))),
                    Span::raw(format!(" {:>age_width$}", duration(age))),
                ])
            })
            .collect();
        if issues.len() > shown {
            lines.push(Line::styled(
                format!("and {} more", issues.len() - shown),
                style::muted(self.theme),
            ));
        }
        lines
    }

    /// One split of completed work: each key's issues, its points when
    /// scope is in points, and a bar of its share of the largest.
    fn shares<K>(
        &self,
        title: &str,
        shares: &[Share<K>],
        name: impl Fn(&K) -> String,
    ) -> Vec<Line<'static>> {
        let mut lines = vec![self.title(title, None)];
        if shares.is_empty() {
            lines.push(Line::styled(
                "Nothing completed in range.",
                style::muted(self.theme),
            ));
            return lines;
        }
        let largest = shares.iter().map(|share| share.scope).fold(0.0, f64::max);
        for share in shares.iter().take(SHARES_SHOWN) {
            let mut text = format!(
                "{:<SHARE_NAME_WIDTH$} {:>3} {:<6}",
                fit(&name(&share.key), SHARE_NAME_WIDTH),
                share.issues,
                issues(share.issues as usize),
            );
            if self.analytics.unit == Unit::Points {
                text.push_str(&format!("{:>5} pts ", scope(share.scope)));
            } else {
                text.push(' ');
            }
            let bar = if largest > 0.0 {
                ((share.scope / largest) * SHARE_BAR_WIDTH as f64).round() as usize
            } else {
                0
            };
            lines.push(Line::from(vec![
                Span::raw(text),
                Span::styled("█".repeat(bar.max(1)), style::accent(self.theme)),
            ]));
        }
        if shares.len() > SHARES_SHOWN {
            lines.push(Line::styled(
                format!("and {} more", shares.len() - SHARES_SHOWN),
                style::muted(self.theme),
            ));
        }
        lines
    }
}

impl View<'_> {
    /// Created vs completed with the open-issue trend, throughput, the
    /// active cycle's burn-up, velocity and the timings.
    fn team_charts(&self, team: &TeamView, width: usize) -> Vec<Line<'static>> {
        let mut lines = self.flow(width, Some(&team.analytics.open_trend));
        for section in [
            self.throughput(),
            self.burn_up(team.analytics.burn_up.as_ref()),
            self.velocity(team),
            self.timing(),
        ] {
            lines.push(Line::default());
            lines.extend(section);
        }
        lines
    }

    /// Issue age, triage health, stale issues, the splits by priority and
    /// label, and the per-person breakdown for a team the viewer
    /// administers.
    fn team_lists(&self, team: &TeamView, width: usize) -> Vec<Line<'static>> {
        let mut lines = self.in_progress(width, TEAM_ISSUES_SHOWN);
        let splits = &self.analytics.splits;
        let by_priority = self.shares("Completed by priority", &splits.by_priority, |key| {
            format!(
                "{} {}",
                self.icons.priority(*key),
                icons::priority_name(*key)
            )
        });
        let by_label = self.shares("Completed by label", &splits.by_label, |key| match key {
            Some(id) => self.names.label(id),
            None => "No label".to_owned(),
        });
        let mut sections = vec![
            self.triage(team, width),
            self.stale(team, width),
            by_priority,
            by_label,
        ];
        if let Some(breakdown) = &team.analytics.per_person {
            sections.push(self.per_person(breakdown, width));
        }
        for section in sections {
            lines.push(Line::default());
            lines.extend(section);
        }
        lines
    }

    /// The active cycle's scope and completed scope per day, on one scale,
    /// and its scope creep.
    fn burn_up(&self, burn_up: Option<&BurnUp>) -> Vec<Line<'static>> {
        let Some(burn_up) = burn_up else {
            return vec![
                self.title("Active cycle", None),
                Line::styled("No active cycle.", style::muted(self.theme)),
            ];
        };
        let cycle = self.names.0.cycle(&burn_up.cycle_id);
        let name = cycle.map_or_else(
            || burn_up.cycle_id.clone(),
            |cycle| {
                cycle
                    .name
                    .clone()
                    .unwrap_or_else(|| format!("Cycle {}", cycle.number))
            },
        );
        let length = cycle.map_or(0, |cycle| {
            cycle
                .ends_at
                .duration_since(cycle.starts_at)
                .as_hours()
                .div_euclid(24)
        });
        let note = format!("{name}, day {} of {length}", burn_up.days.len());
        let mut lines = vec![self.title("Active cycle", Some(note))];
        let unit = unit_word(burn_up.unit);
        let max = burn_up
            .days
            .iter()
            .map(|day| day.scope.max(day.completed))
            .fold(0.0, f64::max);
        let in_scope: Vec<f64> = burn_up.days.iter().map(|day| day.scope).collect();
        let completed: Vec<f64> = burn_up.days.iter().map(|day| day.completed).collect();
        for (label, values) in [("Scope", in_scope), ("Completed", completed)] {
            let last = values.last().copied().unwrap_or(0.0);
            let mut spans = vec![Span::raw(format!("{label:<LABEL_WIDTH$}"))];
            spans.extend(self.bars(&values, max));
            spans.push(Span::styled(
                format!("  {} {unit}", scope(last)),
                style::muted(self.theme),
            ));
            lines.push(Line::from(spans));
        }
        lines.push(Line::from(vec![
            Span::raw("Scope creep "),
            Span::raw(format!("+{} {unit}", scope(burn_up.scope_creep))),
            Span::styled(" added after day one", style::muted(self.theme)),
        ]));
        lines
    }

    /// One bar per value on a scale up to `max`; a zero is a muted dot.
    fn bars(&self, values: &[f64], max: f64) -> Vec<Span<'static>> {
        values
            .iter()
            .map(|&value| {
                if value <= 0.0 || max <= 0.0 {
                    Span::styled("·", style::muted(self.theme))
                } else {
                    Span::styled(bar(value / max).to_string(), style::accent(self.theme))
                }
            })
            .collect()
    }

    /// What each of the last finished cycles completed against what it
    /// committed to on its first day, oldest first.
    fn velocity(&self, team: &TeamView) -> Vec<Line<'static>> {
        let velocity = &team.analytics.velocity;
        let unit = unit_word(team.analytics.unit);
        if velocity.is_empty() {
            return vec![
                self.title("Velocity", None),
                Line::styled("No finished cycles.", style::muted(self.theme)),
            ];
        }
        let average =
            velocity.iter().map(|cycle| cycle.completed).sum::<f64>() / velocity.len() as f64;
        let note = format!(
            "last {} cycles, {} {unit} a cycle",
            velocity.len(),
            one_decimal(average)
        );
        let mut lines = vec![self.title("Velocity", Some(note))];
        let max = velocity
            .iter()
            .map(|cycle| cycle.completed.max(cycle.committed))
            .fold(0.0, f64::max);
        for cycle in velocity {
            let share = cycle.committed_completed_share.map_or_else(
                || "nothing committed".to_owned(),
                |share| format!("{:.0}% of committed", share * 100.0),
            );
            let mut spans = vec![Span::raw(format!(
                "{:<LABEL_WIDTH$}",
                format!("Cycle {}", cycle.number)
            ))];
            let filled = if max > 0.0 {
                ((cycle.completed / max) * SHARE_BAR_WIDTH as f64).round() as usize
            } else {
                0
            };
            spans.push(Span::styled(
                format!("{:<SHARE_BAR_WIDTH$}", "█".repeat(filled)),
                style::accent(self.theme),
            ));
            spans.push(Span::raw(format!(
                " {:>4} of {:>4} {unit}  ",
                scope(cycle.completed),
                scope(cycle.committed),
            )));
            spans.push(Span::styled(share, style::muted(self.theme)));
            lines.push(Line::from(spans));
        }
        lines
    }

    /// What waits in triage now, longest waiting first, and the median
    /// triage time of the range.
    fn triage(&self, team: &TeamView, width: usize) -> Vec<Line<'static>> {
        let triage = &team.analytics.triage;
        let median = triage.median_triage_time.map_or_else(
            || "no issue left triage in range".to_owned(),
            |median| format!("median triage time {}", duration(median)),
        );
        let note = format!("{} waiting, {median}", triage.waiting.len());
        let mut lines = vec![self.title("Triage", Some(note))];
        let now = self.names.0.as_of().unwrap_or_default();
        let waiting: Vec<(&str, SignedDuration)> = triage
            .waiting
            .iter()
            .map(|id| {
                let waited = self
                    .names
                    .0
                    .issue(id)
                    .map_or(SignedDuration::ZERO, |issue| {
                        now.duration_since(issue.created_at)
                    });
                (id.as_str(), waited)
            })
            .collect();
        lines.extend(self.issue_rows(&waiting, width, TEAM_ISSUES_SHOWN));
        lines
    }

    /// The open issues with no update for the stale threshold, longest
    /// untouched first.
    fn stale(&self, team: &TeamView, width: usize) -> Vec<Line<'static>> {
        let stale = &team.analytics.stale;
        let days = team.stale_days;
        let note = format!(
            "{}, no update in {days} {}",
            stale.len(),
            if days == 1 { "day" } else { "days" }
        );
        let mut lines = vec![self.title("Stale issues", Some(note))];
        let rows: Vec<(&str, SignedDuration)> = stale
            .iter()
            .map(|issue| (issue.issue_id.as_str(), issue.since_update))
            .collect();
        lines.extend(self.issue_rows(&rows, width, TEAM_ISSUES_SHOWN));
        lines
    }

    /// Each assignee's completed, in-progress, created and open work, in
    /// member order with unassigned work last.
    fn per_person(&self, breakdown: &PerPersonBreakdown, width: usize) -> Vec<Line<'static>> {
        let mut lines = vec![self.title("Per person", None)];
        let open = match breakdown.unit {
            Unit::Points => "Open pts",
            Unit::Issues => "Open",
        };
        let name_width = width.saturating_sub(4 * PERSON_COLUMN_WIDTH).max(8);
        let row = |name: &str, cells: [String; 4]| {
            let mut text = format!("{:<name_width$}", fit(name, name_width));
            for cell in cells {
                text.push_str(&format!("{cell:>PERSON_COLUMN_WIDTH$}"));
            }
            text
        };
        lines.push(Line::styled(
            row("", ["Done", "Started", "Created", open].map(str::to_owned)),
            style::muted(self.theme),
        ));
        if breakdown.people.is_empty() {
            lines.push(Line::styled(
                "No issues in range.",
                style::muted(self.theme),
            ));
        }
        for person in &breakdown.people {
            let name = person
                .assignee_id
                .as_deref()
                .map_or_else(|| "Unassigned".to_owned(), |id| self.names.member(id));
            lines.push(Line::raw(row(
                &name,
                [
                    person.completed.to_string(),
                    person.in_progress.to_string(),
                    person.created.to_string(),
                    scope(person.open_scope),
                ],
            )));
        }
        lines
    }
}

/// Names of what the splits and the in-progress list refer to, falling back
/// to the id when the store does not have it.
struct Names<'a>(&'a Store);

impl Names<'_> {
    fn project(&self, id: &str) -> String {
        self.0
            .project(id)
            .map_or_else(|| id.to_owned(), |project| project.name.clone())
    }

    fn label(&self, id: &str) -> String {
        self.0
            .label(id)
            .map_or_else(|| id.to_owned(), |label| label.name.clone())
    }

    fn member(&self, id: &str) -> String {
        self.0
            .member(id)
            .map_or_else(|| id.to_owned(), |member| member.name.clone())
    }

    /// The identifier and title of the issue `id`.
    fn issue(&self, id: &str) -> (String, String) {
        self.0.issue(id).map_or_else(
            || (id.to_owned(), String::new()),
            |issue| (issue.identifier.clone(), issue.title.clone()),
        )
    }
}

const WEEKDAYS: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];

fn month_name(month: i8) -> &'static str {
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    usize::try_from(month - 1)
        .ok()
        .and_then(|index| MONTHS.get(index))
        .copied()
        .unwrap_or("")
}

/// The heatmap's glyph for a day with `completed` issues; each count has its
/// own shape, so the map reads without color.
fn shade(completed: u32) -> char {
    match completed {
        0 => '·',
        1 => '░',
        2 => '▒',
        3 => '▓',
        _ => '█',
    }
}

/// The bar for a value that is `share` of the largest, at least the
/// lowest bar.
fn bar(share: f64) -> char {
    const BARS: [char; 8] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
    let level = (share * 8.0).ceil() as usize;
    BARS[level.clamp(1, 8) - 1]
}

fn unit_word(unit: Unit) -> &'static str {
    match unit {
        Unit::Points => "pts",
        Unit::Issues => "issues",
    }
}

fn signed(value: i64) -> String {
    if value == 0 {
        "0".to_owned()
    } else {
        format!("{value:+}")
    }
}

/// Points as a whole number, or with one decimal when they have a fraction.
fn scope(value: f64) -> String {
    if value.fract() == 0.0 {
        format!("{value:.0}")
    } else {
        format!("{value:.1}")
    }
}

fn one_decimal(value: f64) -> String {
    format!("{value:.1}")
}

fn issues(count: usize) -> &'static str {
    if count == 1 { "issue" } else { "issues" }
}

/// A duration as hours under a day, then days.
fn duration(duration: SignedDuration) -> String {
    let hours = duration.as_secs() / 3600;
    if hours < 24 {
        return format!("{hours}h");
    }
    let days = duration.as_secs_f64() / 86_400.0;
    if days < 10.0 {
        format!("{days:.1}d")
    } else {
        format!("{days:.0}d")
    }
}

/// `text` cut to `width` characters, ending in `…` when it was longer.
fn fit(text: &str, width: usize) -> String {
    if text.chars().count() <= width {
        return text.to_owned();
    }
    let mut cut: String = text.chars().take(width.saturating_sub(1)).collect();
    cut.push('…');
    cut
}
