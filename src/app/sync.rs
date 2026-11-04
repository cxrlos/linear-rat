//! Keeping the store fresh (ADR 0003): when to poll and reconcile, how their
//! results merge, and what the sync indicator says. Pure: the time comes in
//! with [`Event::Tick`](super::Event::Tick).

use std::time::Duration;

use jiff::{SignedDuration, Timestamp};

use super::{Effect, Load, LoadError, State, Update};
use crate::config::SyncSettings;
use crate::linear::{AssignedIssues, ClientError, TeamChanges};

/// How far before a team's watermark each poll reaches back, so a change
/// that Linear stamped a little late is not missed.
pub const OVERLAP: SignedDuration = SignedDuration::from_mins(2);

/// Without a key press for this long, the viewer is idle and polls slow down.
pub const IDLE_AFTER: SignedDuration = SignedDuration::from_mins(2);

/// How often the store's issues are checked for ones that disappeared.
pub const RECONCILE_EVERY: SignedDuration = SignedDuration::from_hours(3);

/// One team to poll, and the time its delta starts after: its watermark less
/// [`OVERLAP`], or `None` for a team that has none yet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TeamPoll {
    pub team_id: String,
    pub since: Option<Timestamp>,
}

/// Why a poll or reconciliation did not finish.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SyncFailure {
    /// Linear's rate limit held it back.
    Throttled,
    /// Anything else; the message says what.
    Failed(String),
}

impl From<&ClientError> for SyncFailure {
    fn from(error: &ClientError) -> Self {
        match error {
            ClientError::RateLimited => Self::Throttled,
            other => Self::Failed(super::load::chain(other)),
        }
    }
}

/// What the sync indicator shows, and whether it calls for attention.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Indicator {
    pub text: String,
    /// Sync is failing or throttled.
    pub alert: bool,
}

/// How fresh the store is and what sync is doing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncStatus {
    /// The time of the latest tick; `None` before the first.
    now: Option<Timestamp>,
    /// When a poll last finished without a problem, by the local clock.
    last_synced: Option<Timestamp>,
    /// When the last poll of this session finished, with or without one.
    last_polled: Option<Timestamp>,
    last_reconciled: Option<Timestamp>,
    last_activity: Option<Timestamp>,
    focused: bool,
    polling: bool,
    reconciling: bool,
    /// This session's load of My Issues while the store shown came from the
    /// snapshot. Nothing polls until it confirms the viewer and workspace.
    refresh: Refresh,
    problem: Option<SyncFailure>,
}

/// Where this session's load of My Issues stands while the snapshot shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Refresh {
    /// Not painting from the snapshot, or its load came back.
    Done,
    /// The load is in flight.
    Loading,
    /// The load could not reach or ask Linear at this time (`None` before
    /// the first tick), and is tried again a poll interval later.
    Failed(Option<Timestamp>),
}

impl Default for SyncStatus {
    fn default() -> Self {
        Self {
            now: None,
            last_synced: None,
            last_polled: None,
            last_reconciled: None,
            last_activity: None,
            // A terminal that never reports focus is taken as focused.
            focused: true,
            polling: false,
            reconciling: false,
            refresh: Refresh::Done,
            problem: None,
        }
    }
}

impl SyncStatus {
    /// The status of a session painting from a snapshot that last synced at
    /// `synced_at`, until its own load comes back.
    pub fn from_snapshot(synced_at: Option<Timestamp>) -> Self {
        Self {
            last_synced: synced_at,
            refresh: Refresh::Loading,
            ..Self::default()
        }
    }

    /// Forgets every sync of a store that was thrown away.
    pub(super) fn forget_syncs(&mut self) {
        self.last_synced = None;
        self.last_polled = None;
        self.last_reconciled = None;
    }

    /// Whether the store came from the snapshot and is waiting for this
    /// session's load.
    pub fn is_refreshing(&self) -> bool {
        self.refresh != Refresh::Done
    }

    /// When the last poll finished without a problem, by the local clock.
    pub fn last_synced(&self) -> Option<Timestamp> {
        self.last_synced
    }

