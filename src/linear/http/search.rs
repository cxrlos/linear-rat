//! Linear's server-side issue search, kept to its own 30-a-minute bucket.

use serde::Deserialize;
use serde_json::{Map, Value, json};

use super::assigned::{Decoded, IssueNode, LabelNode, push_new};
use super::team::issue_filter;
use super::{HttpClient, Urgency};
use crate::linear::model::Label;
use crate::linear::{ClientError, Response, SearchQuery, SearchResults};

const SEARCH_ISSUES: &str = include_str!("../graphql/search_issues.graphql");

/// The search's variables: the term, `includeComments` only when comments
/// count, and a filter only when the search keeps to a team.
fn variables(query: &SearchQuery) -> Value {
    let mut variables = Map::new();
    variables.insert("term".to_owned(), json!(query.term));
    if query.include_comments {
        variables.insert("includeComments".to_owned(), json!(true));
    }
    if let Some(within) = &query.within {
        let mut filter = match issue_filter(&within.filter) {
            Value::Object(fields) => fields,
            _ => Map::new(),
        };
        filter.insert("team".to_owned(), json!({ "id": { "eq": within.team_id } }));
        variables.insert("filter".to_owned(), Value::Object(filter));
    }
    Value::Object(variables)
}

pub(super) async fn search(
    client: &HttpClient,
    query: &SearchQuery,
) -> Result<SearchResults, ClientError> {
    let wait = client.search_bucket().take(client.timer.now());
    if !wait.is_zero() {
        tracing::info!(
            seconds = wait.as_secs_f64(),
            "waiting for Linear's search budget"
        );
        client.timer.sleep(wait).await;
    }
    let Response { data, errors } = client
        .send::<SearchData>(SEARCH_ISSUES, &variables(query), Urgency::Urgent)
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
