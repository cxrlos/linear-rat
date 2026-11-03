use std::fs;
use std::io::{self, ErrorKind};
use std::path::PathBuf;

use anyhow::Context;
use etcetera::{BaseStrategy, choose_base_strategy};

use super::AuthError;

/// Remembers the URL key of the workspace whose key `lrat auth login` stored,
/// because that URL key is the keyring account and lrat needs it to find the
/// entry again. It holds no secret.
#[derive(Debug, Clone)]
pub struct WorkspaceFile {
    path: PathBuf,
}

impl WorkspaceFile {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    /// `$XDG_STATE_HOME/lrat/keyring-workspace` (by default
    /// `~/.local/state/lrat`, on macOS too), next to the logs.
    pub fn in_state_dir() -> anyhow::Result<Self> {
        let base = choose_base_strategy().context("finding the home directory")?;
        let state = base
            .state_dir()
            .context("finding the XDG state directory")?;
        Ok(Self::new(state.join("lrat").join("keyring-workspace")))
    }

    /// The remembered URL key, or `None` when nothing was stored.
    pub fn read(&self) -> Result<Option<String>, AuthError> {
        match fs::read_to_string(&self.path) {
            Ok(text) => Ok(Some(text.trim().to_owned()).filter(|key| !key.is_empty())),
            Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
            Err(error) => Err(self.error("reading", error)),
        }
    }

    pub fn write(&self, url_key: &str) -> Result<(), AuthError> {
        if let Some(dir) = self.path.parent() {
            fs::create_dir_all(dir)
                .map_err(|error| self.error("creating the directory of", error))?;
        }
        fs::write(&self.path, format!("{url_key}\n")).map_err(|error| self.error("writing", error))
    }

    pub fn remove(&self) -> Result<(), AuthError> {
        match fs::remove_file(&self.path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
            Err(error) => Err(self.error("removing", error)),
        }
    }

    fn error(&self, action: &'static str, source: io::Error) -> AuthError {
        AuthError::WorkspaceFile {
            action,
            path: self.path.display().to_string(),
            source,
        }
    }
}
