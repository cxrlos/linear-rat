// clippy's allow-unwrap-in-tests does not cover helpers outside #[test] functions.
#![allow(clippy::unwrap_used)]

use std::collections::HashMap;
use std::io;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use linear_rat::auth::{
    self, ApiKey, AuthError, Credentials, InvalidApiKey, KeyStore, KeyringError, WorkspaceFile,
};
use linear_rat::linear::model::{Viewer, Workspace};
use linear_rat::linear::{ClientError, FakeClient, LinearClient};

const SECRET: &str = "lin_api_fictional0000000000000000000000000000";
const OTHER_SECRET: &str = "lin_api_fictional1111111111111111111111111111";

/// The fake client's workspace.
const URL_KEY: &str = "cheddar-works";

#[derive(Default)]
struct MemoryStore(Mutex<HashMap<String, String>>);

impl MemoryStore {
    fn holding(account: &str, key: &str) -> Self {
        let store = Self::default();
        store
            .0
            .lock()
            .unwrap()
            .insert(account.to_owned(), key.to_owned());
        store
    }

    fn entries(&self) -> HashMap<String, String> {
        self.0.lock().unwrap().clone()
    }
}

impl KeyStore for MemoryStore {
    fn get(&self, account: &str) -> Result<Option<ApiKey>, KeyringError> {
        Ok(self
            .0
            .lock()
            .unwrap()
            .get(account)
            .map(|key| ApiKey::new(key).unwrap()))
    }

    fn set(&self, account: &str, key: &ApiKey) -> Result<(), KeyringError> {
        self.0
            .lock()
            .unwrap()
            .insert(account.to_owned(), key.expose().to_owned());
        Ok(())
    }

    fn delete(&self, account: &str) -> Result<bool, KeyringError> {
        Ok(self.0.lock().unwrap().remove(account).is_some())
    }
}

/// A keyring that cannot be reached, as on a headless box with no Secret
/// Service running.
#[derive(Debug)]
struct UnavailableStore;

impl KeyStore for UnavailableStore {
    fn get(&self, _: &str) -> Result<Option<ApiKey>, KeyringError> {
        Err(unavailable())
    }

    fn set(&self, _: &str, _: &ApiKey) -> Result<(), KeyringError> {
        Err(unavailable())
    }

    fn delete(&self, _: &str) -> Result<bool, KeyringError> {
        Err(unavailable())
    }
}

fn unavailable() -> KeyringError {
    KeyringError::new("org.freedesktop.secrets was not provided by any .service files")
}

/// A keyring that stores and reads but refuses to delete, as when the viewer
/// dismisses an unlock prompt.
#[derive(Default)]
struct UndeletableStore(MemoryStore);

impl KeyStore for UndeletableStore {
    fn get(&self, account: &str) -> Result<Option<ApiKey>, KeyringError> {
        self.0.get(account)
    }

    fn set(&self, account: &str, key: &ApiKey) -> Result<(), KeyringError> {
        self.0.set(account, key)
    }

    fn delete(&self, _: &str) -> Result<bool, KeyringError> {
        Err(KeyringError::new("the collection is locked"))
    }
}

/// A client for a key Linear does not accept.
struct RejectingClient;

impl LinearClient for RejectingClient {
    async fn viewer(&self) -> Result<(Viewer, Workspace), ClientError> {
        Err(ClientError::InvalidKey)
    }
}

/// A fresh directory for one test's workspace file.
fn workspace_file() -> WorkspaceFile {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!(
        "auth-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::SeqCst)
    ));
    WorkspaceFile::new(dir.join("keyring-workspace"))
}

fn key(raw: &str) -> ApiKey {
    ApiKey::new(raw).unwrap()
}

/// Connects the fake client, recording which key it was given.
fn fake_connect(
    used: &Mutex<Vec<String>>,
) -> impl FnOnce(&ApiKey) -> Result<FakeClient, ClientError> {
    move |key| {
        used.lock().unwrap().push(key.expose().to_owned());
        Ok(FakeClient)
    }
}

#[tokio::test]
async fn login_checks_the_key_then_stores_it_under_the_workspace_url_key() {
    let credentials = Credentials::new(MemoryStore::default(), workspace_file(), None);
    let used = Mutex::new(Vec::new());

    let output = auth::login(&credentials, key(SECRET), fake_connect(&used))
        .await
        .unwrap();

    assert_eq!(*used.lock().unwrap(), [SECRET]);
    assert_eq!(
        credentials.store().entries(),
        HashMap::from([(URL_KEY.to_owned(), SECRET.to_owned())])
    );
    assert_eq!(
        output,
        "Logged in to Cheddar Works (cheddar-works) as Remy Fromage.\n\
         The API key is in the OS keyring (service linear-rat, account cheddar-works)."
    );
}

#[tokio::test]
async fn login_with_a_rejected_key_stores_nothing_and_says_why() {
    let credentials = Credentials::new(MemoryStore::default(), workspace_file(), None);

    let error = auth::login(&credentials, key(SECRET), |_| Ok(RejectingClient))
        .await
        .unwrap_err();

    assert!(matches!(error, AuthError::LoginRejected), "{error:?}");
    let message = error.to_string();
    assert!(
        message.contains("Linear rejected this API key"),
        "{message}"
    );
    assert!(message.contains("Nothing was stored"), "{message}");
    assert!(!message.contains(SECRET));
    assert!(credentials.store().entries().is_empty());
}

