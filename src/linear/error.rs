use std::fmt;

use serde::Deserialize;
use serde_json::Value;

/// A GraphQL response that carried data. `errors` is not empty when Linear
/// could only resolve part of the query: the fields it could not resolve are
/// `null` in `data`.
#[derive(Debug, Clone, PartialEq)]
pub struct Response<T> {
    pub data: T,
    pub errors: Vec<GraphQlError>,
}

/// One entry of a GraphQL response's `errors`.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct GraphQlError {
    pub message: String,
    /// Where in `data` the error happened, as field names and list indexes.
    #[serde(default)]
    pub path: Vec<Value>,
    #[serde(default)]
    pub extensions: Option<Value>,
}

impl GraphQlError {
    /// Linear's machine-readable error code, for example
    /// `AUTHENTICATION_ERROR` or `RATELIMITED`.
    pub fn code(&self) -> Option<&str> {
        self.extensions.as_ref()?.get("code")?.as_str()
    }
}

impl fmt::Display for GraphQlError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)?;
        if let Some(code) = self.code() {
            write!(f, " ({code})")?;
        }
        Ok(())
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    /// Linear did not accept the API key: it is wrong, revoked or expired.
    #[error("Linear rejected the API key (wrong, revoked or expired)")]
    InvalidKey,
    #[error("the API key cannot be sent in an HTTP header")]
    KeyNotAHeader,
    #[error("timed out connecting to Linear")]
    ConnectTimeout,
    #[error("Linear did not answer in time")]
    RequestTimeout,
    #[error("could not reach Linear")]
    Transport(#[source] reqwest::Error),
    #[error("could not set up the HTTP client")]
    Setup(#[source] reqwest::Error),
    #[error("Linear answered with HTTP {0}")]
    Status(u16),
    /// Errors with no data, or with data too incomplete to use.
    #[error("Linear returned errors: {}", join(.0))]
    GraphQl(Vec<GraphQlError>),
    #[error("could not read Linear's response: {0}")]
    Decode(String),
}

impl ClientError {
    pub(crate) fn from_transport(error: reqwest::Error) -> Self {
        match (error.is_timeout(), error.is_connect()) {
            (true, true) => Self::ConnectTimeout,
            (true, false) => Self::RequestTimeout,
            (false, _) => Self::Transport(error.without_url()),
        }
    }
}

fn join(errors: &[GraphQlError]) -> String {
    errors
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join("; ")
}
