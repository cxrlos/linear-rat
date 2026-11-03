use crossterm::event::KeyEvent;

use super::LoadError;
use crate::linear::AssignedIssues;

/// Something that happened, fed to [`update`](super::update).
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    /// The runtime is running; sent once, before any input.
    Start,
    Key(KeyEvent),
    Resize {
        columns: u16,
        rows: u16,
    },
    /// The result of [`Effect::LoadMyIssues`](super::Effect::LoadMyIssues).
    MyIssuesLoaded(Result<AssignedIssues, LoadError>),
}
