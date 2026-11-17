//! An issue's parent, sub-issues and relations: setting and clearing the
//! parent, adding and removing relations, and going to a related issue, all
//! through the issue picker. Relation writes show at once and are rolled
//! back if Linear rejects them, as edits are.

use super::detail::DetailLoad;
use super::edit::{self, Edit};
use super::issue_picker::{self, IssuePicker, IssuePurpose, Mode, Scope};
use super::modal::{Choice, Purpose};
use super::picker::{Entry, Picker};
use super::{Effect, Modal, State, Update, detail};
use crate::linear::model::{Issue, IssueRelation, RelationKind, RelationType};

/// A relation on its way to Linear, and the key of the issue the viewer
/// changed, for messages. A new relation carries a stand-in id until
/// Linear answers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelationWrite {
    pub relation: IssueRelation,
    pub identifier: String,
}

/// The kinds a new relation can have, in the order the chooser lists them.
const ADDABLE: [RelationKind; 4] = [
    RelationKind::Blocks,
    RelationKind::BlockedBy,
    RelationKind::Related,
    RelationKind::DuplicateOf,
];

/// The focused issue, unless it is a stand-in Linear has not confirmed,
/// which gets a notice instead.
fn focused(state: &mut State) -> Option<Issue> {
    let issue = state.focused_issue()?.clone();
    if super::is_local(&issue.id) {
        state.notice = Some(format!(
            "{} is not created yet; change it once Linear confirms it",
            issue.identifier
        ));
        return None;
    }
    Some(issue)
}

/// Whether the detail of `issue`, which brings its relations and
/// sub-issues, has loaded. If not, asks for it and says so.
fn family_loaded(state: &mut State, issue: &Issue) -> (bool, Vec<Effect>) {
    if state.detail.load(&issue.id) == Some(&DetailLoad::Loaded) {
        return (true, Vec::new());
    }
    state.notice = Some(format!(
        "Loading the relations of {}; try again once they show",
        issue.identifier
    ));
    (
        false,
        detail::load_now(state, &issue.id).into_iter().collect(),
    )
}

/// Opens the issue picker for the focused issue's parent, leaving out the
/// issue itself and its sub-issues at any depth. It waits for the detail,
/// which brings the sub-issues, so the store knows the parents that the
/// issues Linear's search finds hang from.
pub(super) fn open_parent(state: &mut State) -> Update {
    let Some(issue) = focused(state) else {
        return Update::redraw();
    };
    let (loaded, effects) = family_loaded(state, &issue);
    if !loaded {
        return Update {
            redraw: true,
            effects,
        };
    }
    let picker = IssuePicker::new(
        IssuePurpose::SetParent {
            issue_id: issue.id.clone(),
        },
        Mode::One,
        Scope {
            excluded: vec![issue.id.clone()],
            only: None,
            not_under: Some(issue.id.clone()),
        },
        issue.parent_id.clone(),
    );
    issue_picker::open(state, picker)
}

/// Makes the focused issue a top-level issue again.
pub(super) fn clear_parent(state: &mut State) -> Update {
    let Some(issue) = focused(state) else {
        return Update::redraw();
    };
    if issue.parent_id.is_none() {
        state.notice = Some(format!("{} has no parent", issue.identifier));
        return Update::redraw();
    }
    edit::write(state, &issue.id, Edit::Parent(None))
}

/// Opens the chooser of the kind of relation to add to the focused issue.
pub(super) fn open_add(state: &mut State) -> Update {
    if focused(state).is_none() {
        return Update::redraw();
    }
    let entries = ADDABLE
        .into_iter()
        .map(|kind| Entry {
            label: kind.name().to_owned(),
            value: Some(kind_id(kind).to_owned()),
        })
        .collect();
    state.modals.push(Modal::Choice(Choice {
        purpose: Purpose::RelationKind,
        list: Picker::new(entries, None),
    }));
    Update::redraw()
}

