use std::fmt;
use std::str::FromStr;
use std::sync::Arc;

use keyring_core::{CredentialStore, Entry, Error};

use super::{ApiKey, KEY_VAR, SERVICE};

/// Somewhere to keep API keys, one per account. The binary only ever uses
/// [`OsKeyring`]; tests supply an in-memory store.
pub trait KeyStore {
    fn get(&self, account: &str) -> Result<Option<ApiKey>, KeyringError>;
    fn set(&self, account: &str, key: &ApiKey) -> Result<(), KeyringError>;
    /// Returns whether there was an entry to delete.
    fn delete(&self, account: &str) -> Result<bool, KeyringError>;
}

/// Reads an entry of the OS keyring that lrat does not own, such as one the
/// viewer made for their Linear key. Tests supply an in-memory reader.
pub trait EntryReader {
    /// The secret of `entry`, or `None` when there is no such entry.
    fn read(&self, entry: &KeyringEntry) -> Result<Option<String>, KeyringError>;
}

/// A keyring entry named by its service and account, written
/// `SERVICE/ACCOUNT`. It splits at the first `/`, so the account may hold one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyringEntry {
    service: String,
    account: String,
}

impl KeyringEntry {
    /// The entry `lrat auth login --from-keyring` reads when given no value.
    pub const DEFAULT: &str = "linear/api-key";

    /// The keyring service, before the first `/`.
    pub fn service(&self) -> &str {
        &self.service
    }

    /// The keyring account, after the first `/`.
    pub fn account(&self) -> &str {
        &self.account
    }
}

impl FromStr for KeyringEntry {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.split_once('/') {
            Some((service, account)) if !service.is_empty() && !account.is_empty() => Ok(Self {
                service: service.to_owned(),
                account: account.to_owned(),
            }),
            _ => Err("expected SERVICE/ACCOUNT, such as linear/api-key".to_owned()),
        }
    }
}

impl fmt::Display for KeyringEntry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "service {}, account {}", self.service, self.account)
    }
}

/// The OS keyring could not be used. There is no fallback store, so the
/// message points at `LINEAR_API_KEY`.
#[derive(Debug, thiserror::Error)]
#[error("the OS keyring is unavailable ({reason}); set {KEY_VAR} to use lrat without it")]
pub struct KeyringError {
    reason: String,
}

impl KeyringError {
    pub fn new(reason: impl Into<String>) -> Self {
        Self {
            reason: reason.into(),
        }
    }
}

impl From<Error> for KeyringError {
    // Display only: the Debug of some variants (BadEncoding, BadDataFormat)
    // holds the raw secret bytes.
    fn from(error: Error) -> Self {
        Self::new(error.to_string())
    }
}

/// The macOS Keychain or, on Linux, the Secret Service. It connects on each
/// use, so a command that never needs the keyring (`LINEAR_API_KEY` is set)
/// works on a machine without one.
#[derive(Debug, Default, Clone, Copy)]
pub struct OsKeyring;

impl OsKeyring {
    fn entry(self, account: &str) -> Result<Entry, KeyringError> {
        Ok(platform_store()?.build(SERVICE, account, None)?)
    }
}

#[cfg(target_os = "linux")]
impl EntryReader for OsKeyring {
    fn read(&self, entry: &KeyringEntry) -> Result<Option<String>, KeyringError> {
        use dbus_secret_service::{EncryptionType, SecretService};

        let unavailable = |error: dbus_secret_service::Error| KeyringError::new(error.to_string());
        let service = SecretService::connect(EncryptionType::Dh).map_err(unavailable)?;
        for attributes in secret_service_attributes(entry) {
            let found = service
                .search_items(attributes.into_iter().collect())
                .map_err(unavailable)?;
            let mut items: Vec<_> = found.unlocked.into_iter().chain(found.locked).collect();
            if items.len() > 1 {
                return Err(KeyringError::new(
                    "more than one entry has that service and account",
                ));
            }
            if let Some(item) = items.pop() {
                item.unlock().map_err(unavailable)?;
                let secret = item.get_secret().map_err(unavailable)?;
                return String::from_utf8(secret)
                    .map(Some)
                    .map_err(|_| KeyringError::new("the entry does not hold text"));
            }
        }
        Ok(None)
    }
}

#[cfg(not(target_os = "linux"))]
impl EntryReader for OsKeyring {
    fn read(&self, entry: &KeyringEntry) -> Result<Option<String>, KeyringError> {
        let found = platform_store()?.build(&entry.service, &entry.account, None)?;
        match found.get_password() {
            Ok(secret) => Ok(Some(secret)),
            Err(Error::NoEntry) => Ok(None),
            Err(Error::Ambiguous(_)) => Err(KeyringError::new(
                "more than one entry has that service and account",
            )),
            Err(error) => Err(error.into()),
        }
    }
}

#[cfg(target_os = "macos")]
fn platform_store() -> Result<Arc<CredentialStore>, KeyringError> {
    Ok(apple_native_keyring_store::keychain::Store::new()?)
}

#[cfg(target_os = "linux")]
fn platform_store() -> Result<Arc<CredentialStore>, KeyringError> {
    Ok(dbus_secret_service_keyring_store::Store::new()?)
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn platform_store() -> Result<Arc<CredentialStore>, KeyringError> {
    Err(KeyringError::new(
        "lrat supports the keyring on Linux and macOS only",
    ))
}

impl KeyStore for OsKeyring {
    fn get(&self, account: &str) -> Result<Option<ApiKey>, KeyringError> {
        match self.entry(account)?.get_password() {
            Ok(stored) => ApiKey::new(&stored)
                .map(Some)
                .map_err(|_| KeyringError::new("the stored entry is not a valid API key")),
            Err(Error::NoEntry) => Ok(None),
            Err(error) => Err(error.into()),
        }
    }

    fn set(&self, account: &str, key: &ApiKey) -> Result<(), KeyringError> {
        Ok(self.entry(account)?.set_password(key.expose())?)
    }

    fn delete(&self, account: &str) -> Result<bool, KeyringError> {
        match self.entry(account)?.delete_credential() {
            Ok(()) => Ok(true),
            Err(Error::NoEntry) => Ok(false),
            Err(error) => Err(error.into()),
        }
    }
}

/// The Secret Service attributes that can name an entry, tried in order:
/// `account`, as `secret-tool store ... service S account A` writes it, then
/// `username`, as the keyring crate writes it.
pub fn secret_service_attributes(entry: &KeyringEntry) -> [[(&str, &str); 2]; 2] {
    [
        [
            ("service", entry.service.as_str()),
            ("account", entry.account.as_str()),
        ],
        [
            ("service", entry.service.as_str()),
            ("username", entry.account.as_str()),
        ],
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secret_service_entries_match_on_account_first_then_username() {
        let entry: KeyringEntry = "linear/api-key".parse().unwrap();

        assert_eq!(
            secret_service_attributes(&entry),
            [
                [("service", "linear"), ("account", "api-key")],
                [("service", "linear"), ("username", "api-key")],
            ]
        );
    }
}
