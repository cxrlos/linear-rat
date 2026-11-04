//! The real client: hand-written GraphQL documents posted to Linear with
//! reqwest (ADR 0002).

mod analytics;
mod assigned;
mod changes;
mod create;
mod detail;
mod edit;
mod search;
mod team;

use std::collections::HashSet;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use jiff::Timestamp;
use reqwest::StatusCode;
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE, HeaderMap, HeaderValue, RETRY_AFTER};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use super::governor::{Governor, SystemTimer, Timer, Urgency};
use super::model::{Issue, Label, Member, Team, Viewer, WorkflowState, Workspace};
use super::search_bucket::SearchBucket;
use super::{
    AnalyticsIssues, AnalyticsWindow, AssignedIssues, ClientError, GraphQlError, IssueCreate,
    IssueDetail, IssueUpdate, LinearClient, PostedComment, Response, SearchResults, TeamChanges,
    TeamIssuesPage, TeamOptions, TeamQuery,
};
use crate::auth::ApiKey;

/// Linear's GraphQL endpoint.
pub const ENDPOINT: &str = "https://api.linear.app/graphql";

const VIEWER: &str = include_str!("graphql/viewer.graphql");

/// How many times a rate-limited request is sent again before giving up.
const RATE_LIMIT_RETRIES: u32 = 3;

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
/// against the OS trust store. Clones share one budget governor and one
/// search bucket.
#[derive(Debug, Clone)]
pub struct HttpClient {
    http: reqwest::Client,
    endpoint: String,
    authorization: HeaderValue,
    governor: Arc<Mutex<Governor>>,
    search_bucket: Arc<Mutex<SearchBucket>>,
    timer: Arc<dyn Timer>,
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
            governor: Arc::default(),
            search_bucket: Arc::default(),
            timer: Arc::new(SystemTimer),
        })
    }

    /// This client telling time and waiting with `timer` instead of the
    /// system clock.
    pub fn with_timer(self, timer: Arc<dyn Timer>) -> Self {
        Self { timer, ..self }
    }

    /// Sends one GraphQL `document` with `variables`, as a request the viewer
    /// is waiting on. A response whose data decodes into `T` returns it with
    /// any errors that came alongside. Errors with no data, or with data `T`
    /// cannot hold (a required field is null), are [`ClientError::GraphQl`].
    pub async fn execute<T: DeserializeOwned>(
        &self,
        document: &str,
        variables: Value,
    ) -> Result<Response<T>, ClientError> {
        self.send(document, &variables, Urgency::Urgent).await
    }

    /// Sends `document` once the governor allows it, and again after each
    /// rate-limited answer, up to [`RATE_LIMIT_RETRIES`] times.
    async fn send<T: DeserializeOwned>(
        &self,
        document: &str,
        variables: &Value,
        urgency: Urgency,
    ) -> Result<Response<T>, ClientError> {
        let mut retries = 0;
        loop {
            let wait = self.governor().wait(urgency, self.timer.now());
            if !wait.is_zero() {
                tracing::info!(
                    seconds = wait.as_secs_f64(),
                    "waiting for Linear's rate-limit budget"
                );
                self.timer.sleep(wait).await;
            }
            match self.post(document, variables).await? {
                Attempt::Answered(response) => return Ok(response),
                Attempt::RateLimited { .. } if retries == RATE_LIMIT_RETRIES => {
                    return Err(ClientError::RateLimited);
                }
                Attempt::RateLimited { retry_after } => {
                    retries += 1;
                    tracing::warn!(
                        retry_after = retry_after.map(|wait| wait.as_secs()),
                        "Linear rate-limited a request"
                    );
                    self.governor().rate_limited(self.timer.now(), retry_after);
                }
            }
        }
    }

    async fn post<T: DeserializeOwned>(
        &self,
        document: &str,
        variables: &Value,
    ) -> Result<Attempt<T>, ClientError> {
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
        let headers = response.headers().clone();
        self.governor().observe(&headers);
        let bytes = response
            .bytes()
            .await
            .map_err(ClientError::from_transport)?;

        if status == StatusCode::UNAUTHORIZED {
            return Err(ClientError::InvalidKey);
        }
        if status == StatusCode::TOO_MANY_REQUESTS {
            return Ok(rate_limited(&headers));
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
        if errors.iter().any(is_rate_limit) {
            return Ok(rate_limited(&headers));
        }
        let decoded = data.map(T::deserialize);
        match decoded {
            Some(Ok(data)) => Ok(Attempt::Answered(Response { data, errors })),
            _ if !errors.is_empty() => Err(ClientError::GraphQl(errors)),
            _ if !status.is_success() => Err(ClientError::Status(status.as_u16())),
            Some(Err(error)) => Err(ClientError::Decode(error.to_string())),
            None => Err(ClientError::Decode("the response has no data".to_owned())),
        }
    }

    /// Every page of a Relay connection: sends `document` with `variables`
    /// and `after` set to each page's `endCursor` until `hasNextPage` is
    /// false. The first page goes out as `first_page`; the rest are
    /// background. A cursor Linear has given before ends the load with an
    /// error, so a server that cycles cannot keep it going forever. The
    /// errors that came alongside any page's data are returned with the pages.
    async fn all_pages<P: DeserializeOwned>(
        &self,
        document: &str,
        mut variables: Value,
        first_page: Urgency,
        page_info: fn(&P) -> &PageInfo,
    ) -> Result<Response<Vec<P>>, ClientError> {
        let mut pages = Vec::new();
        let mut errors = Vec::new();
        let mut urgency = first_page;
        let mut sent_cursors = HashSet::new();
        variables["after"] = Value::Null;
        loop {
            let response: Response<P> = self.send(document, &variables, urgency).await?;
            for error in &response.errors {
                tracing::warn!(message = %error.message, "a page also returned an error");
            }
            errors.extend(response.errors);
            let info = page_info(&response.data);
            let next = match (info.has_next_page, &info.end_cursor) {
                (false, _) => None,
                (true, Some(cursor)) if sent_cursors.insert(cursor.clone()) => Some(cursor.clone()),
                (true, _) => {
                    return Err(ClientError::Decode(
                        "Linear said there are more pages but gave no cursor it had not \
                         given before"
                            .to_owned(),
                    ));
                }
            };
            pages.push(response.data);
            let Some(cursor) = next else {
                return Ok(Response {
                    data: pages,
                    errors,
                });
            };
            variables["after"] = Value::String(cursor);
            urgency = Urgency::Background;
        }
    }

    fn governor(&self) -> MutexGuard<'_, Governor> {
        // The governor holds plain numbers, so a panic elsewhere cannot leave
        // it half-updated in a way that matters.
        self.governor.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn search_bucket(&self) -> MutexGuard<'_, SearchBucket> {
        // One timestamp, which a panic cannot leave half-written.
        self.search_bucket
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
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

    async fn assigned_issues(&self) -> Result<AssignedIssues, ClientError> {
        assigned::load(self).await
    }

    async fn changed_issues(
        &self,
        team_id: &str,
        since: Option<Timestamp>,
    ) -> Result<TeamChanges, ClientError> {
        changes::changed(self, team_id, since).await
    }

    async fn existing_issues(&self, ids: &[String]) -> Result<Vec<String>, ClientError> {
        changes::existing(self, ids).await
    }

    fn throttled(&self) -> bool {
        !self
            .governor()
            .wait(Urgency::Background, self.timer.now())
            .is_zero()
    }

    async fn workflow_states(&self, team_id: &str) -> Result<Vec<WorkflowState>, ClientError> {
        edit::workflow_states(self, team_id).await
    }

    async fn team_members(&self, team_id: &str) -> Result<Vec<Member>, ClientError> {
        edit::team_members(self, team_id).await
    }

    async fn team_labels(&self, team_id: &str) -> Result<Vec<Label>, ClientError> {
        edit::team_labels(self, team_id).await
    }

    async fn update_issue(
        &self,
        issue_id: &str,
        update: &IssueUpdate,
    ) -> Result<Issue, ClientError> {
        edit::update_issue(self, issue_id, update).await
    }

    async fn issue_detail(
        &self,
        issue_id: &str,
        urgency: Urgency,
    ) -> Result<IssueDetail, ClientError> {
        detail::load(self, issue_id, urgency).await
    }

    async fn search_issues(&self, term: &str) -> Result<SearchResults, ClientError> {
        search::search(self, term).await
    }

    async fn teams(&self) -> Result<Vec<Team>, ClientError> {
        team::teams(self).await
    }

    async fn team_options(&self, team_id: &str) -> Result<TeamOptions, ClientError> {
        team::options(self, team_id).await
    }

    async fn team_issues(
        &self,
        query: &TeamQuery,
        after: Option<&str>,
    ) -> Result<TeamIssuesPage, ClientError> {
        team::issues(self, query, after).await
    }

    async fn count_team_issues(&self, query: &TeamQuery) -> Result<usize, ClientError> {
        team::count(self, query).await
    }

    async fn analytics_issues(
        &self,
        window: AnalyticsWindow,
    ) -> Result<AnalyticsIssues, ClientError> {
        analytics::load(self, window).await
    }

    async fn create_comment(
        &self,
        issue_id: &str,
        body: &str,
    ) -> Result<PostedComment, ClientError> {
        create::create_comment(self, issue_id, body).await
    }

    async fn create_issue(&self, input: &IssueCreate) -> Result<Issue, ClientError> {
        create::create_issue(self, input).await
    }
}

/// What came back from one send.
enum Attempt<T> {
    Answered(Response<T>),
    RateLimited { retry_after: Option<Duration> },
}

fn rate_limited<T>(headers: &HeaderMap) -> Attempt<T> {
    let retry_after = headers
        .get(RETRY_AFTER)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.trim().parse().ok())
        .map(Duration::from_secs);
    Attempt::RateLimited { retry_after }
}

/// A Relay connection's `pageInfo`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PageInfo {
    has_next_page: bool,
    end_cursor: Option<String>,
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

fn is_rate_limit(error: &GraphQlError) -> bool {
    error.code() == Some("RATELIMITED")
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
