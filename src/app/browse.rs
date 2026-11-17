//! The Cycles and Projects screens: a team's recent cycles and the projects
//! of the viewer's teams. Enter lists the focused one's issues in the
//! results list, so every issue action works on them, and Esc comes back
//! here with the focus where it was.

use std::cmp::Reverse;

use super::modal::{Choice, Purpose};
use super::{Action, Arrangement, Effect, Modal, Progress, Screen, State, results, team_issues};
use crate::linear::model::{Cycle, ProjectOverview};
use crate::linear::{ProjectList, SearchResults};

/// The Cycles screen: which team it shows, that team's recent cycles newest
/// first, and where the focus is.
#[derive(Debug, Default)]
pub struct Cycles {
    team_id: Option<String>,
    load: Progress,
    cycles: Vec<Cycle>,
    focused: usize,
    /// The cycle whose issues are on their way, to list once they are in.
    opening: Option<String>,
}

impl Cycles {
    /// The team shown, once one is picked.
    pub fn team_id(&self) -> Option<&str> {
        self.team_id.as_deref()
    }

    /// How far the team's cycles have loaded.
    pub fn progress(&self) -> &Progress {
        &self.load
    }

    /// The cycles, newest first.
    pub fn cycles(&self) -> &[Cycle] {
        &self.cycles
    }

    pub fn focused_index(&self) -> usize {
        self.focused
    }

    pub fn focused(&self) -> Option<&Cycle> {
        self.cycles.get(self.focused)
    }

    /// The cycle whose issues are loading, by id.
    pub fn opening(&self) -> Option<&str> {
        self.opening.as_deref()
    }
}

/// The Projects screen: the projects of the viewer's teams, most recently
/// updated first, and where the focus is.
#[derive(Debug, Default)]
pub struct Projects {
    load: Progress,
    projects: Vec<ProjectOverview>,
    focused: usize,
    /// The project whose issues are on their way, to list once they are in.
    opening: Option<String>,
}

impl Projects {
    /// How far the projects have loaded.
    pub fn progress(&self) -> &Progress {
        &self.load
    }

    pub fn projects(&self) -> &[ProjectOverview] {
        &self.projects
    }

    pub fn focused_index(&self) -> usize {
        self.focused
    }

    pub fn focused(&self) -> Option<&ProjectOverview> {
        self.projects.get(self.focused)
    }

    /// The project whose issues are loading, by id.
    pub fn opening(&self) -> Option<&str> {
        self.opening.as_deref()
    }
}

/// How many of a cycle's issues are completed and how many it has, as of
/// its last day of history. `None` for a cycle that has not started.
pub fn cycle_counts(cycle: &Cycle) -> Option<(u32, u32)> {
    let total = *cycle.issue_count_history.last()?;
    let done = cycle
        .completed_issue_count_history
        .last()
        .copied()
        .unwrap_or(0);
    Some((done, total))
}

/// Asks for what the Cycles screen needs as it shows: the viewer's teams,
/// then the first team's cycles; or the cycles again after they failed.
pub(super) fn open_cycles(state: &mut State) -> Vec<Effect> {
    match (&state.cycles.team_id, &state.cycles.load) {
        (Some(team_id), Progress::Failed(_)) => {
            let team_id = team_id.clone();
            switch_team(state, team_id)
        }
        (Some(_), _) => Vec::new(),
        (None, _) => match first_team(state) {
            Some(team_id) => switch_team(state, team_id),
            None => team_issues::open(state),
        },
    }
}

/// Once the viewer's teams are in, shows the first one's cycles if the
/// Cycles screen is waiting for a team.
pub(super) fn teams_loaded(state: &mut State) -> Vec<Effect> {
    if state.screen != Screen::Cycles || state.cycles.team_id.is_some() {
        return Vec::new();
    }
    match first_team(state) {
        Some(team_id) => switch_team(state, team_id),
        None => Vec::new(),
    }
}

fn first_team(state: &State) -> Option<String> {
    state
        .store
        .viewer_teams()
        .next()
        .map(|team| team.id.clone())
}

/// Shows `team_id`'s cycles, from the top, once they load.
fn switch_team(state: &mut State, team_id: String) -> Vec<Effect> {
    state.cycles = Cycles {
        team_id: Some(team_id.clone()),
        load: Progress::Loading,
        ..Cycles::default()
    };
    vec![Effect::LoadCycles(team_id)]
}

/// The team switcher of the Cycles screen, the team shown selected.
pub(super) fn team_choice(state: &State) -> Option<Choice> {
    let shown = state.cycles.team_id.clone();
    team_issues::team_switcher(state, Purpose::CyclesTeam, shown.as_ref())
}

/// Closes the team switcher on its selected entry, or the `nth` visible one
/// when given, and shows that team. Returns `None`, with the switcher left
/// open, when there is no such entry.
pub(super) fn pick_team(state: &mut State, nth: Option<usize>) -> Option<Vec<Effect>> {
    let Some(Modal::Choice(choice)) = state.modals.top() else {
        return None;
    };
    let team_id = choice.chosen(nth).cloned()?;
    state.modals.pop();
    Some(match team_id {
        Some(team_id) if state.cycles.team_id.as_ref() != Some(&team_id) => {
            switch_team(state, team_id)
        }
        _ => Vec::new(),
    })
}

/// Takes in `team_id`'s cycles if that team is still shown. Returns
/// whether the screen changed.
pub(super) fn cycles_loaded(
    state: &mut State,
    team_id: &str,
    result: Result<Vec<Cycle>, String>,
) -> bool {
    let cycles = &mut state.cycles;
    if cycles.team_id.as_deref() != Some(team_id) {
        return false;
    }
    match result {
        Ok(mut loaded) => {
            loaded.sort_by_key(|cycle| Reverse(cycle.starts_at));
            cycles.cycles = loaded;
            cycles.focused = 0;
            cycles.load = Progress::Done;
        }
        Err(error) => cycles.load = Progress::Failed(error),
    }
    true
}

