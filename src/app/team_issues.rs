//! Team Issues: one of the viewer's teams, its issues a page at a time,
//! narrowed by a filter, with the focused issue and its detail.

use super::arrange::linear_order;
use super::modal::{Choice, Purpose};
use super::picker::{Entry, Picker};
use super::{
    Action, Arrangement, Effect, Failure, IssueList, Modal, Names, Pane, State, Store, arrange,
};
use crate::linear::model::{Issue, Team};
use crate::linear::{TeamFilter, TeamIssuesPage, TeamOptions, TeamQuery};
use crate::ui;

/// A field Team Issues can be filtered by.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilterField {
    State,
    Assignee,
    Label,
    Cycle,
    Project,
}

impl FilterField {
    pub const ALL: [FilterField; 5] = [
        FilterField::State,
        FilterField::Assignee,
        FilterField::Label,
        FilterField::Cycle,
        FilterField::Project,
    ];

    pub fn name(self) -> &'static str {
        match self {
            FilterField::State => "State",
            FilterField::Assignee => "Assignee",
            FilterField::Label => "Label",
            FilterField::Cycle => "Cycle",
            FilterField::Project => "Project",
        }
    }

    /// The id `filter` holds for this field.
    pub fn value(self, filter: &TeamFilter) -> Option<&String> {
        match self {
            FilterField::State => filter.state.as_ref(),
            FilterField::Assignee => filter.assignee.as_ref(),
            FilterField::Label => filter.label.as_ref(),
            FilterField::Cycle => filter.cycle.as_ref(),
            FilterField::Project => filter.project.as_ref(),
        }
    }

    fn value_mut(self, filter: &mut TeamFilter) -> &mut Option<String> {
        match self {
            FilterField::State => &mut filter.state,
            FilterField::Assignee => &mut filter.assignee,
            FilterField::Label => &mut filter.label,
            FilterField::Cycle => &mut filter.cycle,
            FilterField::Project => &mut filter.project,
        }
    }
}

/// How far a load has come.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub enum Progress {
    #[default]
    NotAsked,
    Loading,
    Done,
    Failed(String),
}

/// The Team Issues screen: which team and filter it shows, the issues
/// loaded so far in the order Linear sent them, and where the focus is.
/// The issues themselves live in the store.
#[derive(Debug, Default)]
pub struct TeamIssues {
    teams: Progress,
    /// How far the filter options of the team shown have loaded.
    options: Progress,
    query: Option<TeamQuery>,
    ids: Vec<String>,
    /// The cursor of the next page, while there is one.
    next: Option<String>,
    page: Progress,
    /// The page in flight: `None` for the first, else the cursor it starts
    /// after.
    loading_after: Option<String>,
    count: Option<usize>,
    /// Whether the count failed, so the next page asks for it again.
    count_failed: bool,
    /// The loaded issues as they show: grouped, sorted, and where the focus
    /// is.
    list: IssueList,
    /// How the list is grouped and sorted; `None` until the viewer or the
    /// config chose, which is [`Arrangement::TEAM_ISSUES`].
    arrangement: Option<Arrangement>,
    /// The issue to focus once a reload brings it back, as after a sort
    /// that asks Linear for another order.
    refocus: Option<String>,
    names: Names,
}

impl TeamIssues {
    /// How far the viewer's teams have loaded.
    pub fn teams_progress(&self) -> &Progress {
        &self.teams
    }

    /// How far the first page, or the page asked for since, has loaded.
    pub fn page_progress(&self) -> &Progress {
        &self.page
    }

    /// The team and filter shown, once a team is picked.
    pub fn query(&self) -> Option<&TeamQuery> {
        self.query.as_ref()
    }

    /// The ids of the issues loaded so far, in the order Linear sent them.
    pub fn ids(&self) -> &[String] {
        &self.ids
    }

