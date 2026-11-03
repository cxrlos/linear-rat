use crate::keys::Key;

/// Popups and overlays, drawn over the screen in order. The top one gets keys
/// first, and closing it reveals the one below.
#[derive(Debug, Default)]
pub struct ModalStack(Vec<Modal>);

/// One overlay, such as a picker, the palette or the cheatsheet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Modal {
    /// Every action and its keys for the context below.
    Cheatsheet,
    /// The keys that can follow `prefix`, the start of a chord.
    WhichKey { prefix: Vec<Key> },
}

impl ModalStack {
    pub fn top(&self) -> Option<&Modal> {
        self.0.last()
    }

    pub fn push(&mut self, modal: Modal) {
        self.0.push(modal);
    }

    pub fn pop(&mut self) -> Option<Modal> {
        self.0.pop()
    }

    /// Bottom first, the order they are drawn in.
    pub fn iter(&self) -> impl DoubleEndedIterator<Item = &Modal> {
        self.0.iter()
    }
}
