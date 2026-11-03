//! The runtime loop: turns terminal input into events, feeds them to the app
//! and carries out the effects it asks for.

use std::future::{self, Future};
use std::io;
use std::ops::ControlFlow;
use std::pin::Pin;

use anyhow::Context;
use crossterm::event::Event as TerminalEvent;
use futures_util::{Stream, StreamExt};
use ratatui::backend::Backend;

use crate::app::{App, Effect, Event, LoadError};
use crate::linear::LinearClient;

/// A load in flight, finishing with the event that carries its result.
type Pending<'a> = Pin<Box<dyn Future<Output = Event> + 'a>>;

/// Runs until the app asks to quit, `shutdown` resolves, or terminal input
/// ends. Waits without drawing while nothing happens. Loads go through
/// `client`, which is `None` when there is no usable API key; the app then
/// already shows why. Input keeps being handled while a load is in flight.
pub async fn run<B, C, I, S>(
    app: &mut App<B>,
    client: Option<&C>,
    mut input: I,
    shutdown: S,
) -> anyhow::Result<()>
where
    B: Backend,
    B::Error: Send + Sync + 'static,
    C: LinearClient,
    I: Stream<Item = io::Result<TerminalEvent>> + Unpin,
    S: Future<Output = ()>,
{
    tokio::pin!(shutdown);
    let mut pending: Option<Pending<'_>> = None;
    let mut next = Some(Event::Start);
    loop {
        if let Some(event) = next.take() {
            for effect in app.handle(event).context("drawing the screen")? {
                if execute(effect, client, &mut pending).is_break() {
                    return Ok(());
                }
            }
        }
        next = tokio::select! {
            () = &mut shutdown => {
                tracing::info!("shutdown signal received");
                return Ok(());
            }
            event = finish(&mut pending) => {
                pending = None;
                Some(event)
            }
            terminal_event = input.next() => match terminal_event {
                Some(event) => from_terminal(event.context("reading terminal input")?),
                None => return Ok(()),
            },
        };
    }
}

fn execute<'a, C: LinearClient>(
    effect: Effect,
    client: Option<&'a C>,
    pending: &mut Option<Pending<'a>>,
) -> ControlFlow<()> {
    match effect {
        Effect::Quit => return ControlFlow::Break(()),
        Effect::LoadMyIssues => *pending = Some(Box::pin(load_my_issues(client))),
    }
    ControlFlow::Continue(())
}

async fn load_my_issues<C: LinearClient>(client: Option<&C>) -> Event {
    let Some(client) = client else {
        return Event::MyIssuesLoaded(Err(LoadError::NoKey));
    };
    let result = client
        .assigned_issues()
        .await
        .map_err(|error| LoadError::from(&error));
    match &result {
        Ok(loaded) => tracing::info!(issues = loaded.issues.len(), "loaded My Issues"),
        Err(error) => tracing::warn!(?error, "loading My Issues failed"),
    }
    Event::MyIssuesLoaded(result)
}

/// Waits for the load in flight, or forever when there is none.
async fn finish(pending: &mut Option<Pending<'_>>) -> Event {
    match pending {
        Some(load) => load.await,
        None => future::pending().await,
    }
}

fn from_terminal(event: TerminalEvent) -> Option<Event> {
    match event {
        TerminalEvent::Key(key) => Some(Event::Key(key)),
        TerminalEvent::Resize(columns, rows) => Some(Event::Resize { columns, rows }),
        _ => None,
    }
}
