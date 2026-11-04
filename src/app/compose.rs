//! Long text written in the viewer's `$EDITOR`: a comment on an issue, or
//! the description of a new one. The runtime suspends the screen for the
//! editor and hands its buffer back; an empty buffer cancels, anything else
//! goes to Linear as one mutation.

use std::collections::BTreeMap;

use super::{Effect, State, Update};
use crate::linear::model::{Issue, Priority, WorkflowState};
use crate::linear::{IssueCreate, PostedComment};
use crate::ui;

/// What the editor is writing.
#[derive(Debug, Clone, PartialEq)]
pub enum Writing {
    /// A comment on the issue with this id.
    Comment {
        issue_id: String,
        identifier: String,
    },
    /// The description of a new issue, whose other fields the form chose.
    Description(IssueDraft),
}

/// A new issue as the create form left it, waiting for its description.
#[derive(Debug, Clone, PartialEq)]
pub struct IssueDraft {
    pub team_id: String,
    /// The team's key, as `ENG`.
    pub team_key: String,
    pub state: WorkflowState,
    pub assignee_id: Option<String>,
    pub priority: Priority,
    pub title: String,
}

/// A comment on its way to Linear.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewComment {
    pub issue_id: String,
    pub identifier: String,
    pub body: String,
}

/// A new issue on its way to Linear, and the id of the stand-in the screens
/// show until Linear answers.
#[derive(Debug, Clone, PartialEq)]
pub struct NewIssue {
    pub local_id: String,
    pub input: IssueCreate,
}

/// What the viewer wrote that Linear did not take, so the next editor for
/// the same thing starts from it, and how many stand-ins there have been.
#[derive(Debug, Default)]
pub struct Drafts {
    comments: BTreeMap<String, String>,
    description: Option<String>,
    stand_ins: u64,
}

/// Stand-in issues carry ids with this prefix. Linear's ids are UUIDs, so
/// they never clash.
const LOCAL_PREFIX: &str = "local-";

/// Whether `issue_id` is a stand-in for an issue Linear has not confirmed.
pub fn is_local(issue_id: &str) -> bool {
    issue_id.starts_with(LOCAL_PREFIX)
}

/// Opens the editor for a comment on the focused issue, starting from what
/// was written for it before if Linear did not take that.
pub(super) fn comment(state: &mut State) -> Update {
    let Some(issue) = state.focused_issue() else {
        return Update::default();
    };
    if is_local(&issue.id) {
        state.notice = Some(format!(
            "{} is not created yet; comment once Linear confirms it",
            issue.identifier
        ));
        return Update::redraw();
    }
    let writing = Writing::Comment {
        issue_id: issue.id.clone(),
        identifier: issue.identifier.clone(),
    };
    let text = state
        .drafts
        .comments
        .get(&issue.id)
        .cloned()
        .unwrap_or_default();
    Update {
        redraw: false,
        effects: vec![Effect::OpenEditor { writing, text }],
    }
}

/// Opens the editor for the description of `draft`.
pub(super) fn describe(state: &State, draft: IssueDraft) -> Effect {
    Effect::OpenEditor {
        writing: Writing::Description(draft),
        text: state.drafts.description.clone().unwrap_or_default(),
    }
}

/// Takes in the editor's buffer for `writing`, or why the editor failed.
pub(super) fn closed(
    state: &mut State,
    writing: Writing,
    result: Result<String, String>,
) -> Update {
    let text = match result {
        Ok(text) => text,
        Err(error) => {
            state.notice = Some(format!("Could not run the editor: {error}"));
            return Update::redraw();
        }
    };
    let text = text.trim();
    let effect = match writing {
        Writing::Comment {
            issue_id,
            identifier,
        } => {
            if text.is_empty() {
                state.drafts.comments.remove(&issue_id);
                state.notice = Some(format!("No comment added to {identifier}: it was empty"));
                return Update::redraw();
            }
            state.notice = Some(format!("Adding the comment to {identifier}…"));
            Effect::CreateComment(NewComment {
                issue_id,
                identifier,
                body: text.to_owned(),
            })
        }
        Writing::Description(draft) => {
            if text.is_empty() {
                state.drafts.description = None;
                state.notice = Some("No issue created: the description was empty".to_owned());
                return Update::redraw();
            }
            Effect::CreateIssue(show_stand_in(state, draft, text))
        }
    };
    Update {
        redraw: true,
        effects: vec![effect],
    }
}

