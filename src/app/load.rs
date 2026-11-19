use std::error::Error;

use super::Failure;
use crate::auth::AuthError;
use crate::linear::ClientError;

/// How far My Issues is with loading the viewer's issues.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub enum Load {
    /// The issues are in, or there was nothing to load.
    #[default]
    Loaded,
    Loading,
    Failed(LoadError),
}

/// Why My Issues could not load. Each kind has its own error screen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoadError {
    /// Neither `LINEAR_API_KEY` nor the configured keyring holds a key.
    NoKey,
    /// Linear did not accept the key: it is wrong, revoked or expired. The
    /// screen names where the key came from.
    KeyRejected,
    /// The configured keyring could not be read; the message says what to do.
    KeyringUnavailable(String),
    /// A key was found but cannot be used, such as a malformed
    /// `LINEAR_API_KEY`.
    KeyUnusable(String),
    /// Linear could not be reached in time.
    Unreachable(Failure),
    /// Linear answered, but not with the issues.
    Failed(Failure),
}

impl From<&AuthError> for LoadError {
    fn from(error: &AuthError) -> Self {
        match error {
            AuthError::NoKey => Self::NoKey,
            AuthError::Rejected(_) | AuthError::LoginRejected | AuthError::EntryRejected(_) => {
                Self::KeyRejected
            }
            AuthError::Keyring(keyring) => Self::KeyringUnavailable(keyring.to_string()),
            AuthError::Client(client) => Self::from(client),
            AuthError::Malformed(_)
            | AuthError::MalformedEnv(_)
            | AuthError::WorkspaceFile { .. }
            | AuthError::ReadKey(_)
            | AuthError::NoEntry(_)
            | AuthError::ReadEntry { .. }
            | AuthError::MalformedEntry { .. } => Self::KeyUnusable(chain(error)),
        }
    }
}

/// What loading My Issues is called in the error log.
const LOAD_MY_ISSUES: &str = "load My Issues";

impl From<&ClientError> for LoadError {
    fn from(error: &ClientError) -> Self {
        let failure = || Failure::from_client(LOAD_MY_ISSUES, error);
        match error {
            ClientError::InvalidKey => Self::KeyRejected,
            ClientError::KeyNotAHeader => Self::KeyUnusable(chain(error)),
            ClientError::ConnectTimeout
            | ClientError::RequestTimeout
            | ClientError::Transport(_) => Self::Unreachable(failure()),
            ClientError::Setup(_)
            | ClientError::Status(_)
            | ClientError::RateLimited { .. }
            | ClientError::GraphQl(_)
            | ClientError::Decode(_) => Self::Failed(failure()),
        }
    }
}

/// `error` and each of its sources, joined by `: `.
pub(super) fn chain(error: &dyn Error) -> String {
    let mut text = error.to_string();
    let mut source = error.source();
    while let Some(cause) = source {
        text.push_str(": ");
        text.push_str(&cause.to_string());
        source = cause.source();
    }
    text
}

#[cfg(test)]
mod tests {
    use std::io;

    use super::*;
    use crate::auth::{KeySource, KeyringError};
    use crate::linear::ErrorKind;

    #[test]
    fn auth_problems_map_to_their_own_screens() {
        assert_eq!(LoadError::from(&AuthError::NoKey), LoadError::NoKey);
        assert_eq!(
            LoadError::from(&AuthError::Rejected(KeySource::Env)),
            LoadError::KeyRejected
        );
        assert_eq!(
            LoadError::from(&AuthError::Keyring(KeyringError::new("locked"))),
            LoadError::KeyringUnavailable(
                "the OS keyring is unavailable (locked); \
                 set LINEAR_API_KEY to use lrat without it"
                    .to_owned()
            )
        );
    }

    #[test]
    fn an_unreadable_workspace_file_keeps_its_cause() {
        let error = AuthError::WorkspaceFile {
            action: "reading",
            path: "/state/lrat/keyring-workspace".to_owned(),
            source: io::Error::other("permission denied"),
        };

        assert_eq!(
            LoadError::from(&error),
            LoadError::KeyUnusable(
                "reading the workspace file /state/lrat/keyring-workspace: permission denied"
                    .to_owned()
            )
        );
    }

    #[test]
    fn client_errors_split_into_rejected_unreachable_and_failed() {
        assert_eq!(
            LoadError::from(&ClientError::InvalidKey),
            LoadError::KeyRejected
        );
        assert_eq!(
            LoadError::from(&ClientError::ConnectTimeout),
            LoadError::Unreachable(Failure::new(
                ErrorKind::Offline,
                "load My Issues",
                "timed out connecting to Linear"
            ))
        );
        assert_eq!(
            LoadError::from(&ClientError::Status(503)),
            LoadError::Failed(Failure::new(
                ErrorKind::ServerError,
                "load My Issues",
                "Linear answered with HTTP 503"
            ))
        );
    }
}
