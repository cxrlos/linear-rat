use std::future::{self, Future};
use std::io;
use std::time::Duration;

use crossterm::event::{Event as TerminalEvent, KeyCode, KeyEvent, KeyModifiers};
use futures_util::stream::{self, Stream, StreamExt};
use linear_rat::app::{App, State};
use linear_rat::auth::KeySource;
use linear_rat::linear::model::{Viewer, Workspace};
use linear_rat::linear::{AssignedIssues, ClientError, FakeClient, LinearClient};
use linear_rat::runtime;
use ratatui::backend::TestBackend;

fn press(c: char) -> io::Result<TerminalEvent> {
    Ok(TerminalEvent::Key(KeyEvent::new(
        KeyCode::Char(c),
        KeyModifiers::NONE,
    )))
}

/// Terminal input that delivers `events` and then stays open, as a real
/// terminal does.
fn input(
    events: Vec<io::Result<TerminalEvent>>,
) -> impl Stream<Item = io::Result<TerminalEvent>> + Unpin {
    stream::iter(events).chain(stream::pending())
}

async fn run(
    events: Vec<io::Result<TerminalEvent>>,
    shutdown: impl Future<Output = ()>,
) -> Option<anyhow::Result<()>> {
    let mut app = App::new(TestBackend::new(80, 24), State::default()).unwrap();
    let running = runtime::run(&mut app, Some(&FakeClient), input(events), shutdown);
    tokio::time::timeout(Duration::from_millis(200), running)
        .await
        .ok()
}

/// Runs `app` with `client` and no input until it settles, then hands back
/// what it shows.
async fn settle(mut app: App<TestBackend>, client: Option<&impl LinearClient>) -> String {
    let running = runtime::run(&mut app, client, input(Vec::new()), future::pending());
    let stopped = tokio::time::timeout(Duration::from_millis(200), running).await;
    assert!(stopped.is_err(), "the runtime stopped: {stopped:?}");
    let buffer = app.backend().buffer();
    let width = usize::from(buffer.area.width);
    buffer
        .content()
        .chunks(width)
        .map(|row| row.iter().map(|cell| cell.symbol()).collect::<String>())
        .collect::<Vec<_>>()
        .join("\n")
}

/// A client for a key Linear does not accept.
struct RejectingClient;

impl LinearClient for RejectingClient {
    async fn viewer(&self) -> Result<(Viewer, Workspace), ClientError> {
        Err(ClientError::InvalidKey)
    }

    async fn assigned_issues(&self) -> Result<AssignedIssues, ClientError> {
        Err(ClientError::InvalidKey)
    }
}

#[tokio::test]
async fn stops_when_q_is_pressed() {
    let result = run(vec![press('x'), press('q')], future::pending()).await;

    assert!(matches!(result, Some(Ok(()))));
}

#[tokio::test]
async fn keeps_running_until_asked_to_stop() {
    let result = run(vec![press('x')], future::pending()).await;

    assert!(result.is_none());
}

#[tokio::test]
async fn stops_on_a_shutdown_signal() {
    let result = run(Vec::new(), future::ready(())).await;

    assert!(matches!(result, Some(Ok(()))));
}

#[tokio::test]
async fn stops_with_an_error_when_terminal_input_fails() {
    let broken = Err(io::Error::other("tty gone"));

    let result = run(vec![broken], future::pending()).await;

    let error = result.unwrap().unwrap_err();
    assert!(format!("{error:#}").contains("tty gone"));
}

#[tokio::test]
async fn loads_my_issues_through_the_client_at_start() {
    let app = App::new(TestBackend::new(80, 24), State::loading(None)).unwrap();

    let shown = settle(app, Some(&FakeClient)).await;

    let demo = App::new(TestBackend::new(80, 24), State::demo()).unwrap();
    let demo_shown = settle(demo, None::<&FakeClient>).await;
    assert_eq!(shown, demo_shown);
}

#[tokio::test]
async fn a_rejected_key_ends_on_its_error_screen_naming_the_source() {
    let app = App::new(
        TestBackend::new(80, 24),
        State::loading(Some(KeySource::Env)),
    )
    .unwrap();

    let shown = settle(app, Some(&RejectingClient)).await;

    assert!(shown.contains("API key rejected"), "{shown}");
    assert!(shown.contains("from LINEAR_API_KEY"), "{shown}");
}

#[tokio::test]
async fn q_quits_while_the_load_is_still_in_flight() {
    struct SlowClient;
    impl LinearClient for SlowClient {
        async fn viewer(&self) -> Result<(Viewer, Workspace), ClientError> {
            future::pending().await
        }
        async fn assigned_issues(&self) -> Result<AssignedIssues, ClientError> {
            future::pending().await
        }
    }
    let mut app = App::new(TestBackend::new(80, 24), State::loading(None)).unwrap();

    let running = runtime::run(
        &mut app,
        Some(&SlowClient),
        input(vec![press('q')]),
        future::pending(),
    );
    let result = tokio::time::timeout(Duration::from_millis(200), running).await;

    assert!(matches!(result, Ok(Ok(()))), "{result:?}");
}
