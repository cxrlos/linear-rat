//! Rendering: a pure function of [`State`].

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout};
use ratatui::widgets::{Block, Tabs};

use crate::app::{Screen, State};

pub fn render(state: &State, frame: &mut Frame) {
    let [tabs_area, body_area] =
        Layout::vertical([Constraint::Length(1), Constraint::Fill(1)]).areas(frame.area());

    let selected = Screen::ALL
        .iter()
        .position(|&screen| screen == state.screen);
    let tabs = Tabs::new(Screen::ALL.map(Screen::title)).select(selected);
    frame.render_widget(tabs, tabs_area);
    frame.render_widget(Block::bordered().title(state.screen.title()), body_area);
}
