//! Team Issues: a header with the team, how many issues are loaded of how
//! many match, and the filters, over the list and the focused issue's
//! detail.

use super::text;
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Stylize;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Wrap};

use super::my_issues::{self, Focus, View};
use super::{detail, style};
use crate::app::{Action, FilterField, Progress, State, can_load_more, filter_label};
use crate::keys;

/// How many rows the header takes above the panes: one once a team shows,
/// none before.
pub fn header_rows(state: &State) -> u16 {
    u16::from(state.team_issues.query().is_some())
}

/// The header and the panes under it.
fn split(state: &State, area: Rect) -> [Rect; 2] {
    Layout::vertical([Constraint::Length(header_rows(state)), Constraint::Fill(1)]).areas(area)
}

/// Where the detail pane is in a body of `area`, or `None` when only the
/// list shows.
pub(super) fn detail_area(state: &State, area: Rect) -> Option<Rect> {
    let [_, panes_area] = split(state, area);
    my_issues::detail_area(state, panes_area, state.team_issues.pane())
}

pub fn render(state: &State, frame: &mut Frame, area: Rect) {
    let [header_area, panes_area] = split(state, area);
    let team_issues = &state.team_issues;
    frame.render_widget(header(state, header_area.width), header_area);

    let view = View::new(state, team_issues.names());
    let (list, detail) = my_issues::panes(state, panes_area, team_issues.pane());
    if let Some((list_area, focus)) = list {
        render_list(&view, state, frame, list_area, focus);
    }
    if let Some((detail_area, focus)) = detail {
        my_issues::render_detail(state, detail::issue(state), frame, detail_area, focus);
    }
}

/// The first key that runs `action` here, as accented text and a space.
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

/// The key that starts the filter chords, which opens which-key on the
/// filters, when every filter is bound to a chord behind the same key.
fn filter_prefix(state: &State) -> Vec<Span<'static>> {
    let contexts = keys::screen_contexts(state);
    let mut firsts = FilterField::ALL.into_iter().map(|field| {
        state
            .keymap
            .keys_for(Action::FilterBy(field), &contexts)
            .first()
            .filter(|sequence| sequence.keys().len() > 1)
            .and_then(|sequence| sequence.keys().first().copied())
    });
    let Some(Some(prefix)) = firsts.next() else {
        return Vec::new();
    };
    if !firsts.all(|first| first == Some(prefix)) {
        return Vec::new();
    }
    vec![
        Span::styled(prefix.to_string(), style::accent(&state.theme)),
        Span::raw(" "),
    ]
}

/// `t ENG Engineering · 50 of 70 m more · f Filters State Todo, Label bug`,
/// with the filters shortened to `State, Label` and then to `2` when the
/// line is wider than `width`.
fn header(state: &State, width: u16) -> Line<'static> {
    let theme = &state.theme;
    let team_issues = &state.team_issues;
    let Some(query) = team_issues.query() else {
        return Line::default();
    };
    let separator = || Span::styled(" · ", style::muted(theme));
    let mut spans = key(state, Action::SwitchTeam);
    let team = state
        .store
        .team(&query.team_id)
        .map_or(query.team_id.clone(), |team| {
            format!("{} {}", team.key, team.name)
        });
    spans.push(Span::raw(team).bold());
    spans.push(separator());
    let total = team_issues
        .total()
        .map_or("…".to_owned(), |total| total.to_string());
    spans.push(Span::raw(format!("{} of {total}", team_issues.ids().len())));
    if team_issues.has_more() {
        spans.push(Span::raw(" "));
        if can_load_more(state) {
            spans.extend(key(state, Action::LoadMore));
            spans.push(Span::raw("more"));
        } else {
            spans.push(Span::styled("loading more", style::muted(theme)));
        }
    }
    spans.push(separator());
    spans.extend(filter_prefix(state));
    spans.push(Span::styled("Filters ", style::muted(theme)));
    // Each filter's name and, but for Blocked, its value.
    let mut filters: Vec<(&str, Option<String>)> = FilterField::ALL
        .into_iter()
        .filter_map(|field| filter_label(state, field).map(|value| (field.name(), Some(value))))
        .collect();
    if query.filter.blocked {
        filters.push(("Blocked", None));
    }
    if filters.is_empty() {
        spans.push(Span::raw("none"));
        return Line::from(spans);
    }
    let room = usize::from(width).saturating_sub(Line::from(spans.clone()).width());
    let named = filters
        .iter()
        .map(|(name, value)| match value {
            Some(value) => format!("{name} {value}"),
            None => (*name).to_owned(),
        })
        .collect::<Vec<_>>()
        .join(", ");
    let fields = filters
        .iter()
        .map(|(name, _)| *name)
        .collect::<Vec<_>>()
        .join(", ");
    let text = [named, fields]
        .into_iter()
        .find(|candidate| text::width(candidate) <= room)
        .unwrap_or_else(|| filters.len().to_string());
    spans.push(Span::raw(text));
    Line::from(spans)
}

/// `Team Issues · group: none · sort: priority, urgent first`. When the
/// sort orders only the loaded issues, `sorted: loaded 50 of 70` follows
/// the name, ahead of what a narrow pane cuts off.
fn list_title(state: &State) -> String {
    let team_issues = &state.team_issues;
    if team_issues.sort_covers_all() {
        return my_issues::list_title(
            "Team Issues",
            team_issues.arrangement(),
            state.ui.team_issues,
            "",
        );
    }
    let total = team_issues
        .total()
        .map_or("…".to_owned(), |total| total.to_string());
    let name = format!(
        "Team Issues · sorted: loaded {} of {total}",
        team_issues.ids().len()
    );
    my_issues::list_title(&name, team_issues.arrangement(), state.ui.team_issues, "")
}

fn render_list(view: &View, state: &State, frame: &mut Frame, area: Rect, focus: Focus) {
    let theme = &state.theme;
    let team_issues = &state.team_issues;
    let block = my_issues::pane_block(list_title(state), area, focus, theme);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let message = |text: String| Paragraph::new(text).wrap(Wrap { trim: true });
    let empty = match (team_issues.teams_progress(), team_issues.page_progress()) {
        (Progress::NotAsked | Progress::Loading, _) => {
            Some(message("Loading your teams…".to_owned()).fg(theme.muted))
        }
        (Progress::Failed(error), _) => Some(message(error.clone()).style(style::error(theme))),
        _ if team_issues.query().is_none() => {
            Some(message("You are not a member of any team.".to_owned()).fg(theme.muted))
        }
        (_, Progress::Failed(_))
            if team_issues.ids().is_empty()
                && team_issues.query().is_some_and(|query| {
                    state.store.capabilities().reach(&query.team_id).is_denied()
                }) =>
        {
            Some(message("Not accessible with this key".to_owned()).fg(theme.muted))
        }
        (_, Progress::Failed(error)) if team_issues.ids().is_empty() => {
            Some(message(error.clone()).style(style::error(theme)))
        }
        (_, Progress::Loading) if team_issues.ids().is_empty() => {
            Some(message("Loading issues…".to_owned()).fg(theme.muted))
        }
        _ if team_issues.ids().is_empty() => {
            let filtered = team_issues
                .query()
                .is_some_and(|query| !query.filter.is_empty());
            let text = if filtered {
                "No issues match these filters."
            } else {
                "This team has no issues."
            };
            Some(message(text.to_owned()).fg(theme.muted))
        }
        _ => None,
    };
    if let Some(empty) = empty {
        frame.render_widget(empty, inner);
        return;
    }

    my_issues::render_groups(view, frame, inner, team_issues.list(), focus);
}
