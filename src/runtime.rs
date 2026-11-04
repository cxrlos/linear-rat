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
use crate::{browser, clipboard};

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
                if execute(app, effect, client, &mut pending, &mut System)?.is_break() {
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

/// The world outside lrat that effects reach. Tests swap in a fake.
trait Outside {
    fn copy(&mut self, text: &str) -> anyhow::Result<()>;
    fn open(&mut self, url: &str) -> anyhow::Result<()>;
}

/// The terminal's clipboard and the viewer's browser.
struct System;

impl Outside for System {
    fn copy(&mut self, text: &str) -> anyhow::Result<()> {
        clipboard::copy(&mut io::stdout(), text).context("could not copy to the clipboard")?;
        match clipboard::tmux_problem() {
            Some(problem) => Err(anyhow::anyhow!(problem)),
            None => Ok(()),
        }
    }

    fn open(&mut self, url: &str) -> anyhow::Result<()> {
        browser::open(url)
    }
}

/// Carries out `effect`. A load starts in `pending` and finishes later; a
/// failure to copy or open becomes the notice rather than ending the loop.
fn execute<'a, B, C>(
    app: &mut App<B>,
    effect: Effect,
    client: Option<&'a C>,
    pending: &mut Option<Pending<'a>>,
    outside: &mut impl Outside,
) -> anyhow::Result<ControlFlow<()>>
where
    B: Backend,
    B::Error: Send + Sync + 'static,
    C: LinearClient,
{
    let outcome = match effect {
        Effect::Quit => return Ok(ControlFlow::Break(())),
        Effect::LoadMyIssues => {
            *pending = Some(Box::pin(load_my_issues(client)));
            Ok(())
        }
        Effect::Copy(text) => outside.copy(&text),
        Effect::OpenUrl(url) => outside.open(&url),
    };
    if let Err(error) = outcome {
        let failed = Event::EffectFailed(format!("{error:#}"));
        for effect in app.handle(failed).context("drawing the screen")? {
            if execute(app, effect, client, pending, outside)?.is_break() {
                return Ok(ControlFlow::Break(()));
            }
        }
    }
    Ok(ControlFlow::Continue(()))
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

#[cfg(test)]
mod tests {
    use ratatui::backend::TestBackend;

    use super::*;
    use crate::app::State;
    use crate::linear::FakeClient;

    /// Records what lrat asked for, and fails the requests it is told to.
    #[derive(Default)]
    struct Fake {
        copied: Vec<String>,
        opened: Vec<String>,
        failure: Option<&'static str>,
    }

    impl Outside for Fake {
        fn copy(&mut self, text: &str) -> anyhow::Result<()> {
            self.copied.push(text.to_owned());
            self.failure
                .map_or(Ok(()), |failure| Err(anyhow::anyhow!(failure)))
        }

        fn open(&mut self, url: &str) -> anyhow::Result<()> {
            self.opened.push(url.to_owned());
            self.failure
                .map_or(Ok(()), |failure| Err(anyhow::anyhow!(failure)))
        }
    }

    fn screen(app: &App<TestBackend>) -> String {
        let buffer = app.backend().buffer();
        (0..buffer.area.height)
            .map(|y| {
                (0..buffer.area.width)
                    .map(|x| buffer[(x, y)].symbol())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Carries out `effect` with no client and no load in flight.
    fn run_effect(
        app: &mut App<TestBackend>,
        effect: Effect,
        outside: &mut Fake,
    ) -> ControlFlow<()> {
        let mut pending = None;
        execute(app, effect, None::<&FakeClient>, &mut pending, outside).unwrap()
    }

    #[test]
    fn a_copy_goes_to_the_clipboard_and_the_loop_goes_on() {
        let mut app = App::new(TestBackend::new(80, 24), State::demo()).unwrap();
        let mut outside = Fake::default();

        let flow = run_effect(&mut app, Effect::Copy("DES-34".to_owned()), &mut outside);

        assert_eq!(flow, ControlFlow::Continue(()));
        assert_eq!(outside.copied, ["DES-34"]);
    }

    #[test]
    fn an_open_url_goes_to_the_browser() {
        let mut app = App::new(TestBackend::new(80, 24), State::demo()).unwrap();
        let mut outside = Fake::default();

        let url = "https://linear.app/squeakworks/issue/DES-34";
        let flow = run_effect(&mut app, Effect::OpenUrl(url.to_owned()), &mut outside);

        assert_eq!(flow, ControlFlow::Continue(()));
        assert_eq!(outside.opened, [url]);
    }

    #[test]
    fn a_failed_effect_becomes_the_notice_and_the_loop_goes_on() {
        let mut app = App::new(TestBackend::new(80, 24), State::demo()).unwrap();
        let mut outside = Fake {
            failure: Some("could not open the browser with xdg-open"),
            ..Fake::default()
        };

        let flow = run_effect(
            &mut app,
            Effect::OpenUrl("https://x".to_owned()),
            &mut outside,
        );

        assert_eq!(flow, ControlFlow::Continue(()));
        assert!(
            screen(&app).contains("could not open the browser with xdg-open"),
            "{}",
            screen(&app)
        );
    }

    #[test]
    fn quit_ends_the_loop_without_going_outside() {
        let mut app = App::new(TestBackend::new(80, 24), State::demo()).unwrap();
        let mut outside = Fake::default();

        let flow = run_effect(&mut app, Effect::Quit, &mut outside);

        assert_eq!(flow, ControlFlow::Break(()));
        assert!(outside.copied.is_empty() && outside.opened.is_empty());
    }
}
