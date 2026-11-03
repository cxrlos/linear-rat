/// Popups and overlays, drawn over the screen in order. The top one gets keys
/// first, and closing it reveals the one below.
#[derive(Debug, Default)]
pub struct ModalStack(Vec<Modal>);

/// One overlay, such as a picker, the palette or the cheatsheet. None exist
/// yet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Modal {}

impl ModalStack {
    pub fn top(&self) -> Option<&Modal> {
        self.0.last()
    }
}
