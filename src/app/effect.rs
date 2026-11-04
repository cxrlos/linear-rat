use super::TeamPoll;

/// A request to the outside world, returned by [`update`](super::update) and
/// carried out by the runtime.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effect {
    Quit,
    /// Load the viewer's open assigned issues; the result comes back as
    /// [`Event::MyIssuesLoaded`](super::Event::MyIssuesLoaded).
    LoadMyIssues,
    /// Put this text on the system clipboard.
    Copy(String),
    /// Open this URL in the viewer's browser.
    OpenUrl(String),
    /// Ask each team for its issues changed since its watermark, one team
    /// after another; the result comes back as
    /// [`Event::Polled`](super::Event::Polled).
    Poll(Vec<TeamPoll>),
    /// Ask which of these issue ids still exist; the result comes back as
    /// [`Event::Reconciled`](super::Event::Reconciled).
    Reconcile(Vec<String>),
    /// Write the store to the snapshot cache.
    SaveSnapshot,
}
