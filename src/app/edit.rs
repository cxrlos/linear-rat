//! Changing an issue's workflow state, assignee or priority. A picker
//! chooses the new value; the change shows at once, goes to Linear as one
//! `issueUpdate`, and is rolled back if Linear rejects it.

use super::picker::{Entry, Note, Picker};
use super::{Effect, Modal, State, Store, Update};
use crate::linear::model::{Issue, Priority, WorkflowState};
use crate::linear::{IssueUpdate, Patch};
use crate::ui::icons;

/// A field of an issue the viewer can change with a picker.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Editable {
    State,
    Assignee,
    Priority,
}

impl Editable {
    /// How messages name the field.
    pub fn name(self) -> &'static str {
        match self {
            Editable::State => "state",
            Editable::Assignee => "assignee",
            Editable::Priority => "priority",
        }
    }

    /// The value `issue` has now for this field.
    pub fn current(self, issue: &Issue) -> Edit {
        match self {
            Editable::State => Edit::State(issue.state.clone()),
            Editable::Assignee => Edit::Assignee(issue.assignee_id.clone()),
            Editable::Priority => Edit::Priority(issue.priority),
        }
    }
}

/// A new value for one field of an issue.
#[derive(Debug, Clone, PartialEq)]
pub enum Edit {
    State(WorkflowState),
    /// A member's id, or `None` to unassign.
    Assignee(Option<String>),
    Priority(Priority),
}

impl Edit {
    pub fn field(&self) -> Editable {
        match self {
            Edit::State(_) => Editable::State,
            Edit::Assignee(_) => Editable::Assignee,
            Edit::Priority(_) => Editable::Priority,
        }
    }

    pub fn apply_to(&self, issue: &mut Issue) {
        match self {
            Edit::State(state) => issue.state = state.clone(),
            Edit::Assignee(assignee) => issue.assignee_id.clone_from(assignee),
            Edit::Priority(priority) => issue.priority = *priority,
        }
    }

    /// The `issueUpdate` input that makes this change and no other.
    pub fn update(&self) -> IssueUpdate {
        match self {
            Edit::State(state) => IssueUpdate {
                state_id: Some(state.id.clone()),
                ..IssueUpdate::default()
            },
            Edit::Assignee(assignee) => IssueUpdate {
                assignee_id: assignee.clone().map_or(Patch::Clear, Patch::Set),
                ..IssueUpdate::default()
            },
            Edit::Priority(priority) => IssueUpdate {
                priority: Some(*priority),
                ..IssueUpdate::default()
            },
        }
    }
}

/// An edit on its way to Linear: the change already shown, and the value
/// it replaced, to restore if Linear rejects it.
#[derive(Debug, Clone, PartialEq)]
pub struct Write {
    pub issue_id: String,
    pub identifier: String,
    pub edit: Edit,
    pub previous: Edit,
}

/// An open picker for one field of one issue.
#[derive(Debug, Clone, PartialEq)]
pub struct EditPicker {
    pub issue_id: String,
    pub identifier: String,
    pub team_id: String,
    pub field: Editable,
    pub list: Picker<Edit>,
}

impl EditPicker {
    /// What the picker's border says, as `State of ENG-12`.
    pub fn title(&self) -> String {
        let field = match self.field {
            Editable::State => "State",
            Editable::Assignee => "Assignee",
            Editable::Priority => "Priority",
        };
        format!("{field} of {}", self.identifier)
    }

    /// Fills the list from what `store` holds. Returns whether the store
    /// still lacks entries the picker needs, so they must be loaded.
    fn fill(&mut self, store: &Store) -> bool {
        let Some(issue) = store.issue(&self.issue_id) else {
            return false;
        };
        let (entries, missing) = match self.field {
            Editable::State => state_entries(store, &self.team_id),
            Editable::Assignee => assignee_entries(store, &self.team_id),
            Editable::Priority => (priority_entries(), false),
        };
        let current = self.field.current(issue);
        let current = entries.iter().position(|entry| entry.value == current);
        self.list.set_entries(entries, current);
        if missing {
            if self.list.note().is_none() {
                self.list.set_note(Some(Note::Loading));
            }
        } else {
            self.list.set_note(None);
        }
        missing
    }
}

