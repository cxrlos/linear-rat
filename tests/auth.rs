// clippy's allow-unwrap-in-tests does not cover helpers outside #[test] functions.
#![allow(clippy::unwrap_used)]

use std::collections::HashMap;
use std::fs;
use std::io;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use jiff::Timestamp;
use linear_rat::auth::{
    self, ApiKey, AuthError, Credentials, EntryReader, InvalidApiKey, KeyStore, KeyringEntry,
    KeyringError, WorkspaceFile,
};
use linear_rat::linear::model::{Issue, Label, Member, Team, Viewer, WorkflowState, Workspace};
use linear_rat::linear::{
    AnalyticsIssues, AnalyticsWindow, AssignedIssues, ClientError, FakeClient, IssueDetail,
    IssueUpdate, LinearClient, SearchResults, TeamAnalyticsIssues, TeamChanges, TeamIssuesPage,
    TeamOptions, TeamQuery, Urgency,
};

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

    async fn assigned_issues(&self) -> Result<AssignedIssues, ClientError> {
        Err(ClientError::InvalidKey)
    }

    async fn changed_issues(
        &self,
        _: &str,
        _: Option<Timestamp>,
    ) -> Result<TeamChanges, ClientError> {
        Err(ClientError::InvalidKey)
    }

    async fn existing_issues(&self, _: &[String]) -> Result<Vec<String>, ClientError> {
        Err(ClientError::InvalidKey)
    }

    async fn workflow_states(&self, _team_id: &str) -> Result<Vec<WorkflowState>, ClientError> {
        Err(ClientError::InvalidKey)
    }

    async fn team_members(&self, _team_id: &str) -> Result<Vec<Member>, ClientError> {
        Err(ClientError::InvalidKey)
    }

    async fn team_labels(&self, _team_id: &str) -> Result<Vec<Label>, ClientError> {
        Err(ClientError::InvalidKey)
    }

    async fn update_issue(
        &self,
        _issue_id: &str,
        _update: &IssueUpdate,
    ) -> Result<Issue, ClientError> {
        Err(ClientError::InvalidKey)
    }

    async fn issue_detail(&self, _: &str, _: Urgency) -> Result<IssueDetail, ClientError> {
        Err(ClientError::InvalidKey)
    }

    async fn search_issues(&self, _: &str) -> Result<SearchResults, ClientError> {
        Err(ClientError::InvalidKey)
    }

    async fn teams(&self) -> Result<Vec<Team>, ClientError> {
        Err(ClientError::InvalidKey)
    }

    async fn team_options(&self, _: &str) -> Result<TeamOptions, ClientError> {
        Err(ClientError::InvalidKey)
    }

    async fn team_issues(
        &self,
        _: &TeamQuery,
        _: Option<&str>,
    ) -> Result<TeamIssuesPage, ClientError> {
        Err(ClientError::InvalidKey)
    }

    async fn count_team_issues(&self, _: &TeamQuery) -> Result<usize, ClientError> {
        Err(ClientError::InvalidKey)
    }

    async fn analytics_issues(&self, _: AnalyticsWindow) -> Result<AnalyticsIssues, ClientError> {
        Err(ClientError::InvalidKey)
    }

    async fn create_comment(
        &self,
        _: &str,
        _: &str,
    ) -> Result<linear_rat::linear::PostedComment, ClientError> {
        Err(ClientError::InvalidKey)
    }

    async fn create_issue(
        &self,
        _: &linear_rat::linear::IssueCreate,
    ) -> Result<Issue, ClientError> {
        Err(ClientError::InvalidKey)
    }

    async fn administrable_teams(&self) -> Result<Vec<String>, ClientError> {
        Err(ClientError::InvalidKey)
    }

    async fn team_analytics(
        &self,
        _: &str,
        _: AnalyticsWindow,
    ) -> Result<TeamAnalyticsIssues, ClientError> {
        Err(ClientError::InvalidKey)
    }
}

/// A workspace file path in a fresh directory for one test.
fn workspace_path() -> PathBuf {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!(
        "auth-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::SeqCst)
    ));
    dir.join("keyring-workspace")
}

fn workspace_file() -> WorkspaceFile {
    WorkspaceFile::new(workspace_path())
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
async fn a_revoked_linear_api_key_says_to_replace_or_unset_it() {
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
        message.contains("put it in LINEAR_API_KEY, or unset LINEAR_API_KEY"),
        "{message}"
    );
    assert!(!message.contains("lrat auth login"), "{message}");
}