    /// How many issues match, counting those not loaded yet. `None` until
    /// the count is in.
    pub fn total(&self) -> Option<usize> {
        if self.next.is_none() && self.page == Progress::Done {
            return Some(self.ids.len());
        }
        self.count.map(|count| count.max(self.ids.len()))
    }

    /// Whether Linear has more pages than those loaded.
    pub fn has_more(&self) -> bool {
        self.next.is_some()
    }

    /// The loaded issues as they show.
    pub fn list(&self) -> &IssueList {
        &self.list
    }

    /// The id of the issue the list cursor is on.
    pub fn focused_id(&self) -> Option<&str> {
        self.list.focused_issue().map(|issue| issue.id.as_str())
    }

    pub fn pane(&self) -> Pane {
        self.list.pane()
    }

    /// The names its rows and detail look up.
    pub fn names(&self) -> &Names {
        &self.names
    }

    /// How the list is grouped and sorted.
    pub fn arrangement(&self) -> Arrangement {
        self.arrangement.unwrap_or(Arrangement::TEAM_ISSUES)
    }

    /// Groups and sorts the list as `arrangement` says from the next
    /// build on, as at startup before any team shows.
    pub(super) fn set_arrangement(&mut self, arrangement: Arrangement) {
        self.arrangement = Some(arrangement);
    }

    /// Whether the sort orders every matching issue, not only those loaded:
    /// Linear sends pages in its order, or every page is in.
    pub fn sort_covers_all(&self) -> bool {
        let sort = self.arrangement().sort;
        !self.has_more() || sort.is_some_and(|sort| linear_order(sort).is_some())
    }

    /// Carries out a navigation action on a list that shows `rows` rows at
    /// once. Returns whether anything changed. Once the viewer moves, a
    /// reload no longer brings the focus back to the issue it was on.
    pub fn perform(&mut self, action: Action, rows: usize) -> bool {
        let changed = self.list.perform(action, rows);
        if changed {
            self.refocus = None;
        }
        changed
    }

    /// Scrolls the least that brings the focused issue on screen in a list
    /// of `rows` rows. Returns whether the scroll moved.
    pub fn scroll_into_view(&mut self, rows: usize) -> bool {
        self.list.scroll_into_view(rows)
    }

    /// Builds the list again from `store`, keeping the focused issue, or
    /// the one a reload is to bring back once it is in.
    pub(super) fn rebuild(&mut self, store: &Store, rows: usize) {
        let issues: Vec<Issue> = self
            .ids
            .iter()
            .filter_map(|id| store.issue(id))
            .cloned()
            .collect();
        let mut list = IssueList::new(arrange(issues, self.arrangement(), &self.names));
        list.keep_place_of(&self.list, rows);
        if let Some(id) = &self.refocus
            && list.focus_id(id)
        {
            self.refocus = None;
            list.scroll_into_view(rows);
        }
        self.list = list;
    }

    /// Puts a new issue at the top of the list and focuses it, when it is
    /// of the team shown and matches the filter.
    pub(super) fn show_created(&mut self, issue: &Issue) {
        let Some(query) = &self.query else {
            return;
        };
        if query.team_id == issue.team_id && query.filter.matches(issue, false) {
            self.ids.insert(0, issue.id.clone());
            self.count = self.count.map(|count| count + 1);
            self.list.reset();
            self.refocus = Some(issue.id.clone());
        }
    }

    /// Puts the id Linear gave a new issue in place of its stand-in's.
    pub(super) fn replace_created(&mut self, local_id: &str, id: &str) {
        for listed in self.ids.iter_mut().chain(&mut self.refocus) {
            if listed == local_id {
                id.clone_into(listed);
            }
        }
    }

    /// Takes the issue with `id` off the list.
    pub(super) fn forget(&mut self, id: &str) {
        let before = self.ids.len();
        self.ids.retain(|listed| listed != id);
        if self.ids.len() < before {
            self.count = self.count.map(|count| count.saturating_sub(1));
        }
    }

