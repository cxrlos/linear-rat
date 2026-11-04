//! The Analytics screen's state: the range the viewer picked, the time zone
//! weeks are counted in, what has been asked of Linear and how far the view
//! is scrolled. Its data lives in the store.

use jiff::Timestamp;
use jiff::tz::TimeZone;

use super::{Effect, State, Update};
use crate::analytics::{Period, WeekRange};
use crate::linear::{AnalyticsIssues, AnalyticsWindow};

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
        }
    }
}

impl AnalyticsScreen {
    /// The time zone weeks and days are counted in.
    pub fn time_zone(&self) -> &TimeZone {
        &self.time_zone
    }

    /// Whether a load is in flight.
    pub fn is_loading(&self) -> bool {
        self.in_flight > 0
    }

    /// Why the last load failed, if no load has succeeded since.
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    /// How many rows the view is scrolled down.
    pub fn scroll(&self) -> u16 {
        self.scroll
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
/// longer range adds.
pub(super) fn request_missing(state: &mut State) -> Vec<Effect> {
    let Some(since) = state.analytics_period().as_ref().and_then(start_of) else {
        return Vec::new();
    };
    let screen = &mut state.analytics_screen;
    let covered = screen.requested_since.or(state.store.analytics_since());
    let window = match covered {
        None => AnalyticsWindow { since, until: None },
        Some(covered) if since < covered => AnalyticsWindow {
            since,
            until: Some(covered),
        },
        Some(_) => return Vec::new(),
    };
    screen.requested_since = Some(since);
    screen.in_flight += 1;
    vec![Effect::LoadAnalytics(window)]
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