    /// The time between polls under `settings` now, or `None` while polling
    /// is paused because the terminal lost focus.
    pub fn poll_interval(&self, settings: &SyncSettings) -> Option<Duration> {
        if !self.focused && settings.pause_when_unfocused {
            return None;
        }
        let idle = match (self.now, self.last_activity) {
            (Some(now), Some(active)) => now.duration_since(active) >= IDLE_AFTER,
            _ => false,
        };
        Some(if idle {
            settings.idle_poll
        } else {
            settings.active_poll
        })
    }

    /// `synced 12s ago`, muted; accented with what is wrong when sync is
    /// failing or throttled. `None` before anything has synced or failed.
    pub fn indicator(&self) -> Option<Indicator> {
        let ago = match (self.now, self.last_synced) {
            (Some(now), Some(synced)) => Some(format!("synced {} ago", age(now, synced))),
            _ => None,
        };
        let problem = match &self.problem {
            None => {
                return ago.map(|text| Indicator { text, alert: false });
            }
            Some(SyncFailure::Throttled) => "throttled",
            Some(SyncFailure::Failed(_)) => "sync failing",
        };
        let text = match ago {
            Some(ago) => format!("{problem}, {ago}"),
            None => problem.to_owned(),
        };
        Some(Indicator { text, alert: true })
    }

    /// Due a poll interval after the snapshot's load failed.
    fn load_retry_due(&self, settings: &SyncSettings) -> bool {
        let Refresh::Failed(failed_at) = self.refresh else {
            return false;
        };
        let (Some(now), Some(interval)) = (self.now, self.poll_interval(settings)) else {
            return false;
        };
        failed_at.is_none_or(|failed_at| {
            SignedDuration::try_from(interval)
                .is_ok_and(|interval| now.duration_since(failed_at) >= interval)
        })
    }

    fn poll_due(&self, settings: &SyncSettings) -> bool {
        let Some(now) = self.now else {
            return false;
        };
        let Some(interval) = self.poll_interval(settings) else {
            return false;
        };
        !self.polling
            && self.last_polled.is_none_or(|polled| {
                SignedDuration::try_from(interval)
                    .is_ok_and(|interval| now.duration_since(polled) >= interval)
            })
    }

    /// Due after the first poll and every [`RECONCILE_EVERY`], but only
    /// while sync is healthy, so a failing or throttled Linear is not asked
    /// for more; a failed reconciliation is retried after the next good poll.
    fn reconcile_due(&self, settings: &SyncSettings) -> bool {
        let (Some(now), Some(_)) = (self.now, self.last_polled) else {
            return false;
        };
        !self.reconciling
            && self.problem.is_none()
            && self.poll_interval(settings).is_some()
            && self
                .last_reconciled
                .is_none_or(|reconciled| now.duration_since(reconciled) >= RECONCILE_EVERY)
    }

    /// Records `failure`, and returns whether it is a new one worth a notice.
    fn fail(&mut self, failure: SyncFailure) -> bool {
        let new = self.problem.is_none();
        self.problem = Some(failure);
        new
    }
}

/// How long ago `then` was, as `12s`, `3m`, `5h` or `2d`.
fn age(now: Timestamp, then: Timestamp) -> String {
    let seconds = now.duration_since(then).as_secs().max(0);
    match seconds {
        0..60 => format!("{seconds}s"),
        60..3600 => format!("{}m", seconds / 60),
        3600..86_400 => format!("{}h", seconds / 3600),
        _ => format!("{}d", seconds / 86_400),
    }
}

/// Whether the store is ready to sync: a viewer is known and the screen is
/// not waiting on, or showing the failure of, the first load.
fn can_sync(state: &State) -> bool {
    state.load == Load::Loaded
        && state.store.viewer().is_some()
        && state.sync_status.refresh == Refresh::Done
}

