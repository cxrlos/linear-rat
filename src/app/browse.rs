//! The Cycles and Projects screens: a team's recent cycles and the projects
//! of the viewer's teams. Enter lists the focused one's issues in the
//! results list, so every issue action works on them, and Esc comes back
//! here with the focus where it was. `r` loads what the screen shows again,
//! and showing a screen whose rows are older than the active poll interval
//! reloads it in the background, keeping the rows until the new ones come.

use std::cmp::Reverse;

use jiff::{SignedDuration, Timestamp};

use super::modal::{Choice, Purpose};
use super::{
    Action, Arrangement, Effect, Failure, Listing, Modal, Progress, Screen, State, results, sync,
    team_issues,
};
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
    /// When the cycles shown finished loading, by the latest tick.
    loaded_at: Option<Timestamp>,
    /// The cycles are loading again behind the ones shown.
    reloading: bool,
    /// The cycle whose issue list in Results is loading again.
    relisting: Option<String>,
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
    /// When the projects shown finished loading, by the latest tick.
    loaded_at: Option<Timestamp>,
    /// The projects are loading again behind the ones shown.
    reloading: bool,
    /// The project whose issue list in Results is loading again.
    relisting: Option<String>,
    /// What Linear could not give in the last load, so a reload raises
    /// only problems the viewer has not seen.
    problems: Vec<String>,
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
/// then the first team's cycles; the cycles again after they failed; or,
/// in the background, once they are stale.
pub(super) fn open_cycles(state: &mut State) -> Vec<Effect> {
    match (&state.cycles.team_id, &state.cycles.load) {
        (Some(team_id), Progress::Failed(_)) => {
            let team_id = team_id.clone();
            switch_team(state, team_id)
        }
        (Some(_), Progress::Done) if is_stale(state, state.cycles.loaded_at) => {
            reload_cycles(state)
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

/// Takes in `team_id`'s cycles if that team is still shown, the focus
/// kept on the same cycle. A reload that failed keeps the cycles shown.
/// Returns whether the screen changed.
pub(super) fn cycles_loaded(
    state: &mut State,
    team_id: &str,
    result: Result<Vec<Cycle>, Failure>,
) -> bool {
    let now = state.sync_status.now();
    let cycles = &mut state.cycles;
    if cycles.team_id.as_deref() != Some(team_id) {
        return false;
    }
    let reloading = std::mem::take(&mut cycles.reloading);
    match result {
        Ok(mut loaded) => {
            loaded.sort_by_key(|cycle| Reverse(cycle.starts_at));
            let focused = cycles.focused().map(|cycle| cycle.id.clone());
            cycles.focused = focused
                .and_then(|id| loaded.iter().position(|cycle| cycle.id == id))
                .unwrap_or(0);
            cycles.cycles = loaded;
            cycles.load = Progress::Done;
            cycles.loaded_at = now;
        }
        Err(failure) if reloading => sync::reload_failed(state, failure),
        Err(failure) => {
            cycles.load = Progress::Failed(failure.message.clone());
            state.record(failure);
        }
    }
    true
}

/// Asks for the projects the first time the Projects screen shows, again
/// each time it shows after they failed, and in the background once they
/// are stale.
pub(super) fn open_projects(state: &mut State) -> Vec<Effect> {
    match state.projects.load {
        Progress::Loading => Vec::new(),
        Progress::Done if is_stale(state, state.projects.loaded_at) => reload_projects(state),
        Progress::Done => Vec::new(),
        Progress::NotAsked | Progress::Failed(_) => {
            state.projects.load = Progress::Loading;
            vec![Effect::LoadProjects]
        }
    }
}

/// Takes in the projects, the focus kept on the same project. What Linear
/// could not give becomes the notice, unless one is already showing or a
/// reload brought the same problems as the last load, which the viewer
/// has seen. A reload that failed keeps the projects shown.
pub(super) fn projects_loaded(state: &mut State, result: Result<ProjectList, Failure>) {
    let reloading = std::mem::take(&mut state.projects.reloading);
    match result {
        Ok(list) => {
            let seen = reloading && list.problems == state.projects.problems;
            if state.notice.is_none() && !seen {
                state.notice = super::state::summary(&list.problems);
            }
            state.projects.problems.clone_from(&list.problems);
            state.store.merge_projects(&list);
            state.rebuild_lists();
            let now = state.sync_status.now();
            let projects = &mut state.projects;
            let focused = projects
                .focused()
                .map(|overview| overview.project.id.clone());
            projects.focused = focused
                .and_then(|id| {
                    list.projects
                        .iter()
                        .position(|overview| overview.project.id == id)
                })
                .unwrap_or(0);
            projects.projects = list.projects;
            projects.load = Progress::Done;
            projects.loaded_at = now;
        }
        Err(failure) if reloading => sync::reload_failed(state, failure),
        Err(failure) => {
            state.projects.load = Progress::Failed(failure.message.clone());
            state.record(failure);
        }
    }
}

/// Whether rows that finished loading at `loaded_at` are older than the
/// active poll interval. Never before the first tick.
fn is_stale(state: &State, loaded_at: Option<Timestamp>) -> bool {
    let Some(now) = state.sync_status.now() else {
        return false;
    };
    let Ok(interval) = SignedDuration::try_from(state.sync.active_poll) else {
        return false;
    };
    loaded_at.is_none_or(|loaded_at| now.duration_since(loaded_at) >= interval)
}

/// The screen showing again, as Esc from Results back to it: reloads it in
/// the background when it is stale.
pub(super) fn shown(state: &mut State) -> Vec<Effect> {
    match state.screen {
        Screen::Cycles if state.cycles.load == Progress::Done => open_cycles(state),
        Screen::Projects if state.projects.load == Progress::Done => open_projects(state),
        _ => Vec::new(),
    }
}

fn reload_cycles(state: &mut State) -> Vec<Effect> {
    let cycles = &mut state.cycles;
    match &cycles.team_id {
        Some(team_id) if !cycles.reloading => {
            cycles.reloading = true;
            vec![Effect::LoadCycles(team_id.clone())]
        }
        _ => Vec::new(),
    }
}

fn reload_projects(state: &mut State) -> Vec<Effect> {
    if state.projects.reloading {
        return Vec::new();
    }
    state.projects.reloading = true;
    vec![Effect::LoadProjects]
}

/// Whether `r` can load what the screen shows again now, or `None` when
/// the screen is not Cycles, Projects or a cycle's or project's issues.
pub(super) fn can_reload(state: &State) -> Option<bool> {
    match state.screen {
        Screen::Cycles => Some(
            state.cycles.team_id.is_some()
                && state.cycles.load != Progress::Loading
                && !state.cycles.reloading,
        ),
        Screen::Projects => {
            Some(state.projects.load != Progress::Loading && !state.projects.reloading)
        }
        Screen::Results => match state.results.listing()? {
            Listing::Cycle(_) => Some(state.cycles.relisting.is_none()),
            Listing::Project(_) => Some(state.projects.relisting.is_none()),
        },
        _ => None,
    }
}

/// `r` on Cycles, Projects or a cycle's or project's issues: loads what
/// the screen shows again, keeping it on screen until the new rows come.
/// `None` on any other screen. A screen whose load failed loads from the
/// start.
pub(super) fn reload(state: &mut State) -> Option<Vec<Effect>> {
    if !can_reload(state)? {
        return Some(Vec::new());
    }
    Some(match state.screen {
        Screen::Cycles if state.cycles.load == Progress::Done => reload_cycles(state),
        Screen::Cycles => open_cycles(state),
        Screen::Projects if state.projects.load == Progress::Done => reload_projects(state),
        Screen::Projects => open_projects(state),
        _ => match state.results.listing().cloned() {
            Some(Listing::Cycle(id)) => {
                state.cycles.relisting = Some(id.clone());
                vec![Effect::LoadCycleIssues(id)]
            }
            Some(Listing::Project(id)) => {
                state.projects.relisting = Some(id.clone());
                vec![Effect::LoadProjectIssues(id)]
            }
            None => Vec::new(),
        },
    })
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
    result: Result<SearchResults, Failure>,
) {
    if state.cycles.relisting.as_deref() == Some(cycle_id) {
        state.cycles.relisting = None;
        relisted(state, Listing::Cycle(cycle_id.to_owned()), result);
        return;
    }
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
            state
                .results
                .set_listing(Listing::Cycle(cycle_id.to_owned()));
        }
    });
}

