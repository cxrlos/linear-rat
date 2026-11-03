use std::fmt;

/// A Linear API key. It prints as `<redacted>` in `Debug` and `Display`, so it
/// is safe to put in logs and error chains, and it deliberately has no
/// `Serialize`. Only [`ApiKey::expose`] reveals it, for the `Authorization`
/// header and the keyring.
#[derive(Clone, PartialEq, Eq)]
pub struct ApiKey(String);

impl ApiKey {
    /// Trims surrounding whitespace (a piped key usually ends in a newline).
    /// Fails when nothing is left or the key holds a character that cannot go
    /// in an HTTP header.
    pub fn new(raw: &str) -> Result<Self, InvalidApiKey> {
        let key = raw.trim();
        if key.is_empty() {
            return Err(InvalidApiKey::Empty);
        }
        if !key.chars().all(|c| c.is_ascii_graphic()) {
            return Err(InvalidApiKey::BadCharacter);
        }
        Ok(Self(key.to_owned()))
    }

    /// The key itself. Never log or print the result.
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for ApiKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ApiKey(<redacted>)")
    }
}

impl fmt::Display for ApiKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("<redacted>")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum InvalidApiKey {
    #[error("the API key is empty")]
    Empty,
    #[error(
        "the API key holds a space or a non-ASCII character; paste it exactly as Linear shows it"
    )]
    BadCharacter,
}
