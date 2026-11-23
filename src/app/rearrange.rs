//! The group and sort pickers of the issue list on screen, and what
//! choosing in them does.

use super::modal::{Choice, Purpose};
use super::picker::{Entry, Picker};
use super::{
    Arrangement, Effect, Grouping, Load, Modal, Screen, Sort, State, results, team_issues,
};

/// How the issue list on screen is grouped and sorted, `None` on a screen
/// with no list.
pub fn current(state: &State) -> Option<Arrangement> {
    match state.screen {
        Screen::MyIssues if state.load == Load::Loaded => Some(state.my_issues.arrangement()),
        Screen::TeamIssues if state.team_issues.query().is_some() => {
            Some(state.team_issues.arrangement())
        }
        Screen::Results => Some(state.results.arrangement()),
        _ => None,
    }
}

/// Whether the screen shows an issue list that can be grouped and sorted.
pub fn can_rearrange(state: &State) -> bool {
    current(state).is_some()
}

/// What the list on screen calls its own order, the sort of `None`, or
/// `None` when it has none to offer.
pub fn own_order(screen: Screen) -> Option<&'static str> {
    match screen {
        Screen::MyIssues => Some("active cycle, then priority"),
        Screen::Results => Some("Linear's order"),
        Screen::TeamIssues
        | Screen::Analytics
        | Screen::Cycles
        | Screen::Projects
        | Screen::Inbox => None,
    }
}

/// The picker of groupings, the one in force selected.
pub(super) fn group_choice(state: &State) -> Option<Choice> {
    let arrangement = current(state)?;
    let entries: Vec<Entry<Option<String>>> = Grouping::ALL
        .into_iter()
        .map(|grouping| Entry {
            label: grouping.label().to_owned(),
            value: Some(grouping.name().to_owned()),
        })
        .collect();
    let selected = Grouping::ALL
        .iter()
        .position(|grouping| *grouping == arrangement.group);
    Some(Choice {
        purpose: Purpose::Group,
        list: Picker::new(entries, selected),
    })
}

/// The picker of sorts, after the list's own order where it has one, the
/// one in force selected.
pub(super) fn sort_choice(state: &State) -> Option<Choice> {
    let arrangement = current(state)?;
    let mut entries: Vec<Entry<Option<String>>> = own_order(state.screen)
        .map(|label| Entry {
            label: label.to_owned(),
            value: None,
        })
        .into_iter()
        .collect();
    entries.extend(Sort::all().map(|sort| Entry {
        label: sort.label(),
        value: Some(sort.name()),
    }));
    let in_force = arrangement.sort.map(Sort::name);
    let selected = entries.iter().position(|entry| entry.value == in_force);
    Some(Choice {
        purpose: Purpose::Sort,
        list: Picker::new(entries, selected),
    })
}

/// Closes the group or sort picker on top on its selected entry, or the
/// `nth` visible one when given, and applies it to the list on screen.
/// Returns `None`, with the picker left open, when there is no such entry.
pub(super) fn pick(state: &mut State, nth: Option<usize>) -> Option<Vec<Effect>> {
    let Some(Modal::Choice(choice)) = state.modals.top() else {
        return None;
    };
    let purpose = choice.purpose.clone();
    let name = choice.chosen(nth).cloned()?;
    state.modals.pop();
    let mut arrangement = current(state)?;
    match purpose {
        Purpose::Group => {
            arrangement.group = name.as_deref().and_then(Grouping::from_name)?;
        }
        Purpose::Sort => arrangement.sort = name.as_deref().and_then(Sort::from_name),
        Purpose::Team
        | Purpose::AnalyticsTeam
        | Purpose::CyclesTeam
        | Purpose::Filter(_)
        | Purpose::RelationKind
        | Purpose::RemoveRelations { .. }
        | Purpose::Reaction
        | Purpose::Snooze => {
            return None;
        }
    }
    Some(apply(state, arrangement))
}

/// Groups and sorts the list on screen as `arrangement` says, for the rest
/// of the session, keeping the focused issue.
fn apply(state: &mut State, arrangement: Arrangement) -> Vec<Effect> {
    match state.screen {
        Screen::MyIssues => {
            state.my_issues.arrangement = arrangement;
            state.rebuild_lists();
            Vec::new()
        }
        Screen::TeamIssues => team_issues::rearrange(state, arrangement),
        Screen::Results => {
            results::rearrange(state, arrangement);
            Vec::new()
        }
        Screen::Analytics | Screen::Cycles | Screen::Projects | Screen::Inbox => Vec::new(),
    }
}