    /// Takes the issue with `id` off the list and returns where it was. The
    /// list shows it until it is built again.
    pub(super) fn forget_at(&mut self, id: &str) -> Option<usize> {
        let position = self.ids.iter().position(|listed| listed == id)?;
        self.forget(id);
        Some(position)
    }

    /// Lists the issue with `id` again at `position`, unless it is listed.
    pub(super) fn put_back(&mut self, id: &str, position: usize) {
        if self.ids.iter().any(|listed| listed == id) {
            return;
        }
        self.ids.insert(position.min(self.ids.len()), id.to_owned());
        self.count = self.count.map(|count| count + 1);
    }

    /// Starts over on `query`: nothing loaded, the focus at the top, and the
    /// first page and the count asked for.
    fn reload(&mut self, query: TeamQuery) -> Vec<Effect> {
        *self = Self {
            teams: std::mem::take(&mut self.teams),
            options: std::mem::take(&mut self.options),
            names: std::mem::take(&mut self.names),
            arrangement: self.arrangement,
            query: Some(query.clone()),
            page: Progress::Loading,
            ..Self::default()
        };
        vec![
            Effect::LoadTeamIssues {
                query: query.clone(),
                after: None,
            },
            Effect::CountTeamIssues(query),
        ]
    }
}

/// Asks for the viewer's teams the first time Team Issues shows, and again
/// each time it shows after they failed.
pub(super) fn open(state: &mut State) -> Vec<Effect> {
    let team_issues = &mut state.team_issues;
    if matches!(team_issues.teams, Progress::Loading | Progress::Done) {
        return Vec::new();
    }
    team_issues.teams = Progress::Loading;
    vec![Effect::LoadTeams]
}

/// Takes in the viewer's teams and shows the first one.
pub(super) fn teams_loaded(state: &mut State, result: Result<Vec<Team>, Failure>) -> Vec<Effect> {
    match result {
        Ok(teams) => {
            state.store.set_viewer_teams(&teams);
            for team in &teams {
                state.learn_reach(&team.id, Ok(()));
            }
            state.team_issues.teams = Progress::Done;
            match teams.first() {
                Some(team) => switch(state, team.id.clone()),
                None => Vec::new(),
            }
        }
        Err(failure) => {
            state.team_issues.teams = Progress::Failed(failure.message.clone());
            state.record(failure);
            Vec::new()
        }
    }
}

/// Shows `team_id` with no filter, and asks for its filter options unless
/// they are in.
fn switch(state: &mut State, team_id: String) -> Vec<Effect> {
    let order = state
        .team_issues
        .arrangement()
        .sort
        .and_then(linear_order)
        .unwrap_or_default();
    let mut effects = state.team_issues.reload(TeamQuery {
        team_id: team_id.clone(),
        filter: TeamFilter::default(),
        order,
    });
    state.team_issues.options = Progress::NotAsked;
    effects.splice(0..0, ask_for_options(state, team_id));
    effects
}

/// Asks for the filter options of `team_id`, the team shown, unless they
/// are in or on their way.
fn ask_for_options(state: &mut State, team_id: String) -> Option<Effect> {
    if state.store.has_team_options(&team_id) {
        state.team_issues.options = Progress::Done;
        return None;
    }
    if state.team_issues.options == Progress::Loading {
        return None;
    }
    state.team_issues.options = Progress::Loading;
    Some(Effect::LoadTeamOptions(team_id))
}

