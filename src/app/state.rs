use std::collections::BTreeMap;
use std::fmt::Display;

use ratatui::layout::Size;

use jiff::Timestamp;

use super::{Detail, Load, LoadError, ModalStack, MyIssues, Store, SyncStatus};
use crate::auth::KeySource;
use crate::config::{AnalyticsSettings, Loaded, SyncSettings, UiSettings};
use crate::keys::Keymap;
use crate::linear::AssignedIssues;
use crate::linear::model::Issue;
use crate::ui;
use crate::ui::theme::Theme;

/// Everything the screen shows. Rendering reads only this.
#[derive(Debug, Default)]
pub struct State {
    pub screen: Screen,
    pub modals: ModalStack,
    pub keymap: Keymap,
    /// A message for the viewer above the bottom bar, such as a config
    /// error. The next key press clears it.
    pub notice: Option<String>,
    /// Everything loaded from Linear. Screens are built from it.
    pub store: Store,
    /// Why Linear rejected the last edit of an issue, by issue id, shown on
    /// its row until the next edit of that issue.
    pub row_errors: BTreeMap<String, String>,
    pub my_issues: MyIssues,
    /// The focused issue's detail: its scroll and which details have loaded.
    pub detail: Detail,
    /// Whether My Issues is still loading, and why it failed.
    pub load: Load,
    /// Where the API key in use came from, so a rejected key's error screen
    /// can name it. `None` under `--demo`.
    pub key_source: Option<KeySource>,
    /// The terminal size, for actions that move by a page.
    pub viewport: Size,
    /// The colors every screen draws with.
    pub theme: Theme,
    /// `[ui]`: the icon set and the wide-layout breakpoint.
    pub ui: UiSettings,
    /// `[sync]`: the poll intervals and whether to pause unfocused.
    pub sync: SyncSettings,
    /// How fresh the store is and what sync is doing.
    pub sync_status: SyncStatus,
    /// `[analytics]`, kept for the Analytics screen, which does not read it
    /// yet.
    pub analytics: AnalyticsSettings,
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
        // A store from another workspace or viewer, as a snapshot of the
        // last session can be, is thrown away rather than mixed in.
        let other_viewer = self
            .store
            .viewer()
            .is_some_and(|viewer| viewer.id != loaded.viewer.id);
        let other_workspace = self
            .store
            .workspace()
            .is_some_and(|workspace| workspace.id != loaded.workspace.id);
        if other_viewer || other_workspace {
            self.store = Store::default();
            self.sync_status.forget_syncs();
        }
        self.store.merge(loaded);
        self.rebuild_my_issues();
        self.load = Load::Loaded;
    }

    /// This state painting `store` from the snapshot of the last session,
    /// which last synced at `synced_at`, until its own load comes back.
    pub fn with_snapshot(self, store: Store, synced_at: Option<Timestamp>) -> Self {
        let mut state = Self {
            store,
            load: Load::Loaded,
            sync_status: SyncStatus::from_snapshot(synced_at),
            ..self
        };
        state.rebuild_my_issues();
        state
    }

    /// Builds My Issues again from the store, for example after a merge,
    /// keeping the focused issue, the pane and the scroll. When the focused
    /// issue left, the focus stays at its place in the list.
    pub fn rebuild_my_issues(&mut self) {
        let Some(viewer) = self.store.viewer() else {
            return;
        };
        let as_of = self.store.as_of().unwrap_or_default();
        let issues: Vec<_> = self.store.issues().cloned().collect();
        let cycles: Vec<_> = self.store.cycles().cloned().collect();
        let labels: Vec<_> = self.store.labels().cloned().collect();
        let members: Vec<_> = self.store.members().cloned().collect();
        let teams: Vec<_> = self.store.teams().cloned().collect();
        let mut rebuilt = MyIssues::new(viewer, &issues, &cycles, &labels, &members, &teams, as_of);
        let rows = ui::list_rows(self, self.viewport);
        rebuilt.keep_place_of(&self.my_issues, rows);
        self.my_issues = rebuilt;
    }

    /// The issue the current screen has focused, if it shows issues and has
    /// any.
    pub fn focused_issue(&self) -> Option<&Issue> {
        match self.screen {
            Screen::MyIssues => self.my_issues.focused_issue(),
            Screen::TeamIssues | Screen::Analytics => None,
        }
    }

    /// This state with the keymap, theme and settings from `loaded`, and its
    /// first error, if any, as the notice.
    pub fn with_config(self, loaded: &Loaded) -> Self {
        Self {
            keymap: Keymap::new(&loaded.config.keys),
            notice: summary(&loaded.errors),
            theme: loaded.theme.clone(),
            ui: loaded.config.ui,
            sync: loaded.config.sync,
            analytics: loaded.config.analytics,
            ..self
        }
    }
}

/// The first of `messages` and how many more there are, or `None` when there
/// are none.
pub(super) fn summary(messages: &[impl Display]) -> Option<String> {
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
