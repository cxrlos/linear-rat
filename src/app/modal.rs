use super::create::CreateForm;
use super::edit::EditPicker;
use super::links::Links;
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
    /// The detail of one issue from the store, over the screen.
    Issue { id: String },
    /// A list to pick one entry from.
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
            Modal::Picker(_) | Modal::Palette(_) | Modal::Links(_) | Modal::Create(_) => true,
            Modal::Search(search) => matches!(search.phase, Phase::Typing { .. }),
            Modal::Cheatsheet { .. }
            | Modal::WhichKey { .. }
            | Modal::Issue { .. }
            | Modal::Choice(_) => false,
        }
    }

    /// The text field typed keys go to, if this overlay has one taking text
    /// now. Asking for the palette's resets its highlight to the best match,
    /// since what matches changes with the text.
    pub fn text_field(&mut self) -> Option<&mut String> {
        match self {
            Modal::Picker(picker) => Some(picker.list.query_mut()),
            Modal::Links(links) => Some(links.list.query_mut()),
            Modal::Palette(palette) => {
                palette.selected = 0;
                Some(&mut palette.query)
            }
            Modal::Search(search) => search.text_field(),
            Modal::Create(form) => form.text_field(),
            Modal::Cheatsheet { .. }
            | Modal::WhichKey { .. }
            | Modal::Issue { .. }
            | Modal::Choice(_) => None,
        }
    }
}

/// A list to pick one entry from, such as the team switcher, with the entry
/// the cursor is on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Choice {
    pub purpose: Purpose,
    pub entries: Vec<ChoiceEntry>,
    pub cursor: usize,
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

/// One entry of a [`Choice`]: the id it picks and what it shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChoiceEntry {
    /// `None` for "Any", which clears a filter.
    pub id: Option<String>,
    pub label: String,
}

impl Choice {
    /// What the popup is titled.
    pub fn title(&self) -> String {
        match self.purpose {
            Purpose::Team | Purpose::AnalyticsTeam => "Team".to_owned(),
            Purpose::Filter(field) => format!("Filter by {}", field.name().to_lowercase()),
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
