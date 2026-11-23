//! The comments of an issue's detail as threads, and what the viewer does to
//! one of them: focus it, reply to it, edit or delete their own, and resolve
//! or unresolve its thread. Every change shows at once, goes to Linear as
//! one mutation and is undone if Linear rejects it.

use super::compose::{NewComment, Writing};
use super::lifecycle::{Confirm, OnConfirm};
use super::{Effect, Failure, Modal, State, Update, WriteClass, detail};
use crate::linear::model::Comment;
use crate::ui;

/// The threads of `comments`, which are oldest first: each first comment
/// with the replies under it, in the order the first comments were
/// written. A reply whose parent is not among them starts a thread of its
/// own.
pub fn threads(comments: &[Comment]) -> Vec<(&Comment, Vec<&Comment>)> {
    let root_of = |comment: &Comment| -> String {
        let mut current = comment;
        // A chain of replies ends at the first comment; the bound guards
        // against a cycle in data Linear sent.
        for _ in 0..comments.len() {
            let parent = current
                .parent_id
                .as_ref()
                .and_then(|id| comments.iter().find(|known| &known.id == id));
            match parent {
                Some(parent) => current = parent,
                None => break,
            }
        }
        current.id.clone()
    };
    let mut threads: Vec<(&Comment, Vec<&Comment>)> = Vec::new();
    for comment in comments {
        let root = root_of(comment);
        if root == comment.id {
            threads.push((comment, Vec::new()));
        } else if let Some((_, replies)) = threads.iter_mut().find(|(first, _)| first.id == root) {
            replies.push(comment);
        }
    }
    threads
}

/// Whether the thread starting at `root`, with `replies`, shows as one
/// line: it is resolved and the focus is not in it.
pub fn is_collapsed(root: &Comment, replies: &[&Comment], focused: Option<&str>) -> bool {
    root.resolved_at.is_some()
        && focused.is_none_or(|id| root.id != id && replies.iter().all(|reply| reply.id != id))
}

/// The comments the focus moves through, in the order the detail shows
/// them: a collapsed thread only by its first comment.
fn focusable<'a>(comments: &'a [Comment], focused: Option<&str>) -> Vec<&'a Comment> {
    let mut order = Vec::new();
    for (root, replies) in threads(comments) {
        order.push(root);
        if !is_collapsed(root, &replies, focused) {
            order.extend(replies);
        }
    }
    order
}

/// The focused comment of the focused issue's detail, while it is stored.
pub fn focused_comment(state: &State) -> Option<&Comment> {
    let issue = state.focused_issue()?;
    let id = state.detail.focused_comment(&issue.id)?;
    state
        .store
        .comments(&issue.id)?
        .iter()
        .find(|comment| comment.id == id)
}

/// Whether the focused issue's detail has focus and comments to move to.
pub fn can_focus_comments(state: &State) -> bool {
    detail::has_focus(state)
        && state
            .focused_issue()
            .and_then(|issue| state.store.comments(&issue.id))
            .is_some_and(|comments| !comments.is_empty())
}

/// Whether the viewer wrote the focused comment, so they may edit or
/// delete it.
pub fn owns_focused_comment(state: &State) -> bool {
    focused_comment(state).is_some_and(|comment| is_own(state, comment))
}

fn is_own(state: &State, comment: &Comment) -> bool {
    state
        .store
        .viewer()
        .is_some_and(|viewer| comment.author_id.as_deref() == Some(viewer.id.as_str()))
}

/// A change to a comment on its way to Linear, and the copies it changed,
/// to put back if Linear rejects it.
#[derive(Debug, Clone, PartialEq)]
pub struct CommentWrite {
    pub issue_id: String,
    /// The issue's key, for messages.
    pub identifier: String,
    pub comment_id: String,
    pub change: CommentChange,
    previous: Vec<Comment>,
}

/// What a [`CommentWrite`] does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommentChange {
    /// Replace the text with this.
    Edit(String),
    /// Delete it and its replies.
    Delete,
    /// Resolve the thread it starts.
    Resolve,
    /// Open the thread it starts again.
    Unresolve,
}

