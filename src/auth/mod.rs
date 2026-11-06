//! The API key: where it comes from, where it is kept, and the `lrat auth`
//! commands.
//!
//! The key comes from `LINEAR_API_KEY` when that is set, otherwise from the OS
//! keyring entry with service [`SERVICE`] and the workspace's URL key as the
//! account. Since the account is only known after a login, the URL key of the
//! stored entry is remembered in a [`WorkspaceFile`].

mod key;
mod keyring;
mod workspace_file;

use std::fmt;
use std::io;

pub use key::{ApiKey, InvalidApiKey};
pub use keyring::{EntryReader, KeyStore, KeyringEntry, KeyringError, OsKeyring};
pub use workspace_file::WorkspaceFile;

use crate::linear::{ClientError, LinearClient};

/// The keyring service name of lrat's entry.
pub const SERVICE: &str = "linear-rat";

/// The environment variable that overrides the keyring.
pub const KEY_VAR: &str = "LINEAR_API_KEY";

/// Where a viewer creates personal API keys.
const KEYS_URL: &str = "https://linear.app/settings/account/security";

/// Where the key in use came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeySource {
    Env,
    Keyring { account: String },
}

impl KeySource {
    /// Which kind of source, without the keyring account, for the log.
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Env => KEY_VAR,
            Self::Keyring { .. } => "OS keyring",
        }
    }
}

