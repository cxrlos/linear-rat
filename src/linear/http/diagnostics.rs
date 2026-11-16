//! What `lrat doctor` asks Linear about the key.

use jiff::Timestamp;
use serde::Deserialize;
use serde_json::json;

use super::{HttpClient, Urgency};
use crate::linear::{ClientError, Diagnostics, RateLimit, Response, Role};

const DIAGNOSTICS: &str = include_str!("../graphql/diagnostics.graphql");

pub(super) async fn load(client: &HttpClient) -> Result<Diagnostics, ClientError> {
    let Response { data, errors } = client
        .send::<DiagnosticsData>(DIAGNOSTICS, &json!({}), Urgency::Urgent)
        .await?;
    for error in &errors {
        tracing::warn!(message = %error.message, "the diagnostics query also returned an error");
    }
    let server_time = client.governor().server_time();
    let DiagnosticsData {
        viewer,
        rate_limit_status,
    } = data;
    Ok(Diagnostics {
        role: Role {
            admin: viewer.admin,
            owner: viewer.owner,
            guest: viewer.guest,
        },
        rate_limits: rate_limit_status
            .limits
            .into_iter()
            .map(|limit| RateLimit {
                kind: limit.kind,
                allowed: limit.allowed_amount,
                remaining: limit.remaining_amount,
                // Linear sends the reset as Unix milliseconds in a float.
                reset: Timestamp::from_millisecond(limit.reset as i64).ok(),
            })
            .collect(),
        server_time,
    })
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct DiagnosticsData {
    viewer: ViewerRole,
    rate_limit_status: RateLimitStatus,
}

#[derive(Deserialize)]
struct ViewerRole {
    admin: bool,
    owner: bool,
    guest: bool,
}

#[derive(Deserialize)]
struct RateLimitStatus {
    limits: Vec<LimitNode>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct LimitNode {
    #[serde(rename = "type")]
    kind: String,
    allowed_amount: f64,
    remaining_amount: f64,
    reset: f64,
}