impl CommentChange {
    /// What it does, after "Could not".
    fn verb(&self) -> &'static str {
        match self {
            CommentChange::Edit(_) => "edit the comment",
            CommentChange::Delete => "delete the comment",
            CommentChange::Resolve => "resolve the thread",
            CommentChange::Unresolve => "unresolve the thread",
        }
    }
}

/// Moves the focus to the next comment, or the previous one when not
/// `forward`, starting from the first or the last, and scrolls it into
/// view. It stays at either end.
pub(super) fn move_focus(state: &mut State, forward: bool) -> Update {
    let Some(issue_id) = state.focused_issue().map(|issue| issue.id.clone()) else {
        return Update::default();
    };
    let Some(comments) = state.store.comments(&issue_id) else {
        return Update::default();
    };
    let current = state.detail.focused_comment(&issue_id);
    let order = focusable(comments, current);
    let at = current.and_then(|id| order.iter().position(|comment| comment.id == id));
    let next = match (at, forward) {
        (None, true) => order.first(),
        (None, false) => order.last(),
        (Some(at), true) => order.get(at + 1),
        (Some(at), false) => at.checked_sub(1).and_then(|at| order.get(at)),
    };
    let Some(next) = next.map(|comment| comment.id.clone()) else {
        return Update::default();
    };
    state.detail.focus_comment(&issue_id, Some(next));
    scroll_to_focus(state, &issue_id);
    Update::redraw()
}

/// Scrolls the detail of `issue_id` so the focused comment shows, its top
/// first when it is taller than the pane.
fn scroll_to_focus(state: &mut State, issue_id: &str) {
    let Some((start, end)) = ui::focused_comment_lines(state, state.viewport) else {
        return;
    };
    let (rows, _) = ui::detail_rows(state, state.viewport);
    state.detail.show_lines(issue_id, start, end, rows);
}

/// Drops the focus from the focused comment.
pub(super) fn unfocus(state: &mut State) -> Update {
    let Some(issue_id) = state.focused_issue().map(|issue| issue.id.clone()) else {
        return Update::default();
    };
    state.detail.focus_comment(&issue_id, None);
    Update::redraw()
}

/// The focused issue's key and the focused comment, unless the comment is
/// one Linear has not confirmed yet, which gets a notice instead.
fn saved_focus(state: &mut State) -> Option<(String, Comment)> {
    let comment = focused_comment(state)?.clone();
    let identifier = state.focused_issue()?.identifier.clone();
    if super::is_local(&comment.id) {
        state.notice = Some("That reply is not saved yet; try again once Linear has it".to_owned());
        return None;
    }
    Some((identifier, comment))
}

/// The first comment of the thread `comment` is in.
fn thread_root(state: &State, comment: &Comment) -> Comment {
    let comments = state.store.comments(&comment.issue_id).unwrap_or_default();
    threads(comments)
        .into_iter()
        .find(|(root, replies)| {
            root.id == comment.id || replies.iter().any(|reply| reply.id == comment.id)
        })
        .map_or_else(|| comment.clone(), |(root, _)| root.clone())
}

/// Whose `comment` is, for a refusal, as `Ana Brie's`.
fn whose(state: &State, comment: &Comment) -> String {
    comment
        .author_id
        .as_deref()
        .and_then(|id| state.store.member(id))
        .map_or_else(
            || "someone else's".to_owned(),
            |member| format!("{}'s", member.name),
        )
}

/// Opens the editor for a reply to the focused comment's thread, starting
/// from what was written for it before if Linear did not take that.
pub(super) fn reply(state: &mut State) -> Update {
    let Some((identifier, comment)) = saved_focus(state) else {
        return Update::redraw();
    };
    let parent_id = thread_root(state, &comment).id;
    let text = state.drafts.reply(&parent_id);
    let writing = Writing::Reply {
        issue_id: comment.issue_id,
        identifier,
        parent_id,
    };
    Update {
        redraw: false,
        effects: vec![Effect::OpenEditor { writing, text }],
    }
}

