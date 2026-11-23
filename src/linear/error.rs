use std::fmt;
use std::time::Duration;

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

    /// The kind of failure this error is, from its `code` or `type` and, for
    /// a query Linear does not understand, its message. Linear does not
    /// document its full set of codes, so an unknown one is
    /// [`ErrorKind::Other`] and stays in the message as it came.
    pub fn kind(&self) -> ErrorKind {
        if self.message.starts_with("Cannot query field")
            || self.message.starts_with("Unknown argument")
        {
            return ErrorKind::SchemaDrift;
        }
        let names: Vec<String> = ["code", "type"]
            .into_iter()
            .filter_map(|name| self.extensions.as_ref()?.get(name)?.as_str())
            .map(|value| value.to_ascii_uppercase().replace(' ', "_"))
            .collect();
        let is = |wanted: &[&str]| names.iter().any(|name| wanted.contains(&name.as_str()));
        if is(&["FORBIDDEN"]) {
            ErrorKind::Forbidden
        } else if is(&["INVALID_INPUT", "INPUT_ERROR", "BAD_USER_INPUT"]) {
            ErrorKind::Validation {
                field: self.field(),
            }
        } else if is(&["GRAPHQL_VALIDATION_FAILED"]) {
            ErrorKind::SchemaDrift
        } else if is(&["INTERNAL_SERVER_ERROR", "INTERNAL_ERROR"]) {
            ErrorKind::ServerError
        } else if is(&["AUTHENTICATION_ERROR"]) {
            ErrorKind::KeyInvalid
        } else if is(&["RATELIMITED"]) {
            ErrorKind::RateLimited { retry_after: None }
        } else {
            ErrorKind::Other
        }
    }

    /// The input field a validation error names, from the first of its
    /// `validationErrors`.
    fn field(&self) -> Option<String> {
        let first = self
            .extensions
            .as_ref()?
            .get("validationErrors")?
            .as_array()?
            .first()?;
        Some(first.get("property")?.as_str()?.to_owned())
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
    /// Linear kept refusing requests for its rate limit, after every retry.
    /// `retry_after` is the wait its last answer asked for, when it said.
    #[error("Linear's rate limit is used up")]
    RateLimited { retry_after: Option<Duration> },
    /// Errors with no data, or with data too incomplete to use.
    #[error("Linear returned errors: {}", join(.0))]
    GraphQl(Vec<GraphQlError>),
    #[error("could not read Linear's response: {0}")]
    Decode(String),
    /// The host Linear signed an upload URL for refused the file or could
    /// not be reached. It says nothing about the API key.
    #[error("the upload failed: {0}")]
    Upload(String),
    /// The file to attach could not be read.
    #[error("could not read the file: {0}")]
    ReadFile(String),
}

impl ClientError {
    pub(crate) fn from_transport(error: reqwest::Error) -> Self {
        match (error.is_timeout(), error.is_connect()) {
            (true, true) => Self::ConnectTimeout,
            (true, false) => Self::RequestTimeout,
            (false, _) => Self::Transport(error.without_url()),
        }
    }

    /// What kind of failure this is, for its message and suggested fix.
    pub fn kind(&self) -> ErrorKind {
        match self {
            Self::InvalidKey | Self::KeyNotAHeader | Self::Status(401) => ErrorKind::KeyInvalid,
            Self::ConnectTimeout | Self::RequestTimeout | Self::Transport(_) => ErrorKind::Offline,
            Self::Status(403) => ErrorKind::Forbidden,
            Self::Status(429) => ErrorKind::RateLimited { retry_after: None },
            Self::Status(500..=599) => ErrorKind::ServerError,
            Self::Setup(_) | Self::Status(_) => ErrorKind::Other,
            Self::RateLimited { retry_after } => ErrorKind::RateLimited {
                retry_after: *retry_after,
            },
            Self::GraphQl(errors) => errors
                .iter()
                .map(GraphQlError::kind)
                .find(|kind| *kind != ErrorKind::Other)
                .unwrap_or(ErrorKind::Other),
            Self::Decode(_) => ErrorKind::SchemaDrift,
            Self::Upload(_) | Self::ReadFile(_) => ErrorKind::Other,
        }
    }
}

/// The kinds of failure the viewer can tell apart, each with a one-line
/// message and a suggested fix.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ErrorKind {
    /// Linear could not be reached, or did not answer in time.
    Offline,
    /// Linear's rate limit held the request back; `retry_after` is how long
    /// Linear asked to wait, when it said.
    RateLimited {
        retry_after: Option<Duration>,
    },
    /// Linear did not accept the API key.
    KeyInvalid,
    /// The API key is valid but not allowed to do this.
    Forbidden,
    /// Linear did not accept a value sent to it; `field` names it when the
    /// error does.
    Validation {
        field: Option<String>,
    },
    /// Linear failed on its side (HTTP 5xx).
    ServerError,
    /// Linear's answer no longer fits what lrat asks for: a field it does
    /// not know, or data lrat cannot read.
    SchemaDrift,
    Other,
}

impl ErrorKind {
    /// A short name, as the error log lists it.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Offline => "offline",
            Self::RateLimited { .. } => "rate limited",
            Self::KeyInvalid => "key invalid",
            Self::Forbidden => "forbidden",
            Self::Validation { .. } => "validation",
            Self::ServerError => "server error",
            Self::SchemaDrift => "schema drift",
            Self::Other => "error",
        }
    }

    /// What went wrong, in one line.
    pub fn summary(&self) -> String {
        match self {
            Self::Offline => "Linear could not be reached".to_owned(),
            Self::RateLimited {
                retry_after: Some(wait),
            } => format!("Linear's rate limit is used up for {}", wait_text(*wait)),
            Self::RateLimited { retry_after: None } => "Linear's rate limit is used up".to_owned(),
            Self::KeyInvalid => "The API key is missing or rejected".to_owned(),
            Self::Forbidden => "The API key is not allowed to do this".to_owned(),
            Self::Validation { field: Some(field) } => {
                format!("Linear did not accept the value of {field}")
            }
            Self::Validation { field: None } => "Linear did not accept a value".to_owned(),
            Self::ServerError => "Linear failed on its side".to_owned(),
            Self::SchemaDrift => "Linear's API no longer matches this lrat".to_owned(),
            Self::Other => "Something failed".to_owned(),
        }
    }

    /// What the viewer can do about it, in one line.
    pub fn fix(&self) -> &'static str {
        match self {
            Self::Offline => "Check the network; lrat tries again on the next poll.",
            Self::RateLimited { .. } => "Wait for it to lift; polls slow down until then.",
            Self::KeyInvalid => "Run `lrat auth login` with a new key.",
            Self::Forbidden => "Use a key with access to this, or ask a workspace admin.",
            Self::Validation { .. } => "Change the value and try again.",
            Self::ServerError => "Try again in a moment.",
            Self::SchemaDrift => "Update lrat; if it is current, report the message.",
            Self::Other => "The message below says what; `lrat doctor` checks the setup.",
        }
    }
}

/// `wait` as `45s`, or in whole minutes, rounded up, from a minute on.
fn wait_text(wait: Duration) -> String {
    let seconds = wait.as_secs();
    if seconds < 60 {
        format!("{seconds}s")
    } else {
        format!("{}m", seconds.div_ceil(60))
    }
}

fn join(errors: &[GraphQlError]) -> String {
    errors
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join("; ")
}
