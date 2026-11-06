//! The Analytics screen's state: the view and range the viewer picked, the
//! team the Team view shows, the time zone weeks are counted in, what has
//! been asked of Linear and how far the view is scrolled. Its data lives in
//! the store.

use std::collections::BTreeMap;

use jiff::Timestamp;
use jiff::tz::TimeZone;

use super::modal::{Choice, Purpose};
use super::{Effect, Modal, State, Update};
use crate::analytics::{Period, WeekRange};
use crate::linear::model::Team;
use crate::linear::{AnalyticsIssues, AnalyticsWindow, TeamAnalyticsIssues};

/// Which analytics the screen shows.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum AnalyticsView {
    /// The viewer's own work.
    #[default]
    Personal,
    /// One of the viewer's teams.
    Team,
}

#[derive(Debug, Clone)]
pub struct AnalyticsScreen {
    /// `None` until the viewer picks one: `[analytics] default-range-weeks`.
    range: Option<WeekRange>,
    time_zone: TimeZone,
    /// The earliest moment asked for, so a range already asked for is not
    /// asked for again while its load is in flight.
    requested_since: Option<Timestamp>,
    in_flight: usize,
    /// Why the last load failed, until one succeeds.
    error: Option<String>,
    scroll: u16,
    view: AnalyticsView,
    /// The team the Team view shows; `None` shows the first of the viewer's
    /// teams.
    team_id: Option<String>,
    /// Per team id, the earliest moment asked for, as `requested_since`.
    team_requested_since: BTreeMap<String, Timestamp>,
    team_in_flight: usize,
    /// Why the last load of a team failed, by team id, until one succeeds.
    team_errors: BTreeMap<String, String>,
    /// Whether the teams the viewer administers were asked for, and the ask
    /// has not failed.
    administrable_asked: bool,
}

impl Default for AnalyticsScreen {
    fn default() -> Self {
        Self {
            range: None,
            time_zone: TimeZone::UTC,
            requested_since: None,
            in_flight: 0,
            error: None,
            scroll: 0,
            view: AnalyticsView::default(),
            team_id: None,
            team_requested_since: BTreeMap::new(),
            team_in_flight: 0,
            team_errors: BTreeMap::new(),
            administrable_asked: false,
        }
    }
}

impl AnalyticsScreen {
    /// The time zone weeks and days are counted in.
    pub fn time_zone(&self) -> &TimeZone {
        &self.time_zone
    }

    /// Whether a load the view shown waits on is in flight.
    pub fn is_loading(&self) -> bool {
        match self.view {
            AnalyticsView::Personal => self.in_flight > 0,
            AnalyticsView::Team => self.in_flight + self.team_in_flight > 0,
        }
    }

    /// Why the last personal analytics load failed, if no load has succeeded
    /// since.
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    /// Why the last team analytics load of `team_id` failed, if no load of
    /// it has succeeded since.
    pub fn team_error(&self, team_id: &str) -> Option<&str> {
        self.team_errors.get(team_id).map(String::as_str)
    }

    /// How many rows the view is scrolled down.
    pub fn scroll(&self) -> u16 {
        self.scroll
    }

    /// The view shown.
    pub fn view(&self) -> AnalyticsView {
        self.view
    }
}

impl State {
    /// This state counting analytics weeks and days in `time_zone`.
    pub fn with_time_zone(mut self, time_zone: TimeZone) -> Self {
        self.analytics_screen.time_zone = time_zone;
        self
    }

    /// The range the Analytics screen shows.
    pub fn analytics_range(&self) -> WeekRange {
        self.analytics_screen
            .range
            .unwrap_or(self.analytics.default_range)
    }

    /// The team the Team view shows: the one picked, or the first of the
    /// viewer's teams. `None` until the viewer's teams have loaded.
    pub fn analytics_team(&self) -> Option<&Team> {
        match &self.analytics_screen.team_id {
            Some(id) => self.store.team(id),
            None => self.store.viewer_teams().next(),
        }
    }

