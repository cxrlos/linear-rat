//! A filterable list to choose one entry from, or in multi-select mode to
//! mark any number of entries in. Typing narrows it, the selection moves
//! through what is left, and the first nine entries left can be picked (or
//! toggled) by number. Generic over what an entry stands for.

use std::collections::BTreeSet;

/// The entries, what the viewer typed, and which entry is selected.
#[derive(Debug, Clone, PartialEq)]
pub struct Picker<T> {
    entries: Vec<Entry<T>>,
    query: String,
    /// An index into `entries`. When the query hides it, the first entry
    /// left is selected instead.
    selected: usize,
    /// The entry the field holds now, marked in the list.
    current: Option<usize>,
    note: Option<Note>,
    /// The indices of the marked entries in multi-select mode, `None` in
    /// single-select mode.
    marked: Option<BTreeSet<usize>>,
}

/// One choice: what the list shows and filters on, and what it stands for.
#[derive(Debug, Clone, PartialEq)]
pub struct Entry<T> {
    pub label: String,
    pub value: T,
}

/// A line under the entries about what is not in the list yet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Note {
    /// More entries are on their way.
    Loading,
    /// They could not be loaded, and why.
    Failed(String),
}

impl<T> Picker<T> {
    /// A picker over `entries` with nothing typed. The `current` entry is
    /// marked and selected first.
    pub fn new(entries: Vec<Entry<T>>, current: Option<usize>) -> Self {
        Self {
            entries,
            query: String::new(),
            selected: current.unwrap_or(0),
            current,
            note: None,
            marked: None,
        }
    }

    /// A multi-select picker over `entries` with the entries at `marked`
    /// marked and the first entry selected.
    pub fn multi(entries: Vec<Entry<T>>, marked: impl IntoIterator<Item = usize>) -> Self {
        let mut picker = Self::new(entries, None);
        picker.marked = Some(BTreeSet::new());
        picker.set_marked(marked);
        picker
    }

    /// Whether entries are marked and confirmed together rather than chosen
    /// one at a time.
    pub fn is_multi(&self) -> bool {
        self.marked.is_some()
    }

    /// Marks exactly the entries at `marked`, in multi-select mode.
    pub fn set_marked(&mut self, marked: impl IntoIterator<Item = usize>) {
        let count = self.entries.len();
        if let Some(set) = &mut self.marked {
            *set = marked.into_iter().filter(|&index| index < count).collect();
        }
    }

    /// Whether `index` among all entries is marked.
    pub fn is_marked(&self, index: usize) -> bool {
        self.marked.as_ref().is_some_and(|set| set.contains(&index))
    }

    /// The values of the marked entries, in entry order.
    pub fn marked(&self) -> Vec<&T> {
        self.entries
            .iter()
            .enumerate()
            .filter(|(index, _)| self.is_marked(*index))
            .map(|(_, entry)| &entry.value)
            .collect()
    }

    /// Marks or unmarks the selected entry. Returns whether it did, which a
    /// single-select picker never does.
    pub fn toggle_selected(&mut self) -> bool {
        let Some(position) = self.selected_position() else {
            return false;
        };
        let index = self.visible()[position].0;
        self.toggle(index)
    }

    /// Marks or unmarks the `n`th visible entry, counting from 1, and
    /// selects it.
    pub fn toggle_nth(&mut self, n: usize) -> bool {
        let Some(index) = n
            .checked_sub(1)
            .and_then(|position| self.visible().get(position).map(|(index, _)| *index))
        else {
            return false;
        };
        let toggled = self.toggle(index);
        if toggled {
            self.selected = index;
        }
        toggled
    }

    fn toggle(&mut self, index: usize) -> bool {
        let Some(set) = &mut self.marked else {
            return false;
        };
        if !set.remove(&index) {
            set.insert(index);
        }
        true
    }

    /// The values of every entry, in order.
    pub fn values(&self) -> impl Iterator<Item = &T> {
        self.entries.iter().map(|entry| &entry.value)
    }

    /// Replaces the entries, keeping what was typed. The selection goes to
    /// the `current` entry. A multi-select picker unmarks every entry; mark
    /// the new ones with [`Picker::set_marked`].
    pub fn set_entries(&mut self, entries: Vec<Entry<T>>, current: Option<usize>) {
        self.entries = entries;
        self.selected = current.unwrap_or(0);
        self.current = current;
        if let Some(set) = &mut self.marked {
            set.clear();
        }
    }

    /// Selects the entry at `index` among all entries, if there is one.
    pub fn select(&mut self, index: usize) {
        if index < self.entries.len() {
            self.selected = index;
        }
    }

    pub fn note(&self) -> Option<&Note> {
        self.note.as_ref()
    }

    pub fn set_note(&mut self, note: Option<Note>) {
        self.note = note;
    }

    /// What the viewer typed. The text-entry hook edits it in place.
    pub fn query(&self) -> &str {
        &self.query
    }

    pub fn query_mut(&mut self) -> &mut String {
        &mut self.query
    }

