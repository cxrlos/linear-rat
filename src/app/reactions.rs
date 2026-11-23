//! The viewer's reactions on the focused comment, or on the issue when no
//! comment is focused: picked from eight common emoji, added when the
//! viewer has not given that one yet and removed when they have. Each shows
//! at once and is undone if Linear rejects it.

use super::comments;
use super::modal::{Choice, Purpose};
use super::picker::{Entry, Picker};
use super::{Effect, Failure, Modal, State, Update, WriteClass, detail};
use crate::linear::ReactionTarget;
use crate::linear::model::Reaction;

/// The emoji the picker offers, with the name typing finds them by and the
/// shortcodes Linear may store them as.
const REACTIONS: [(&str, &str, &[&str]); 8] = [
    ("👍", "thumbs up", &["+1", "thumbsup"]),
    ("👎", "thumbs down", &["-1", "thumbsdown"]),
    ("😄", "smile", &["smile"]),
    ("🎉", "party", &["tada"]),
    ("😕", "confused", &["confused"]),
    ("❤️", "heart", &["heart", "❤"]),
    ("🚀", "rocket", &["rocket"]),
    ("👀", "eyes", &["eyes"]),
];

/// How `stored`, an emoji as Linear stored it, shows: one of the picker's
/// emoji when it is that one under a shortcode, otherwise as it came.
pub fn shown_emoji(stored: &str) -> &str {
    REACTIONS
        .iter()
        .find(|(emoji, _, aliases)| *emoji == stored || aliases.contains(&stored))
        .map_or(stored, |(emoji, ..)| emoji)
}

/// A reaction change on its way to Linear: what it is on, for which issue,
/// and what it does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReactionWrite {
    pub issue_id: String,
    /// The issue's key, for messages.
    pub identifier: String,
    pub target: ReactionTarget,
    pub change: ReactionChange,
}

/// What a [`ReactionWrite`] does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReactionChange {
    /// Add this stand-in's emoji; the stand-in shows until Linear answers.
    Add(Reaction),
    /// Remove this reaction of the viewer's.
    Remove(Reaction),
}

/// Whether the detail has focus on a saved issue to react to.
pub fn can_react(state: &State) -> bool {
    detail::has_focus(state)
        && state
            .focused_issue()
            .is_some_and(|issue| !super::is_local(&issue.id))
}

/// What a reaction now goes on: the focused comment, or the focused issue.
fn target(state: &State) -> Option<(String, String, ReactionTarget)> {
    let issue = state.focused_issue()?;
    let target = match comments::focused_comment(state) {
        Some(comment) => ReactionTarget::Comment(comment.id.clone()),
        None => ReactionTarget::Issue(issue.id.clone()),
    };
    Some((issue.id.clone(), issue.identifier.clone(), target))
}

/// The viewer's reaction with `emoji` on `target`, if they gave one.
fn own<'a>(state: &'a State, target: &ReactionTarget, emoji: &str) -> Option<&'a Reaction> {
    let viewer = state.store.viewer()?;
    state.store.reactions(target).iter().find(|reaction| {
        reaction.user_id.as_deref() == Some(viewer.id.as_str())
            && shown_emoji(&reaction.emoji) == emoji
    })
}

/// Opens the emoji picker for the focused comment, or the issue.
pub(super) fn open(state: &mut State) -> Update {
    let Some((_, _, target)) = target(state) else {
        return Update::default();
    };
    if let ReactionTarget::Comment(id) = &target
        && super::is_local(id)
    {
        state.notice = Some("That reply is not saved yet; react once Linear has it".to_owned());
        return Update::redraw();
    }
    let entries = REACTIONS
        .iter()
        .map(|&(emoji, name, _)| {
            let yours = if own(state, &target, emoji).is_some() {
                " · yours, removes it"
            } else {
                ""
            };
            Entry {
                label: format!("{emoji} {name}{yours}"),
                value: Some(emoji.to_owned()),
            }
        })
        .collect();
    state.modals.push(Modal::Choice(Choice {
        purpose: Purpose::Reaction,
        list: Picker::new(entries, None),
    }));
    Update::redraw()
}

