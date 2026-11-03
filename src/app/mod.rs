//! The app driver: owns the state and the terminal, applies events and redraws.

mod effect;
mod event;
mod modal;
mod state;
mod update;

use ratatui::Terminal;
use ratatui::backend::Backend;

pub use effect::Effect;
pub use event::Event;
pub use modal::{Modal, ModalStack};
pub use state::{Screen, State};
pub use update::{Update, update};

use crate::ui;

pub struct App<B: Backend> {
    state: State,
    terminal: Terminal<B>,
    frames_drawn: u64,
}

impl<B: Backend> App<B> {
    /// Builds the app on `backend` and draws the first frame.
    pub fn new(backend: B) -> Result<Self, B::Error> {
        let mut app = Self {
            state: State::default(),
            terminal: Terminal::new(backend)?,
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
