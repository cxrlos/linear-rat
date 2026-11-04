use std::future::Future;
use std::io;

use anyhow::Context;
use clap::Parser;
use crossterm::event::EventStream;
use jiff::tz::TimeZone;
use linear_rat::app::{App, Load, LoadError, State};
use linear_rat::auth::{self, Credentials, OsKeyring, WorkspaceFile};
use linear_rat::cli::{AuthCommand, Cli, Command};
use linear_rat::config::{self, Loaded};
use linear_rat::linear::{FakeClient, HttpClient, LinearClient};
use linear_rat::snapshot::SnapshotCache;
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
    tracing::info!(
        theme = loaded.config.theme,
        icons = loaded.config.ui.icons.name(),
        "config loaded"
    );
    let shutdown =
        terminal::shutdown_signal().context("listening for SIGINT, SIGTERM and SIGHUP")?;
    let result = if cli.demo {
        let state = State::loading(None)
            .with_config(&loaded)
            .with_time_zone(TimeZone::system());
        run_tui(state, Some(&FakeClient), None, shutdown).await
    } else {
        let (state, client) = connect();
        let snapshots = snapshot_cache();
        let state = state
            .with_config(&loaded)
            .with_time_zone(TimeZone::system());
        let state = from_snapshot(state, snapshots.as_ref());
        run_tui(state, client.as_ref(), snapshots.as_ref(), shutdown).await
    };

    match &result {
        Ok(()) => tracing::info!("quit"),
        Err(error) => tracing::error!("{error:#}"),
    }
    result
}

/// The state to start in and the client to load with: the key from
/// `LINEAR_API_KEY` or the OS keyring, or, when there is no usable key, the
/// error screen that says why and no client.
fn connect() -> (State, Option<HttpClient>) {
    let resolved = WorkspaceFile::in_state_dir()
        .map_err(|error| LoadError::KeyUnusable(format!("{error:#}")))
        .and_then(|workspace_file| {
            let credentials =
                Credentials::new(OsKeyring, workspace_file, std::env::var(auth::KEY_VAR).ok());
            credentials
                .resolve()
                .map_err(|error| LoadError::from(&error))
        })
        .and_then(|(key, source)| {
            let client = HttpClient::new(&key).map_err(|error| LoadError::from(&error))?;
            Ok((client, source))
        });
    match resolved {
        Ok((client, source)) => {
            tracing::info!(source = source.kind(), "using the API key");
            (State::loading(Some(source)), Some(client))
        }
        Err(error) => {
            tracing::warn!(?error, "no usable API key");
            (State::failed(error), None)
        }
    }
}

/// The snapshot cache under the XDG cache dir, or `None`, logged, when there
/// is no home directory to put it in.
fn snapshot_cache() -> Option<SnapshotCache> {
    SnapshotCache::in_cache_dir()
        .inspect_err(|error| tracing::warn!("no snapshot cache: {error:#}"))
        .ok()
}

/// `state`, painting the last session's snapshot while it waits for its own
/// load. A state that cannot load, for want of a usable key, stays as it is.
fn from_snapshot(state: State, snapshots: Option<&SnapshotCache>) -> State {
    if state.load != Load::Loading {
        return state;
    }
    match snapshots.and_then(SnapshotCache::load) {
        Some(saved) => {
            tracing::info!("painting from the snapshot");
            state.with_snapshot(saved.store, saved.synced_at)
        }
        None => state,
    }
}

async fn run_tui<C: LinearClient>(
    state: State,
    client: Option<&C>,
    snapshots: Option<&SnapshotCache>,
    shutdown: impl Future<Output = ()>,
) -> anyhow::Result<()> {
    let guard = TerminalGuard::enter().context("preparing the terminal")?;
    let result = match App::new(CrosstermBackend::new(io::stdout()), state) {
        Ok(mut app) => {
            runtime::run(&mut app, client, snapshots, EventStream::new(), shutdown).await
        }
        Err(error) => Err(error).context("drawing the first frame"),
    };
    drop(guard);
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
