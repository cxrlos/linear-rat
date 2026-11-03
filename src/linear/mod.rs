//! Everything lrat needs from Linear goes through [`LinearClient`], so tests
//! and `--demo` swap the real client for [`FakeClient`].

pub mod demo;
mod error;
pub mod http;
pub mod model;

use std::future::Future;

pub use error::{ClientError, GraphQlError, Response};
pub use http::{ClientConfig, HttpClient};
use model::{Viewer, Workspace};

/// Queries, mutations and live events.
pub trait LinearClient {
    /// The viewer and the workspace the API key belongs to. Login and
    /// `auth status` use it to check a key.
    fn viewer(&self) -> impl Future<Output = Result<(Viewer, Workspace), ClientError>> + Send;
}

/// A client that never touches the network.
#[derive(Debug, Default, Clone, Copy)]
pub struct FakeClient;

impl LinearClient for FakeClient {
    async fn viewer(&self) -> Result<(Viewer, Workspace), ClientError> {
        Ok((
            Viewer {
                id: "viewer-remy".to_owned(),
                name: "Remy Fromage".to_owned(),
            },
            Workspace {
                id: "workspace-cheddar".to_owned(),
                name: "Cheddar Works".to_owned(),
                url_key: "cheddar-works".to_owned(),
            },
        ))
    }
}