/// Closes the picker on the selected emoji, or the `nth` visible one, and
/// adds it, or removes it when the viewer gave it already.
pub(super) fn chosen(state: &mut State, nth: Option<usize>) -> Update {
    let Some(Modal::Choice(choice)) = state.modals.top() else {
        return Update::default();
    };
    let Some(Some(emoji)) = choice.chosen(nth).cloned() else {
        return Update::default();
    };
    state.modals.pop();
    let Some((issue_id, identifier, target)) = target(state) else {
        return Update::redraw();
    };
    let change = match own(state, &target, &emoji).cloned() {
        Some(reaction) if super::is_local(&reaction.id) => {
            state.notice =
                Some("That reaction is not saved yet; try again once Linear has it".to_owned());
            return Update::redraw();
        }
        Some(reaction) => {
            state.store.remove_reaction(&target, &reaction.id);
            ReactionChange::Remove(reaction)
        }
        None => {
            let stand_in = Reaction {
                id: state.drafts.next_stand_in("reaction"),
                emoji,
                user_id: state.store.viewer().map(|viewer| viewer.id.clone()),
                created_at: state.now(),
            };
            state.store.add_reaction(&target, stand_in.clone());
            ReactionChange::Add(stand_in)
        }
    };
    let write = ReactionWrite {
        issue_id,
        identifier,
        target,
        change,
    };
    state.detail.reaction_writes.push(write.clone());
    Update {
        redraw: true,
        effects: vec![Effect::React(write)],
    }
}

/// Takes in Linear's answer to `write`: an added reaction replaces its
/// stand-in, or on a failure the change is undone and the error shows.
pub(super) fn reacted(
    state: &mut State,
    write: &ReactionWrite,
    result: Result<Option<Reaction>, Failure>,
) -> Update {
    state
        .detail
        .reaction_writes
        .retain(|in_flight| in_flight != write);
    if let Some(team_id) = state
        .store
        .issue(&write.issue_id)
        .map(|issue| issue.team_id.clone())
    {
        let outcome = result.as_ref().map(|_| ()).map_err(|failure| &failure.kind);
        state.learn_write(&team_id, WriteClass::React, outcome);
    }
    match (&write.change, result) {
        (ReactionChange::Add(stand_in), Ok(stored)) => {
            state.store.remove_reaction(&write.target, &stand_in.id);
            if let Some(stored) = stored {
                state.store.add_reaction(&write.target, stored);
            }
        }
        (ReactionChange::Remove(_), Ok(_)) => {}
        (change, Err(failure)) => {
            let what = match change {
                ReactionChange::Add(stand_in) => {
                    state.store.remove_reaction(&write.target, &stand_in.id);
                    format!("add {}", shown_emoji(&stand_in.emoji))
                }
                ReactionChange::Remove(reaction) => {
                    state.store.add_reaction(&write.target, reaction.clone());
                    format!("remove your {}", shown_emoji(&reaction.emoji))
                }
            };
            state.notice = Some(format!(
                "Could not {what} on {}: {failure}",
                write.identifier
            ));
            state.record(failure);
        }
    }
    Update::redraw()
}

/// Puts the reaction changes still in flight back over the store, after a
/// detail load replaced the reactions they touch.
pub(super) fn reapply_in_flight(state: &mut State) {
    for write in state.detail.reaction_writes.clone() {
        match write.change {
            ReactionChange::Add(stand_in) => state.store.add_reaction(&write.target, stand_in),
            ReactionChange::Remove(reaction) => {
                state.store.remove_reaction(&write.target, &reaction.id);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shortcodes_show_as_the_pickers_emoji() {
        assert_eq!(shown_emoji("+1"), "👍");
        assert_eq!(shown_emoji("tada"), "🎉");
        assert_eq!(shown_emoji("❤"), "❤️");
        assert_eq!(shown_emoji("🦀"), "🦀");
    }
}