/// Takes in a project's issues, as [`cycle_issues_loaded`] does, listing
/// them by project milestone.
pub(super) fn project_issues_loaded(
    state: &mut State,
    project_id: &str,
    result: Result<SearchResults, Failure>,
) {
    if state.projects.relisting.as_deref() == Some(project_id) {
        state.projects.relisting = None;
        relisted(state, Listing::Project(project_id.to_owned()), result);
        return;
    }
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
            state
                .results
                .set_listing(Listing::Project(project_id.to_owned()));
        }
    });
}

/// Takes in the issues of `listing` loaded again by `r`. They go into the
/// store, and replace the results, the focus kept on the same issue, if
/// the results are still that list. A failure keeps the list as it was.
fn relisted(state: &mut State, listing: Listing, result: Result<SearchResults, Failure>) {
    match result {
        Ok(found) => {
            take_in(state, Ok(found), false, |state, ids| {
                if state.results.listing() == Some(&listing) {
                    state.results.replace_ids(ids);
                    results::rebuild(state);
                }
            });
        }
        Err(failure) => sync::reload_failed(state, failure),
    }
}

/// Merges `result`'s issues into the store and hands their ids to `show`.
/// A failure goes to the error log, and becomes the notice too when the
/// screen was waiting for it.
fn take_in(
    state: &mut State,
    result: Result<SearchResults, Failure>,
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
        Err(failure) if waiting => state.fail(failure),
        Err(failure) => state.record(failure),
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