#[tokio::test]
async fn login_while_linear_api_key_is_set_says_it_still_wins() {
    let credentials = Credentials::new(
        MemoryStore::default(),
        workspace_file(),
        Some(OTHER_SECRET.to_owned()),
    );
    let used = Mutex::new(Vec::new());

    let output = auth::login(&credentials, key(SECRET), fake_connect(&used))
        .await
        .unwrap();

    assert_eq!(*used.lock().unwrap(), [SECRET]);
    assert!(
        output.ends_with("LINEAR_API_KEY is set, so lrat uses it instead of the keyring."),
        "{output}"
    );
}

#[tokio::test]
async fn status_uses_linear_api_key_over_the_keyring_and_names_it() {
    let credentials = Credentials::new(
        MemoryStore::holding(URL_KEY, SECRET),
        workspace_file(),
        Some(OTHER_SECRET.to_owned()),
    );
    credentials.workspace_file().write(URL_KEY).unwrap();
    let used = Mutex::new(Vec::new());

    let output = auth::status(&credentials, fake_connect(&used))
        .await
        .unwrap();

    assert_eq!(*used.lock().unwrap(), [OTHER_SECRET]);
    assert_eq!(
        output,
        "Key source: LINEAR_API_KEY\n\
         Workspace:  Cheddar Works (cheddar-works)\n\
         Viewer:     Remy Fromage"
    );
}

#[tokio::test]
async fn status_reads_the_keyring_when_linear_api_key_is_unset() {
    let credentials = Credentials::new(
        MemoryStore::holding(URL_KEY, SECRET),
        workspace_file(),
        Some("  ".to_owned()),
    );
    credentials.workspace_file().write(URL_KEY).unwrap();
    let used = Mutex::new(Vec::new());

    let output = auth::status(&credentials, fake_connect(&used))
        .await
        .unwrap();

    assert_eq!(*used.lock().unwrap(), [SECRET]);
    assert!(
        output.starts_with(
            "Key source: the OS keyring (service linear-rat, account cheddar-works)\n"
        ),
        "{output}"
    );
    assert!(!output.contains(SECRET));
}

#[tokio::test]
async fn status_without_a_key_says_how_to_add_one() {
    let credentials = Credentials::new(MemoryStore::default(), workspace_file(), None);

    let error = auth::status(&credentials, |_| Ok(FakeClient))
        .await
        .unwrap_err();

    assert!(matches!(error, AuthError::NoKey), "{error:?}");
    assert_eq!(
        error.to_string(),
        "no API key: run `lrat auth login`, or set LINEAR_API_KEY"
    );
}

#[tokio::test]
async fn status_with_a_revoked_key_names_where_the_key_came_from() {
    let credentials = Credentials::new(
        MemoryStore::default(),
        workspace_file(),
        Some(SECRET.to_owned()),
    );

    let error = auth::status(&credentials, |_| Ok(RejectingClient))
        .await
        .unwrap_err();

    let message = error.to_string();
    assert!(
        message.starts_with("Linear rejected the API key from LINEAR_API_KEY"),
        "{message}"
    );
    assert!(!message.contains(SECRET));
}

#[tokio::test]
async fn logout_deletes_the_entry_so_status_finds_no_key() {
    let credentials = Credentials::new(MemoryStore::default(), workspace_file(), None);
    auth::login(&credentials, key(SECRET), |_| Ok(FakeClient))
        .await
        .unwrap();

    let output = auth::logout(&credentials).unwrap();

    assert_eq!(
        output,
        "Removed the API key for cheddar-works from the OS keyring."
    );
    assert!(credentials.store().entries().is_empty());
    let error = auth::status(&credentials, |_| Ok(FakeClient))
        .await
        .unwrap_err();
    assert!(matches!(error, AuthError::NoKey), "{error:?}");
}

#[test]
fn logout_when_the_entry_is_already_gone_says_so_and_forgets_the_workspace() {
    let credentials = Credentials::new(MemoryStore::default(), workspace_file(), None);
    credentials.workspace_file().write(URL_KEY).unwrap();

    let output = auth::logout(&credentials).unwrap();

    assert_eq!(
        output,
        "No API key for cheddar-works was in the OS keyring."
    );
    assert_eq!(credentials.workspace_file().read().unwrap(), None);
}

#[test]
fn logout_with_nothing_stored_says_so() {
    let credentials = Credentials::new(MemoryStore::default(), workspace_file(), None);

    assert_eq!(
        auth::logout(&credentials).unwrap(),
        "No API key is stored in the OS keyring."
    );
}

