//! The real client against a local fake HTTP server.

// clippy's allow-unwrap-in-tests does not cover helpers outside #[test] functions.
#![allow(clippy::unwrap_used)]

use std::time::{Duration, Instant};

use linear_rat::auth::ApiKey;
use linear_rat::linear::model::{Viewer, Workspace};
use linear_rat::linear::{ClientConfig, ClientError, HttpClient, LinearClient};
use serde::Deserialize;
use serde_json::json;
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const KEY: &str = "lin_api_fictional0000000000000000000000000000";

fn client_for(server: &MockServer) -> HttpClient {
    client_with(&format!("{}/graphql", server.uri()), Duration::from_secs(5))
}

fn client_with(endpoint: &str, request_timeout: Duration) -> HttpClient {
    let config = ClientConfig {
        endpoint: endpoint.to_owned(),
        connect_timeout: Duration::from_millis(300),
        request_timeout,
    };
    HttpClient::with_config(&ApiKey::new(KEY).unwrap(), config).unwrap()
}

fn viewer_body() -> serde_json::Value {
    json!({
        "data": {
            "viewer": { "id": "u1", "name": "Remy Fromage" },
            "organization": { "id": "o1", "name": "Cheddar Works", "urlKey": "cheddar-works" }
        }
    })
}

#[tokio::test]
async fn sends_the_key_as_the_authorization_header_without_bearer() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/graphql"))
        .and(header("authorization", KEY))
        .and(header("content-type", "application/json"))
        .respond_with(ResponseTemplate::new(200).set_body_json(viewer_body()))
        .expect(1)
        .mount(&server)
        .await;

    let (viewer, workspace) = client_for(&server).viewer().await.unwrap();

    assert_eq!(
        viewer,
        Viewer {
            id: "u1".to_owned(),
            name: "Remy Fromage".to_owned()
        }
    );
    assert_eq!(
        workspace,
        Workspace {
            id: "o1".to_owned(),
            name: "Cheddar Works".to_owned(),
            url_key: "cheddar-works".to_owned()
        }
    );
    let requests = server.received_requests().await.unwrap();
    let body: serde_json::Value = requests[0].body_json().unwrap();
    assert!(body["query"].as_str().unwrap().contains("query Viewer"));
}

#[tokio::test]
async fn keeps_partial_data_and_surfaces_the_errors_beside_it() {
    #[derive(Debug, Deserialize, PartialEq)]
    struct Data {
        viewer: Option<serde_json::Value>,
        organization: Option<serde_json::Value>,
    }
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "data": { "viewer": { "id": "u1" }, "organization": null },
            "errors": [{
                "message": "Entity not found",
                "path": ["organization"],
                "extensions": { "code": "INPUT_ERROR" }
            }]
        })))
        .mount(&server)
        .await;

    let response = client_for(&server)
        .execute::<Data>("query { viewer { id } organization { id } }", json!({}))
        .await
        .unwrap();

    assert_eq!(response.data.viewer, Some(json!({ "id": "u1" })));
    assert_eq!(response.data.organization, None);
    assert_eq!(response.errors.len(), 1);
    assert_eq!(response.errors[0].message, "Entity not found");
    assert_eq!(response.errors[0].path, vec![json!("organization")]);
    assert_eq!(response.errors[0].code(), Some("INPUT_ERROR"));
}

#[tokio::test]
async fn errors_without_data_are_an_error() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "data": null,
            "errors": [{ "message": "Something broke" }]
        })))
        .mount(&server)
        .await;

    let error = client_for(&server).viewer().await.unwrap_err();

    assert!(
        matches!(&error, ClientError::GraphQl(errors) if errors[0].message == "Something broke"),
        "{error:?}"
    );
}

#[tokio::test]
async fn partial_data_the_query_cannot_use_surfaces_linears_errors() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "data": {
                "viewer": { "id": "u1", "name": "Remy Fromage" },
                "organization": null
            },
            "errors": [{ "message": "Workspace is suspended", "path": ["organization"] }]
        })))
        .mount(&server)
        .await;

    let error = client_for(&server).viewer().await.unwrap_err();

    assert!(
        matches!(&error, ClientError::GraphQl(errors) if errors[0].message == "Workspace is suspended"),
        "{error:?}"
    );
    assert!(
        error.to_string().contains("Workspace is suspended"),
        "{error}"
    );
}

#[tokio::test]
async fn an_authentication_error_beside_data_means_the_key_is_invalid() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "data": { "viewer": null, "organization": null },
            "errors": [{
                "message": "Authentication required, not authenticated",
                "extensions": { "code": "AUTHENTICATION_ERROR" }
            }]
        })))
        .mount(&server)
        .await;

    let error = client_for(&server).viewer().await.unwrap_err();

    assert!(matches!(error, ClientError::InvalidKey), "{error:?}");
}

#[tokio::test]
async fn an_authentication_error_means_the_key_is_invalid() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(400).set_body_json(json!({
            "errors": [{
                "message": "Authentication required, not authenticated",
                "extensions": {
                    "type": "authentication error",
                    "code": "AUTHENTICATION_ERROR",
                    "statusCode": 401,
                    "userPresentableMessage": "You need to authenticate to access this operation."
                }
            }]
        })))
        .mount(&server)
        .await;

    let error = client_for(&server).viewer().await.unwrap_err();

    assert!(matches!(error, ClientError::InvalidKey), "{error:?}");
}

#[tokio::test]
async fn an_http_401_means_the_key_is_invalid() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(401))
        .mount(&server)
        .await;

    let error = client_for(&server).viewer().await.unwrap_err();

    assert!(matches!(error, ClientError::InvalidKey), "{error:?}");
}

#[tokio::test]
async fn a_slow_response_hits_the_request_timeout() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(viewer_body())
                .set_delay(Duration::from_secs(5)),
        )
        .mount(&server)
        .await;
    let client = client_with(
        &format!("{}/graphql", server.uri()),
        Duration::from_millis(200),
    );

    let started = Instant::now();
    let error = client.viewer().await.unwrap_err();

    assert!(matches!(error, ClientError::RequestTimeout), "{error:?}");
    assert!(started.elapsed() < Duration::from_secs(3));
}

#[tokio::test]
async fn a_server_that_never_accepts_hits_the_connect_timeout() {
    // A listener whose accept queue is full drops further SYNs, so the next
    // connection attempt hangs in the handshake until the connect timeout.
    let socket = tokio::net::TcpSocket::new_v4().unwrap();
    socket.bind("127.0.0.1:0".parse().unwrap()).unwrap();
    let address = socket.local_addr().unwrap();
    let _listener = socket.listen(0).unwrap();
    let mut fillers = Vec::new();
    for _ in 0..8 {
        match tokio::time::timeout(
            Duration::from_millis(100),
            tokio::net::TcpStream::connect(address),
        )
        .await
        {
            Ok(Ok(stream)) => fillers.push(stream),
            _ => break,
        }
    }
    let client = client_with(
        &format!("http://{address}/graphql"),
        Duration::from_secs(10),
    );

    let started = Instant::now();
    let error = client.viewer().await.unwrap_err();

    assert!(matches!(error, ClientError::ConnectTimeout), "{error:?}");
    assert!(started.elapsed() < Duration::from_secs(3));
}
