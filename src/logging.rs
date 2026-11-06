//! File logging under the XDG state dir. Nothing is ever logged to the
//! terminal, which belongs to the UI.

use std::fs;
use std::path::PathBuf;

use anyhow::{Context, anyhow};
use etcetera::{BaseStrategy, choose_base_strategy};
use tracing::Subscriber;
use tracing_appender::non_blocking::WorkerGuard;
use tracing_appender::rolling::{RollingFileAppender, Rotation};
use tracing_subscriber::EnvFilter;
use tracing_subscriber::filter::{FilterExt, LevelFilter, filter_fn};
use tracing_subscriber::fmt::MakeWriter;
use tracing_subscriber::prelude::*;

const LEVEL_VAR: &str = "LRAT_LOG";

/// Starts logging to `$XDG_STATE_HOME/lrat/lrat.<date>.log` (by default
/// `~/.local/state/lrat`, on macOS too) at the level in `LRAT_LOG`, `info` if
/// unset. Keep the returned guard alive: dropping it flushes and stops logging.
pub fn init() -> anyhow::Result<WorkerGuard> {
    let dir = log_dir()?;
    fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
    let appender = RollingFileAppender::builder()
        .rotation(Rotation::DAILY)
        .filename_prefix("lrat")
        .filename_suffix("log")
        .max_log_files(7)
        .build(&dir)
        .with_context(|| format!("opening a log file in {}", dir.display()))?;
    let (writer, guard) = tracing_appender::non_blocking(appender);

    let filter = EnvFilter::builder()
        .with_default_directive(LevelFilter::INFO.into())
        .with_env_var(LEVEL_VAR);
    let (level, invalid_level) = match filter.from_env() {
        Ok(level) => (level, None),
        Err(error) => (filter.parse_lossy(""), Some(error)),
    };
    subscriber(writer, level)
        .try_init()
        .map_err(|error| anyhow!(error))
        .context("starting the logger")?;
    if let Some(error) = invalid_level {
        tracing::warn!(%error, "ignoring {LEVEL_VAR}, logging at info");
    }
    Ok(guard)
}

/// The subscriber [`init`] installs: plain lines to `writer`, at `level`.
/// tungstenite's handshake is never logged, whatever `level` asks for: at
/// trace it writes the whole upgrade request, API key included.
pub fn subscriber<W>(writer: W, level: EnvFilter) -> impl Subscriber + Send + Sync
where
    W: for<'w> MakeWriter<'w> + Send + Sync + 'static,
{
    let no_handshake =
        filter_fn(|metadata| !metadata.target().starts_with("tungstenite::handshake"));
    tracing_subscriber::registry().with(
        tracing_subscriber::fmt::layer()
            .with_writer(writer)
            .with_ansi(false)
            .with_filter(level.and(no_handshake)),
    )
}

fn log_dir() -> anyhow::Result<PathBuf> {
    let base = choose_base_strategy().context("finding the home directory")?;
    let state = base
        .state_dir()
        .context("finding the XDG state directory")?;
    Ok(state.join("lrat"))
}
