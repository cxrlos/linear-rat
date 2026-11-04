//! Changing an issue's workflow state, assignee, priority, labels or
//! estimate. A picker chooses the new value (labels are marked in a
//! multi-select picker); the change shows at once, goes to Linear as one
//! `issueUpdate`, and is rolled back if Linear rejects it.

use super::picker::{Entry, Note, Picker};
use super::{Effect, Modal, State, Store, Update};
use crate::linear::model::{Estimation, Issue, Priority, WorkflowState};
use crate::linear::{IssueUpdate, Patch};
use crate::ui::icons;

/// A field of an issue the viewer can change with a picker.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Editable {
    State,
    Assignee,
    Priority,
    Labels,
    Estimate,
}

impl Editable {
    /// How messages name the field.
    pub fn name(self) -> &'static str {
        match self {
            Editable::State => "state",
            Editable::Assignee => "assignee",
            Editable::Priority => "priority",
            Editable::Labels => "labels",
            Editable::Estimate => "estimate",
        }
    }

    /// The value `issue` has now for this field.
    pub fn current(self, issue: &Issue) -> Edit {
        match self {
            Editable::State => Edit::State(issue.state.clone()),
            Editable::Assignee => Edit::Assignee(issue.assignee_id.clone()),
            Editable::Priority => Edit::Priority(issue.priority),
            Editable::Labels => Edit::Labels(issue.label_ids.clone()),
            Editable::Estimate => Edit::Estimate(issue.estimate.map(points)),
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
    /// Every label the issue carries, by id. In a label picker, each entry
    /// holds the one label it stands for.
    Labels(Vec<String>),
    /// Points, or `None` for no estimate.
    Estimate(Option<u32>),
}

impl Edit {
    pub fn field(&self) -> Editable {
        match self {
            Edit::State(_) => Editable::State,
            Edit::Assignee(_) => Editable::Assignee,
            Edit::Priority(_) => Editable::Priority,
            Edit::Labels(_) => Editable::Labels,
            Edit::Estimate(_) => Editable::Estimate,
        }
    }

    pub fn apply_to(&self, issue: &mut Issue) {
        match self {
            Edit::State(state) => issue.state = state.clone(),
            Edit::Assignee(assignee) => issue.assignee_id.clone_from(assignee),
            Edit::Priority(priority) => issue.priority = *priority,
            Edit::Labels(ids) => issue.label_ids.clone_from(ids),
            Edit::Estimate(estimate) => issue.estimate = estimate.map(f64::from),
        }
    }

    /// The `issueUpdate` input that makes this change and no other. Labels
    /// are all sent as added, as if the issue had none; [`Write::update`]
    /// sends only what changed.
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
            Edit::Labels(ids) => IssueUpdate {
                added_label_ids: ids.clone(),
                ..IssueUpdate::default()
            },
            Edit::Estimate(estimate) => IssueUpdate {
                estimate: estimate.map_or(Patch::Clear, Patch::Set),
                ..IssueUpdate::default()
            },
        }
    }
}

