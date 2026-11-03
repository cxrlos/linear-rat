//! The runtime loop: turns terminal input into events, feeds them to the app
//! and carries out the effects it asks for.

use std::future::Future;
use std::io;
use std::ops::ControlFlow;

use anyhow::Context;
use crossterm::event::Event as TerminalEvent;
use futures_util::{Stream, StreamExt};
use ratatui::backend::Backend;

use crate::app::{App, Effect, Event};

/// Runs until the app asks to quit, `shutdown` resolves, or terminal input
/// ends. Waits without drawing while nothing happens.
pub async fn run<B, I, S>(mut app: App<B>, mut input: I, shutdown: S) -> anyhow::Result<()>
where
    B: Backend,
    B::Error: Send + Sync + 'static,
    I: Stream<Item = io::Result<TerminalEvent>> + Unpin,
    S: Future<Output = ()>,
{
    tokio::pin!(shutdown);
    loop {
        let terminal_event = tokio::select! {
            () = &mut shutdown => {
                tracing::info!("shutdown signal received");
                return Ok(());
            }
            next = input.next() => match next {
                Some(event) => event.context("reading terminal input")?,
                None => return Ok(()),
            },
        };
        let Some(event) = from_terminal(terminal_event) else {
            continue;
        };
        for effect in app.handle(event).context("drawing the screen")? {
            if execute(effect).is_break() {
                return Ok(());
            }
        }
    }
}

fn execute(effect: Effect) -> ControlFlow<()> {
    match effect {
        Effect::Quit => ControlFlow::Break(()),
    }
}

fn from_terminal(event: TerminalEvent) -> Option<Event> {
    match event {
        TerminalEvent::Key(key) => Some(Event::Key(key)),
        TerminalEvent::Resize(columns, rows) => Some(Event::Resize { columns, rows }),
        _ => None,
    }
}