/// A workspace file whose directory can be read but not written, so reading
/// finds nothing (or the old URL key) and writing fails.
fn unwritable_workspace_file(previous: Option<&str>) -> (WorkspaceFile, PathBuf) {
    let path = workspace_path();
    let file = WorkspaceFile::new(&path);
    let dir = path.parent().unwrap().to_owned();
    fs::create_dir_all(&dir).unwrap();
    if let Some(url_key) = previous {
        file.write(url_key).unwrap();
    }
    fs::set_permissions(&dir, fs::Permissions::from_mode(0o500)).unwrap();
    if previous.is_some() {
        fs::set_permissions(&path, fs::Permissions::from_mode(0o400)).unwrap();
    }
    (file, dir)
}

fn make_writable(dir: &PathBuf) {
    fs::set_permissions(dir, fs::Permissions::from_mode(0o700)).unwrap();
}

#[tokio::test]
async fn a_first_login_stores_nothing_when_the_workspace_file_cannot_be_written() {
    let (file, dir) = unwritable_workspace_file(None);
    let credentials = Credentials::new(MemoryStore::default(), file, None);

    let result = auth::login(&credentials, key(SECRET), |_| Ok(FakeClient)).await;
    make_writable(&dir);

    assert!(
        matches!(result, Err(AuthError::WorkspaceFile { .. })),
        "{result:?}"
    );
    assert!(credentials.store().entries().is_empty());
}

#[tokio::test]
async fn a_login_that_cannot_update_the_workspace_file_keeps_the_previous_login() {
    let (file, dir) = unwritable_workspace_file(Some("old-workspace"));
    let credentials = Credentials::new(
        MemoryStore::holding("old-workspace", OTHER_SECRET),
        file,
        None,
    );

    let result = auth::login(&credentials, key(SECRET), |_| Ok(FakeClient)).await;
    make_writable(&dir);

    assert!(
        matches!(result, Err(AuthError::WorkspaceFile { .. })),
        "{result:?}"
    );
    assert_eq!(
        credentials.store().entries(),
        HashMap::from([("old-workspace".to_owned(), OTHER_SECRET.to_owned())])
    );
    assert_eq!(
        credentials.workspace_file().read().unwrap().as_deref(),
        Some("old-workspace")
    );
}

