use super::create::CreateForm;
use super::edit::EditPicker;
use super::links::Links;
use super::picker::Picker;
use super::{FilterField, Palette, Phase, Search};
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
    /// Find an action or a loaded issue by typing.
    Palette(Palette),
    /// Search every issue in the workspace on Linear.
    Search(Search),
    /// A list to pick a team or a filter value from.
    Choice(Choice),
    /// A list of an issue's links to open one of.
    Links(Links),
    /// The form that creates an issue.
    Create(CreateForm),
}

impl Modal {
    /// Whether typed keys go to a text field rather than the keymap now.
    pub fn takes_text(&self) -> bool {
        match self {
            Modal::Picker(_)
            | Modal::Palette(_)
            | Modal::Links(_)
            | Modal::Create(_)
            | Modal::Choice(_) => true,
            Modal::Search(search) => !matches!(search.phase, Phase::Searching { .. }),
            Modal::Cheatsheet { .. } | Modal::WhichKey { .. } => false,
        }
    }

    /// The text field typed keys go to, if this overlay has one taking text
    /// now. Asking for the palette's resets its highlight to the best match,
    /// since what matches changes with the text.
    pub fn text_field(&mut self) -> Option<&mut String> {
        match self {
            Modal::Picker(picker) => Some(picker.list.query_mut()),
            Modal::Links(links) => Some(links.list.query_mut()),
            Modal::Choice(choice) => Some(choice.list.query_mut()),
            Modal::Palette(palette) => {
                palette.selected = 0;
                Some(&mut palette.query)
            }
            Modal::Search(search) => search.text_field(),
            Modal::Create(form) => form.text_field(),
            Modal::Cheatsheet { .. } | Modal::WhichKey { .. } => None,
        }
    }
}

/// A picker of teams or filter values, and what choosing one does. An entry
/// holds the id it picks, `None` for "Any", which clears a filter.
#[derive(Debug, Clone, PartialEq)]
pub struct Choice {
    pub purpose: Purpose,
    pub list: Picker<Option<String>>,
}

/// What picking an entry of a [`Choice`] does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Purpose {
    /// Show this team in Team Issues.
    Team,
    /// Filter Team Issues by this field.
    Filter(FilterField),
    /// Show this team in the Team view of Analytics.
    AnalyticsTeam,
}

impl Choice {
    /// What the popup is titled.
    pub fn title(&self) -> String {
        match self.purpose {
            Purpose::Team | Purpose::AnalyticsTeam => "Team".to_owned(),
            Purpose::Filter(field) => format!("Filter by {}", field.name().to_lowercase()),
        }
    }

    /// The id of the selected entry, or of the `nth` visible one when
    /// given, counting from 1. `None` when there is no such entry.
    pub fn chosen(&self, nth: Option<usize>) -> Option<&Option<String>> {
        match nth {
            Some(n) => self.list.nth(n),
            None => self.list.selected(),
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

    /// Keeps only the overlays `keep` says to.
    pub fn retain(&mut self, keep: impl FnMut(&Modal) -> bool) {
        self.0.retain(keep);
    }
}