fn kind_id(kind: RelationKind) -> &'static str {
    match kind {
        RelationKind::Blocks => "blocks",
        RelationKind::BlockedBy => "blocked-by",
        RelationKind::Related => "related",
        RelationKind::DuplicateOf => "duplicate-of",
        RelationKind::DuplicatedBy => "duplicated-by",
    }
}

/// Closes the kind chooser on the selected kind, or the `nth` visible one,
/// and opens the issue picker for the issues to relate that way, leaving
/// out the issue itself and those already related so.
pub(super) fn kind_chosen(state: &mut State, nth: Option<usize>) -> Update {
    let Some(Modal::Choice(choice)) = state.modals.top() else {
        return Update::default();
    };
    let Some(Some(id)) = choice.chosen(nth).cloned() else {
        return Update::default();
    };
    let Some(kind) = ADDABLE.into_iter().find(|&kind| kind_id(kind) == id) else {
        return Update::default();
    };
    state.modals.pop();
    let Some(issue) = state.focused_issue().cloned() else {
        return Update::redraw();
    };
    let mut excluded: Vec<String> = state
        .store
        .relations(&issue.id)
        .into_iter()
        .filter(|&(existing, _, _)| existing == kind)
        .map(|(_, _, other)| other.to_owned())
        .collect();
    excluded.push(issue.id.clone());
    let picker = IssuePicker::new(
        IssuePurpose::AddRelation {
            issue_id: issue.id,
            kind,
        },
        Mode::Several,
        Scope {
            excluded,
            only: None,
            not_under: None,
        },
        None,
    );
    issue_picker::open(state, picker)
}

/// Opens the issue picker over the issues the focused issue has relations
/// with, to remove them.
pub(super) fn open_remove(state: &mut State) -> Update {
    let Some(issue) = focused(state) else {
        return Update::redraw();
    };
    let (loaded, effects) = family_loaded(state, &issue);
    if !loaded {
        return Update {
            redraw: true,
            effects,
        };
    }
    let mut others: Vec<String> = Vec::new();
    for (_, _, other) in state.store.relations(&issue.id) {
        if !others.iter().any(|known| known == other) {
            others.push(other.to_owned());
        }
    }
    if others.is_empty() {
        state.notice = Some(format!("{} has no relations", issue.identifier));
        return Update::redraw();
    }
    let picker = IssuePicker::new(
        IssuePurpose::RemoveRelation { issue_id: issue.id },
        Mode::Several,
        Scope {
            excluded: Vec::new(),
            only: Some(others),
            not_under: None,
        },
        None,
    );
    issue_picker::open(state, picker)
}

/// Opens the issue picker over the focused issue's parent, sub-issues and
/// related issues, to go to one.
pub(super) fn open_related(state: &mut State) -> Update {
    let Some(issue) = focused(state) else {
        return Update::redraw();
    };
    let (loaded, effects) = family_loaded(state, &issue);
    if !loaded {
        return Update {
            redraw: true,
            effects,
        };
    }
    let mut family: Vec<String> = issue.parent_id.iter().cloned().collect();
    family.extend(
        state
            .store
            .children(&issue.id)
            .into_iter()
            .map(|child| child.id.clone()),
    );
    for (_, _, other) in state.store.relations(&issue.id) {
        if !family.iter().any(|known| known == other) {
            family.push(other.to_owned());
        }
    }
    if family.is_empty() {
        state.notice = Some(format!(
            "{} has no parent, sub-issues or relations",
            issue.identifier
        ));
        return Update::redraw();
    }
    let picker = IssuePicker::new(
        IssuePurpose::GoToRelated { issue_id: issue.id },
        Mode::One,
        Scope {
            excluded: Vec::new(),
            only: Some(family),
            not_under: None,
        },
        None,
    );
    issue_picker::open(state, picker)
}

