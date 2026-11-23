//! The viewer's inbox: loading issue notifications, and marking them read,
//! snoozing and archiving them, one mutation each.

use jiff::Timestamp;
use serde::Deserialize;
use serde_json::{Value, json};

use super::assigned::{Connection, Decoded, IssueNode, PersonNode, optional_timestamp, timestamp};
use super::{HttpClient, Urgency};
use crate::linear::model::{Member, Notification};
use crate::linear::{ClientError, NotificationUpdate, NotificationsLoaded, Patch, Response};

const NOTIFICATIONS: &str = include_str!("../graphql/notifications.graphql");
const UPDATE: &str = include_str!("../graphql/notification_update.graphql");
const ARCHIVE: &str = include_str!("../graphql/notification_archive.graphql");
const MARK_READ_ALL: &str = include_str!("../graphql/notification_mark_read_all.graphql");

/// With `since` `None`, one page of the newest unarchived notifications, as
/// the viewer waits on it. Otherwise every page of those updated after
/// `since`, archived ones included, in the background.
pub(super) async fn load(
    client: &HttpClient,
    since: Option<Timestamp>,
) -> Result<NotificationsLoaded, ClientError> {
    let Response {
        data: pages,
        errors,
    } = match since {
        None => {
            let response: Response<NotificationsPage> = client
                .send(NOTIFICATIONS, &json!({}), Urgency::Urgent)
                .await?;
            Response {
                data: vec![response.data],
                errors: response.errors,
            }
        }
        Some(since) => {
            let variables = json!({
                "filter": { "updatedAt": { "gt": since.to_string() } },
                "includeArchived": true,
            });
            client
                .all_pages(
                    NOTIFICATIONS,
                    variables,
                    Urgency::Background,
                    |page: &NotificationsPage| &page.notifications.page_info,
                )
                .await?
        }
    };
    for error in &errors {
        tracing::warn!(message = %error.message, "the notifications also returned an error");
    }
    let nodes = pages.into_iter().flat_map(|page| page.notifications.nodes);
    Ok(decode(nodes, client.timer.now()))
}

/// The issue notifications among `nodes`, with their issues and people.
/// The rest are counted as skipped.
pub(super) fn decode(
    nodes: impl IntoIterator<Item = NotificationNode>,
    as_of: Timestamp,
) -> NotificationsLoaded {
    let mut decoded = Decoded::default();
    let mut loaded = NotificationsLoaded {
        as_of,
        ..NotificationsLoaded::default()
    };
    for node in nodes {
        let Some((notification, actor, issue)) = node.into_notification() else {
            loaded.skipped += 1;
            continue;
        };
        if let Some(actor) = actor {
            super::assigned::push_new(&mut loaded.members, actor.clone(), |known| {
                known.id == actor.id
            });
        }
        if !decoded.issues.iter().any(|known| known.id == issue.id) {
            decoded.add(issue);
        }
        loaded.notifications.push(notification);
    }
    for member in decoded.members {
        super::assigned::push_new(&mut loaded.members, member.clone(), |known| {
            known.id == member.id
        });
    }
    loaded.issues = decoded.issues;
    loaded.cycles = decoded.cycles;
    loaded.teams = decoded.teams;
    if loaded.skipped > 0 {
        tracing::debug!(
            skipped = loaded.skipped,
            "skipped notifications that are not about an issue"
        );
    }
    loaded
}

pub(super) async fn update(
    client: &HttpClient,
    id: &str,
    update: &NotificationUpdate,
) -> Result<(), ClientError> {
    let mut input = serde_json::Map::new();
    match &update.read_at {
        Patch::Unchanged => {}
        Patch::Clear => {
            input.insert("readAt".to_owned(), Value::Null);
        }
        Patch::Set(at) => {
            input.insert("readAt".to_owned(), Value::String(at.to_string()));
        }
    }
    if let Some(until) = update.snoozed_until_at {
        input.insert(
            "snoozedUntilAt".to_owned(),
            Value::String(until.to_string()),
        );
    }
    let variables = json!({ "id": id, "input": input });
    run(
        client,
        UPDATE,
        "notificationUpdate",
        variables,
        Urgency::Urgent,
    )
    .await
}

pub(super) async fn archive(
    client: &HttpClient,
    id: &str,
    urgency: Urgency,
) -> Result<(), ClientError> {
    run(
        client,
        ARCHIVE,
        "notificationArchive",
        json!({ "id": id }),
        urgency,
    )
    .await
}

pub(super) async fn mark_issue_read(
    client: &HttpClient,
    issue_id: &str,
    read_at: Timestamp,
) -> Result<(), ClientError> {
    let variables = json!({ "issueId": issue_id, "readAt": read_at.to_string() });
    run(
        client,
        MARK_READ_ALL,
        "notificationMarkReadAll",
        variables,
        Urgency::Urgent,
    )
    .await
}

/// Sends `document` and reads `success` from its `field`.
async fn run(
    client: &HttpClient,
    document: &str,
    field: &str,
    variables: Value,
    urgency: Urgency,
) -> Result<(), ClientError> {
    let response: Response<Value> = client.send(document, &variables, urgency).await?;
    let done = response
        .data
        .get(field)
        .and_then(|payload| payload.get("success"))
        .and_then(Value::as_bool);
    match done {
        Some(true) => Ok(()),
        _ if !response.errors.is_empty() => Err(ClientError::GraphQl(response.errors)),
        _ => Err(ClientError::Decode(format!(
            "Linear did not confirm {field}"
        ))),
    }
}

#[derive(Deserialize)]
struct NotificationsPage {
    notifications: Connection<NotificationNode>,
}

/// One notification as the notification documents read it. Only an
/// `IssueNotification` has an issue.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct NotificationNode {
    pub(super) id: String,
    #[serde(rename = "type")]
    pub(super) kind: String,
    #[serde(deserialize_with = "timestamp")]
    pub(super) created_at: Timestamp,
    #[serde(deserialize_with = "timestamp")]
    pub(super) updated_at: Timestamp,
    #[serde(default, deserialize_with = "optional_timestamp")]
    pub(super) read_at: Option<Timestamp>,
    #[serde(default, deserialize_with = "optional_timestamp")]
    pub(super) snoozed_until_at: Option<Timestamp>,
    #[serde(default, deserialize_with = "optional_timestamp")]
    pub(super) archived_at: Option<Timestamp>,
    pub(super) actor: Option<PersonNode>,
    #[serde(default)]
    pub(super) comment_id: Option<String>,
    #[serde(default)]
    pub(super) reaction_emoji: Option<String>,
    #[serde(default)]
    pub(super) issue: Option<IssueNode>,
}

impl NotificationNode {
    /// The notification, who did it and its issue, or `None` when it is
    /// not about an issue.
    pub(super) fn into_notification(self) -> Option<(Notification, Option<Member>, IssueNode)> {
        let issue = self.issue?;
        let actor = self.actor.map(|person| Member {
            id: person.id,
            name: person.name,
            display_name: person.display_name,
        });
        let notification = Notification {
            id: self.id,
            kind: self.kind,
            issue_id: issue.id.clone(),
            team_id: issue.team.id.clone(),
            comment_id: self.comment_id,
            actor_id: actor.as_ref().map(|actor| actor.id.clone()),
            reaction_emoji: self.reaction_emoji,
            created_at: self.created_at,
            updated_at: self.updated_at,
            read_at: self.read_at,
            snoozed_until_at: self.snoozed_until_at,
            archived_at: self.archived_at,
        };
        Some((notification, actor, issue))
    }
}
