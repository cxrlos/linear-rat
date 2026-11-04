use super::edit::EditPicker;
use crate::keys::Key;

/// Popups and overlays, drawn over the screen in order. The top one gets keys
/// first, and closing it reveals the one below.
#[derive(Debug, Default)]
pub struct ModalStack(Vec<Modal>);

/// One overlay, such as a picker, the palette or the cheatsheet.
#[derive(Debug, Clone, PartialEq)]
pub enum Modal {
    /// Every action and its keys for the context below, scrolled down by
    /// `scroll` rows when it is taller than the screen.
    Cheatsheet { scroll: u16 },
    /// The keys that can follow `prefix`, the start of a chord.
    WhichKey { prefix: Vec<Key> },
    /// A list to choose a new value for one field of an issue from.
    Picker(EditPicker),
}

impl Modal {
    /// The text field typed keys go to, for an overlay that has one.
    pub fn text_field(&mut self) -> Option<&mut String> {
        match self {
            Modal::Picker(picker) => Some(picker.list.query_mut()),
            Modal::Cheatsheet { .. } | Modal::WhichKey { .. } => None,
        }
    }
}

impl ModalStack {
    pub fn top(&self) -> Option<&Modal> {
        self.0.last()
    }

    pub fn top_mut(&mut self) -> Option<&mut Modal> {
        self.0.last_mut()
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

    pub fn iter_mut(&mut self) -> impl DoubleEndedIterator<Item = &mut Modal> {
        self.0.iter_mut()
    }
}