/// The load, poll or reconciliation that is due now, if any, marked as
/// started.
pub(super) fn due(state: &mut State) -> Vec<Effect> {
    if state.sync_status.load_retry_due(&state.sync) {
        return vec![retry_load(state)];
    }
    if !can_sync(state) {
        return Vec::new();
    }
    if state.sync_status.poll_due(&state.sync) {
        return vec![start_poll(state)];
    }
    if state.sync_status.reconcile_due(&state.sync) {
        state.sync_status.reconciling = true;
        // A stand-in for an issue being created has no id Linear knows.
        let ids = state
            .store
            .issues()
            .map(|issue| issue.id.clone())
            .filter(|id| !super::is_local(id))
            .collect();
        return vec![Effect::Reconcile(ids)];
    }
    Vec::new()
}

impl State {
    /// Whether a refresh can start now: the snapshot's failed load can be
    /// tried again, or the store is ready to sync and no poll is in flight.
    pub fn can_refresh(&self) -> bool {
        match self.sync_status.refresh {
            Refresh::Failed(_) => true,
            Refresh::Loading => false,
            Refresh::Done => can_sync(self) && !self.sync_status.polling,
        }
    }
}

/// `r`: polls now, focused or not, unless a poll is already in flight. While
/// the snapshot's load has failed, tries that load again instead.
pub(super) fn refresh(state: &mut State) -> Update {
    if !state.can_refresh() {
        return Update::default();
    }
    let effect = match state.sync_status.refresh {
        Refresh::Failed(_) => retry_load(state),
        Refresh::Loading | Refresh::Done => start_poll(state),
    };
    Update {
        redraw: false,
        effects: vec![effect],
    }
}

fn retry_load(state: &mut State) -> Effect {
    state.sync_status.refresh = Refresh::Loading;
    Effect::LoadMyIssues
}

fn start_poll(state: &mut State) -> Effect {
    state.sync_status.polling = true;
    let teams = state
        .store
        .teams()
        .map(|team| TeamPoll {
            team_id: team.id.clone(),
            since: state
                .store
                .watermark(&team.id)
                .and_then(|watermark| watermark.checked_sub(OVERLAP).ok()),
        })
        .collect();
    Effect::Poll(teams)
}

/// Runs `change` on the state and redraws if the indicator changed.
fn redraw_if_indicator_changes(state: &mut State, change: impl FnOnce(&mut State)) -> bool {
    let before = state.sync_status.indicator();
    change(state);
    state.sync_status.indicator() != before
}

pub(super) fn tick(state: &mut State, now: Timestamp) -> Update {
    let redraw = redraw_if_indicator_changes(state, |state| {
        let status = &mut state.sync_status;
        status.now = Some(now);
        status.last_activity.get_or_insert(now);
    });
    Update {
        redraw,
        effects: due(state),
    }
}

/// A key press or focus coming back: the viewer is active.
pub(super) fn activity(state: &mut State) {
    let status = &mut state.sync_status;
    if let Some(now) = status.now {
        status.last_activity = Some(now);
    }
}

pub(super) fn focus(state: &mut State, focused: bool) -> Update {
    state.sync_status.focused = focused;
    if !focused {
        return Update::default();
    }
    activity(state);
    Update {
        redraw: false,
        effects: due(state),
    }
}

pub(super) fn throttled(state: &mut State) -> Update {
    let redraw = redraw_if_indicator_changes(state, |state| {
        state.sync_status.problem = Some(SyncFailure::Throttled);
    });
    Update {
        redraw,
        effects: Vec::new(),
    }
}

/// Merges a poll's changes, newer data winning, and records how it went.
pub(super) fn polled(
    state: &mut State,
    changes: &[TeamChanges],
    failure: Option<SyncFailure>,
) -> Update {
    let mut changed = false;
    let mut moved = false;
    for team in changes {
        let before = state.store.watermark(&team.team_id);
        changed |= state.store.apply(team);
        moved |= state.store.watermark(&team.team_id) != before;
    }
    let indicator_changed = redraw_if_indicator_changes(state, |state| {
        let status = &mut state.sync_status;
        status.polling = false;
        status.last_polled = status.now;
        match failure {
            None => {
                status.last_synced = status.now;
                status.problem = None;
            }
            Some(failure) => note_failure(state, failure),
        }
    });
    if changed {
        state.rebuild_my_issues();
    }
    let mut effects = Vec::new();
    if changed || moved {
        effects.push(Effect::SaveSnapshot);
    }
    effects.extend(due(state));
    Update {
        redraw: changed || indicator_changed,
        effects,
    }
}

