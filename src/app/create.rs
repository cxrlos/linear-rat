//! The create form: a picker each for the team, workflow state, assignee and
//! priority of a new issue, then a text field for its title. The description
//! comes after, from `$EDITOR`.

use super::compose::{self, IssueDraft};
use super::edit::{self, Edit, Editable};
use super::picker::{Entry, Note, Picker};
use super::{Effect, Modal, Screen, State, Update};
use crate::linear::model::{Priority, StateType, Team, WorkflowState};

/// The form, with what has been chosen so far and the field being chosen.
#[derive(Debug, Clone, PartialEq)]
pub struct CreateForm {
    pub team: Option<Team>,
    pub state: Option<WorkflowState>,
    /// `Some(None)` once "Unassigned" is chosen.
    pub assignee: Option<Option<String>>,
    pub priority: Option<Priority>,
    pub title: String,
    pub step: Step,
}

/// The field the form is on.
#[derive(Debug, Clone, PartialEq)]
pub enum Step {
    /// Choosing the team, by id.
    Team(Picker<String>),
    /// Choosing the workflow state, assignee or priority.
    Field(Editable, Picker<Edit>),
    /// Typing the title.
    Title,
}

impl CreateForm {
    /// What the popup's border says, as `New ENG issue: assignee`.
    pub fn heading(&self) -> String {
        let field = match &self.step {
            Step::Team(_) => "team",
            Step::Field(field, _) => field.name(),
            Step::Title => "title",
        };
        match &self.team {
            Some(team) => format!("New {} issue: {field}", team.key),
            None => format!("New issue: {field}"),
        }
    }

    /// The text field typed keys go to: the picker's filter, or the title.
    pub fn text_field(&mut self) -> Option<&mut String> {
        Some(match &mut self.step {
            Step::Team(picker) => picker.query_mut(),
            Step::Field(_, picker) => picker.query_mut(),
            Step::Title => &mut self.title,
        })
    }

    /// How many entries the picker on screen leaves, and which of them is
    /// selected; `None` while typing the title.
    pub fn list(&self) -> Option<(usize, Option<usize>)> {
        match &self.step {
            Step::Team(picker) => Some((picker.visible().len(), picker.selected_position())),
            Step::Field(_, picker) => Some((picker.visible().len(), picker.selected_position())),
            Step::Title => None,
        }
    }

    /// The team chosen, as the field pickers fill from.
    fn team_id(&self) -> Option<&str> {
        self.team.as_ref().map(|team| team.id.as_str())
    }

    /// Refills the field picker from `store`. Returns whether the store still
    /// lacks what it lists.
    fn fill(&mut self, store: &super::Store) -> bool {
        let Some(team_id) = self.team_id().map(str::to_owned) else {
            return false;
        };
        let Step::Field(field, picker) = &mut self.step else {
            return false;
        };
        let (entries, missing) = match field {
            Editable::State => edit::state_entries(store, &team_id),
            Editable::Assignee => edit::assignee_entries(store, &team_id),
            Editable::Priority => (edit::priority_entries(), false),
            // The form does not set labels or an estimate.
            Editable::Labels | Editable::Estimate => (Vec::new(), false),
        };
        let default = entries
            .iter()
            .position(|entry| match &entry.value {
                Edit::State(state) => state.state_type == StateType::Unstarted,
                Edit::Assignee(_) => true,
                Edit::Priority(priority) => *priority == Priority::None,
                Edit::Labels(_) | Edit::Estimate(_) => false,
            })
            .unwrap_or(0);
        picker.set_entries(entries, None);
        picker.select(default);
        if missing {
            if picker.note().is_none() {
                picker.set_note(Some(Note::Loading));
            }
        } else {
            picker.set_note(None);
        }
        missing
    }
}

/// Opens the form on the team picker: the viewer's teams, or the teams
/// loaded so far until those are known, with the team in view selected.
pub(super) fn open(state: &mut State) -> Update {
    let mut teams: Vec<Team> = state.store.viewer_teams().cloned().collect();
    if teams.is_empty() {
        teams = state.store.teams().cloned().collect();
    }
    if teams.is_empty() {
        state.notice = Some("No team to create an issue in has loaded yet".to_owned());
        return Update::redraw();
    }
    let in_view = match state.screen {
        Screen::TeamIssues => state.team_issues.query().map(|query| query.team_id.clone()),
        _ => state.focused_issue().map(|issue| issue.team_id.clone()),
    };
    let selected = teams
        .iter()
        .position(|team| Some(&team.id) == in_view.as_ref())
        .unwrap_or(0);
    let entries = teams
        .into_iter()
        .map(|team| Entry {
            label: format!("{} {}", team.key, team.name),
            value: team.id,
        })
        .collect();
    let mut picker = Picker::new(entries, None);
    picker.select(selected);
    state.modals.push(Modal::Create(CreateForm {
        team: None,
        state: None,
        assignee: None,
        priority: None,
        title: String::new(),
        step: Step::Team(picker),
    }));
    Update::redraw()
}

