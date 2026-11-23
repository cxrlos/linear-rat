//! Adding and removing the viewer's reactions: one `reactionCreate` or
//! `reactionDelete` each.

use serde::Deserialize;
use serde_json::json;

use super::HttpClient;
use super::comment::ReactionNode;
use super::comment_change;
use crate::linear::model::Reaction;
use crate::linear::{ClientError, ReactionTarget, Response};

const CREATE: &str = include_str!("../graphql/reaction_create.graphql");
const DELETE: &str = include_str!("../graphql/reaction_delete.graphql");

pub(super) async fn add(
    client: &HttpClient,
    target: &ReactionTarget,
    emoji: &str,
) -> Result<Reaction, ClientError> {
    let input = match target {
        ReactionTarget::Issue(id) => json!({ "issueId": id, "emoji": emoji }),
        ReactionTarget::Comment(id) => json!({ "commentId": id, "emoji": emoji }),
    };
    let response: Response<CreateData> = client.execute(CREATE, json!({ "input": input })).await?;
    let payload = response.data.reaction_create;
    match payload.reaction {
        Some(reaction) if payload.success => Ok(reaction.into_reaction()),
        _ if !response.errors.is_empty() => Err(ClientError::GraphQl(response.errors)),
        _ => Err(ClientError::Decode(
            "Linear did not confirm the reaction".to_owned(),
        )),
    }
}

pub(super) async fn remove(client: &HttpClient, id: &str) -> Result<(), ClientError> {
    comment_change::run(client, DELETE, "reactionDelete", json!({ "id": id })).await
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CreateData {
    reaction_create: CreatePayload,
}

#[derive(Deserialize)]
struct CreatePayload {
    success: bool,
    reaction: Option<ReactionNode>,
}