/// Takes in a team's filter options, and fills in the filter popup if one
/// is open.
pub(super) fn options_loaded(state: &mut State, result: Result<TeamOptions, Failure>) {
    match result {
        Ok(options) => {
            if state.notice.is_none() {
                state.notice = options.problems.first().cloned();
            }
            state.record_problems("load the team's filters", &options.problems);
            state.store.merge_team_options(&options);
            refresh_names(state);
            let shown = state.team_issues.query.as_ref().map(|query| &query.team_id);
            if shown == Some(&options.team_id) {
                state.team_issues.options = Progress::Done;
            }
            if let Some(Modal::Choice(Choice {
                purpose: Purpose::Filter(field),
                ..
            })) = state.modals.top()
                && let Some((entries, current)) = filter_entries(state, *field)
                && let Some(Modal::Choice(choice)) = state.modals.top_mut()
            {
                choice.list.set_entries(entries, current);
            }
        }
        // The error names no team; a popup opened later asks again.
        Err(failure) => {
            if state.team_issues.options == Progress::Loading {
                state.team_issues.options = Progress::Failed(failure.message.clone());
            }
            state.fail(failure);
        }
    }
}

/// The filter popup for `field`, and a request for the options it lists
/// when they are not in.
pub(super) fn open_filter(state: &mut State, field: FilterField) -> (Option<Choice>, Vec<Effect>) {
    let Some(team_id) = state
        .team_issues
        .query
        .as_ref()
        .map(|query| query.team_id.clone())
    else {
        return (None, Vec::new());
    };
    let effects = ask_for_options(state, team_id).into_iter().collect();
    (filter_choice(state, field), effects)
}

/// Takes in a page if it answers the request in flight; an answer to an
/// older request is dropped. Returns whether the screen changed.
pub(super) fn page_loaded(
    state: &mut State,
    query: &TeamQuery,
    after: Option<String>,
    result: Result<TeamIssuesPage, Failure>,
) -> bool {
    let team_issues = &state.team_issues;
    if team_issues.query.as_ref() != Some(query)
        || team_issues.page != Progress::Loading
        || team_issues.loading_after != after
    {
        return false;
    }
    let outcome = result.as_ref().map(|_| ()).map_err(|failure| &failure.kind);
    state.learn_reach(&query.team_id, outcome);
    match result {
        Ok(page) => {
            if state.notice.is_none() {
                state.notice = page.problems.first().cloned();
            }
            state.record_problems("load the team's issues", &page.problems);
            state.store.merge_team_page(&page);
            let team_issues = &mut state.team_issues;
            team_issues
                .ids
                .extend(page.issues.iter().map(|issue| issue.id.clone()));
            team_issues.next = page.next;
            team_issues.page = Progress::Done;
            team_issues.loading_after = None;
            refresh_names(state);
            rebuild(state);
        }
        Err(failure) => {
            let team_issues = &mut state.team_issues;
            team_issues.loading_after = None;
            if team_issues.ids.is_empty() {
                team_issues.page = Progress::Failed(failure.message.clone());
                state.record(failure);
            } else {
                team_issues.page = Progress::Done;
                state.fail(failure);
            }
        }
    }
    true
}

/// Takes in the count if it is for the query shown. Returns whether the
/// screen changed.
pub(super) fn counted(
    state: &mut State,
    query: &TeamQuery,
    result: Result<usize, Failure>,
) -> bool {
    if state.team_issues.query.as_ref() != Some(query) {
        return false;
    }
    match result {
        Ok(count) => state.team_issues.count = Some(count),
        Err(failure) => {
            state.team_issues.count_failed = true;
            state.fail(failure);
        }
    }
    true
}

/// Asks for the next page, unless one is in flight or there is none, and
/// for the count again if it failed.
pub(super) fn load_more(state: &mut State) -> Vec<Effect> {
    let team_issues = &mut state.team_issues;
    let (Some(query), Some(next)) = (&team_issues.query, &team_issues.next) else {
        return Vec::new();
    };
    if team_issues.page == Progress::Loading {
        return Vec::new();
    }
    let mut effects = vec![Effect::LoadTeamIssues {
        query: query.clone(),
        after: Some(next.clone()),
    }];
    if team_issues.count_failed {
        effects.push(Effect::CountTeamIssues(query.clone()));
    }
    team_issues.loading_after = Some(next.clone());
    team_issues.page = Progress::Loading;
    team_issues.count_failed = false;
    effects
}

