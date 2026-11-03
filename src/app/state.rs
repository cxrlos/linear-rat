use super::ModalStack;

/// Everything the screen shows. Rendering reads only this.
#[derive(Debug, Default)]
pub struct State {
    pub screen: Screen,
    pub modals: ModalStack,
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