    /// The weeks the Analytics screen shows, measured at the store's
    /// `as_of`, or `None` before anything has loaded.
    pub fn analytics_period(&self) -> Option<Period> {
        let now = self.store.as_of()?;
        Some(Period::new(
            now,
            self.analytics_screen.time_zone.clone(),
            self.analytics_range(),
        ))
    }
}

/// Asks for the part of the shown range that is neither loaded nor asked
/// for: the whole range up to now at first, then only the older weeks a
/// longer range adds. The Team view also asks for its team's part, and once
/// for the teams the viewer administers.
pub(super) fn request_missing(state: &mut State) -> Vec<Effect> {
    let mut effects = request_missing_personal(state);
    if state.analytics_screen.view == AnalyticsView::Team {
        effects.extend(request_missing_team(state));
    }
    effects
}

fn request_missing_personal(state: &mut State) -> Vec<Effect> {
    let Some(since) = state.analytics_period().as_ref().and_then(start_of) else {
        return Vec::new();
    };
    let screen = &mut state.analytics_screen;
    let covered = screen.requested_since.or(state.store.analytics_since());
    let Some(window) = missing(since, covered) else {
        return Vec::new();
    };
    screen.requested_since = Some(since);
    screen.in_flight += 1;
    vec![Effect::LoadAnalytics(window)]
}

fn request_missing_team(state: &mut State) -> Vec<Effect> {
    let mut effects = Vec::new();
    if state.store.administrable_teams().is_none() && !state.analytics_screen.administrable_asked {
        state.analytics_screen.administrable_asked = true;
        effects.push(Effect::LoadAdministrableTeams);
    }
    let Some(team_id) = state.analytics_team().map(|team| team.id.clone()) else {
        return effects;
    };
    let Some(since) = state.analytics_period().as_ref().and_then(start_of) else {
        return effects;
    };
    let covered = state
        .analytics_screen
        .team_requested_since
        .get(&team_id)
        .copied()
        .or(state.store.team_analytics_since(&team_id));
    let Some(window) = missing(since, covered) else {
        return effects;
    };
    let screen = &mut state.analytics_screen;
    screen.team_requested_since.insert(team_id.clone(), since);
    screen.team_in_flight += 1;
    effects.push(Effect::LoadTeamAnalytics { team_id, window });
    effects
}

/// The window from `since` that `covered` (how far back loads reach up to
/// now) leaves out, if any.
fn missing(since: Timestamp, covered: Option<Timestamp>) -> Option<AnalyticsWindow> {
    match covered {
        None => Some(AnalyticsWindow { since, until: None }),
        Some(covered) if since < covered => Some(AnalyticsWindow {
            since,
            until: Some(covered),
        }),
        Some(_) => None,
    }
}

/// Takes in the result of loading `window`. A failure lets the next request
/// ask for the window again.
pub(super) fn loaded(
    state: &mut State,
    window: AnalyticsWindow,
    result: Result<AnalyticsIssues, String>,
) {
    let screen = &mut state.analytics_screen;
    screen.in_flight = screen.in_flight.saturating_sub(1);
    match result {
        Ok(loaded) => {
            screen.error = None;
            if state.notice.is_none() {
                state.notice = loaded.problems.first().cloned();
            }
            state.store.merge_analytics(window, &loaded);
        }
        Err(error) => {
            screen.error = Some(error);
            screen.requested_since = state.store.analytics_since();
        }
    }
}

/// Takes in the result of loading team `team_id` over `window`. A failure
/// lets the next request ask for the window again.
pub(super) fn team_loaded(
    state: &mut State,
    team_id: &str,
    window: AnalyticsWindow,
    result: Result<TeamAnalyticsIssues, String>,
) {
    let screen = &mut state.analytics_screen;
    screen.team_in_flight = screen.team_in_flight.saturating_sub(1);
    match result {
        Ok(loaded) => {
            screen.team_errors.remove(team_id);
            if state.notice.is_none() {
                state.notice = loaded.problems.first().cloned();
            }
            state.store.merge_team_analytics(window, &loaded);
        }
        Err(error) => {
            screen.team_errors.insert(team_id.to_owned(), error);
            match state.store.team_analytics_since(team_id) {
                Some(since) => screen
                    .team_requested_since
                    .insert(team_id.to_owned(), since),
                None => screen.team_requested_since.remove(team_id),
            };
        }
    }
}

