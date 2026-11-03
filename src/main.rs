use std::io;

use anyhow::Context;
use clap::Parser;
use crossterm::event::EventStream;
use linear_rat::app::{App, State};
use linear_rat::cli::Cli;
use linear_rat::terminal::{self, TerminalGuard};
use linear_rat::{logging, runtime};
use ratatui::backend::CrosstermBackend;

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    if !cli.demo {
        anyhow::bail!("connecting to a Linear workspace is not built yet; run `lrat --demo`");
    }
    let _log_guard = logging::init().context("setting up the log file")?;
    tracing::info!(version = env!("CARGO_PKG_VERSION"), "starting");

    let shutdown = terminal::shutdown_signal().context("listening for SIGTERM and SIGHUP")?;
    let guard = TerminalGuard::enter().context("preparing the terminal")?;
    let result = match App::new(CrosstermBackend::new(io::stdout()), State::default()) {
        Ok(app) => runtime::run(app, EventStream::new(), shutdown).await,
        Err(error) => Err(error).context("drawing the first frame"),
    };
    drop(guard);

    match &result {
        Ok(()) => tracing::info!("quit"),
        Err(error) => tracing::error!("{error:#}"),
    }
    result
}