/// Opens the editor on the focused comment's text, if the viewer wrote it.
pub(super) fn edit(state: &mut State) -> Update {
    let Some((identifier, comment)) = saved_focus(state) else {
        return Update::redraw();
    };
    if !is_own(state, &comment) {
        state.notice = Some(format!(
            "Only its author can edit a comment; this one is {}",
            whose(state, &comment)
        ));
        return Update::redraw();
    }
    let text = state.drafts.edit(&comment.id).unwrap_or(comment.body);
    let writing = Writing::EditComment {
        issue_id: comment.issue_id,
        identifier,
        comment_id: comment.id,
    };
    Update {
        redraw: false,
        effects: vec![Effect::OpenEditor { writing, text }],
    }
}

/// Asks the viewer to confirm deleting the focused comment, if they wrote
/// it.
pub(super) fn ask_to_delete(state: &mut State) -> Update {
    let Some((_, comment)) = saved_focus(state) else {
        return Update::redraw();
    };
    if !is_own(state, &comment) {
        state.notice = Some(format!(
            "Only its author can delete a comment; this one is {}",
            whose(state, &comment)
        ));
        return Update::redraw();
    }
    let replies = state
        .store
        .comments(&comment.issue_id)
        .unwrap_or_default()
        .iter()
        .filter(|reply| reply.parent_id.as_deref() == Some(comment.id.as_str()))
        .count();
    let mut body = Vec::new();
    match replies {
        0 => {}
        1 => body.push("Its reply is deleted too.".to_owned()),
        n => body.push(format!("Its {n} replies are deleted too.")),
    }
    body.push("Linear cannot bring it back.".to_owned());
    let subject = comment.body.lines().next().unwrap_or_default().to_owned();
    state.modals.push(Modal::Confirm(Confirm {
        title: "Delete your comment?".to_owned(),
        subject,
        body,
        verb: "delete",
        on_confirm: OnConfirm::DeleteComment {
            issue_id: comment.issue_id,
            comment_id: comment.id,
        },
    }));
    Update::redraw()
}

/// Deletes the comment `comment_id` of the issue `issue_id` and its
/// replies, after the viewer confirmed it. The focus moves to the comment
/// after it, or before it at the end.
pub(super) fn delete(state: &mut State, issue_id: &str, comment_id: &str) -> Update {
    let Some(identifier) = state.store.issue(issue_id).map(|i| i.identifier.clone()) else {
        return Update::redraw();
    };
    let neighbour = state.store.comments(issue_id).and_then(|comments| {
        let order = focusable(comments, Some(comment_id));
        let at = order.iter().position(|comment| comment.id == comment_id)?;
        order[at + 1..]
            .iter()
            .chain(order[..at].iter().rev())
            .find(|other| other.parent_id.as_deref() != Some(comment_id))
            .map(|other| other.id.clone())
    });
    let previous = state.store.remove_comment(comment_id);
    if state.detail.focused_comment(issue_id) == Some(comment_id) {
        state.detail.focus_comment(issue_id, neighbour);
    }
    send(
        state,
        CommentWrite {
            issue_id: issue_id.to_owned(),
            identifier,
            comment_id: comment_id.to_owned(),
            change: CommentChange::Delete,
            previous,
        },
    )
}

/// Resolves the thread of the focused comment, or opens it again when it
/// is resolved.
pub(super) fn toggle_resolved(state: &mut State) -> Update {
    let Some((identifier, comment)) = saved_focus(state) else {
        return Update::redraw();
    };
    let root = thread_root(state, &comment);
    let change = if root.resolved_at.is_some() {
        CommentChange::Unresolve
    } else {
        CommentChange::Resolve
    };
    let at = (change == CommentChange::Resolve).then(|| state.now());
    state.store.resolve_comment(&root.id, at);
    send(
        state,
        CommentWrite {
            issue_id: root.issue_id.clone(),
            identifier,
            comment_id: root.id.clone(),
            change,
            previous: vec![root],
        },
    )
}

