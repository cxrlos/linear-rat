use std::error::Error;

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
    /// Neither `LINEAR_API_KEY` nor the OS keyring holds a key.
    NoKey,
    /// Linear did not accept the key: it is wrong, revoked or expired. The
    /// screen names where the key came from.
    KeyRejected,
    /// The OS keyring could not be read; the message says what to do.
    KeyringUnavailable(String),
    /// A key was found but cannot be used, such as a malformed
    /// `LINEAR_API_KEY`.
    KeyUnusable(String),
    /// Linear could not be reached in time.
    Unreachable(String),
    /// Linear answered, but not with the issues.
    Failed(String),
}

impl From<&AuthError> for LoadError {
    fn from(error: &AuthError) -> Self {
        match error {
            AuthError::NoKey => Self::NoKey,
            AuthError::Rejected(_) | AuthError::LoginRejected => Self::KeyRejected,
            AuthError::Keyring(keyring) => Self::KeyringUnavailable(keyring.to_string()),
            AuthError::Client(client) => Self::from(client),
            AuthError::Malformed(_)
            | AuthError::MalformedEnv(_)
            | AuthError::WorkspaceFile { .. }
            | AuthError::ReadKey(_) => Self::KeyUnusable(chain(error)),
        }
    }
}

impl From<&ClientError> for LoadError {
    fn from(error: &ClientError) -> Self {
        match error {
            ClientError::InvalidKey => Self::KeyRejected,
            ClientError::KeyNotAHeader => Self::KeyUnusable(chain(error)),
            ClientError::ConnectTimeout
            | ClientError::RequestTimeout
            | ClientError::Transport(_) => Self::Unreachable(chain(error)),
            ClientError::Setup(_)
            | ClientError::Status(_)
            | ClientError::RateLimited
            | ClientError::GraphQl(_)
            | ClientError::Decode(_) => Self::Failed(chain(error)),
        }
    }
}

/// `error` and each of its sources, joined by `: `.
fn chain(error: &dyn Error) -> String {
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
            LoadError::Unreachable("timed out connecting to Linear".to_owned())
        );
        assert_eq!(
            LoadError::from(&ClientError::Status(503)),
            LoadError::Failed("Linear answered with HTTP 503".to_owned())
        );
    }
}
