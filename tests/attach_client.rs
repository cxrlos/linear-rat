//! The real client's attachments against a local fake server standing in
//! for both Linear and the upload host: `fileUpload`, the PUT to the URL it
//! signs, `attachmentCreate` and `attachmentLinkURL`.

// clippy's allow-unwrap-in-tests does not cover helpers outside #[test] functions.
#![allow(clippy::unwrap_used)]

use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use linear_rat::auth::ApiKey;
use linear_rat::linear::{
    ClientConfig, ClientError, ErrorKind, FileUpload, HttpClient, LinearClient,
};
use serde_json::{Value, json};
use wiremock::matchers::{body_string_contains, method, path};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

const KEY: &str = "lin_api_fictional0000000000000000000000000000";
const ASSET: &str = "https://uploads.linear.app/workspace/asset-1/rind.log";

fn client_for(server: &MockServer) -> HttpClient {
    let config = ClientConfig {
        endpoint: format!("{}/graphql", server.uri()),
        connect_timeout: Duration::from_millis(300),
        request_timeout: Duration::from_secs(5),
    };
    HttpClient::with_config(&ApiKey::new(KEY).unwrap(), config).unwrap()
}

/// A file of `bytes` for one test, and what the prompt would make of it.
fn file(test: &str, bytes: &[u8]) -> FileUpload {
    let dir = std::env::temp_dir().join(format!("lrat-{test}-{}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    let path: PathBuf = dir.join("rind.log");
    fs::write(&path, bytes).unwrap();
    FileUpload {
        path,
        name: "rind.log".to_owned(),
        content_type: "text/plain".to_owned(),
        size: bytes.len() as u64,
    }
}

fn attachment_node(url: &str, title: &str) -> Value {
    json!({
        "id": "attachment-9",
        "url": url,
        "title": title,
        "subtitle": null,
        "sourceType": null,
        "metadata": {}
    })
}

/// Linear signing an upload to `/upload/rind` on `server` with two headers,
/// accepting the PUT, and storing the attachment.
async fn mount_upload(server: &MockServer, put_status: u16) {
    Mock::given(method("POST"))
        .and(body_string_contains("fileUpload"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({ "data": { "fileUpload": {
            "success": true,
            "uploadFile": {
                "uploadUrl": format!("{}/upload/rind?signature=abc", server.uri()),
                "assetUrl": ASSET,
                "headers": [
                    { "key": "Content-Type", "value": "text/plain" },
                    { "key": "x-goog-meta-signed", "value": "yes" }
                ]
            }
        } } })),
        )
        .mount(server)
        .await;
    Mock::given(method("PUT"))
        .and(path("/upload/rind"))
        .respond_with(ResponseTemplate::new(put_status))
        .mount(server)
        .await;
    Mock::given(method("POST"))
        .and(body_string_contains("attachmentCreate"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "data": {
            "attachmentCreate": { "success": true, "attachment": attachment_node(ASSET, "rind.log") }
        } })))
        .mount(server)
        .await;
}

async fn requests(server: &MockServer) -> Vec<Request> {
    server.received_requests().await.unwrap()
}

#[tokio::test]
async fn a_file_goes_to_the_signed_url_with_only_the_returned_headers_then_attaches_its_asset_url()
{
    let server = MockServer::start().await;
    mount_upload(&server, 200).await;
    let upload = file("attach-ok", b"aged twelve months");
    let heard = Arc::new(Mutex::new(Vec::new()));
    let progress = {
        let heard = Arc::clone(&heard);
        move |sent| heard.lock().unwrap().push(sent)
    };

    let attachment = client_for(&server)
        .attach_file("issue-1", &upload, progress)
        .await
        .unwrap();

    assert_eq!(attachment.url, ASSET);
    assert_eq!(attachment.title, "rind.log");
    assert_eq!(attachment.issue_id, "issue-1");
    let requests = requests(&server).await;
    assert_eq!(requests.len(), 3, "{requests:?}");

    let asked: Value = requests[0].body_json().unwrap();
    assert_eq!(
        asked["variables"],
        json!({ "contentType": "text/plain", "filename": "rind.log", "size": 18 })
    );

    let put = &requests[1];
    assert_eq!(put.method.as_str(), "PUT");
    assert_eq!(put.url.query(), Some("signature=abc"));
    assert_eq!(put.body, b"aged twelve months");
    assert_eq!(put.headers.get("content-type").unwrap(), "text/plain");
    assert_eq!(put.headers.get("x-goog-meta-signed").unwrap(), "yes");
    assert_eq!(put.headers.get("content-length").unwrap(), "18");
    assert!(
        put.headers.get("authorization").is_none(),
        "{:?}",
        put.headers
    );

    let attached: Value = requests[2].body_json().unwrap();
    assert_eq!(
        attached["variables"]["input"],
        json!({ "issueId": "issue-1", "url": ASSET, "title": "rind.log" })
    );
    assert_eq!(heard.lock().unwrap().last(), Some(&18));
}

#[tokio::test]
async fn a_refused_upload_attaches_nothing_and_is_not_about_the_api_key() {
    let server = MockServer::start().await;
    mount_upload(&server, 403).await;
    let upload = file("attach-refused", b"aged");

    let error = client_for(&server)
        .attach_file("issue-1", &upload, |_| {})
        .await
        .unwrap_err();

    assert!(matches!(error, ClientError::Upload(_)), "{error:?}");
    assert_eq!(error.kind(), ErrorKind::Other);
    assert!(error.to_string().contains("403"), "{error}");
    let requests = requests(&server).await;
    assert_eq!(requests.len(), 2, "attachmentCreate went out: {requests:?}");
}

#[tokio::test]
async fn a_file_gone_before_the_upload_asks_linear_for_nothing() {
    let server = MockServer::start().await;
    mount_upload(&server, 200).await;
    let mut upload = file("attach-gone", b"aged");
    upload.path = upload.path.with_file_name("not-there.log");

    let error = client_for(&server)
        .attach_file("issue-1", &upload, |_| {})
        .await
        .unwrap_err();

    assert!(matches!(error, ClientError::ReadFile(_)), "{error:?}");
    assert!(requests(&server).await.is_empty());
}

#[tokio::test]
async fn a_link_sends_the_issue_url_and_title_and_reads_back_the_attachment() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "data": {
            "attachmentLinkURL": {
                "success": true,
                "attachment": attachment_node("https://example.com/cave", "Cave notes")
            }
        } })))
        .mount(&server)
        .await;

    let attachment = client_for(&server)
        .attach_link("issue-1", "https://example.com/cave", Some("Cave notes"))
        .await
        .unwrap();

    assert_eq!(attachment.title, "Cave notes");
    let sent: Value = requests(&server).await[0].body_json().unwrap();
    assert!(
        sent["query"]
            .as_str()
            .unwrap()
            .contains("attachmentLinkURL")
    );
    assert_eq!(
        sent["variables"],
        json!({ "issueId": "issue-1", "url": "https://example.com/cave", "title": "Cave notes" })
    );
}

#[tokio::test]
async fn a_link_without_a_title_lets_linear_name_it() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "data": {
            "attachmentLinkURL": {
                "success": true,
                "attachment": attachment_node("https://example.com/cave", "example.com")
            }
        } })))
        .mount(&server)
        .await;

    client_for(&server)
        .attach_link("issue-1", "https://example.com/cave", None)
        .await
        .unwrap();

    let sent: Value = requests(&server).await[0].body_json().unwrap();
    assert_eq!(
        sent["variables"],
        json!({ "issueId": "issue-1", "url": "https://example.com/cave", "title": null })
    );
}