#[tokio::test]
async fn a_relogin_that_cannot_update_the_workspace_file_keeps_the_old_key() {
    let (file, dir) = unwritable_workspace_file(Some(URL_KEY));
    let credentials = Credentials::new(MemoryStore::holding(URL_KEY, OTHER_SECRET), file, None);

    let result = auth::login(&credentials, key(SECRET), |_| Ok(FakeClient)).await;
    make_writable(&dir);

    assert!(
        matches!(result, Err(AuthError::WorkspaceFile { .. })),
        "{result:?}"
    );
    assert_eq!(credentials.store().entries()[URL_KEY], OTHER_SECRET);
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

#[test]
fn a_key_source_kind_names_where_the_key_came_from_without_the_account() {
    let keyring = linear_rat::auth::KeySource::Keyring {
        account: URL_KEY.to_owned(),
    };

    assert_eq!(linear_rat::auth::KeySource::Env.kind(), "LINEAR_API_KEY");
    assert_eq!(keyring.kind(), "OS keyring");
    assert!(!keyring.kind().contains(URL_KEY));
}

/// Other keyring entries, by service and account, as the viewer made them.
#[derive(Default)]
struct MemoryEntries(HashMap<(String, String), String>);

impl MemoryEntries {
    fn holding(service: &str, account: &str, secret: &str) -> Self {
        Self(HashMap::from([(
            (service.to_owned(), account.to_owned()),
            secret.to_owned(),
        )]))
    }
}

impl EntryReader for MemoryEntries {
    fn read(&self, entry: &KeyringEntry) -> Result<Option<String>, KeyringError> {
        Ok(self
            .0
            .get(&(entry.service().to_owned(), entry.account().to_owned()))
            .cloned())
    }
}

impl EntryReader for UnavailableStore {
    fn read(&self, _: &KeyringEntry) -> Result<Option<String>, KeyringError> {
        Err(unavailable())
    }
}

fn entry(value: &str) -> KeyringEntry {
    value.parse().unwrap()
}

#[tokio::test]
async fn login_from_the_default_entry_checks_that_key_and_stores_it_as_lrat_s_own() {
    let credentials = Credentials::new(MemoryStore::default(), workspace_file(), None);
    let entries = MemoryEntries::holding("linear", "api-key", &format!("{SECRET}\n"));
    let used = Mutex::new(Vec::new());

    let output = auth::login_from_keyring(
        &credentials,
        &entries,
        &entry(KeyringEntry::DEFAULT),
        fake_connect(&used),
    )
    .await
    .unwrap();

    assert_eq!(*used.lock().unwrap(), [SECRET]);
    assert_eq!(
        credentials.store().entries(),
        HashMap::from([(URL_KEY.to_owned(), SECRET.to_owned())])
    );
    assert_eq!(
        output,
        "Read the API key from the OS keyring (service linear, account api-key).\n\
         Logged in to Cheddar Works (cheddar-works) as Remy Fromage.\n\
         The API key is in the OS keyring (service linear-rat, account cheddar-works)."
    );
    assert!(!output.contains(SECRET));
}

#[tokio::test]
async fn login_from_a_custom_entry_reads_that_service_and_account() {
    let credentials = Credentials::new(MemoryStore::default(), workspace_file(), None);
    let mut entries = MemoryEntries::holding("linear", "api-key", OTHER_SECRET);
    entries.0.insert(
        ("work".to_owned(), "me@example.com".to_owned()),
        SECRET.to_owned(),
    );
    let used = Mutex::new(Vec::new());

    auth::login_from_keyring(
        &credentials,
        &entries,
        &entry("work/me@example.com"),
        fake_connect(&used),
    )
    .await
    .unwrap();

    assert_eq!(*used.lock().unwrap(), [SECRET]);
    assert_eq!(credentials.store().entries()[URL_KEY], SECRET);
}

#[tokio::test]
async fn login_from_a_missing_entry_names_it_and_stores_nothing() {
    let credentials = Credentials::new(MemoryStore::default(), workspace_file(), None);
    let used = Mutex::new(Vec::new());

    let error = auth::login_from_keyring(
        &credentials,
        &MemoryEntries::default(),
        &entry("work/linear-key"),
        fake_connect(&used),
    )
    .await
    .unwrap_err();

    assert!(matches!(error, AuthError::NoEntry(_)), "{error:?}");
    assert_eq!(
        error.to_string(),
        "the OS keyring has no entry with service work, account linear-key. Nothing was \
         stored. Add the API key there, or name another entry as --from-keyring SERVICE/ACCOUNT"
    );
    assert!(used.lock().unwrap().is_empty());
    assert!(credentials.store().entries().is_empty());
    assert_eq!(credentials.workspace_file().read().unwrap(), None);
}

#[tokio::test]
async fn login_from_an_unavailable_keyring_says_so_and_stores_nothing() {
    let credentials = Credentials::new(MemoryStore::default(), workspace_file(), None);
    let used = Mutex::new(Vec::new());

    let error = auth::login_from_keyring(
        &credentials,
        &UnavailableStore,
        &entry(KeyringEntry::DEFAULT),
        fake_connect(&used),
    )
    .await
    .unwrap_err();

    let message = error.to_string();
    assert!(
        message.starts_with(
            "could not read the entry with service linear, account api-key: \
             the OS keyring is unavailable (org.freedesktop.secrets"
        ),
        "{message}"
    );
    assert!(message.ends_with("Nothing was stored."), "{message}");
    assert!(used.lock().unwrap().is_empty());
    assert!(credentials.store().entries().is_empty());
}

#[tokio::test]
async fn login_from_an_entry_holding_a_rejected_key_names_the_entry_and_stores_nothing() {
    let credentials = Credentials::new(MemoryStore::default(), workspace_file(), None);
    let entries = MemoryEntries::holding("linear", "api-key", SECRET);

    let error = auth::login_from_keyring(
        &credentials,
        &entries,
        &entry(KeyringEntry::DEFAULT),
        |_| Ok(RejectingClient),
    )
    .await
    .unwrap_err();

    let message = error.to_string();
    assert!(matches!(error, AuthError::EntryRejected(_)), "{error:?}");
    assert!(
        message.starts_with(
            "Linear rejected the API key in the OS keyring entry with service linear, \
             account api-key: it is wrong, revoked or expired. Nothing was stored."
        ),
        "{message}"
    );
    assert!(!message.contains(SECRET), "{message}");
    assert!(!format!("{error:?}").contains(SECRET));
    assert!(credentials.store().entries().is_empty());
    assert_eq!(credentials.workspace_file().read().unwrap(), None);
}

#[tokio::test]
async fn login_from_an_entry_that_is_not_an_api_key_hides_its_value_and_stores_nothing() {
    let credentials = Credentials::new(MemoryStore::default(), workspace_file(), None);
    let secret = "my secret passphrase";
    let entries = MemoryEntries::holding("linear", "api-key", secret);

    let error = auth::login_from_keyring(
        &credentials,
        &entries,
        &entry(KeyringEntry::DEFAULT),
        |_| Ok(FakeClient),
    )
    .await
    .unwrap_err();

    let message = error.to_string();
    assert!(
        message.starts_with(
            "the OS keyring entry with service linear, account api-key does not hold a \
             valid API key: the API key holds a space"
        ),
        "{message}"
    );
    assert!(!message.contains(secret), "{message}");
    assert!(!format!("{error:?}").contains(secret));
    assert!(credentials.store().entries().is_empty());
}