/// Asks for the projects the first time the Projects screen shows, and
/// again each time it shows after they failed.
pub(super) fn open_projects(state: &mut State) -> Vec<Effect> {
    let projects = &mut state.projects;
    if matches!(projects.load, Progress::Loading | Progress::Done) {
        return Vec::new();
    }
    projects.load = Progress::Loading;
    vec![Effect::LoadProjects]
}

/// Takes in the projects. What Linear could not give becomes the notice,
/// unless one is already showing.
pub(super) fn projects_loaded(state: &mut State, result: Result<ProjectList, String>) {
    match result {
        Ok(list) => {
            if state.notice.is_none() {
                state.notice = super::state::summary(&list.problems);
            }
            state.store.merge_projects(&list);
            state.rebuild_lists();
            let projects = &mut state.projects;
            projects.focused = projects.focused.min(list.projects.len().saturating_sub(1));
            projects.projects = list.projects;
            projects.load = Progress::Done;
        }
        Err(error) => state.projects.load = Progress::Failed(error),
    }
}

/// Moves the focus of the Cycles or Projects screen for a list of `rows`
/// rows. Returns whether it moved.
pub(super) fn perform(state: &mut State, action: Action, rows: usize) -> bool {
    let (focused, len) = match state.screen {
        Screen::Cycles => (&mut state.cycles.focused, state.cycles.cycles.len()),
        Screen::Projects => (&mut state.projects.focused, state.projects.projects.len()),
        _ => return false,
    };
    let last = len.saturating_sub(1);
    let page = (rows / 2).max(1);
    let next = match action {
        Action::MoveDown => focused.saturating_add(1),
        Action::MoveUp => focused.saturating_sub(1),
        Action::MoveToTop => 0,
        Action::MoveToBottom => last,
        Action::HalfPageDown => focused.saturating_add(page),
        Action::HalfPageUp => focused.saturating_sub(page),
        _ => *focused,
    }
    .min(last);
    let moved = next != *focused;
    *focused = next;
    moved
}

/// Asks for the issues of the focused cycle or project, unless they are on
/// their way already.
pub(super) fn list_issues(state: &mut State) -> Vec<Effect> {
    match state.screen {
        Screen::Cycles => {
            let Some(id) = state.cycles.focused().map(|cycle| cycle.id.clone()) else {
                return Vec::new();
            };
            if state.cycles.opening.as_ref() == Some(&id) {
                return Vec::new();
            }
            state.cycles.opening = Some(id.clone());
            vec![Effect::LoadCycleIssues(id)]
        }
        Screen::Projects => {
            let Some(id) = state
                .projects
                .focused()
                .map(|overview| overview.project.id.clone())
            else {
                return Vec::new();
            };
            if state.projects.opening.as_ref() == Some(&id) {
                return Vec::new();
            }
            state.projects.opening = Some(id.clone());
            vec![Effect::LoadProjectIssues(id)]
        }
        _ => Vec::new(),
    }
}

/// Takes in a cycle's issues. They go into the store whatever happened
/// since; they open in the results list, by workflow state, only if the
/// Cycles screen still shows and waits on that cycle.
pub(super) fn cycle_issues_loaded(
    state: &mut State,
    cycle_id: &str,
    result: Result<SearchResults, String>,
) {
    let waiting = state.cycles.opening.as_deref() == Some(cycle_id);
    if waiting {
        state.cycles.opening = None;
    }
    let title = state
        .cycles
        .cycles
        .iter()
        .find(|cycle| cycle.id == cycle_id)
        .map(cycle_title);
    let shown = waiting && state.screen == Screen::Cycles;
    take_in(state, result, waiting, |state, ids| {
        if let (true, Some(title)) = (shown, title) {
            results::show_arranged(state, title, ids, Arrangement::CYCLE_ISSUES);
        }
    });
}

/// Takes in a project's issues, as [`cycle_issues_loaded`] does, listing
/// them by project milestone.
pub(super) fn project_issues_loaded(
    state: &mut State,
    project_id: &str,
    result: Result<SearchResults, String>,
) {
    let waiting = state.projects.opening.as_deref() == Some(project_id);
    if waiting {
        state.projects.opening = None;
    }
    let title = state
        .projects
        .projects
        .iter()
        .find(|overview| overview.project.id == project_id)
        .map(|overview| overview.project.name.clone());
    let shown = waiting && state.screen == Screen::Projects;
    take_in(state, result, waiting, |state, ids| {
        if let (true, Some(title)) = (shown, title) {
            results::show_arranged(state, title, ids, Arrangement::PROJECT_ISSUES);
        }
    });
}

/// Merges `result`'s issues into the store and hands their ids to `show`.
/// A failure becomes the notice when the screen was waiting for it.
fn take_in(
    state: &mut State,
    result: Result<SearchResults, String>,
    waiting: bool,
    show: impl FnOnce(&mut State, Vec<String>),
) {
    match result {
        Ok(found) => {
            if state.notice.is_none() {
                state.notice = super::state::summary(&found.problems);
            }
            state.store.merge_search(&found);
            state.rebuild_lists();
            show(
                state,
                found.issues.into_iter().map(|issue| issue.id).collect(),
            );
        }
        Err(error) if waiting => state.notice = Some(error),
        Err(_) => {}
    }
}

/// What the Cycles screen and the results list call a cycle: its name, or
/// `Cycle 12`.
pub fn cycle_title(cycle: &Cycle) -> String {
    cycle
        .name
        .clone()
        .unwrap_or_else(|| format!("Cycle {}", cycle.number))
}