/// Relates the issue `issue_id` to each of `others` as `kind` says: each
/// shows at once under a stand-in id and goes to Linear as one
/// `issueRelationCreate`.
pub(super) fn add(
    state: &mut State,
    issue_id: &str,
    kind: RelationKind,
    others: &[String],
) -> Update {
    let Some(identifier) = state.store.issue(issue_id).map(|i| i.identifier.clone()) else {
        return Update::redraw();
    };
    let mut effects = Vec::new();
    for other in others {
        let (relation_type, from, to) = match kind {
            RelationKind::Blocks => (RelationType::Blocks, issue_id, other.as_str()),
            RelationKind::BlockedBy => (RelationType::Blocks, other.as_str(), issue_id),
            RelationKind::Related => (RelationType::Related, issue_id, other.as_str()),
            RelationKind::DuplicateOf => (RelationType::Duplicate, issue_id, other.as_str()),
            RelationKind::DuplicatedBy => (RelationType::Duplicate, other.as_str(), issue_id),
        };
        let relation = IssueRelation {
            id: format!(
                "{}relation-{from}-{to}-{}",
                super::compose::LOCAL_PREFIX,
                kind_id(kind)
            ),
            kind: relation_type,
            issue_id: from.to_owned(),
            related_issue_id: to.to_owned(),
        };
        state.store.add_relation(relation.clone());
        effects.push(Effect::CreateRelation(RelationWrite {
            relation,
            identifier: identifier.clone(),
        }));
    }
    state.row_errors.remove(issue_id);
    Update {
        redraw: true,
        effects,
    }
}

/// Removes every relation between the issue `issue_id` and each of
/// `others`: each goes at once and to Linear as one `issueRelationDelete`.
/// A relation Linear has not confirmed yet is left alone.
pub(super) fn remove(state: &mut State, issue_id: &str, others: &[String]) -> Update {
    let Some(identifier) = state.store.issue(issue_id).map(|i| i.identifier.clone()) else {
        return Update::redraw();
    };
    let doomed: Vec<IssueRelation> = state
        .store
        .relations(issue_id)
        .into_iter()
        .filter(|(_, relation, other)| {
            others.iter().any(|chosen| chosen == other) && !super::is_local(&relation.id)
        })
        .map(|(_, relation, _)| relation.clone())
        .collect();
    let mut effects = Vec::new();
    for relation in doomed {
        state.store.remove_relation(&relation.id);
        effects.push(Effect::DeleteRelation(RelationWrite {
            relation,
            identifier: identifier.clone(),
        }));
    }
    state.row_errors.remove(issue_id);
    Update {
        redraw: true,
        effects,
    }
}

/// The issue of `write` the viewer changed: the source, unless the stand-in
/// was made from the other end.
fn changed_issue(state: &State, write: &RelationWrite) -> String {
    let relation = &write.relation;
    match state.store.issue(&relation.issue_id) {
        Some(issue) if issue.identifier == write.identifier => relation.issue_id.clone(),
        _ => relation.related_issue_id.clone(),
    }
}

/// Takes in Linear's answer to creating `write`'s relation: the stored
/// relation replaces the stand-in, or the stand-in goes and the error shows
/// on the row and in the notice.
pub(super) fn created(
    state: &mut State,
    write: &RelationWrite,
    result: Result<IssueRelation, String>,
) -> Update {
    state.store.remove_relation(&write.relation.id);
    match result {
        Ok(relation) => state.store.add_relation(relation),
        Err(error) => {
            state.notice = Some(format!(
                "Could not add the relation to {}: {error}",
                write.identifier
            ));
            let issue_id = changed_issue(state, write);
            state.row_errors.insert(issue_id, error);
        }
    }
    Update::redraw()
}

/// Takes in Linear's answer to deleting `write`'s relation: on a failure
/// the relation comes back and the error shows on the row and in the
/// notice.
pub(super) fn deleted(
    state: &mut State,
    write: &RelationWrite,
    result: Result<(), String>,
) -> Update {
    if let Err(error) = result {
        state.store.add_relation(write.relation.clone());
        state.notice = Some(format!(
            "Could not remove the relation from {}: {error}",
            write.identifier
        ));
        let issue_id = changed_issue(state, write);
        state.row_errors.insert(issue_id, error);
    }
    Update::redraw()
}