/// Takes in the viewer's new text for the comment `comment_id`: an empty
/// or unchanged text sends nothing, anything else shows at once and goes
/// to Linear.
pub(super) fn edited(
    state: &mut State,
    issue_id: String,
    identifier: String,
    comment_id: String,
    text: &str,
) -> Update {
    let stored = state
        .store
        .comments(&issue_id)
        .and_then(|comments| comments.iter().find(|comment| comment.id == comment_id))
        .cloned();
    let Some(stored) = stored else {
        state.notice = Some(format!("That comment on {identifier} is gone"));
        return Update::redraw();
    };
    if text.is_empty() {
        state.notice = Some("Comment not changed: it was empty; X deletes it".to_owned());
        return Update::redraw();
    }
    if text == stored.body.trim() {
        state.drafts.forget_edit(&comment_id);
        state.notice = Some("Comment not changed".to_owned());
        return Update::redraw();
    }
    let mut changed = stored.clone();
    changed.body = text.to_owned();
    changed.edited_at = Some(state.now());
    state.store.replace_comment(changed);
    send(
        state,
        CommentWrite {
            issue_id,
            identifier,
            comment_id,
            change: CommentChange::Edit(text.to_owned()),
            previous: vec![stored],
        },
    )
}

/// Remembers `write` as in flight and asks for it to be sent.
fn send(state: &mut State, write: CommentWrite) -> Update {
    state.detail.comment_writes.push(write.clone());
    Update {
        redraw: true,
        effects: vec![Effect::ChangeComment(write)],
    }
}

/// Shows `reply`, a stand-in for a reply Linear has not stored yet, and
/// remembers it until Linear answers.
pub(super) fn show_reply(state: &mut State, reply: Comment) {
    state.store.put_back_comments(vec![reply.clone()]);
    state.detail.replies.push(reply);
}

/// Forgets the stand-in reply `local_id` once Linear answered.
pub(super) fn drop_reply(state: &mut State, local_id: &str) {
    state.detail.replies.retain(|reply| reply.id != local_id);
    state.store.remove_comment(local_id);
}

/// Takes in Linear's answer to `write`: on a failure the comments it
/// changed come back, the error shows and an edit's text is kept for the
/// next edit of the comment.
pub(super) fn changed(
    state: &mut State,
    write: &CommentWrite,
    result: Result<(), Failure>,
) -> Update {
    state
        .detail
        .comment_writes
        .retain(|in_flight| in_flight != write);
    if let Some(team_id) = state
        .store
        .issue(&write.issue_id)
        .map(|issue| issue.team_id.clone())
    {
        let outcome = result.as_ref().map(|_| ()).map_err(|failure| &failure.kind);
        state.learn_write(&team_id, WriteClass::ChangeComment, outcome);
    }
    match result {
        Ok(()) => {
            if let CommentChange::Edit(_) = write.change {
                state.drafts.forget_edit(&write.comment_id);
            }
        }
        Err(failure) => {
            roll_back(state, write);
            let kept = match &write.change {
                CommentChange::Edit(text) => {
                    state.drafts.keep_edit(&write.comment_id, text);
                    ". The next edit of it starts from what you wrote"
                }
                _ => "",
            };
            state.notice = Some(format!(
                "Could not {} on {}: {failure}{kept}",
                write.change.verb(),
                write.identifier
            ));
            state.record(failure);
        }
    }
    Update::redraw()
}

/// Puts back what `write` changed.
fn roll_back(state: &mut State, write: &CommentWrite) {
    match write.change {
        CommentChange::Delete => state.store.put_back_comments(write.previous.clone()),
        CommentChange::Edit(_) | CommentChange::Resolve | CommentChange::Unresolve => {
            for previous in &write.previous {
                state.store.replace_comment(previous.clone());
            }
        }
    }
}