/// Drops what the reconciliation found gone, and records how it went.
pub(super) fn reconciled(
    state: &mut State,
    checked: &[String],
    result: Result<Vec<String>, SyncFailure>,
) -> Update {
    state.sync_status.reconciling = false;
    let changed = match result {
        Ok(existing) => {
            state.sync_status.last_reconciled = state.sync_status.now;
            state.store.retain_existing(checked, &existing)
        }
        Err(failure) => {
            let redraw = redraw_if_indicator_changes(state, |state| note_failure(state, failure));
            return Update {
                redraw,
                effects: Vec::new(),
            };
        }
    };
    if !changed {
        return Update::default();
    }
    state.rebuild_my_issues();
    Update {
        redraw: true,
        effects: vec![Effect::SaveSnapshot],
    }
}

/// Marks sync as failing or throttled. The first failure in a row also says
/// why in the notice, unless one is showing; later ones only keep the
/// indicator accented.
fn note_failure(state: &mut State, failure: SyncFailure) {
    let message = match &failure {
        SyncFailure::Failed(message) => Some(format!("Sync failed: {message}")),
        SyncFailure::Throttled => None,
    };
    if state.sync_status.fail(failure) && state.notice.is_none() {
        state.notice = message;
    }
}

/// My Issues loaded: it fills the screen, and polling may start.
pub(super) fn loaded(state: &mut State, loaded: &AssignedIssues) -> Update {
    state.show_assigned(loaded);
    state.sync_status.refresh = Refresh::Done;
    Update {
        redraw: true,
        effects: due(state),
    }
}

/// My Issues did not load. While the snapshot is showing, a failure to reach
/// or ask Linear keeps it on screen, marks sync failing and tries the load
/// again a poll interval later; anything else, such as a rejected key, shows
/// its error screen.
pub(super) fn load_failed(state: &mut State, error: LoadError) -> Update {
    let refreshing = state.sync_status.is_refreshing();
    match error {
        LoadError::Unreachable(message) | LoadError::Failed(message) if refreshing => {
            state.sync_status.refresh = Refresh::Failed(state.sync_status.now);
            note_failure(state, SyncFailure::Failed(message));
        }
        error => {
            state.sync_status.refresh = Refresh::Done;
            state.load = Load::Failed(error);
        }
    }
    Update {
        redraw: true,
        effects: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: Timestamp = Timestamp::constant(1_793_696_400, 0);

    fn at(seconds: i64) -> Timestamp {
        NOW.checked_add(SignedDuration::from_secs(seconds)).unwrap()
    }

    #[test]
    fn ages_are_seconds_then_minutes_hours_and_days() {
        assert_eq!(age(at(59), NOW), "59s");
        assert_eq!(age(at(60), NOW), "1m");
        assert_eq!(age(at(3 * 3600 + 5), NOW), "3h");
        assert_eq!(age(at(2 * 86_400), NOW), "2d");
        assert_eq!(age(NOW, at(5)), "0s");
    }

    #[test]
    fn an_idle_viewer_polls_at_the_idle_interval_and_an_unfocused_one_not_at_all() {
        let settings = SyncSettings::default();
        let mut status = SyncStatus {
            now: Some(at(119)),
            last_activity: Some(NOW),
            ..SyncStatus::default()
        };
        assert_eq!(status.poll_interval(&settings), Some(settings.active_poll));

        status.now = Some(at(120));
        assert_eq!(status.poll_interval(&settings), Some(settings.idle_poll));

        status.focused = false;
        assert_eq!(status.poll_interval(&settings), None);

        let keep_polling = SyncSettings {
            pause_when_unfocused: false,
            ..settings
        };
        assert_eq!(
            status.poll_interval(&keep_polling),
            Some(keep_polling.idle_poll)
        );
    }
}
