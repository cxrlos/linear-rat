use std::future::{self, Future};
use std::io;
use std::time::Duration;

use crossterm::event::{Event as TerminalEvent, KeyCode, KeyEvent, KeyModifiers};
use futures_util::stream::{self, Stream, StreamExt};
use linear_rat::app::App;
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
    let app = App::new(TestBackend::new(80, 24)).unwrap();
    let running = runtime::run(app, input(events), shutdown);
    tokio::time::timeout(Duration::from_millis(200), running)
        .await
        .ok()
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
