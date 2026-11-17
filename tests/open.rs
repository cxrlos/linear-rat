//! `lrat open <KEY>`: resolving an issue key and handing its URL to the
//! browser opener, against the fake client and a local fake server.

// clippy's allow-unwrap-in-tests does not cover helpers outside #[test] functions.
#![allow(clippy::unwrap_used)]

use std::cell::RefCell;
use std::path::PathBuf;
use std::time::Duration;

use clap::Parser;
use linear_rat::auth::{ApiKey, Credentials, KeyStore, KeyringError, KeyringStore, WorkspaceFile};
use linear_rat::cli::{Cli, Command};
use linear_rat::linear::{ClientConfig, ClientError, FakeClient, HttpClient, LinearClient};
use linear_rat::open::{self, IssueKey};
use serde_json::json;
use wiremock::matchers::method;
use wiremock::{Mock, MockServer, ResponseTemplate};

const SECRET: &str = "lin_api_fictional0000000000000000000000000000";

/// A keyring with nothing in it.
struct EmptyStore;

impl KeyStore for EmptyStore {
    fn kind(&self) -> KeyringStore {
        KeyringStore::Os
    }

    fn get(&self, _: &str) -> Result<Option<ApiKey>, KeyringError> {
        Ok(None)
    }

    fn set(&self, _: &str, _: &ApiKey) -> Result<(), KeyringError> {
        Ok(())
    }

    fn delete(&self, _: &str) -> Result<bool, KeyringError> {
        Ok(false)
    }
}

/// A workspace file that does not exist, so only `LINEAR_API_KEY` can give a
/// key.
fn no_workspace_file() -> WorkspaceFile {
    WorkspaceFile::new(PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("open-no-workspace-file"))
}

fn with_key() -> Credentials<EmptyStore> {
    Credentials::new(EmptyStore, no_workspace_file(), Some(SECRET.to_owned()))
}

fn without_key() -> Credentials<EmptyStore> {
    Credentials::new(EmptyStore, no_workspace_file(), None)
}

fn key(raw: &str) -> IssueKey {
    raw.parse().unwrap()
}

/// Runs `lrat open` for `raw` with `client`, recording what was opened.
async fn run<C: LinearClient>(
    credentials: &Credentials<EmptyStore>,
    raw: &str,
    client: C,
) -> (Result<String, String>, Vec<String>) {
    let opened = RefCell::new(Vec::new());
    let result = open::open(
        credentials,
        &key(raw),
        |_| Ok(client),
        |url| {
            opened.borrow_mut().push(url.to_owned());
            Ok(())
        },
    )
    .await
    .map_err(|error| format!("{:#}", anyhow::Error::from(error)));
    (result, opened.into_inner())
}

#[tokio::test]
async fn a_known_key_hands_its_issue_url_to_the_opener() {
    let (result, opened) = run(&with_key(), "DES-34", FakeClient).await;

    assert_eq!(opened, ["https://linear.app/squeakworks/issue/DES-34"]);
    let printed = result.unwrap();
    assert!(printed.starts_with("Opened DES-34 "), "{printed}");
}

#[tokio::test]
async fn the_key_matches_whatever_its_case() {
    let (_, opened) = run(&with_key(), "des-34", FakeClient).await;

    assert_eq!(opened, ["https://linear.app/squeakworks/issue/DES-34"]);
}

#[tokio::test]
async fn an_unknown_key_says_there_is_no_such_issue_and_opens_nothing() {
    let (result, opened) = run(&with_key(), "eng-9999", FakeClient).await;

    assert_eq!(result.unwrap_err(), "no issue ENG-9999 in this workspace");
    assert!(opened.is_empty());
}

#[tokio::test]
async fn without_a_key_it_says_what_the_tui_says() {
    let (result, opened) = run(&without_key(), "DES-34", FakeClient).await;

    assert_eq!(
        result.unwrap_err(),
        "no API key: run `lrat auth login`, or set LINEAR_API_KEY"
    );
    assert!(opened.is_empty());
}