impl fmt::Display for KeySource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Env => f.write_str(KEY_VAR),
            Self::Keyring { account } => {
                write!(f, "the OS keyring (service {SERVICE}, account {account})")
            }
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum AuthError {
    #[error("no API key: run `lrat auth login`, or set {KEY_VAR}")]
    NoKey,
    #[error(
        "Linear rejected this API key: it is wrong, revoked or expired. Nothing was stored. \
         Create a personal API key at {KEYS_URL} and try again"
    )]
    LoginRejected,
    #[error("{}", rejected(.0))]
    Rejected(KeySource),
    #[error(transparent)]
    Malformed(InvalidApiKey),
    #[error("{KEY_VAR} does not hold a valid API key: {0}")]
    MalformedEnv(InvalidApiKey),
    #[error(transparent)]
    Keyring(#[from] KeyringError),
    #[error("{action} the workspace file {path}")]
    WorkspaceFile {
        action: &'static str,
        path: String,
        #[source]
        source: io::Error,
    },
    #[error(
        "the OS keyring has no entry with {0}. Nothing was stored. Add the API key there, \
         or name another entry as --from-keyring SERVICE/ACCOUNT"
    )]
    NoEntry(KeyringEntry),
    #[error("could not read the entry with {entry}: {reason}. Nothing was stored.")]
    ReadEntry {
        entry: KeyringEntry,
        reason: KeyringError,
    },
    #[error(
        "the OS keyring entry with {entry} does not hold a valid API key: {reason}. \
         Nothing was stored."
    )]
    MalformedEntry {
        entry: KeyringEntry,
        reason: InvalidApiKey,
    },
    #[error(
        "Linear rejected the API key in the OS keyring entry with {0}: it is wrong, revoked \
         or expired. Nothing was stored. Create a personal API key at {KEYS_URL}, put it in \
         that entry and try again"
    )]
    EntryRejected(KeyringEntry),
    #[error("reading the API key")]
    ReadKey(#[source] io::Error),
    #[error("checking the API key with Linear")]
    Client(#[source] ClientError),
}

/// The places a key can come from, as `lrat` sees them on this machine.
#[derive(Debug)]
pub struct Credentials<S> {
    store: S,
    workspace_file: WorkspaceFile,
    /// Held as an [`ApiKey`] so `Debug` redacts it; a malformed value is kept
    /// as its error and reported when the key is resolved.
    env_key: Option<Result<ApiKey, InvalidApiKey>>,
}

impl<S: KeyStore> Credentials<S> {
    /// `env_key` is the value of `LINEAR_API_KEY`; blank counts as unset.
    pub fn new(store: S, workspace_file: WorkspaceFile, env_key: Option<String>) -> Self {
        let env_key = env_key
            .map(|value| ApiKey::new(&value))
            .filter(|parsed| !matches!(parsed, Err(InvalidApiKey::Empty)));
        Self {
            store,
            workspace_file,
            env_key,
        }
    }

    pub fn store(&self) -> &S {
        &self.store
    }

    pub fn workspace_file(&self) -> &WorkspaceFile {
        &self.workspace_file
    }

    /// The key to use and where it came from: `LINEAR_API_KEY` first, then
    /// the keyring entry of the remembered workspace.
    pub fn resolve(&self) -> Result<(ApiKey, KeySource), AuthError> {
        if let Some(parsed) = &self.env_key {
            let key = parsed.clone().map_err(AuthError::MalformedEnv)?;
            return Ok((key, KeySource::Env));
        }
        let Some(account) = self.workspace_file.read()? else {
            return Err(AuthError::NoKey);
        };
        match self.store.get(&account)? {
            Some(key) => Ok((key, KeySource::Keyring { account })),
            None => Err(AuthError::NoKey),
        }
    }
}

/// Reads a key piped to `lrat auth login --stdin`.
pub fn read_key(mut input: impl io::Read) -> Result<ApiKey, AuthError> {
    let mut raw = String::new();
    input.read_to_string(&mut raw).map_err(AuthError::ReadKey)?;
    ApiKey::new(&raw).map_err(AuthError::Malformed)
}

/// Asks for a key on the terminal without echoing it.
pub fn prompt_key() -> Result<ApiKey, AuthError> {
    let raw = rpassword::prompt_password("Linear API key: ").map_err(AuthError::ReadKey)?;
    ApiKey::new(&raw).map_err(AuthError::Malformed)
}

/// `lrat auth login`: checks `key` with Linear, then stores it in the keyring
/// under the workspace's URL key, replacing the entry of any workspace logged
/// in before. Returns what to print.
pub async fn login<S, C>(
    credentials: &Credentials<S>,
    key: ApiKey,
    connect: impl FnOnce(&ApiKey) -> Result<C, ClientError>,
) -> Result<String, AuthError>
where
    S: KeyStore,
    C: LinearClient,
{
    let client = connect(&key).map_err(AuthError::Client)?;
    let (viewer, workspace) = client.viewer().await.map_err(|error| match error {
        ClientError::InvalidKey => AuthError::LoginRejected,
        other => AuthError::Client(other),
    })?;

    let account = &workspace.url_key;
    let previous = credentials.workspace_file.read()?;
    let replaced = credentials.store.get(account)?;
    credentials.store.set(account, &key)?;
    // Point at the new entry before touching the old one, so a failed delete
    // cannot leave the workspace file naming a workspace that was replaced.
    if let Err(error) = credentials.workspace_file.write(account) {
        // Without the pointer, status and logout cannot find the new entry, so
        // put the keyring back as it was.
        let undo = match &replaced {
            Some(old) => credentials.store.set(account, old),
            None => credentials.store.delete(account).map(drop),
        };
        if let Err(undo_error) = undo {
            tracing::warn!(%undo_error, "could not undo storing the API key");
        }
        return Err(error);
    }
    let mut stale = None;
    if let Some(previous) = previous
        && previous != *account
        && let Err(error) = credentials.store.delete(&previous)
    {
        tracing::warn!(%error, "could not delete the previous workspace's API key");
        stale = Some(previous);
    }

    let source = KeySource::Keyring {
        account: account.clone(),
    };
    let mut output = format!(
        "Logged in to {} ({account}) as {}.\nThe API key is in {source}.",
        workspace.name, viewer.name
    );
    if let Some(stale) = stale {
        output.push_str(&format!(
            "\nCould not delete the previous API key, for {stale}, from the OS keyring; \
             delete it there by hand."
        ));
    }
    if credentials.env_key.is_some() {
        output.push_str(&format!(
            "\n{KEY_VAR} is set, so lrat uses it instead of the keyring."
        ));
    }
    Ok(output)
}

/// `lrat auth login --from-keyring`: reads the key from `entry`, an existing
/// keyring entry lrat does not own, then logs in with it like [`login`]. The
/// entry read is left as it is. Returns what to print.
pub async fn login_from_keyring<S, R, C>(
    credentials: &Credentials<S>,
    reader: &R,
    entry: &KeyringEntry,
    connect: impl FnOnce(&ApiKey) -> Result<C, ClientError>,
) -> Result<String, AuthError>
where
    S: KeyStore,
    R: EntryReader,
    C: LinearClient,
{
    let secret = reader
        .read(entry)
        .map_err(|reason| AuthError::ReadEntry {
            entry: entry.clone(),
            reason,
        })?
        .ok_or_else(|| AuthError::NoEntry(entry.clone()))?;
    let key = ApiKey::new(&secret).map_err(|reason| AuthError::MalformedEntry {
        entry: entry.clone(),
        reason,
    })?;
    let output = login(credentials, key, connect)
        .await
        .map_err(|error| match error {
            AuthError::LoginRejected => AuthError::EntryRejected(entry.clone()),
            other => other,
        })?;
    Ok(format!(
        "Read the API key from the OS keyring ({entry}).\n{output}"
    ))
}

/// `lrat auth status`: which key is in use, and the workspace and viewer it
/// belongs to. Never prints the key.
pub async fn status<S, C>(
    credentials: &Credentials<S>,
    connect: impl FnOnce(&ApiKey) -> Result<C, ClientError>,
) -> Result<String, AuthError>
where
    S: KeyStore,
    C: LinearClient,
{
    let (key, source) = credentials.resolve()?;
    let client = connect(&key).map_err(AuthError::Client)?;
    let (viewer, workspace) = match client.viewer().await {
        Ok(found) => found,
        Err(ClientError::InvalidKey) => return Err(AuthError::Rejected(source)),
        Err(other) => return Err(AuthError::Client(other)),
    };
    Ok(format!(
        "Key source: {source}\nWorkspace:  {} ({})\nViewer:     {}",
        workspace.name, workspace.url_key, viewer.name
    ))
}

/// `lrat auth logout`: deletes lrat's keyring entry. `LINEAR_API_KEY` is left
/// alone.
pub fn logout<S: KeyStore>(credentials: &Credentials<S>) -> Result<String, AuthError> {
    let Some(account) = credentials.workspace_file.read()? else {
        return Ok(with_env_note(
            credentials,
            "No API key is stored in the OS keyring.".to_owned(),
        ));
    };
    let deleted = credentials.store.delete(&account)?;
    credentials.workspace_file.remove()?;
    let output = if deleted {
        format!("Removed the API key for {account} from the OS keyring.")
    } else {
        format!("No API key for {account} was in the OS keyring.")
    };
    Ok(with_env_note(credentials, output))
}

/// Why the key from `source` failed and how to replace it. A key from
/// `LINEAR_API_KEY` wins over the keyring, so logging in again would not help.
fn rejected(source: &KeySource) -> String {
    let fix = match source {
        KeySource::Env => format!("put it in {KEY_VAR}, or unset {KEY_VAR} to use the OS keyring"),
        KeySource::Keyring { .. } => "run `lrat auth login`".to_owned(),
    };
    format!(
        "Linear rejected the API key from {source}: it is wrong, revoked or expired. \
         Create a new personal API key at {KEYS_URL}, then {fix}"
    )
}

fn with_env_note<S>(credentials: &Credentials<S>, mut output: String) -> String {
    if credentials.env_key.is_some() {
        output.push_str(&format!(
            "\n{KEY_VAR} is still set, so lrat keeps using it."
        ));
    }
    output
}
