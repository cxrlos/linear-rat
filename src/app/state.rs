use std::fmt::Display;

use ratatui::layout::Size;

use super::{Load, LoadError, ModalStack, MyIssues};
use crate::auth::KeySource;
use crate::config::Loaded;
use crate::keys::Keymap;
use crate::linear::AssignedIssues;
use crate::linear::model::Issue;

/// Everything the screen shows. Rendering reads only this.
#[derive(Debug, Default)]
pub struct State {
    pub screen: Screen,
    pub modals: ModalStack,
    pub keymap: Keymap,
    /// A message for the viewer above the bottom bar, such as a config
    /// error. The next key press clears it.
    pub notice: Option<String>,
    pub my_issues: MyIssues,
    /// Whether My Issues is still loading, and why it failed.
    pub load: Load,
    /// Where the API key in use came from, so a rejected key's error screen
    /// can name it. `None` under `--demo`.
    pub key_source: Option<KeySource>,
    /// The terminal size, for actions that move by a page.
    pub viewport: Size,
}

impl State {
    /// The app on the demo workspace with its issues already loaded, as
    /// `lrat --demo` shows it once the load is done.
    pub fn demo() -> Self {
        let mut state = Self::default();
        state.show_assigned(&AssignedIssues::demo());
        state
    }

    /// The app waiting for My Issues to load with the key from `key_source`.
    pub fn loading(key_source: Option<KeySource>) -> Self {
        Self {
            load: Load::Loading,
            key_source,
            ..Self::default()
        }
    }

    /// The app showing why My Issues cannot load.
    pub fn failed(error: LoadError) -> Self {
        Self {
            load: Load::Failed(error),
            ..Self::default()
        }
    }

    /// Fills My Issues with `loaded`, grouped as of when it was loaded. What
    /// Linear could not give becomes the notice, unless one is already
    /// showing.
    pub fn show_assigned(&mut self, loaded: &AssignedIssues) {
        if self.notice.is_none() {
            self.notice = summary(&loaded.problems);
        }
        self.my_issues = MyIssues::new(
            &loaded.viewer,
            &loaded.issues,
            &loaded.cycles,
            &loaded.labels,
            &loaded.members,
            &loaded.teams,
            loaded.as_of,
        );
        self.load = Load::Loaded;
    }

    /// The issue the current screen has focused, if it shows issues and has
    /// any.
    pub fn focused_issue(&self) -> Option<&Issue> {
        match self.screen {
            Screen::MyIssues => self.my_issues.focused_issue(),
            Screen::TeamIssues | Screen::Analytics => None,
        }
    }

    /// This state with the keymap from `loaded`, and its first error, if
    /// any, as the notice.
    pub fn with_config(self, loaded: &Loaded) -> Self {
        Self {
            keymap: Keymap::new(&loaded.config.keys),
            notice: summary(&loaded.errors),
            ..self
        }
    }
}

/// The first of `messages` and how many more there are, or `None` when there
/// are none.
fn summary(messages: &[impl Display]) -> Option<String> {
    messages.first().map(|first| match messages.len() {
        1 => first.to_string(),
        n => format!("{first} (and {} more; see the log)", n - 1),
    })
}

/// The top-level screens, in tab order.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    #[default]
    MyIssues,
    TeamIssues,
    Analytics,
}

impl Screen {
    pub const ALL: [Screen; 3] = [Screen::MyIssues, Screen::TeamIssues, Screen::Analytics];

    pub fn title(self) -> &'static str {
        match self {
            Screen::MyIssues => "My Issues",
            Screen::TeamIssues => "Team Issues",
            Screen::Analytics => "Analytics",
        }
    }
}