/// Moves the open form's selection by `step`.
pub(super) fn select(state: &mut State, step: isize) -> Update {
    let Some(Modal::Create(form)) = state.modals.top_mut() else {
        return Update::default();
    };
    let redraw = match &mut form.step {
        Step::Team(picker) => picker.select_by(step),
        Step::Field(_, picker) => picker.select_by(step),
        Step::Title => false,
    };
    Update {
        redraw,
        effects: Vec::new(),
    }
}

/// Takes the selected entry, or the `nth` visible one when given, and moves
/// to the next field. On the title, closes the form and hands the issue to
/// the editor for its description, or cancels when the title is empty.
pub(super) fn accept(state: &mut State, nth: Option<usize>) -> Update {
    let Some(Modal::Create(form)) = state.modals.top_mut() else {
        return Update::default();
    };
    let mut effects = Vec::new();
    match &form.step {
        Step::Team(picker) => {
            let Some(team_id) = chosen(picker, nth).cloned() else {
                return Update::default();
            };
            form.team = state.store.team(&team_id).cloned();
            form.step = Step::Field(Editable::State, Picker::new(Vec::new(), None));
            if form.fill(&state.store) {
                effects.push(Effect::LoadWorkflowStates { team_id });
            }
        }
        Step::Field(_, picker) => {
            let Some(value) = chosen(picker, nth).cloned() else {
                return Update::default();
            };
            let next = match value {
                Edit::State(workflow_state) => {
                    form.state = Some(workflow_state);
                    Some(Editable::Assignee)
                }
                Edit::Assignee(assignee) => {
                    form.assignee = Some(assignee);
                    Some(Editable::Priority)
                }
                Edit::Priority(priority) => {
                    form.priority = Some(priority);
                    None
                }
                Edit::Labels(_) | Edit::Estimate(_) => return Update::default(),
            };
            match next {
                Some(field) => {
                    form.step = Step::Field(field, Picker::new(Vec::new(), None));
                    if form.fill(&state.store)
                        && field == Editable::Assignee
                        && let Some(team_id) = form.team_id()
                    {
                        effects.push(Effect::LoadTeamMembers {
                            team_id: team_id.to_owned(),
                        });
                    }
                }
                None => form.step = Step::Title,
            }
        }
        Step::Title if nth.is_some() => return Update::default(),
        Step::Title => {
            let form = form.clone();
            state.modals.pop();
            return finish(state, form);
        }
    }
    Update {
        redraw: true,
        effects,
    }
}

fn chosen<T>(picker: &Picker<T>, nth: Option<usize>) -> Option<&T> {
    match nth {
        Some(n) => picker.nth(n),
        None => picker.selected(),
    }
}

/// Hands the finished form to the editor, unless the title is empty.
fn finish(state: &mut State, form: CreateForm) -> Update {
    let title = form.title.trim();
    let (Some(team), Some(workflow_state), Some(assignee_id), Some(priority)) =
        (form.team, form.state, form.assignee, form.priority)
    else {
        return Update::redraw();
    };
    if title.is_empty() {
        state.notice = Some("No issue created: the title was empty".to_owned());
        return Update::redraw();
    }
    let draft = IssueDraft {
        team_id: team.id,
        team_key: team.key,
        state: workflow_state,
        assignee_id,
        priority,
        title: title.to_owned(),
    };
    Update {
        redraw: true,
        effects: vec![compose::describe(state, draft)],
    }
}

/// Refills an open form choosing `field` of team `team_id` once what it
/// lists has loaded, or says in it why it did not.
pub(super) fn loaded(state: &mut State, field: Editable, team_id: &str, failure: Option<&str>) {
    for modal in state.modals.iter_mut() {
        let Modal::Create(form) = modal else {
            continue;
        };
        let on_field = matches!(&form.step, Step::Field(on, _) if *on == field);
        if !on_field || form.team_id() != Some(team_id) {
            continue;
        }
        match failure {
            Some(error) => {
                if let Step::Field(_, picker) = &mut form.step {
                    picker.set_note(Some(Note::Failed(error.to_owned())));
                }
            }
            None => {
                form.fill(&state.store);
            }
        }
    }
}