/// Whether the next page can be asked for now.
pub fn can_load_more(state: &State) -> bool {
    state.team_issues.has_more() && state.team_issues.page != Progress::Loading
}

/// An entry of a team switcher or filter: the id it picks, `None` for
/// "Any".
type ChoiceEntry = Entry<Option<String>>;

/// A team switcher for `purpose` over the viewer's teams, the team
/// `shown` marked and selected. `None` while the viewer has no teams.
pub(super) fn team_switcher(
    state: &State,
    purpose: Purpose,
    shown: Option<&String>,
) -> Option<Choice> {
    let entries: Vec<ChoiceEntry> = state
        .store
        .viewer_teams()
        .map(|team| Entry {
            label: format!("{} {}", team.key, team.name),
            value: Some(team.id.clone()),
        })
        .collect();
    if entries.is_empty() {
        return None;
    }
    let current = entries
        .iter()
        .position(|entry| entry.value.as_ref() == shown);
    Some(Choice {
        purpose,
        list: Picker::new(entries, current),
    })
}

/// The team switcher of Team Issues, the team shown selected.
pub(super) fn team_choice(state: &State) -> Option<Choice> {
    let shown = state.team_issues.query.as_ref().map(|query| &query.team_id);
    team_switcher(state, Purpose::Team, shown)
}

/// The picker of the values `field` can filter by for the team shown.
fn filter_choice(state: &State, field: FilterField) -> Option<Choice> {
    let (entries, current) = filter_entries(state, field)?;
    Some(Choice {
        purpose: Purpose::Filter(field),
        list: Picker::new(entries, current),
    })
}

/// The values `field` can filter by for the team shown, after "Any", with
/// where the value in force is among them.
fn filter_entries(state: &State, field: FilterField) -> Option<(Vec<ChoiceEntry>, Option<usize>)> {
    let query = state.team_issues.query.as_ref()?;
    let mut entries = vec![Entry {
        label: "Any".to_owned(),
        value: None,
    }];
    entries.extend(
        filter_values(state, &query.team_id, field)
            .into_iter()
            .map(|(id, label)| Entry {
                label,
                value: Some(id),
            }),
    );
    let current = field.value(&query.filter);
    let current = entries
        .iter()
        .position(|entry| entry.value.as_ref() == current);
    Some((entries, current))
}

