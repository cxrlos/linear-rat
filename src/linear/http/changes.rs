//! The delta poll and the reconciliation: what changed in a team since a
//! watermark, and which stored issues still exist.

use std::collections::HashSet;

use jiff::Timestamp;
use serde::Deserialize;
use serde_json::json;

use super::assigned::{self, Connection, Decoded, IssueNode};
use super::{HttpClient, Urgency};
use crate::linear::{ClientError, Response, TeamChanges};

const TEAM_ISSUES_CHANGED: &str = include_str!("../graphql/team_issues_changed.graphql");
const EXISTING_ISSUES: &str = include_str!("../graphql/existing_issues.graphql");

/// The most ids one reconciliation request filters on.
const IDS_PER_REQUEST: usize = 100;

pub(super) async fn changed(
    client: &HttpClient,
    team_id: &str,
    since: Option<Timestamp>,
) -> Result<TeamChanges, ClientError> {
    let Response {
        data: pages,
        errors,
    } = match since {
        Some(since) => {
            client
                .all_pages(
                    TEAM_ISSUES_CHANGED,
                    json!({ "teamId": team_id, "since": since.to_string() }),
                    Urgency::Background,
                    |page: &ChangedPage| &page.issues.page_info,
                )
                .await?
        }
        None => {
            let variables = json!({
                "teamId": team_id,
                "since": Timestamp::UNIX_EPOCH.to_string(),
                "first": 1,
                "after": null,
            });
            let response: Response<ChangedPage> = client
                .send(TEAM_ISSUES_CHANGED, &variables, Urgency::Background)
                .await?;
            Response {
                data: vec![response.data],
                errors: response.errors,
            }
        }
    };
    for error in &errors {
        tracing::warn!(message = %error.message, "the delta poll also returned an error");
    }
    let mut decoded = Decoded::default();
    let mut trashed = Vec::new();
    let mut newest: Option<Timestamp> = None;
    for node in pages.into_iter().flat_map(|page| page.issues.nodes) {
        newest = newest.max(Some(node.issue.updated_at));
        if node.trashed == Some(true) {
            trashed.push(node.issue.id);
        } else {
            decoded.add(node.issue);
        }
    }
    let labels = match assigned::labels(client, &decoded.label_ids).await {
        Ok(response) => response.data,
        Err(error) => {
            tracing::warn!(%error, "the label lookup of the delta poll failed");
            Vec::new()
        }
    };
    Ok(TeamChanges {
        team_id: team_id.to_owned(),
        issues: decoded.issues,
        trashed,
        cycles: decoded.cycles,
        labels,
        members: decoded.members,
        teams: decoded.teams,
        newest,
        as_of: client.timer.now(),
    })
}

pub(super) async fn existing(
    client: &HttpClient,
    ids: &[String],
) -> Result<Vec<String>, ClientError> {
    let mut found = HashSet::new();
    for chunk in ids.chunks(IDS_PER_REQUEST) {
        let response = client
            .all_pages(
                EXISTING_ISSUES,
                json!({ "ids": chunk }),
                Urgency::Background,
                |page: &ExistingPage| &page.issues.page_info,
            )
            .await?;
        found.extend(
            response
                .data
                .into_iter()
                .flat_map(|page| page.issues.nodes)
                .filter(|node| node.trashed != Some(true))
                .map(|node| node.id),
        );
    }
    Ok(ids
        .iter()
        .filter(|id| found.contains(*id))
        .cloned()
        .collect())
}

#[derive(Deserialize)]
struct ChangedPage {
    issues: Connection<ChangedNode>,
}

#[derive(Deserialize)]
struct ChangedNode {
    #[serde(flatten)]
    issue: IssueNode,
    trashed: Option<bool>,
}

#[derive(Deserialize)]
struct ExistingPage {
    issues: Connection<ExistingNode>,
}

#[derive(Deserialize)]
struct ExistingNode {
    id: String,
    trashed: Option<bool>,
}
