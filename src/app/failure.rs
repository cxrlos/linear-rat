//! Failures the viewer is told about, and the error log that keeps the most
//! recent of them for the messages overlay.

use std::collections::VecDeque;
use std::fmt;

use jiff::Timestamp;

use super::load::chain;
use crate::linear::{ClientError, ErrorKind};

/// How many failures the error log keeps; older ones drop off.
pub const ERROR_LOG_CAP: usize = 100;

/// Why something failed: its kind, the action that failed, and the full
/// message, which is what the notice shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Failure {
    pub kind: ErrorKind,
    /// What lrat was doing, as `load the team's issues` or `edit ENG-12`.
    pub action: String,
    pub message: String,
}

impl Failure {
    pub fn new(kind: ErrorKind, action: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            kind,
            action: action.into(),
            message: message.into(),
        }
    }

    /// `error`, which ended `action`, with each of its causes as the
    /// message.
    pub fn from_client(action: impl Into<String>, error: &ClientError) -> Self {
        Self::new(error.kind(), action, chain(error))
    }

    /// This failure with `context` before its message, as `context:
    /// message`.
    #[must_use]
    pub fn context(mut self, context: &str) -> Self {
        self.message = format!("{context}: {}", self.message);
        self
    }
}

impl fmt::Display for Failure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

/// One failure in the error log, with when it was recorded: the time of the
/// latest tick, `None` before the first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoggedFailure {
    pub at: Option<Timestamp>,
    pub failure: Failure,
}

/// The most recent failures of this session, up to [`ERROR_LOG_CAP`].
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ErrorLog(VecDeque<LoggedFailure>);

impl ErrorLog {
    pub fn record(&mut self, at: Option<Timestamp>, failure: Failure) {
        if self.0.len() == ERROR_LOG_CAP {
            self.0.pop_front();
        }
        self.0.push_back(LoggedFailure { at, failure });
    }

    pub fn newest_first(&self) -> impl Iterator<Item = &LoggedFailure> {
        self.0.iter().rev()
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_log_keeps_only_the_newest_failures() {
        let mut log = ErrorLog::default();
        for n in 0..ERROR_LOG_CAP + 5 {
            log.record(None, Failure::new(ErrorKind::Other, "count", n.to_string()));
        }

        assert_eq!(log.len(), ERROR_LOG_CAP);
        let newest: Vec<&str> = log
            .newest_first()
            .map(|logged| logged.failure.message.as_str())
            .take(2)
            .collect();
        assert_eq!(newest, ["104", "103"]);
        assert_eq!(
            log.newest_first()
                .last()
                .map(|l| l.failure.message.as_str()),
            Some("5")
        );
    }
}