/// The ids and names `field` can take in `team_id`, in the order a viewer
/// looks for them.
fn filter_values(state: &State, team_id: &str, field: FilterField) -> Vec<(String, String)> {
    let store = &state.store;
    match field {
        FilterField::State => {
            let mut states: Vec<_> = store
                .all_workflow_states()
                .filter(|state| state.team_id == team_id)
                .collect();
            states.sort_by(|a, b| {
                (a.state_type, a.position)
                    .partial_cmp(&(b.state_type, b.position))
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
            states
                .into_iter()
                .map(|state| (state.id.clone(), state.name.clone()))
                .collect()
        }
        FilterField::Assignee => sorted_by_name(
            store
                .team_members(team_id)
                .into_iter()
                .flatten()
                .map(|member| (member.id.clone(), member.name.clone())),
        ),
        FilterField::Label => sorted_by_name(
            store
                .labels()
                .filter(|label| label.team_id.as_deref().is_none_or(|id| id == team_id))
                .map(|label| (label.id.clone(), label.name.clone())),
        ),
        FilterField::Cycle => {
            let mut cycles: Vec<_> = store
                .cycles()
                .filter(|cycle| cycle.team_id == team_id)
                .collect();
            cycles.sort_by_key(|cycle| std::cmp::Reverse(cycle.number));
            cycles
                .into_iter()
                .map(|cycle| {
                    let name = cycle
                        .name
                        .clone()
                        .unwrap_or_else(|| format!("Cycle {}", cycle.number));
                    (cycle.id.clone(), name)
                })
                .collect()
        }
        FilterField::Project => sorted_by_name(
            store
                .team_projects(team_id)
                .map(|project| (project.id.clone(), project.name.clone())),
        ),
    }
}

fn sorted_by_name(values: impl Iterator<Item = (String, String)>) -> Vec<(String, String)> {
    let mut values: Vec<_> = values.collect();
    values.sort_by(|a, b| a.1.cmp(&b.1).then_with(|| a.0.cmp(&b.0)));
    values
}

/// The name of the value `field` is filtered by, for the header.
pub fn filter_label(state: &State, field: FilterField) -> Option<String> {
    let query = state.team_issues.query.as_ref()?;
    let id = field.value(&query.filter)?;
    let name = filter_values(state, &query.team_id, field)
        .into_iter()
        .find_map(|(value, name)| (&value == id).then_some(name));
    Some(name.unwrap_or_else(|| id.clone()))
}

/// Closes the team switcher or filter on top on its selected entry, or the
/// `nth` visible one when given, and carries it out. Returns `None`, with
/// the picker left open, when there is no such entry.
pub(super) fn pick(state: &mut State, nth: Option<usize>) -> Option<Vec<Effect>> {
    let Some(Modal::Choice(choice)) = state.modals.top() else {
        return None;
    };
    let purpose = choice.purpose;
    let id = choice.chosen(nth).cloned()?;
    state.modals.pop();
    Some(carry_out(state, purpose, id))
}

/// Shows the team `id`, or filters by it, unless that is shown already.
fn carry_out(state: &mut State, purpose: Purpose, id: Option<String>) -> Vec<Effect> {
    match purpose {
        Purpose::Team => match id {
            Some(team_id)
                if state.team_issues.query.as_ref().map(|query| &query.team_id)
                    != Some(&team_id) =>
            {
                switch(state, team_id)
            }
            _ => Vec::new(),
        },
        Purpose::AnalyticsTeam
        | Purpose::CyclesTeam
        | Purpose::Group
        | Purpose::Sort
        | Purpose::RelationKind
        | Purpose::Reaction => Vec::new(),
        Purpose::Filter(field) => {
            let Some(mut query) = state.team_issues.query.clone() else {
                return Vec::new();
            };
            let value = field.value_mut(&mut query.filter);
            if *value == id {
                return Vec::new();
            }
            *value = id;
            state.team_issues.reload(query)
        }
    }
}

/// Shows only the issues another issue blocks, or every issue again, and
/// loads the list afresh.
pub(super) fn toggle_blocked(state: &mut State) -> Vec<Effect> {
    let Some(mut query) = state.team_issues.query.clone() else {
        return Vec::new();
    };
    query.filter.blocked = !query.filter.blocked;
    state.team_issues.reload(query)
}

/// Builds the names rows look up again from the store.
fn refresh_names(state: &mut State) {
    state.team_issues.names = Names::from_store(&state.store);
}

/// Builds the list again from the store, keeping the focus.
pub(super) fn rebuild(state: &mut State) {
    let rows = ui::team_list_rows(state, state.viewport);
    state.team_issues.rebuild(&state.store, rows);
}

/// Groups and sorts Team Issues as `arrangement` says, keeping the focused
/// issue. A sort Linear can send pages in reloads them in that order, and
/// the focus comes back to the issue once it is in.
pub(super) fn rearrange(state: &mut State, arrangement: Arrangement) -> Vec<Effect> {
    let team_issues = &mut state.team_issues;
    team_issues.arrangement = Some(arrangement);
    let order = arrangement.sort.and_then(linear_order).unwrap_or_default();
    match &team_issues.query {
        Some(query) if query.order != order => {
            let focused = team_issues.focused_id().map(str::to_owned);
            let query = TeamQuery {
                order,
                ..query.clone()
            };
            let effects = team_issues.reload(query);
            team_issues.refocus = focused;
            effects
        }
        _ => {
            rebuild(state);
            Vec::new()
        }
    }
}