/// The team's workflow states by state type, as work moves through them,
/// then by their order in the team. Missing until they are loaded.
fn state_entries(store: &Store, team_id: &str) -> (Vec<Entry<Edit>>, bool) {
    let Some(states) = store.workflow_states(team_id) else {
        return (Vec::new(), true);
    };
    let mut states = states.to_vec();
    states.sort_by(|a, b| {
        (a.state_type, a.position)
            .partial_cmp(&(b.state_type, b.position))
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let entries = states
        .into_iter()
        .map(|state| Entry {
            label: state.name.clone(),
            value: Edit::State(state),
        })
        .collect();
    (entries, false)
}

/// The viewer and "unassigned" first, then the team's other members by
/// name. The members are missing until they are loaded.
fn assignee_entries(store: &Store, team_id: &str) -> (Vec<Entry<Edit>>, bool) {
    let viewer = store.viewer();
    let mut entries: Vec<Entry<Edit>> = viewer
        .map(|viewer| Entry {
            label: format!("Me ({})", viewer.name),
            value: Edit::Assignee(Some(viewer.id.clone())),
        })
        .into_iter()
        .collect();
    entries.push(Entry {
        label: "Unassigned".to_owned(),
        value: Edit::Assignee(None),
    });
    let Some(mut members) = store.team_members(team_id) else {
        return (entries, true);
    };
    members.retain(|member| viewer.is_none_or(|viewer| viewer.id != member.id));
    members.sort_by(|a, b| a.name.cmp(&b.name));
    entries.extend(members.into_iter().map(|member| Entry {
        label: member.name.clone(),
        value: Edit::Assignee(Some(member.id.clone())),
    }));
    (entries, false)
}

fn priority_entries() -> Vec<Entry<Edit>> {
    [
        Priority::Urgent,
        Priority::High,
        Priority::Medium,
        Priority::Low,
        Priority::None,
    ]
    .into_iter()
    .map(|priority| Entry {
        label: icons::priority_name(priority).to_owned(),
        value: Edit::Priority(priority),
    })
    .collect()
}

/// Opens the picker for `field` of the focused issue, and asks for what it
/// lists if the store does not have it yet.
pub(super) fn open(state: &mut State, field: Editable) -> Update {
    let Some(issue) = state.focused_issue() else {
        return Update::default();
    };
    let mut picker = EditPicker {
        issue_id: issue.id.clone(),
        identifier: issue.identifier.clone(),
        team_id: issue.team_id.clone(),
        field,
        list: Picker::new(Vec::new(), None),
    };
    let missing = picker.fill(&state.store);
    let team_id = picker.team_id.clone();
    state.modals.push(Modal::Picker(picker));
    let effects = match (missing, field) {
        (true, Editable::State) => vec![Effect::LoadWorkflowStates { team_id }],
        (true, Editable::Assignee) => vec![Effect::LoadTeamMembers { team_id }],
        _ => Vec::new(),
    };
    Update {
        redraw: true,
        effects,
    }
}

/// Moves the open picker's selection by `step`.
pub(super) fn select(state: &mut State, step: isize) -> Update {
    match state.modals.top_mut() {
        Some(Modal::Picker(picker)) => Update {
            redraw: picker.list.select_by(step),
            effects: Vec::new(),
        },
        _ => Update::default(),
    }
}

/// Closes the open picker on the selected entry, or the `nth` visible one
/// when given, and makes that edit.
pub(super) fn choose(state: &mut State, nth: Option<usize>) -> Update {
    let Some(Modal::Picker(picker)) = state.modals.top() else {
        return Update::default();
    };
    let chosen = match nth {
        Some(n) => picker.list.nth(n),
        None => picker.list.selected(),
    };
    let Some(edit) = chosen.cloned() else {
        return Update::default();
    };
    let issue_id = picker.issue_id.clone();
    state.modals.pop();
    write(state, &issue_id, edit)
}

/// Shows `edit` on the issue at once and asks Linear to make it. Choosing
/// the value the issue already has sends nothing.
fn write(state: &mut State, issue_id: &str, edit: Edit) -> Update {
    let Some(issue) = state.store.issue(issue_id) else {
        return Update::redraw();
    };
    let identifier = issue.identifier.clone();
    let previous = edit.field().current(issue);
    if previous == edit {
        return Update::redraw();
    }
    state.store.apply_edit(issue_id, &edit);
    state.row_errors.remove(issue_id);
    state.rebuild_my_issues();
    Update {
        redraw: true,
        effects: vec![Effect::UpdateIssue(Write {
            issue_id: issue_id.to_owned(),
            identifier,
            edit,
            previous,
        })],
    }
}

/// Takes in Linear's answer to `write`: the issue as stored, or why the edit
/// was rejected, in which case the previous value comes back and the error
/// shows on the row and in the notice.
pub(super) fn finished(
    state: &mut State,
    write: &Write,
    result: Result<Box<Issue>, String>,
) -> Update {
    match result {
        Ok(issue) => {
            state.store.upsert_issue(*issue);
        }
        Err(error) => {
            state.store.roll_back(write);
            state.notice = Some(format!(
                "Could not change the {} of {}: {error}",
                write.edit.field().name(),
                write.identifier
            ));
            state.row_errors.insert(write.issue_id.clone(), error);
        }
    }
    state.rebuild_my_issues();
    Update::redraw()
}

/// Refills the open pickers for `field` of team `team_id` once what they
/// list has loaded, or says in them and in the notice why it did not.
pub(super) fn loaded(
    state: &mut State,
    field: Editable,
    team_id: &str,
    failure: Option<String>,
) -> Update {
    let what = match field {
        Editable::State => "workflow states",
        _ => "members",
    };
    let team = state
        .store
        .teams()
        .find(|team| team.id == team_id)
        .map_or(team_id, |team| team.key.as_str())
        .to_owned();
    for modal in state.modals.iter_mut() {
        let Modal::Picker(picker) = modal else {
            continue;
        };
        if picker.field != field || picker.team_id != team_id {
            continue;
        }
        match &failure {
            Some(error) => picker.list.set_note(Some(Note::Failed(error.clone()))),
            None => {
                picker.fill(&state.store);
            }
        }
    }
    if let Some(error) = failure {
        state.notice = Some(format!("Could not load the {what} of {team}: {error}"));
    }
    Update::redraw()
}
