use crossterm::event::KeyEvent;

/// Something that happened, fed to [`update`](super::update).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    Key(KeyEvent),
    Resize { columns: u16, rows: u16 },
}