/// Puts the comment writes and stand-in replies still in flight back over
/// the store, after a detail load replaced the comments they touch.
pub(super) fn reapply_in_flight(state: &mut State) {
    state.store.put_back_comments(state.detail.replies.clone());
    for write in state.detail.comment_writes.clone() {
        match &write.change {
            CommentChange::Delete => {
                state.store.remove_comment(&write.comment_id);
            }
            CommentChange::Edit(text) => {
                let stored = state
                    .store
                    .comments(&write.issue_id)
                    .and_then(|comments| comments.iter().find(|c| c.id == write.comment_id))
                    .cloned();
                if let Some(mut stored) = stored {
                    stored.body.clone_from(text);
                    stored.edited_at = stored.edited_at.or(Some(state.now()));
                    state.store.replace_comment(stored);
                }
            }
            CommentChange::Resolve => {
                let at = state.now();
                state.store.resolve_comment(&write.comment_id, Some(at));
            }
            CommentChange::Unresolve => {
                state.store.resolve_comment(&write.comment_id, None);
            }
        }
    }
}

/// Takes in the viewer's reply to the thread `parent_id` starts: empty
/// cancels, anything else shows at once under a stand-in and goes to
/// Linear.
pub(super) fn replied(
    state: &mut State,
    issue_id: String,
    identifier: String,
    parent_id: String,
    text: &str,
) -> Update {
    if text.is_empty() {
        state.drafts.forget_reply(&parent_id);
        state.notice = Some(format!("No reply added on {identifier}: it was empty"));
        return Update::redraw();
    }
    let local_id = state.drafts.next_stand_in("reply");
    let reply = Comment {
        id: local_id.clone(),
        issue_id: issue_id.clone(),
        author_id: state.store.viewer().map(|viewer| viewer.id.clone()),
        body: text.to_owned(),
        created_at: state.now(),
        parent_id: Some(parent_id.clone()),
        resolved_at: None,
        edited_at: None,
        reactions: Vec::new(),
    };
    show_reply(state, reply);
    state.notice = Some(format!("Replying on {identifier}…"));
    Update {
        redraw: true,
        effects: vec![Effect::CreateComment(NewComment {
            issue_id,
            identifier,
            body: text.to_owned(),
            parent_id: Some(parent_id),
            local_id: Some(local_id),
        })],
    }
}

#[cfg(test)]
mod tests {
    use jiff::Timestamp;

    use super::*;

    fn comment(id: &str, parent: Option<&str>) -> Comment {
        Comment {
            id: id.to_owned(),
            issue_id: "issue-1".to_owned(),
            author_id: None,
            body: id.to_owned(),
            created_at: Timestamp::UNIX_EPOCH,
            parent_id: parent.map(str::to_owned),
            resolved_at: None,
            edited_at: None,
            reactions: Vec::new(),
        }
    }

    fn ids(comments: &[&Comment]) -> Vec<String> {
        comments.iter().map(|comment| comment.id.clone()).collect()
    }

    #[test]
    fn replies_join_the_thread_of_their_first_comment() {
        let comments = [
            comment("a", None),
            comment("b", None),
            comment("a1", Some("a")),
            comment("a2", Some("a1")),
            comment("orphan", Some("gone")),
        ];

        let threads = threads(&comments);

        let shape: Vec<(String, Vec<String>)> = threads
            .iter()
            .map(|(root, replies)| (root.id.clone(), ids(replies)))
            .collect();
        assert_eq!(
            shape,
            [
                ("a".to_owned(), vec!["a1".to_owned(), "a2".to_owned()]),
                ("b".to_owned(), vec![]),
                ("orphan".to_owned(), vec![]),
            ]
        );
    }

    #[test]
    fn a_resolved_thread_hides_its_replies_until_the_focus_is_in_it() {
        let mut root = comment("a", None);
        root.resolved_at = Some(Timestamp::UNIX_EPOCH);
        let comments = [root, comment("a1", Some("a")), comment("b", None)];

        assert_eq!(ids(&focusable(&comments, None)), ["a", "b"]);
        assert_eq!(ids(&focusable(&comments, Some("a"))), ["a", "a1", "b"]);
        assert_eq!(ids(&focusable(&comments, Some("a1"))), ["a", "a1", "b"]);
    }
}
