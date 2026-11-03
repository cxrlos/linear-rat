use super::ModalStack;
use crate::config::Loaded;
use crate::keys::Keymap;

/// Everything the screen shows. Rendering reads only this.
#[derive(Debug, Default)]
pub struct State {
    pub screen: Screen,
    pub modals: ModalStack,
    pub keymap: Keymap,
    /// A message for the viewer above the bottom bar, such as a config
    /// error. The next key press clears it.
    pub notice: Option<String>,
}

impl State {
    /// The starting state with the keymap from `loaded`, and its first
    /// error, if any, as the notice.
    pub fn with_config(loaded: &Loaded) -> Self {
        let notice = loaded
            .errors
            .first()
            .map(|first| match loaded.errors.len() {
                1 => first.to_string(),
                n => format!("{first} (and {} more; see the log)", n - 1),
            });
        Self {
            keymap: Keymap::new(&loaded.config.keys),
            notice,
            ..Self::default()
        }
    }
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
