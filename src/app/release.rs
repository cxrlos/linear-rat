//! The release notice: when to check GitHub for a newer release, what the
//! answer shows, and skipping a release (ADR 0004).

use jiff::Timestamp;
use semver::Version;

use super::{Effect, State, Update};
use crate::release::{self, Channel, Record};

/// What lrat knows about newer releases, and whether it checks for them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Release {
    /// Whether this session checks GitHub. Off under `--demo`, in tests
    /// and when the viewer turned it off.
    check: bool,
    channel: Channel,
    running: Version,
    record: Record,
    in_flight: bool,
}

impl Default for Release {
    /// Never checks and knows of no release.
    fn default() -> Self {
        Self::new(false, Channel::default(), Record::default())
    }
}

impl Release {
    /// Checking GitHub when `check`, for an install through `channel`, and
    /// knowing what the cache's `record` says.
    pub fn new(check: bool, channel: Channel, record: Record) -> Self {
        Self {
            check,
            channel,
            running: release::running(),
            record,
            in_flight: false,
        }
    }

    /// The newer release to show a notice for: newer than the running
    /// version, not skipped, and on a channel that shows notices.
    pub fn notice(&self) -> Option<&Version> {
        let latest = self.record.latest.as_ref()?;
        let skipped = self
            .record
            .skipped
            .as_ref()
            .is_some_and(|skipped| skipped >= latest);
        (self.channel.shows_notice() && *latest > self.running && !skipped).then_some(latest)
    }

    /// How to upgrade the way lrat was installed.
    pub fn upgrade(&self) -> &'static str {
        self.channel.upgrade()
    }
}

/// At each tick, a check when one is due and none is in flight. It counts
/// as the day's check from when it starts, so a failed one is tried again
/// only a day later.
pub(super) fn tick(state: &mut State, now: Timestamp) -> Option<Effect> {
    let release = &mut state.release;
    if !release.check || release.in_flight || !release.record.due(now) {
        return None;
    }
    release.in_flight = true;
    release.record.checked_at = Some(now);
    Some(Effect::CheckRelease)
}

/// The answer to a check: the latest release, or `None` when the check
/// failed, which the runtime has logged and which shows nothing.
pub(super) fn checked(state: &mut State, latest: Option<Version>) -> Update {
    let release = &mut state.release;
    release.in_flight = false;
    let before = release.notice().cloned();
    if latest.is_some() {
        release.record.latest = latest;
    }
    Update {
        redraw: release.notice() != before.as_ref(),
        effects: vec![Effect::SaveRelease(release.record.clone())],
    }
}

/// Hides the notice until a release newer than the one it shows.
pub(super) fn skip(state: &mut State) -> Update {
    let release = &mut state.release;
    let Some(version) = release.notice().cloned() else {
        return Update::default();
    };
    release.record.skipped = Some(version);
    Update {
        redraw: true,
        effects: vec![Effect::SaveRelease(release.record.clone())],
    }
}

/// Opens the GitHub page of the release the notice shows.
pub(super) fn show_notes(state: &State) -> Update {
    match state.release.notice() {
        Some(version) => Update {
            redraw: false,
            effects: vec![Effect::OpenUrl(release::release_url(version))],
        },
        None => Update::default(),
    }
}