#[tokio::test]
async fn an_opener_that_fails_is_an_error() {
    let result = open::open(
        &with_key(),
        &key("DES-34"),
        |_| Ok(FakeClient),
        |_| Err(anyhow::anyhow!("could not open the browser with xdg-open")),
    )
    .await;

    let message = format!("{:#}", anyhow::Error::from(result.unwrap_err()));
    assert_eq!(message, "could not open the browser with xdg-open");
}

#[test]
fn issue_keys_parse_case_insensitively_and_print_upper_case() {
    assert_eq!(key("eng-123").to_string(), "ENG-123");
    assert_eq!(key("Ops2-7").to_string(), "OPS2-7");
}

#[test]
fn malformed_keys_are_refused_with_the_expected_shape() {
    for raw in [
        "",
        "ENG",
        "ENG-",
        "-123",
        "ENG-12a",
        "ENG-0",
        "1NG-5",
        "ENG_X-5",
        "ENG-123-4",
    ] {
        let error = raw.parse::<IssueKey>().unwrap_err();
        assert_eq!(error, "expected an issue key such as ENG-123", "{raw:?}");
    }
}

#[test]
fn a_malformed_key_is_a_usage_error_before_anything_runs() {
    let error = Cli::try_parse_from(["lrat", "open", "nonsense"]).unwrap_err();

    assert_eq!(error.kind(), clap::error::ErrorKind::ValueValidation);
    let message = error.to_string();
    assert!(
        message.contains("expected an issue key such as ENG-123"),
        "{message}"
    );
    assert!(
        message.contains("invalid value 'nonsense' for '<KEY>'"),
        "{message}"
    );
}

#[test]
fn open_parses_its_key() {
    let cli = Cli::try_parse_from(["lrat", "open", "eng-5"]).unwrap();

    match cli.command {
        Some(Command::Open { key }) => assert_eq!(key.to_string(), "ENG-5"),
        other => panic!("not open: {other:?}"),
    }
}

fn client_for(server: &MockServer) -> HttpClient {
    let config = ClientConfig {
        endpoint: format!("{}/graphql", server.uri()),
        connect_timeout: Duration::from_millis(300),
        request_timeout: Duration::from_secs(5),
    };
    HttpClient::with_config(&ApiKey::new(SECRET).unwrap(), config).unwrap()
}

#[tokio::test]
async fn the_real_client_looks_the_key_up_by_team_key_and_number() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "data": { "issues": { "nodes": [{
                "identifier": "ENG-123",
                "title": "Melt the cheese",
                "url": "https://linear.app/cheddar-works/issue/ENG-123/melt-the-cheese"
            }] } }
        })))
        .expect(1)
        .mount(&server)
        .await;

    let found = client_for(&server).issue_by_key("ENG", 123).await.unwrap();

    let found = found.unwrap();
    assert_eq!(found.identifier, "ENG-123");
    assert_eq!(found.title, "Melt the cheese");
    assert_eq!(
        found.url,
        "https://linear.app/cheddar-works/issue/ENG-123/melt-the-cheese"
    );
    let requests = server.received_requests().await.unwrap();
    let body: serde_json::Value = requests[0].body_json().unwrap();
    assert!(body["query"].as_str().unwrap().contains("query IssueByKey"));
    assert_eq!(body["variables"], json!({ "team": "ENG", "number": 123 }));
}

#[tokio::test]
async fn the_real_client_finds_nothing_for_an_empty_result() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({ "data": { "issues": { "nodes": [] } } })),
        )
        .mount(&server)
        .await;

    let found = client_for(&server).issue_by_key("ENG", 9).await.unwrap();

    assert!(found.is_none());
}

#[tokio::test]
async fn a_rejected_key_is_reported_as_the_client_error() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(401))
        .mount(&server)
        .await;

    let error = client_for(&server)
        .issue_by_key("ENG", 9)
        .await
        .unwrap_err();

    assert!(matches!(error, ClientError::InvalidKey), "{error:?}");
}