/// Takes in the teams the viewer administers. A failure leaves the
/// per-person breakdown out and lets the next request ask again.
pub(super) fn administrable_loaded(state: &mut State, result: Result<Vec<String>, String>) {
    match result {
        Ok(team_ids) => state.store.set_administrable_teams(team_ids),
        Err(error) => {
            state.analytics_screen.administrable_asked = false;
            state.notice = Some(error);
        }
    }
}

/// Shows the other view from the top, and asks for what it is missing.
pub(super) fn toggle_view(state: &mut State) -> Update {
    let screen = &mut state.analytics_screen;
    screen.view = match screen.view {
        AnalyticsView::Personal => AnalyticsView::Team,
        AnalyticsView::Team => AnalyticsView::Personal,
    };
    screen.scroll = 0;
    Update {
        redraw: true,
        effects: request_missing(state),
    }
}

/// The team switcher of the Team view, the team shown selected.
pub(super) fn team_choice(state: &State) -> Option<Choice> {
    let shown = state.analytics_team().map(|team| &team.id);
    super::team_issues::team_switcher(state, Purpose::AnalyticsTeam, shown)
}

/// Closes the team switcher on top on its selected team, or the `nth`
/// visible one when given, and shows that team from the top, asking for
/// what it is missing. Nothing happens when there is no such team.
pub(super) fn pick_team(state: &mut State, nth: Option<usize>) -> Update {
    let Some(Modal::Choice(choice)) = state.modals.top() else {
        return Update::default();
    };
    let Some(picked) = choice.chosen(nth).cloned() else {
        return Update::default();
    };
    state.modals.pop();
    let shown = state.analytics_team().map(|team| team.id.clone());
    let Some(team_id) = picked.filter(|id| Some(id) != shown.as_ref()) else {
        return Update::redraw();
    };
    state.analytics_screen.team_id = Some(team_id);
    state.analytics_screen.scroll = 0;
    Update {
        redraw: true,
        effects: request_missing(state),
    }
}

/// Shows the next shorter or longer range, and asks for what it is missing.
pub(super) fn change_range(state: &mut State, longer: bool) -> Update {
    const RANGES: [WeekRange; 3] = [WeekRange::Four, WeekRange::Twelve, WeekRange::TwentySix];
    let range = state.analytics_range();
    let index = RANGES.iter().position(|&r| r == range).unwrap_or(0);
    let next = if longer {
        RANGES.get(index + 1)
    } else {
        index.checked_sub(1).and_then(|before| RANGES.get(before))
    };
    let Some(&next) = next else {
        return Update::default();
    };
    state.analytics_screen.range = Some(next);
    Update {
        redraw: true,
        effects: request_missing(state),
    }
}

/// Scrolls the view one row down or up, no further than `max`.
pub(super) fn scroll(state: &mut State, down: bool, max: u16) -> Update {
    let scroll = &mut state.analytics_screen.scroll;
    let before = *scroll;
    *scroll = if down {
        scroll.saturating_add(1).min(max)
    } else {
        scroll.saturating_sub(1)
    };
    Update {
        redraw: *scroll != before,
        effects: Vec::new(),
    }
}

/// Keeps the view scrolled no further than `max`, after its size or content
/// changed.
pub(super) fn clamp_scroll(state: &mut State, max: u16) {
    let scroll = &mut state.analytics_screen.scroll;
    *scroll = (*scroll).min(max);
}

/// Midnight at the start of the period's first Monday.
fn start_of(period: &Period) -> Option<Timestamp> {
    let first = period.week_starts().first()?;
    let midnight = first.to_zoned(period.time_zone().clone()).ok()?;
    Some(midnight.timestamp())
}
