//! The real client: hand-written GraphQL documents posted to Linear with
//! reqwest (ADR 0002).

use std::time::Duration;

use reqwest::header::{AUTHORIZATION, CONTENT_TYPE, HeaderValue};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use super::model::{Viewer, Workspace};
use super::{ClientError, GraphQlError, LinearClient, Response};
use crate::auth::ApiKey;

/// Linear's GraphQL endpoint.
pub const ENDPOINT: &str = "https://api.linear.app/graphql";

const VIEWER: &str = include_str!("graphql/viewer.graphql");

/// Where [`HttpClient`] sends requests and how long it waits.
#[derive(Debug, Clone)]
pub struct ClientConfig {
    pub endpoint: String,
    /// Time allowed to open the TCP and TLS connection.
    pub connect_timeout: Duration,
    /// Time allowed for the whole request, from sending to the last byte of
    /// the response.
    pub request_timeout: Duration,
}

impl Default for ClientConfig {
    fn default() -> Self {
        Self {
            endpoint: ENDPOINT.to_owned(),
            connect_timeout: Duration::from_secs(10),
            request_timeout: Duration::from_secs(30),
        }
    }
}

/// The [`LinearClient`] that talks to Linear over HTTPS. TLS is rustls with the
/// aws-lc-rs provider and the platform verifier, so certificates are checked
/// against the OS trust store.
#[derive(Debug, Clone)]
pub struct HttpClient {
    http: reqwest::Client,
    endpoint: String,
    authorization: HeaderValue,
}

impl HttpClient {
    /// A client for Linear's API, authenticated with `key`.
    pub fn new(key: &ApiKey) -> Result<Self, ClientError> {
        Self::with_config(key, ClientConfig::default())
    }

    pub fn with_config(key: &ApiKey, config: ClientConfig) -> Result<Self, ClientError> {
        let http = reqwest::Client::builder()
            .connect_timeout(config.connect_timeout)
            .timeout(config.request_timeout)
            .user_agent(concat!("lrat/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(ClientError::Setup)?;
        // Linear takes a personal API key as the bare header value, no `Bearer`.
        let mut authorization =
            HeaderValue::from_str(key.expose()).map_err(|_| ClientError::KeyNotAHeader)?;
        authorization.set_sensitive(true);
        Ok(Self {
            http,
            endpoint: config.endpoint,
            authorization,
        })
    }

    /// Sends one GraphQL `document` with `variables`. A response whose data
    /// decodes into `T` returns it with any errors that came alongside. Errors
    /// with no data, or with data `T` cannot hold (a required field is null),
    /// are [`ClientError::GraphQl`].
    pub async fn execute<T: DeserializeOwned>(
        &self,
        document: &str,
        variables: Value,
    ) -> Result<Response<T>, ClientError> {
        let body = json!({ "query": document, "variables": variables });
        let response = self
            .http
            .post(&self.endpoint)
            .header(AUTHORIZATION, self.authorization.clone())
            .header(CONTENT_TYPE, "application/json")
            .body(body.to_string())
            .send()
            .await
            .map_err(ClientError::from_transport)?;
        let status = response.status();
        let bytes = response
            .bytes()
            .await
            .map_err(ClientError::from_transport)?;

        if status == reqwest::StatusCode::UNAUTHORIZED {
            return Err(ClientError::InvalidKey);
        }
        // The envelope is read before `data` is decoded into T, so the errors
        // survive when Linear nulls a field that T requires.
        let RawResponse { data, errors } = match serde_json::from_slice::<RawResponse>(&bytes) {
            Ok(raw) => raw,
            Err(_) if !status.is_success() => return Err(ClientError::Status(status.as_u16())),
            Err(error) => return Err(ClientError::Decode(error.to_string())),
        };
        if errors.iter().any(is_authentication) {
            return Err(ClientError::InvalidKey);
        }
        let decoded = data.map(T::deserialize);
        match decoded {
            Some(Ok(data)) => Ok(Response { data, errors }),
            _ if !errors.is_empty() => Err(ClientError::GraphQl(errors)),
            _ if !status.is_success() => Err(ClientError::Status(status.as_u16())),
            Some(Err(error)) => Err(ClientError::Decode(error.to_string())),
            None => Err(ClientError::Decode("the response has no data".to_owned())),
        }
    }
}

impl LinearClient for HttpClient {
    async fn viewer(&self) -> Result<(Viewer, Workspace), ClientError> {
        let response: Response<ViewerData> = self.execute(VIEWER, json!({})).await?;
        for error in &response.errors {
            tracing::warn!(message = %error.message, "the viewer query also returned an error");
        }
        let ViewerData { viewer, workspace } = response.data;
        Ok((
            Viewer {
                id: viewer.id,
                name: viewer.name,
            },
            Workspace {
                id: workspace.id,
                name: workspace.name,
                url_key: workspace.url_key,
            },
        ))
    }
}

#[derive(Deserialize)]
struct RawResponse {
    data: Option<Value>,
    #[serde(default)]
    errors: Vec<GraphQlError>,
}

fn is_authentication(error: &GraphQlError) -> bool {
    error.code() == Some("AUTHENTICATION_ERROR")
}

#[derive(Deserialize)]
struct ViewerData {
    viewer: ViewerNode,
    #[serde(rename = "organization")]
    workspace: WorkspaceNode,
}

#[derive(Deserialize)]
struct ViewerNode {
    id: String,
    name: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct WorkspaceNode {
    id: String,
    name: String,
    url_key: String,
}
