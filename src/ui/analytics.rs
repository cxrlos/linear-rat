//! The Analytics screen's Personal view: how the viewer's own work flowed
//! over the picked range. Every section is plain lines, so the view scrolls
//! as one and nothing depends on a widget's own layout.

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
    DayCount, IssueAge, Percentiles, PersonalAnalytics, Share, Unit, WeekFlow, WeekRange,
    personal_analytics,
};
use crate::app::{Load, State, Store};
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
    frame.render_widget(Paragraph::new(header(state)), header_area);
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

/// The view (only Personal for now) and the range picker, with the picked
/// range in brackets.
fn header(state: &State) -> Line<'static> {
    let theme = &state.theme;
    let mut spans = vec![
        Span::styled("Personal", style::accent(theme)),
        Span::styled("   Range ", style::muted(theme)),
    ];
    let picked = state.analytics_range();
    for range in RANGES {
        let label = format!("{} weeks", range.weeks());
        spans.push(if range == picked {
            Span::styled(format!("[{label}]"), style::accent(theme))
        } else {
            Span::styled(format!(" {label} "), style::muted(theme))
        });
    }
    Line::from(spans)
}

/// Whether a load is in flight or why the last one failed; blank otherwise.
fn status(state: &State) -> Line<'static> {
    let theme = &state.theme;
    if let Some(error) = state.analytics_screen.error() {
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
    analytics: PersonalAnalytics,
    names: Names<'a>,
    theme: &'a Theme,
    icons: IconSet,
    today: Date,
    /// Whether the viewer's teams have loaded, so the unit is known.
    teams_known: bool,
}

impl<'a> View<'a> {
    fn new(state: &'a State) -> Option<Self> {
        let viewer = state.store.viewer()?;
        let period = state.analytics_period()?;
        let teams: Vec<_> = state.store.viewer_teams().cloned().collect();
        let issues: Vec<_> = state.store.issues().cloned().collect();
        let today = period.now().to_zoned(period.time_zone().clone()).date();
        Some(Self {
            analytics: personal_analytics(viewer, &teams, &issues, &period),
            names: Names(&state.store),
            theme: &state.theme,
            icons: state.ui.icons,
            today,
            teams_known: !teams.is_empty(),
        })
    }

    /// Created vs completed, throughput, the heatmap and the timings.
    fn charts(&self, width: usize) -> Vec<Line<'static>> {
        let mut lines = self.flow(width);
        lines.push(Line::default());
        lines.extend(self.throughput());
        lines.push(Line::default());
        lines.extend(self.heatmap());
        lines.push(Line::default());
        lines.extend(self.timing());
        lines
    }

    /// The in-progress issues and the splits of completed work.
    fn lists(&self, width: usize) -> Vec<Line<'static>> {
        let mut lines = self.in_progress(width);
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
    /// fit are left out from the oldest.
    fn flow(&self, width: usize) -> Vec<Line<'static>> {
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

    /// The in-progress issues with their issue age, oldest first.
    fn in_progress(&self, width: usize) -> Vec<Line<'static>> {
        let ages = &self.analytics.in_progress;
        let mut lines = vec![self.title("In progress", Some(format!("{}", ages.len())))];
        if ages.is_empty() {
            lines.push(Line::styled(
                "Nothing in progress.",
                style::muted(self.theme),
            ));
            return lines;
        }
        let rows: Vec<(&IssueAge, String, String)> = ages
            .iter()
            .map(|age| {
                let (identifier, title) = self.names.issue(&age.issue_id);
                (age, identifier, title)
            })
            .collect();
        let id_width = rows
            .iter()
            .map(|(_, identifier, _)| identifier.chars().count())
            .max()
            .unwrap_or(0);
        let age_width = 6;
        let title_width = width.saturating_sub(id_width + age_width + 2);
        for (age, identifier, title) in rows {
            lines.push(Line::from(vec![
                Span::styled(
                    format!("{identifier:<id_width$} "),
                    style::muted(self.theme),
                ),
                Span::raw(format!("{:<title_width$}", fit(&title, title_width))),
                Span::raw(format!(" {:>age_width$}", duration(age.age))),
            ]));
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
