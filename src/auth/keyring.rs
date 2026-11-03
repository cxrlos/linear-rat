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
