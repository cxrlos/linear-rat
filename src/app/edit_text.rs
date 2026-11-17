//! Editing an existing issue's title and description together in
//! `$EDITOR`: the title on the first line, a blank line, then the
//! description as Markdown. A save shows at once and goes to Linear as one
//! `issueUpdate` carrying only what changed, once the runtime has checked
//! that nobody changed the issue while the editor was open.

use jiff::Timestamp;

use super::{Edit, Editable, Effect, State, Update, Write, Writing, is_local};

/// An issue's title and description as the editor opened on them, and when
/// the issue was last updated then.
#[derive(Debug, Clone, PartialEq)]
pub struct TextOpened {
    pub issue_id: String,
    pub identifier: String,
    /// The issue's `updatedAt` when the editor opened. A save refuses to go
    /// out if Linear's has moved since.
    pub seen: Timestamp,
    pub title: String,
    /// Tidied as [`tidy`] does, so it compares with what comes back.
    pub description: String,
}

/// The buffer the editor opens on.
fn buffer(title: &str, description: &str) -> String {
    format!("{title}\n\n{description}")
}

/// A description as lrat compares it: without the blank lines before it or
/// the whitespace after it.
pub(super) fn tidy(description: &str) -> String {
    description
        .trim_start_matches(['\r', '\n'])
        .trim_end()
        .to_owned()
}

/// Opens the editor on the focused issue's title and description.
pub(super) fn open(state: &mut State) -> Update {
    let Some(issue) = state.focused_issue() else {
        return Update::default();
    };
    if is_local(&issue.id) {
        state.notice = Some(format!(
            "{} is not created yet; edit it once Linear confirms it",
            issue.identifier
        ));
        return Update::redraw();
    }
    // An lrat write landing moves `updatedAt`, so the save would be refused
    // as if someone else had changed the issue.
    if state.writes_in_flight.contains_key(&issue.id) {
        state.notice = Some(format!(
            "{} is still being saved; edit it once Linear answers",
            issue.identifier
        ));
        return Update::redraw();
    }
    let opened = TextOpened {
        issue_id: issue.id.clone(),
        identifier: issue.identifier.clone(),
        seen: issue.updated_at,
        title: issue.title.clone(),
        description: tidy(issue.description.as_deref().unwrap_or("")),
    };
    let text = buffer(&opened.title, &opened.description);
    Update {
        redraw: false,
        effects: vec![Effect::OpenEditor {
            writing: Writing::Text(opened),
            text,
        }],
    }
}

/// Takes in the buffer the viewer saved for `opened`. An empty buffer, an
/// empty title or no change sends nothing and says why; anything else shows
/// at once and goes to Linear once its `updatedAt` is checked.
pub(super) fn closed(state: &mut State, opened: TextOpened, text: String) -> Update {
    let identifier = &opened.identifier;
    let (first, rest) = text.split_once('\n').unwrap_or((&text, ""));
    let title = first.trim();
    let why = if text.trim().is_empty() {
        Some("the buffer was empty")
    } else if title.is_empty() {
        Some("the first line, the title, was empty")
    } else if title == opened.title && tidy(rest) == opened.description {
        Some("nothing changed")
    } else {
        None
    };
    if let Some(why) = why {
        state.notice = Some(format!("{identifier} was not changed: {why}"));
        return Update::redraw();
    }
    let Some(issue) = state.store.issue(&opened.issue_id) else {
        state.notice = Some(format!(
            "{identifier} is no longer loaded, so it was not changed"
        ));
        return Update::redraw();
    };
    let edit = Edit::Text {
        title: title.to_owned(),
        description: tidy(rest),
    };
    let previous = Editable::Text.current(issue);
    state.store.apply_edit(&opened.issue_id, &edit);
    state.row_errors.remove(&opened.issue_id);
    super::edit::sent(state, &opened.issue_id);
    state.rebuild_my_issues();
    Update {
        redraw: true,
        effects: vec![Effect::UpdateIssueText {
            write: Write {
                issue_id: opened.issue_id,
                identifier: opened.identifier,
                edit,
                previous,
            },
            seen: opened.seen,
            text,
        }],
    }
}