/// An estimate as Linear's whole points.
fn points(estimate: f64) -> u32 {
    estimate.round() as u32
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

impl Write {
    /// The `issueUpdate` input for this write. Labels go as the ones added
    /// and the ones removed since [`Write::previous`], so labels a teammate
    /// changed meanwhile stay as they are.
    pub fn update(&self) -> IssueUpdate {
        match (&self.edit, &self.previous) {
            (Edit::Labels(now), Edit::Labels(before)) => IssueUpdate {
                added_label_ids: now
                    .iter()
                    .filter(|id| !before.contains(id))
                    .cloned()
                    .collect(),
                removed_label_ids: before
                    .iter()
                    .filter(|id| !now.contains(id))
                    .cloned()
                    .collect(),
                ..IssueUpdate::default()
            },
            _ => self.edit.update(),
        }
    }
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
            Editable::Labels => "Labels",
            Editable::Estimate => "Estimate",
        };
        format!("{field} of {}", self.identifier)
    }

    /// The labels the issue would carry with the marked entries: the ones it
    /// carries that the list does not offer, in their order, then the marked
    /// ones it carries, then the newly marked ones in list order.
    fn marked_labels(&self, issue_labels: &[String]) -> Vec<String> {
        let id = |edit: &Edit| match edit {
            Edit::Labels(ids) => ids.first().cloned(),
            _ => None,
        };
        let listed: Vec<String> = self.list.values().filter_map(id).collect();
        let marked: Vec<String> = self.list.marked().into_iter().filter_map(id).collect();
        let mut labels: Vec<String> = issue_labels
            .iter()
            .filter(|label| !listed.contains(label) || marked.contains(label))
            .cloned()
            .collect();
        for label in marked {
            if !labels.contains(&label) {
                labels.push(label);
            }
        }
        labels
    }

    /// Fills the list from what `store` holds. Returns whether the store
    /// still lacks entries the picker needs, so they must be loaded.
    fn fill(&mut self, store: &Store) -> bool {
        let Some(issue) = store.issue(&self.issue_id) else {
            return false;
        };
        if self.field == Editable::Labels {
            return self.fill_labels(store, &issue.label_ids);
        }
        let (entries, missing) = match self.field {
            Editable::State => state_entries(store, &self.team_id),
            Editable::Assignee => assignee_entries(store, &self.team_id),
            Editable::Priority => (priority_entries(), false),
            Editable::Labels => (Vec::new(), false),
            Editable::Estimate => (estimate_entries(store, &self.team_id, issue), false),
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

    /// Fills a label picker, keeping what the viewer marked among the
    /// entries it already listed. Labels new to the list are marked when the
    /// issue carries them.
    fn fill_labels(&mut self, store: &Store, issue_labels: &[String]) -> bool {
        let wanted = self.marked_labels(issue_labels);
        let (entries, missing) = label_entries(store, &self.team_id);
        let marked: Vec<usize> = entries
            .iter()
            .enumerate()
            .filter(|(_, entry)| {
                matches!(&entry.value, Edit::Labels(ids) if ids.iter().all(|id| wanted.contains(id)))
            })
            .map(|(index, _)| index)
            .collect();
        let selected = self.list.selected().cloned();
        self.list.set_entries(entries, None);
        self.list.set_marked(marked);
        if let Some(selected) = selected {
            self.list.select_value(&selected);
        }
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

/// The labels an issue of the team can carry, by name: the team's own and
/// the workspace's. Until they are loaded, the ones the store has.
fn label_entries(store: &Store, team_id: &str) -> (Vec<Entry<Edit>>, bool) {
    let (mut labels, missing) = match store.team_labels(team_id) {
        Some(labels) => (labels, false),
        None => (
            store
                .labels()
                .filter(|label| label.team_id.as_deref().is_none_or(|id| id == team_id))
                .collect(),
            true,
        ),
    };
    labels.sort_by_key(|label| label.name.to_lowercase());
    let entries = labels
        .into_iter()
        .map(|label| Entry {
            label: label.name.clone(),
            value: Edit::Labels(vec![label.id.clone()]),
        })
        .collect();
    (entries, missing)
}

/// The points of a team's estimation scale with what the picker calls
/// them, smallest first. Linear stores T-shirt sizes as points.
pub fn estimate_scale(estimation: Estimation) -> &'static [(u32, &'static str)] {
    match estimation {
        Estimation::NotUsed => &[],
        Estimation::Exponential => &[
            (1, "1 point"),
            (2, "2 points"),
            (4, "4 points"),
            (8, "8 points"),
            (16, "16 points"),
        ],
        Estimation::Fibonacci => &[
            (1, "1 point"),
            (2, "2 points"),
            (3, "3 points"),
            (5, "5 points"),
            (8, "8 points"),
        ],
        Estimation::Linear => &[
            (1, "1 point"),
            (2, "2 points"),
            (3, "3 points"),
            (4, "4 points"),
            (5, "5 points"),
        ],
        Estimation::TShirt => &[(1, "XS"), (2, "S"), (3, "M"), (5, "L"), (8, "XL")],
    }
}

/// The team's scale, the issue's estimate in its place when the scale does
/// not have it (a zero, or a point of the extended scale), then "No
/// estimate".
fn estimate_entries(store: &Store, team_id: &str, issue: &Issue) -> Vec<Entry<Edit>> {
    let estimation = store
        .team(team_id)
        .map_or(Estimation::NotUsed, |team| team.estimation);
    let mut scale: Vec<(u32, String)> = estimate_scale(estimation)
        .iter()
        .map(|&(points, name)| (points, name.to_owned()))
        .collect();
    if let Some(current) = issue.estimate.map(points)
        && !scale.iter().any(|&(points, _)| points == current)
    {
        let name = match (estimation, current) {
            (Estimation::TShirt, 13) => "XXL".to_owned(),
            (Estimation::TShirt, 21) => "XXXL".to_owned(),
            (_, 1) => "1 point".to_owned(),
            (_, points) => format!("{points} points"),
        };
        scale.push((current, name));
        scale.sort_by_key(|&(points, _)| points);
    }
    scale
        .into_iter()
        .map(|(points, name)| Entry {
            label: name,
            value: Edit::Estimate(Some(points)),
        })
        .chain([Entry {
            label: "No estimate".to_owned(),
            value: Edit::Estimate(None),
        }])
        .collect()
}

/// Whether the team of the focused issue uses estimates. A team the store
/// does not know is taken to use them.
pub fn focused_team_estimates(state: &State) -> bool {
    state.focused_issue().is_some_and(|issue| {
        state
            .store
            .team(&issue.team_id)
            .is_none_or(|team| team.estimation != Estimation::NotUsed)
    })
}

/// The team's workflow states by state type, as work moves through them,
/// then by their order in the team. Missing until they are loaded.
pub(super) fn state_entries(store: &Store, team_id: &str) -> (Vec<Entry<Edit>>, bool) {
    let Some(states) = store.workflow_states(team_id) else {
        return (Vec::new(), true);
    };
    let mut states: Vec<WorkflowState> = states.into_iter().cloned().collect();
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
pub(super) fn assignee_entries(store: &Store, team_id: &str) -> (Vec<Entry<Edit>>, bool) {
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

pub(super) fn priority_entries() -> Vec<Entry<Edit>> {
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
/// lists if the store does not have it yet, or for labels, always.
pub(super) fn open(state: &mut State, field: Editable) -> Update {
    if field == Editable::Estimate && !focused_team_estimates(state) {
        return Update::default();
    }
    let Some(issue) = state.focused_issue() else {
        return Update::default();
    };
    if super::is_local(&issue.id) {
        state.notice = Some(format!(
            "{} is not created yet; change it once Linear confirms it",
            issue.identifier
        ));
        return Update::redraw();
    }
    let list = match field {
        Editable::Labels => Picker::multi(Vec::new(), []),
        _ => Picker::new(Vec::new(), None),
    };
    let mut picker = EditPicker {
        issue_id: issue.id.clone(),
        identifier: issue.identifier.clone(),
        team_id: issue.team_id.clone(),
        field,
        list,
    };
    let missing = picker.fill(&state.store);
    let team_id = picker.team_id.clone();
    state.modals.push(Modal::Picker(picker));
    // Labels are asked for on every open, even when stored: nothing else
    // refreshes them, and the snapshot keeps them across launches.
    let effects = match (missing, field) {
        (true, Editable::State) => vec![Effect::LoadWorkflowStates { team_id }],
        (true, Editable::Assignee) => vec![Effect::LoadTeamMembers { team_id }],
        (_, Editable::Labels) => vec![Effect::LoadTeamLabels { team_id }],
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

/// Marks or unmarks the selected entry of the open multi-select picker, or
/// the `nth` visible one when given.
pub(super) fn toggle(state: &mut State, nth: Option<usize>) -> Update {
    match state.modals.top_mut() {
        Some(Modal::Picker(picker)) => Update {
            redraw: match nth {
                Some(n) => picker.list.toggle_nth(n),
                None => picker.list.toggle_selected(),
            },
            effects: Vec::new(),
        },
        _ => Update::default(),
    }
}

/// Closes the open picker on the selected entry, or the `nth` visible one
/// when given, and makes that edit. A number in a multi-select picker
/// toggles that entry instead, and accepting it makes the marked labels the
/// issue's.
pub(super) fn choose(state: &mut State, nth: Option<usize>) -> Update {
    let Some(Modal::Picker(picker)) = state.modals.top() else {
        return Update::default();
    };
    if picker.list.is_multi() {
        if nth.is_some() {
            return toggle(state, nth);
        }
        let issue_id = picker.issue_id.clone();
        let Some(issue) = state.store.issue(&issue_id) else {
            state.modals.pop();
            return Update::redraw();
        };
        let labels = picker.marked_labels(&issue.label_ids);
        state.modals.pop();
        return write(state, &issue_id, Edit::Labels(labels));
    }
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
        Editable::Labels => "labels",
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
