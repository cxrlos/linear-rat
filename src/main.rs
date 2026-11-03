use std::io;

use anyhow::Context;
use clap::Parser;
use crossterm::event::EventStream;
use linear_rat::app::{App, State};
use linear_rat::auth::{self, Credentials, OsKeyring, WorkspaceFile};
use linear_rat::cli::{AuthCommand, Cli, Command};
use linear_rat::config::{self, Loaded};
use linear_rat::linear::HttpClient;
use linear_rat::terminal::{self, TerminalGuard};
use linear_rat::{logging, runtime};
use ratatui::backend::CrosstermBackend;

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    if let Some(Command::Auth(command)) = cli.command {
        let _log_guard = logging::init().context("setting up the log file")?;
        return run_auth(command).await;
    }
    if !cli.demo {
        anyhow::bail!("connecting to a Linear workspace is not built yet; run `lrat --demo`");
    }
    let _log_guard = logging::init().context("setting up the log file")?;
    tracing::info!(version = env!("CARGO_PKG_VERSION"), "starting");

    let loaded = match config::path() {
        Ok(path) => config::load(&path),
        Err(error) => {
            tracing::warn!("{error:#}; using the default config");
            Loaded::default()
        }
    };
    for error in &loaded.errors {
        tracing::warn!(%error, "config problem");
    }
    let state = State::demo().with_config(&loaded);

    let shutdown =
        terminal::shutdown_signal().context("listening for SIGINT, SIGTERM and SIGHUP")?;
    let guard = TerminalGuard::enter().context("preparing the terminal")?;
    let result = match App::new(CrosstermBackend::new(io::stdout()), state) {
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

async fn run_auth(command: AuthCommand) -> anyhow::Result<()> {
    let credentials = Credentials::new(
        OsKeyring,
        WorkspaceFile::in_state_dir()?,
        std::env::var(auth::KEY_VAR).ok(),
    );
    let output = match command {
        AuthCommand::Login { stdin } => {
            let key = if stdin {
                auth::read_key(io::stdin().lock())?
            } else {
                auth::prompt_key()?
            };
            auth::login(&credentials, key, HttpClient::new).await?
        }
        AuthCommand::Status => auth::status(&credentials, HttpClient::new).await?,
        AuthCommand::Logout => auth::logout(&credentials)?,
    };
    println!("{output}");
    Ok(())
}
