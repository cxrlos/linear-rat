/// A request to the outside world, returned by [`update`](super::update) and
/// carried out by the runtime.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effect {
    Quit,
    /// Load the viewer's open assigned issues; the result comes back as
    /// [`Event::MyIssuesLoaded`](super::Event::MyIssuesLoaded).
    LoadMyIssues,
}
