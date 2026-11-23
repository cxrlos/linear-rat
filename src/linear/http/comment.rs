//! A comment as every comment document reads it: the comments query, the
//! comment mutations and the live comment events all ask for the same
//! fields, so one node decodes them all.

use jiff::Timestamp;
use serde::Deserialize;

use super::assigned::{PersonNode, optional_timestamp, timestamp};
use crate::linear::model::{Comment, Member, Reaction};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct CommentNode {
    pub(super) id: String,
    pub(super) body: String,
    #[serde(deserialize_with = "timestamp")]
    pub(super) created_at: Timestamp,
    #[serde(default, deserialize_with = "optional_timestamp")]
    pub(super) edited_at: Option<Timestamp>,
    #[serde(default, deserialize_with = "optional_timestamp")]
    pub(super) resolved_at: Option<Timestamp>,
    pub(super) parent_id: Option<String>,
    /// Only the live events say which issue; the other documents ask from
    /// the issue's side.
    pub(super) issue_id: Option<String>,
    pub(super) user: Option<PersonNode>,
    #[serde(default)]
    pub(super) reactions: Vec<ReactionNode>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ReactionNode {
    id: String,
    emoji: String,
    #[serde(deserialize_with = "timestamp")]
    created_at: Timestamp,
    user: Option<UserId>,
}

#[derive(Deserialize)]
struct UserId {
    id: String,
}

impl ReactionNode {
    pub(super) fn into_reaction(self) -> Reaction {
        Reaction {
            id: self.id,
            emoji: self.emoji,
            user_id: self.user.map(|user| user.id),
            created_at: self.created_at,
        }
    }
}

impl CommentNode {
    /// The comment on the issue `issue_id`, and its author as a member.
    pub(super) fn into_comment(self, issue_id: &str) -> (Comment, Option<Member>) {
        let comment = Comment {
            id: self.id,
            issue_id: issue_id.to_owned(),
            author_id: self.user.as_ref().map(|user| user.id.clone()),
            body: self.body,
            created_at: self.created_at,
            parent_id: self.parent_id,
            resolved_at: self.resolved_at,
            edited_at: self.edited_at,
            reactions: self
                .reactions
                .into_iter()
                .map(ReactionNode::into_reaction)
                .collect(),
        };
        (comment, self.user.map(PersonNode::into_member))
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn a_reply_keeps_its_parent_resolution_edit_and_reactions() {
        let node: CommentNode = serde_json::from_value(json!({
            "id": "comment-2",
            "body": "Agreed",
            "createdAt": "2026-11-20T10:00:00Z",
            "editedAt": "2026-11-20T10:05:00Z",
            "resolvedAt": null,
            "parentId": "comment-1",
            "user": { "id": "member-ana", "name": "Ana", "displayName": "ana" },
            "reactions": [{
                "id": "reaction-1",
                "emoji": "👍",
                "createdAt": "2026-11-20T10:06:00Z",
                "user": { "id": "member-emile" }
            }]
        }))
        .unwrap();

        let (comment, author) = node.into_comment("issue-1");

        assert_eq!(comment.issue_id, "issue-1");
        assert_eq!(comment.parent_id.as_deref(), Some("comment-1"));
        assert_eq!(comment.resolved_at, None);
        assert!(comment.edited_at.is_some());
        assert_eq!(comment.reactions.len(), 1);
        assert_eq!(comment.reactions[0].emoji, "👍");
        assert_eq!(comment.reactions[0].user_id.as_deref(), Some("member-emile"));
        assert_eq!(author.map(|member| member.id).as_deref(), Some("member-ana"));
    }

    #[test]
    fn a_comment_without_the_new_fields_still_decodes() {
        let node: CommentNode = serde_json::from_value(json!({
            "id": "comment-1",
            "body": "Hi",
            "createdAt": "2026-11-20T10:00:00Z",
            "user": null
        }))
        .unwrap();

        let (comment, author) = node.into_comment("issue-1");

        assert_eq!(comment.parent_id, None);
        assert!(comment.reactions.is_empty());
        assert!(author.is_none());
    }
}
