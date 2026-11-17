//! Classifying the real client's failures into error kinds, each from a
//! response a local fake server gives.

// clippy's allow-unwrap-in-tests does not cover helpers outside #[test] functions.
#![allow(clippy::unwrap_used)]

use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use jiff::Timestamp;
use linear_rat::app::Failure;
use linear_rat::auth::ApiKey;
use linear_rat::linear::{ClientConfig, ClientError, ErrorKind, HttpClient, LinearClient, Timer};
use serde_json::{Value, json};
use wiremock::matchers::method;
use wiremock::{Mock, MockServer, ResponseTemplate};

const KEY: &str = "lin_api_fictional0000000000000000000000000000";

/// A clock that only moves when the client sleeps, so retries are instant.
#[derive(Debug)]
struct FakeTimer(Mutex<Timestamp>);

impl Timer for FakeTimer {
    fn now(&self) -> Timestamp {
        *self.0.lock().unwrap()
    }

    fn sleep(&self, duration: Duration) -> Pin<Box<dyn Future<Output = ()> + Send>> {
        let mut now = self.0.lock().unwrap();
        *now = now.checked_add(duration).unwrap();
        Box::pin(std::future::ready(()))
    }
}

fn client_at(endpoint: String) -> HttpClient {
    let config = ClientConfig {
        endpoint,
        connect_timeout: Duration::from_millis(300),
        request_timeout: Duration::from_secs(5),
    };
    HttpClient::with_config(&ApiKey::new(KEY).unwrap(), config)
        .unwrap()
        .with_timer(Arc::new(FakeTimer(Mutex::new(Timestamp::UNIX_EPOCH))))
}

/// The error the viewer query ends with when Linear answers `response`.
async fn error_for(response: ResponseTemplate) -> ClientError {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(response)
        .mount(&server)
        .await;
    let error = client_at(format!("{}/graphql", server.uri()))
        .viewer()
        .await
        .unwrap_err();
    let logged = Failure::from_client("load the viewer", &error);
    assert!(!format!("{logged:?}").contains(KEY), "{logged:?}");
    error
}

fn errors(error: Value) -> Value {
    json!({ "data": null, "errors": [error] })
}

#[tokio::test]
async fn a_refused_connection_is_offline() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    drop(listener);

    let error = client_at(format!("http://{address}/graphql"))
        .viewer()
        .await
        .unwrap_err();

    assert_eq!(error.kind(), ErrorKind::Offline, "{error:?}");
}

#[tokio::test]
async fn an_http_401_is_an_invalid_key() {
    let error = error_for(ResponseTemplate::new(401)).await;

    assert_eq!(error.kind(), ErrorKind::KeyInvalid);
}

#[tokio::test]
async fn an_http_403_with_a_forbidden_code_is_forbidden() {
    let body = errors(json!({
        "message": "You don't have permission to do this",
        "extensions": { "type": "forbidden", "code": "FORBIDDEN", "userError": true }
    }));

    let error = error_for(ResponseTemplate::new(403).set_body_json(body)).await;

    assert_eq!(error.kind(), ErrorKind::Forbidden);
}

#[tokio::test]
async fn a_bare_http_403_is_forbidden() {
    let error = error_for(ResponseTemplate::new(403)).await;

    assert_eq!(error.kind(), ErrorKind::Forbidden);
}

#[tokio::test]
async fn a_429_that_never_lifts_is_rate_limited_with_its_retry_time() {
    let response = ResponseTemplate::new(429).insert_header("retry-after", "30");

    let error = error_for(response).await;

    assert_eq!(
        error.kind(),
        ErrorKind::RateLimited {
            retry_after: Some(Duration::from_secs(30))
        }
    );
}

#[tokio::test]
async fn a_5xx_is_a_server_error() {
    let error = error_for(ResponseTemplate::new(503)).await;

    assert_eq!(error.kind(), ErrorKind::ServerError);
}

#[tokio::test]
async fn an_internal_error_code_is_a_server_error() {
    let body = errors(json!({
        "message": "Internal server error",
        "extensions": { "code": "INTERNAL_SERVER_ERROR" }
    }));

    let error = error_for(ResponseTemplate::new(500).set_body_json(body)).await;

    assert_eq!(error.kind(), ErrorKind::ServerError);
}

#[tokio::test]
async fn data_that_does_not_decode_is_schema_drift() {
    let body = json!({ "data": { "viewer": { "id": 7 }, "organization": null } });

    let error = error_for(ResponseTemplate::new(200).set_body_json(body)).await;

    assert!(matches!(error, ClientError::Decode(_)), "{error:?}");
    assert_eq!(error.kind(), ErrorKind::SchemaDrift);
}

#[tokio::test]
async fn an_unknown_field_is_schema_drift() {
    let body = errors(json!({
        "message": "Cannot query field \"cheese\" on type \"Issue\".",
        "extensions": { "code": "GRAPHQL_VALIDATION_FAILED" }
    }));

    let error = error_for(ResponseTemplate::new(400).set_body_json(body)).await;

    assert_eq!(error.kind(), ErrorKind::SchemaDrift);
}

/// The shape of `validationErrors` is assumed, not captured from Linear.
#[tokio::test]
async fn invalid_input_is_a_validation_error_naming_its_field() {
    let body = errors(json!({
        "message": "Argument Validation Error",
        "extensions": {
            "type": "invalid input",
            "code": "INVALID_INPUT",
            "userError": true,
            "validationErrors": [{
                "property": "title",
                "constraints": { "maxLength": "title must be shorter than 256 characters" }
            }]
        }
    }));

    let error = error_for(ResponseTemplate::new(400).set_body_json(body)).await;

    assert_eq!(
        error.kind(),
        ErrorKind::Validation {
            field: Some("title".to_owned())
        }
    );
}

#[tokio::test]
async fn an_input_error_without_a_field_is_still_a_validation_error() {
    let body = errors(json!({
        "message": "Entity not found",
        "extensions": { "code": "INPUT_ERROR" }
    }));

    let error = error_for(ResponseTemplate::new(200).set_body_json(body)).await;

    assert_eq!(error.kind(), ErrorKind::Validation { field: None });
}

#[tokio::test]
async fn an_unknown_code_is_other_and_kept_in_the_message() {
    let body = errors(json!({
        "message": "The cheese has gone off",
        "extensions": { "code": "CHEESE_EXPIRED" }
    }));

    let error = error_for(ResponseTemplate::new(400).set_body_json(body)).await;

    assert_eq!(error.kind(), ErrorKind::Other);
    assert!(error.to_string().contains("(CHEESE_EXPIRED)"), "{error}");
}

#[test]
fn every_kind_has_a_one_line_message_and_a_fix() {
    let kinds = [
        ErrorKind::Offline,
        ErrorKind::RateLimited {
            retry_after: Some(Duration::from_secs(90)),
        },
        ErrorKind::RateLimited { retry_after: None },
        ErrorKind::KeyInvalid,
        ErrorKind::Forbidden,
        ErrorKind::Validation {
            field: Some("title".to_owned()),
        },
        ErrorKind::Validation { field: None },
        ErrorKind::ServerError,
        ErrorKind::SchemaDrift,
        ErrorKind::Other,
    ];
    let lines: Vec<String> = kinds
        .iter()
        .map(|kind| format!("{} | {} | {}", kind.name(), kind.summary(), kind.fix()))
        .collect();

    for line in &lines {
        assert!(!line.contains('\n'), "{line}");
        assert!(!line.contains(KEY), "{line}");
    }
    insta::assert_snapshot!(lines.join("\n"));
}