    /// The entries whose label contains every word typed, ignoring case, in
    /// their order, with their index among all entries.
    pub fn visible(&self) -> Vec<(usize, &Entry<T>)> {
        let words: Vec<String> = self
            .query
            .split_whitespace()
            .map(str::to_lowercase)
            .collect();
        self.entries
            .iter()
            .enumerate()
            .filter(|(_, entry)| {
                let label = entry.label.to_lowercase();
                words.iter().all(|word| label.contains(word))
            })
            .collect()
    }

    /// Where the selection is among the visible entries, `None` when none
    /// are visible.
    pub fn selected_position(&self) -> Option<usize> {
        let visible = self.visible();
        if visible.is_empty() {
            return None;
        }
        Some(
            visible
                .iter()
                .position(|(index, _)| *index == self.selected)
                .unwrap_or(0),
        )
    }

    /// Whether `index` among all entries is the field's value now.
    pub fn is_current(&self, index: usize) -> bool {
        self.current == Some(index)
    }

    /// Moves the selection one visible entry down (`step` 1) or up (`step`
    /// -1), stopping at either end. Returns whether it moved.
    pub fn select_by(&mut self, step: isize) -> bool {
        let Some(position) = self.selected_position() else {
            return false;
        };
        let visible = self.visible();
        let last = visible.len() - 1;
        let target = position.saturating_add_signed(step).min(last);
        let before = visible[position].0;
        self.selected = visible[target].0;
        self.selected != before
    }

    /// The selected entry's value.
    pub fn selected(&self) -> Option<&T> {
        let position = self.selected_position()?;
        self.visible().get(position).map(|(_, entry)| &entry.value)
    }

    /// The value of the `n`th visible entry, counting from 1.
    pub fn nth(&self, n: usize) -> Option<&T> {
        let index = n.checked_sub(1)?;
        self.visible().get(index).map(|(_, entry)| &entry.value)
    }
}

impl<T: PartialEq> Picker<T> {
    /// Selects the first entry holding `value`. Returns whether one does.
    pub fn select_value(&mut self, value: &T) -> bool {
        match self.entries.iter().position(|entry| entry.value == *value) {
            Some(index) => {
                self.selected = index;
                true
            }
            None => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Entry, Picker};

    fn picker(labels: &[&str], current: Option<usize>) -> Picker<usize> {
        let entries = labels
            .iter()
            .enumerate()
            .map(|(value, label)| Entry {
                label: (*label).to_owned(),
                value,
            })
            .collect();
        Picker::new(entries, current)
    }

    #[test]
    fn every_typed_word_must_appear_ignoring_case() {
        let mut list = picker(&["In Review", "In Progress", "Review later"], None);

        list.query_mut().push_str("rev IN");

        let labels: Vec<&str> = list
            .visible()
            .iter()
            .map(|(_, e)| e.label.as_str())
            .collect();
        assert_eq!(labels, ["In Review"]);
    }

    #[test]
    fn numbers_count_the_entries_the_filter_leaves() {
        let mut list = picker(&["Todo", "In Progress", "Done"], None);

        list.query_mut().push('o');

        assert_eq!(list.nth(1), Some(&0));
        assert_eq!(list.nth(2), Some(&1));
        assert_eq!(list.nth(3), Some(&2));
        list.query_mut().push('n');
        assert_eq!(list.nth(1), Some(&2));
        assert_eq!(list.nth(2), None);
        assert_eq!(list.nth(0), None);
    }

    #[test]
    fn the_current_entry_starts_selected_and_a_hidden_selection_falls_to_the_first_left() {
        let mut list = picker(&["Low", "Medium", "High"], Some(2));
        assert_eq!(list.selected(), Some(&2));

        list.query_mut().push('m');

        assert_eq!(list.selected(), Some(&1));
        list.query_mut().push('x');
        assert_eq!(list.selected(), None);
        assert_eq!(list.selected_position(), None);
    }

    #[test]
    fn a_single_select_picker_marks_nothing_and_toggles_nothing() {
        let mut list = picker(&["a", "b"], None);

        assert!(!list.is_multi());
        assert!(!list.toggle_selected());
        assert!(!list.is_marked(0));
    }

    #[test]
    fn a_multi_select_picker_toggles_the_selected_and_the_numbered_entries() {
        let entries = ["bug", "feature", "brand"]
            .iter()
            .enumerate()
            .map(|(value, label)| Entry {
                label: (*label).to_owned(),
                value,
            })
            .collect();
        let mut list = Picker::multi(entries, [2]);
        assert!(list.is_multi());
        assert_eq!(list.marked(), [&2]);

        assert!(list.toggle_selected());
        list.query_mut().push('b');
        assert!(list.toggle_nth(2));
        assert!(!list.toggle_nth(3));

        assert_eq!(list.marked(), [&0]);
        assert!(list.is_marked(0));
        assert!(!list.is_marked(2));
    }

    #[test]
    fn the_selection_stops_at_either_end() {
        let mut list = picker(&["a", "b"], None);

        assert!(!list.select_by(-1));
        assert!(list.select_by(1));
        assert!(!list.select_by(1));
        assert_eq!(list.selected(), Some(&1));
    }
}
