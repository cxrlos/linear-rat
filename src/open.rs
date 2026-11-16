//! `lrat open <KEY>`: opens an issue's Linear page in the browser.

use std::fmt;
use std::str::FromStr;

use crate::auth::{ApiKey, AuthError, Credentials, KeyStore};
use crate::linear::{ClientError, LinearClient};

/// An issue's key as typed on the command line, such as `ENG-123`: a team key
/// (a letter, then letters and digits) and a number from 1, matched whatever
/// the case.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssueKey {
    team: String,
    number: u32,
}

impl IssueKey {
    /// The team key, upper case.
    pub fn team(&self) -> &str {
        &self.team
    }

    pub fn number(&self) -> u32 {
        self.number
    }
}

impl FromStr for IssueKey {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let malformed = || "expected an issue key such as ENG-123".to_owned();
        let (team, number) = value.split_once('-').ok_or_else(malformed)?;
        let team_ok = team.starts_with(|c: char| c.is_ascii_alphabetic())
            && team.chars().all(|c| c.is_ascii_alphanumeric());
        let number_ok = !number.is_empty() && number.chars().all(|c| c.is_ascii_digit());
        if !team_ok || !number_ok {
            return Err(malformed());
        }
        match number.parse::<u32>() {
            Ok(number) if number > 0 => Ok(Self {
                team: team.to_ascii_uppercase(),
                number,
            }),
            _ => Err(malformed()),
        }
    }
}

impl fmt::Display for IssueKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}-{}", self.team, self.number)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum OpenError {
    #[error(transparent)]
    Auth(#[from] AuthError),
    #[error("no issue {0} in this workspace")]
    NoIssue(IssueKey),
    #[error("looking up {key}")]
    Lookup {
        key: IssueKey,
        #[source]
        source: ClientError,
    },
    #[error(transparent)]
    Browser(anyhow::Error),
}

/// Looks `key` up with the resolved API key and hands its URL to `opener`.
/// Returns what to print.
pub async fn open<S, C>(
    credentials: &Credentials<S>,
    key: &IssueKey,
    connect: impl FnOnce(&ApiKey) -> Result<C, ClientError>,
    opener: impl FnOnce(&str) -> anyhow::Result<()>,
) -> Result<String, OpenError>
where
    S: KeyStore,
    C: LinearClient,
{
    let (api_key, source) = credentials.resolve()?;
    let client = connect(&api_key).map_err(AuthError::Client)?;
    let issue = match client.issue_by_key(key.team(), key.number()).await {
        Ok(Some(issue)) => issue,
        Ok(None) => return Err(OpenError::NoIssue(key.clone())),
        Err(ClientError::InvalidKey) => return Err(AuthError::Rejected(source).into()),
        Err(source) => {
            return Err(OpenError::Lookup {
                key: key.clone(),
                source,
            });
        }
    };
    opener(&issue.url).map_err(OpenError::Browser)?;
    Ok(format!(
        "Opened {} {}\n{}",
        issue.identifier, issue.title, issue.url
    ))
}
