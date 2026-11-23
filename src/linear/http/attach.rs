//! Attaching to an issue: a link through `attachmentLinkURL`, and a file
//! through `fileUpload`, a PUT of its bytes to the URL Linear signs, then
//! `attachmentCreate` with the uploaded file's asset URL (ADR 0004).

use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::Duration;

use bytes::Bytes;
use http_body::{Body, Frame, SizeHint};
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use serde::Deserialize;
use serde_json::json;

use super::HttpClient;
use super::detail::AttachmentNode;
use crate::linear::model::Attachment;
use crate::linear::{ClientError, FileUpload, GraphQlError, Response};

const FILE_UPLOAD: &str = include_str!("../graphql/file_upload.graphql");
const ATTACHMENT_CREATE: &str = include_str!("../graphql/attachment_create.graphql");
const ATTACHMENT_LINK_URL: &str = include_str!("../graphql/attachment_link_url.graphql");

/// How many bytes go out between progress reports.
const CHUNK: usize = 64 * 1024;

/// Time allowed for the whole PUT. The client's own timeout suits a GraphQL
/// answer, not 50 MB over a slow link.
const UPLOAD_TIMEOUT: Duration = Duration::from_secs(10 * 60);

pub(super) async fn attach_file(
    client: &HttpClient,
    issue_id: &str,
    file: &FileUpload,
    progress: impl FnMut(u64) + Send + Sync + 'static,
) -> Result<Attachment, ClientError> {
    // Read first, so a file gone since the prompt asks Linear for nothing.
    let bytes = tokio::fs::read(&file.path)
        .await
        .map_err(|error| ClientError::ReadFile(format!("{}: {error}", file.path.display())))?;
    let size = bytes.len();
    let response: Response<UploadData> = client
        .execute(
            FILE_UPLOAD,
            json!({
                "contentType": file.content_type,
                "filename": file.name,
                "size": size,
            }),
        )
        .await?;
    let payload = response.data.file_upload;
    let upload = match payload.upload_file {
        Some(upload) if payload.success => upload,
        _ => return Err(unconfirmed(response.errors, "upload")),
    };
    put(client, &upload, bytes, progress).await?;
    let response: Response<CreateData> = client
        .execute(
            ATTACHMENT_CREATE,
            json!({ "input": {
                "issueId": issue_id,
                "url": upload.asset_url,
                "title": file.name,
            } }),
        )
        .await?;
    confirmed(response.data.attachment_create, response.errors, issue_id)
}

pub(super) async fn attach_link(
    client: &HttpClient,
    issue_id: &str,
    url: &str,
    title: Option<&str>,
) -> Result<Attachment, ClientError> {
    let response: Response<LinkData> = client
        .execute(
            ATTACHMENT_LINK_URL,
            json!({ "issueId": issue_id, "url": url, "title": title }),
        )
        .await?;
    confirmed(response.data.attachment_link_url, response.errors, issue_id)
}

/// PUTs `bytes` to the signed URL with only the headers Linear returned:
/// the API key never goes to the upload host.
async fn put(
    client: &HttpClient,
    upload: &UploadFile,
    bytes: Vec<u8>,
    progress: impl FnMut(u64) + Send + Sync + 'static,
) -> Result<(), ClientError> {
    let mut headers = HeaderMap::new();
    for header in &upload.headers {
        let name = HeaderName::from_bytes(header.key.as_bytes())
            .map_err(|_| ClientError::Upload(format!("Linear sent a bad header {}", header.key)))?;
        let value = HeaderValue::from_str(&header.value).map_err(|_| {
            ClientError::Upload(format!("Linear sent a bad value for {}", header.key))
        })?;
        headers.insert(name, value);
    }
    let body = Progress {
        bytes: Bytes::from(bytes),
        sent: 0,
        progress: Box::new(progress),
    };
    let response = client
        .http
        .put(&upload.upload_url)
        .headers(headers)
        .timeout(UPLOAD_TIMEOUT)
        .body(reqwest::Body::wrap(body))
        .send()
        .await
        .map_err(|error| ClientError::Upload(error.without_url().to_string()))?;
    let status = response.status();
    if status.is_success() {
        Ok(())
    } else {
        Err(ClientError::Upload(format!(
            "the upload host answered with HTTP {}",
            status.as_u16()
        )))
    }
}

/// The attachment in `payload`, or why Linear did not confirm it.
fn confirmed(
    payload: AttachmentPayload,
    errors: Vec<GraphQlError>,
    issue_id: &str,
) -> Result<Attachment, ClientError> {
    match payload.attachment {
        Some(attachment) if payload.success => Ok(attachment.into_attachment(issue_id)),
        _ => Err(unconfirmed(errors, "attachment")),
    }
}

/// Why Linear's answer does not confirm the `what`: the errors that came
/// with it, or a plain message when there are none.
fn unconfirmed(errors: Vec<GraphQlError>, what: &str) -> ClientError {
    if errors.is_empty() {
        ClientError::Decode(format!("Linear did not confirm the {what}"))
    } else {
        ClientError::GraphQl(errors)
    }
}

/// A file's bytes as a request body, in chunks of [`CHUNK`], telling
/// `progress` how many have gone out as each is taken. Its exact size goes
/// out as `Content-Length`.
struct Progress {
    bytes: Bytes,
    sent: u64,
    progress: Box<dyn FnMut(u64) + Send + Sync>,
}

impl Body for Progress {
    type Data = Bytes;
    type Error = std::convert::Infallible;

    fn poll_frame(
        self: Pin<&mut Self>,
        _: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Bytes>, Self::Error>>> {
        let this = self.get_mut();
        if this.bytes.is_empty() {
            return Poll::Ready(None);
        }
        let chunk = this.bytes.split_to(CHUNK.min(this.bytes.len()));
        this.sent += chunk.len() as u64;
        (this.progress)(this.sent);
        Poll::Ready(Some(Ok(Frame::data(chunk))))
    }

    fn is_end_stream(&self) -> bool {
        self.bytes.is_empty()
    }

    fn size_hint(&self) -> SizeHint {
        SizeHint::with_exact(self.bytes.len() as u64)
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct UploadData {
    file_upload: UploadPayload,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct UploadPayload {
    success: bool,
    upload_file: Option<UploadFile>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct UploadFile {
    upload_url: String,
    asset_url: String,
    headers: Vec<UploadHeader>,
}

#[derive(Deserialize)]
struct UploadHeader {
    key: String,
    value: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CreateData {
    attachment_create: AttachmentPayload,
}

#[derive(Deserialize)]
struct LinkData {
    #[serde(rename = "attachmentLinkURL")]
    attachment_link_url: AttachmentPayload,
}

#[derive(Deserialize)]
struct AttachmentPayload {
    success: bool,
    attachment: Option<AttachmentNode>,
}