/// Puts a stand-in for `draft` in the store and on the screens, and returns
/// the request that creates the real one.
fn show_stand_in(state: &mut State, draft: IssueDraft, description: &str) -> NewIssue {
    state.drafts.stand_ins += 1;
    let local_id = format!("{LOCAL_PREFIX}{}", state.drafts.stand_ins);
    let input = IssueCreate {
        team_id: draft.team_id,
        state_id: draft.state.id.clone(),
        assignee_id: draft.assignee_id,
        priority: draft.priority,
        title: draft.title,
        description: description.to_owned(),
    };
    let now = state.store.as_of().unwrap_or_default();
    let stand_in = Issue {
        id: local_id.clone(),
        identifier: format!("{}-new", draft.team_key),
        title: input.title.clone(),
        description: Some(input.description.clone()),
        url: String::new(),
        branch_name: String::new(),
        team_id: input.team_id.clone(),
        state: draft.state,
        priority: input.priority,
        estimate: None,
        assignee_id: input.assignee_id.clone(),
        creator_id: state.store.viewer().map(|viewer| viewer.id.clone()),
        cycle_id: None,
        project_id: None,
        label_ids: Vec::new(),
        created_at: now,
        updated_at: now,
        started_at: None,
        completed_at: None,
        canceled_at: None,
        triaged_at: None,
        state_history: Vec::new(),
    };
    state.notice = Some(format!("Creating {}…", input.title));
    state.team_issues.show_created(&stand_in);
    state.store.upsert_issue(stand_in);
    state.rebuild_my_issues();
    focus_in_my_issues(state, &local_id);
    NewIssue { local_id, input }
}

/// Takes in Linear's answer to `comment`: the stored comment joins the
/// issue's comments, or the error shows and the text is kept for the next
/// comment on the issue.
pub(super) fn commented(
    state: &mut State,
    comment: &NewComment,
    result: Result<PostedComment, String>,
) -> Update {
    match result {
        Ok(posted) => {
            state.drafts.comments.remove(&comment.issue_id);
            state.store.add_comment(posted);
            state.notice = Some(format!("Commented on {}", comment.identifier));
        }
        Err(error) => {
            state
                .drafts
                .comments
                .insert(comment.issue_id.clone(), comment.body.clone());
            state.notice = Some(format!(
                "Could not comment on {}: {error}. The next comment on it starts from \
                 what you wrote",
                comment.identifier
            ));
        }
    }
    Update::redraw()
}

/// Takes in Linear's answer to `issue`: its copy replaces the stand-in, or
/// the stand-in goes, the error shows and the description is kept for the
/// next issue.
pub(super) fn created(
    state: &mut State,
    issue: &NewIssue,
    result: Result<Box<Issue>, String>,
) -> Update {
    let focused = state
        .my_issues
        .focused_issue()
        .is_some_and(|focused| focused.id == issue.local_id);
    state.store.remove_issue(&issue.local_id);
    match result {
        Ok(created) => {
            let id = created.id.clone();
            state.drafts.description = None;
            state.notice = Some(format!("Created {}", created.identifier));
            state
                .team_issues
                .replace_created(&issue.local_id, &created.id);
            state.store.upsert_issue(*created);
            state.rebuild_my_issues();
            if focused {
                focus_in_my_issues(state, &id);
            }
        }
        Err(error) => {
            state.drafts.description = Some(issue.input.description.clone());
            state.notice = Some(format!(
                "Could not create {}: {error}. The next new issue's description starts \
                 from what you wrote",
                issue.input.title
            ));
            state.team_issues.forget(&issue.local_id);
            state.rebuild_my_issues();
        }
    }
    Update::redraw()
}

/// Focuses the issue `id` in My Issues, if it lists it, and scrolls to it.
fn focus_in_my_issues(state: &mut State, id: &str) {
    if state.my_issues.focus_id(id) {
        let rows = ui::list_rows(state, state.viewport);
        state.my_issues.scroll_into_view(rows);
    }
}
