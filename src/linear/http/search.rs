//! Linear's server-side issue search, kept to its own 30-a-minute bucket.

use serde::Deserialize;
use serde_json::json;

use super::assigned::{Decoded, IssueNode, LabelNode, push_new};
use super::{HttpClient, Urgency};
use crate::linear::model::Label;
use crate::linear::{ClientError, Response, SearchResults};

const SEARCH_ISSUES: &str = include_str!("../graphql/search_issues.graphql");

pub(super) async fn search(client: &HttpClient, term: &str) -> Result<SearchResults, ClientError> {
    let wait = client.search_bucket().take(client.timer.now());
    if !wait.is_zero() {
        tracing::info!(
            seconds = wait.as_secs_f64(),
            "waiting for Linear's search budget"
        );
        client.timer.sleep(wait).await;
    }
    let Response { data, errors } = client
        .send::<SearchData>(SEARCH_ISSUES, &json!({ "term": term }), Urgency::Urgent)
        .await?;
    let mut found = Decoded::default();
    let mut labels: Vec<Label> = Vec::new();
    for node in data.search_issues.nodes {
        for label in node.labels.nodes {
            let label = Label::from(label);
            let id = label.id.clone();
            push_new(&mut labels, label, |known| known.id == id);
        }
        found.add(node.issue);
    }
    Ok(SearchResults {
        issues: found.issues,
        cycles: found.cycles,
        labels,
        members: found.members,
        teams: found.teams,
        problems: errors
            .iter()
            .map(|error| format!("Linear left something out: {error}"))
            .collect(),
    })
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SearchData {
    search_issues: Nodes<SearchNode>,
}

#[derive(Deserialize)]
struct Nodes<N> {
    nodes: Vec<N>,
}

#[derive(Deserialize)]
struct SearchNode {
    #[serde(flatten)]
    issue: IssueNode,
    labels: Nodes<LabelNode>,
}
