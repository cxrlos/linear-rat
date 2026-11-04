//! The app driver: owns the state and the terminal, applies events and redraws.

mod action;
mod effect;
mod event;
mod load;
mod modal;
mod my_issues;
mod state;
mod store;
mod sync;
mod update;

use ratatui::Terminal;
use ratatui::backend::Backend;

pub use action::{Action, Copyable};
pub use effect::Effect;
pub use event::Event;
pub use load::{Load, LoadError};
pub use modal::{Modal, ModalStack};
pub use my_issues::{Group, MyIssues, Pane};
pub use state::{Screen, State};
pub use store::Store;
pub use sync::{
    IDLE_AFTER, Indicator, OVERLAP, RECONCILE_EVERY, SyncFailure, SyncStatus, TeamPoll,
};
pub use update::{Update, perform, update};

use crate::ui;

pub struct App<B: Backend> {
    state: State,
    terminal: Terminal<B>,
    frames_drawn: u64,
}

impl<B: Backend> App<B> {
    /// Builds the app on `backend` showing `state`, and draws the first frame.
    pub fn new(backend: B, mut state: State) -> Result<Self, B::Error> {
        let terminal = Terminal::new(backend)?;
        state.viewport = terminal.size()?;
        let mut app = Self {
            state,
            terminal,
            frames_drawn: 0,
        };
        app.draw()?;
        Ok(app)
    }

    /// Applies `event`, redraws if the screen changed, and returns the
    /// effects the runtime must carry out.
    pub fn handle(&mut self, event: Event) -> Result<Vec<Effect>, B::Error> {
        let Update { redraw, effects } = update(&mut self.state, event);
        if redraw {
            self.draw()?;
        }
        Ok(effects)
    }

    /// What the screen shows, for effects that read it, such as saving the
    /// snapshot.
    pub fn state(&self) -> &State {
        &self.state
    }

    /// How many frames have been drawn since [`App::new`].
    pub fn frames_drawn(&self) -> u64 {
        self.frames_drawn
    }

    pub fn backend(&self) -> &B {
        self.terminal.backend()
    }

    pub fn backend_mut(&mut self) -> &mut B {
        self.terminal.backend_mut()
    }

    fn draw(&mut self) -> Result<(), B::Error> {
        self.terminal.draw(|frame| ui::render(&self.state, frame))?;
        self.frames_drawn += 1;
        Ok(())
    }
}