#[tokio::test]
async fn login_into_another_workspace_replaces_the_previous_entry() {
    let credentials = Credentials::new(
        MemoryStore::holding("old-workspace", OTHER_SECRET),
        workspace_file(),
        None,
    );
    credentials.workspace_file().write("old-workspace").unwrap();

    auth::login(&credentials, key(SECRET), |_| Ok(FakeClient))
        .await
        .unwrap();

    assert_eq!(
        credentials.store().entries(),
        HashMap::from([(URL_KEY.to_owned(), SECRET.to_owned())])
    );
    assert_eq!(
        credentials.workspace_file().read().unwrap().as_deref(),
        Some(URL_KEY)
    );
}

#[tokio::test]
async fn login_keeps_the_new_key_when_the_previous_entry_cannot_be_deleted() {
    let credentials = Credentials::new(UndeletableStore::default(), workspace_file(), None);
    credentials
        .store()
        .set("old-workspace", &key(OTHER_SECRET))
        .unwrap();
    credentials.workspace_file().write("old-workspace").unwrap();

    let output = auth::login(&credentials, key(SECRET), |_| Ok(FakeClient))
        .await
        .unwrap();

    assert_eq!(
        credentials.workspace_file().read().unwrap().as_deref(),
        Some(URL_KEY)
    );
    assert_eq!(credentials.store().0.entries()[URL_KEY], SECRET);
    assert!(
        output.ends_with(
            "Could not delete the previous API key, for old-workspace, from the OS keyring; \
             delete it there by hand."
        ),
        "{output}"
    );
}

#[tokio::test]
async fn an_unavailable_keyring_is_named_and_linear_api_key_suggested() {
    let credentials = Credentials::new(UnavailableStore, workspace_file(), None);

    let error = auth::login(&credentials, key(SECRET), |_| Ok(FakeClient))
        .await
        .unwrap_err();

    let message = error.to_string();
    assert!(message.contains("OS keyring"), "{message}");
    assert!(message.contains("org.freedesktop.secrets"), "{message}");
    assert!(message.contains("set LINEAR_API_KEY"), "{message}");
}

#[test]
fn a_piped_key_is_read_without_its_trailing_newline() {
    let read = auth::read_key(format!("{SECRET}\n").as_bytes()).unwrap();

    assert_eq!(read.expose(), SECRET);
}

#[test]
fn an_empty_pipe_is_an_error() {
    let error = auth::read_key(&b""[..]).unwrap_err();

    assert!(
        matches!(error, AuthError::Malformed(InvalidApiKey::Empty)),
        "{error:?}"
    );
}

#[test]
fn an_api_key_redacts_itself_in_debug_and_display() {
    let key = ApiKey::new(SECRET).unwrap();

    let debug = format!("{key:?}");
    let display = format!("{key}");

    assert!(!debug.contains(SECRET), "{debug}");
    assert!(!display.contains(SECRET), "{display}");
    assert_eq!(display, "<redacted>");
}

#[test]
fn credentials_holding_linear_api_key_redact_it_in_debug() {
    let credentials = Credentials::new(UnavailableStore, workspace_file(), Some(SECRET.to_owned()));

    let debug = format!("{credentials:?}");

    assert!(debug.contains("<redacted>"), "{debug}");
    assert!(!debug.contains(SECRET), "{debug}");
}

/// `SERIALIZE` is true only when `T: Serialize`: the inherent const is chosen
/// over the trait's default only when its bound holds.
struct SerializeProbe<T>(std::marker::PhantomData<T>);

trait NotSerialize {
    const SERIALIZE: bool = false;
}

impl<T> NotSerialize for SerializeProbe<T> {}

impl<T: serde::Serialize> SerializeProbe<T> {
    const SERIALIZE: bool = true;
}

#[test]
fn an_api_key_cannot_be_serialized() {
    // Checked at compile time: adding Serialize to ApiKey breaks the build.
    const { assert!(SerializeProbe::<String>::SERIALIZE) };
    const { assert!(!SerializeProbe::<ApiKey>::SERIALIZE) };
}

#[test]
fn an_api_key_logged_through_tracing_stays_redacted() {
    let key = ApiKey::new(SECRET).unwrap();
    let log = SharedBuffer::default();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(log.clone())
        .with_ansi(false)
        .finish();

    tracing::subscriber::with_default(subscriber, || {
        tracing::info!(?key, "debug field");
        tracing::info!(%key, "display field");
        tracing::info!("in the message: {key} {key:?}");
    });

    let written = log.contents();
    assert!(written.contains("<redacted>"), "{written}");
    assert!(!written.contains(SECRET), "{written}");
}

#[test]
fn an_api_key_is_trimmed_and_must_not_be_blank() {
    assert_eq!(
        ApiKey::new(&format!("  {SECRET}\n")).unwrap().expose(),
        SECRET
    );
    assert_eq!(ApiKey::new(" \n"), Err(InvalidApiKey::Empty));
    assert_eq!(
        ApiKey::new("lin_api_two words"),
        Err(InvalidApiKey::BadCharacter)
    );
}

#[derive(Clone, Default)]
struct SharedBuffer(Arc<Mutex<Vec<u8>>>);

impl SharedBuffer {
    fn contents(&self) -> String {
        String::from_utf8(self.0.lock().unwrap().clone()).unwrap()
    }
}

impl io::Write for SharedBuffer {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for SharedBuffer {
    type Writer = SharedBuffer;

    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}
