//! Everything lrat has loaded from Linear, by id. Loads, polls, live events
//! and write confirmations all merge into it, in any order, and screens are
//! built from it.

use std::collections::{BTreeMap, HashSet};

use jiff::Timestamp;
use serde::{Deserialize, Serialize};

use super::edit::{Edit, Write};
use crate::linear::model::{
    Attachment, Comment, Cycle, Issue, Label, Member, Project, StateSpan, Team, Viewer,
    WorkflowState, Workspace,
};
use crate::linear::{
    AnalyticsIssues, AnalyticsWindow, AssignedIssues, IssueDetail, PostedComment, SearchResults,
    TeamChanges, TeamIssuesPage, TeamOptions,
};

#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
pub struct Store {
    viewer: Option<Viewer>,
    workspace: Option<Workspace>,
    /// Per team id, the newest `updatedAt` a poll of that team has returned.
    /// Only server values, never the local clock.
    watermarks: BTreeMap<String, Timestamp>,
    issues: BTreeMap<String, Issue>,
    cycles: BTreeMap<String, Cycle>,
    teams: BTreeMap<String, Team>,
    labels: BTreeMap<String, Label>,
    members: BTreeMap<String, Member>,
    /// Comments by issue id, oldest first.
    comments: BTreeMap<String, Vec<Comment>>,
    projects: BTreeMap<String, Project>,
    workflow_states: BTreeMap<String, WorkflowState>,
    /// The ids of every workflow state of a team, by team id, once loaded.
    team_states: BTreeMap<String, Vec<String>>,
    /// The ids of a team's members, by team id, once loaded for a picker
    /// or with the team's filter options.
    team_members: BTreeMap<String, Vec<String>>,
    /// The ids of the labels a team's issues can carry (its own and the
    /// workspace's), by team id, once loaded for a label picker.
    #[serde(default)]
    team_labels: BTreeMap<String, Vec<String>>,
    /// Workflow state history by issue id, oldest first, as the detail
    /// loaded it. Kept apart from the issues so an upsert from a list,
    /// which carries no history, does not drop it.
    state_histories: BTreeMap<String, Vec<StateSpan>>,
    /// Attachments by issue id, as the detail loaded them. Defaults when
    /// missing, so a snapshot written before they were kept still loads.
    #[serde(default)]
    attachments: BTreeMap<String, Vec<Attachment>>,
    /// When the newest data was loaded, which decides the active cycle.
    as_of: Option<Timestamp>,
    /// The ids of the teams the viewer is a member of, in the order Linear
    /// lists them, from the last load that listed them: Team Issues' team
    /// list or an analytics load.
    viewer_teams: Vec<String>,
    /// Project ids by team id, for the teams whose filter options have
    /// loaded.
    team_projects: BTreeMap<String, Vec<String>>,
    /// The windows analytics loads have covered in this session. Left out
    /// of the snapshot, so a new session loads analytics afresh rather than
    /// trusting cycle histories from the last one.
    #[serde(skip)]
    analytics_windows: Vec<AnalyticsWindow>,
}

impl Store {
    /// Stores `issue` unless the stored copy is newer, so updates can arrive
    /// in any order. Returns whether it replaced or added anything.
    pub fn upsert_issue(&mut self, issue: Issue) -> bool {
        if self
            .issues
            .get(&issue.id)
            .is_some_and(|stored| stored.updated_at > issue.updated_at)
        {
            return false;
        }
        self.issues.insert(issue.id.clone(), issue);
        true
    }

    /// Takes in everything `loaded` holds. Issues go through
    /// [`Store::upsert_issue`]; the rest replaces what is stored by id.
    pub fn merge(&mut self, loaded: &AssignedIssues) {
        self.viewer = Some(loaded.viewer.clone());
        self.workspace = Some(loaded.workspace.clone());
        for issue in &loaded.issues {
            self.upsert_issue(issue.clone());
        }
        by_id(&mut self.cycles, &loaded.cycles, |cycle| &cycle.id);
        by_id(&mut self.teams, &loaded.teams, |team| &team.id);
        by_id(&mut self.labels, &loaded.labels, |label| &label.id);
        by_id(&mut self.members, &loaded.members, |member| &member.id);
        self.loaded_at(loaded.as_of);
    }

    /// Takes in one page of a team's issues, through
    /// [`Store::upsert_issue`], and the cycles and members they refer to.
    pub fn merge_team_page(&mut self, page: &TeamIssuesPage) {
        for issue in &page.issues {
            self.upsert_issue(issue.clone());
        }
        by_id(&mut self.cycles, &page.cycles, |cycle| &cycle.id);
        by_id(&mut self.members, &page.members, |member| &member.id);
        self.loaded_at(page.as_of);
    }

    /// Stores the teams the viewer is a member of and remembers them, in
    /// this order. Team Issues offers them and analytics reads their
    /// estimate settings.
    pub fn set_viewer_teams(&mut self, teams: &[Team]) {
        by_id(&mut self.teams, teams, |team| &team.id);
        self.viewer_teams = teams.iter().map(|team| team.id.clone()).collect();
    }

    /// Takes in what a team's filters offer.
    pub fn merge_team_options(&mut self, options: &TeamOptions) {
        by_id(&mut self.workflow_states, &options.states, |state| {
            &state.id
        });
        by_id(&mut self.members, &options.members, |member| &member.id);
        by_id(&mut self.labels, &options.labels, |label| &label.id);
        by_id(&mut self.cycles, &options.cycles, |cycle| &cycle.id);
        by_id(&mut self.projects, &options.projects, |project| &project.id);
        self.team_members.insert(
            options.team_id.clone(),
            options
                .members
                .iter()
                .map(|member| member.id.clone())
                .collect(),
        );
        self.team_projects.insert(
            options.team_id.clone(),
            options
                .projects
                .iter()
                .map(|project| project.id.clone())
                .collect(),
        );
    }

    fn loaded_at(&mut self, as_of: Timestamp) {
        self.as_of = Some(self.as_of.map_or(as_of, |stored| stored.max(as_of)));
    }

    /// Takes in one team's poll: its issues go through
    /// [`Store::upsert_issue`], trashed ones leave, what they refer to
    /// replaces what is stored by id, the team's watermark moves up to the
    /// newest change, and the store's time moves up to the poll's. Returns
    /// whether any issue changed or a cycle started or ended.
    pub fn apply(&mut self, changes: &TeamChanges) -> bool {
        let active_before = self.active_cycle_ids();
        let mut changed = false;
        for issue in &changes.issues {
            if self.issues.get(&issue.id) != Some(issue) {
                changed |= self.upsert_issue(issue.clone());
            }
        }
        for id in &changes.trashed {
            changed |= self.remove_issue(id);
        }
        by_id(&mut self.cycles, &changes.cycles, |cycle| &cycle.id);
        by_id(&mut self.teams, &changes.teams, |team| &team.id);
        by_id(&mut self.labels, &changes.labels, |label| &label.id);
        by_id(&mut self.members, &changes.members, |member| &member.id);
        if let Some(newest) = changes.newest {
            let watermark = self
                .watermarks
                .entry(changes.team_id.clone())
                .or_insert(newest);
            *watermark = (*watermark).max(newest);
        }
        self.as_of = Some(
            self.as_of
                .map_or(changes.as_of, |as_of| as_of.max(changes.as_of)),
        );
        changed || self.active_cycle_ids() != active_before
    }

    /// The cycles that [`Store::as_of`] falls inside.
    fn active_cycle_ids(&self) -> HashSet<String> {
        let Some(now) = self.as_of else {
            return HashSet::new();
        };
        self.cycles
            .values()
            .filter(|cycle| cycle.starts_at <= now && now < cycle.ends_at)
            .map(|cycle| cycle.id.clone())
            .collect()
    }

    /// Drops the issues among `checked` that are not in `existing`, for the
    /// reconciliation. Issues that were not checked stay, since they may
    /// have arrived after the check began. Returns whether any left.
    pub fn retain_existing(&mut self, checked: &[String], existing: &[String]) -> bool {
        let existing: HashSet<&String> = existing.iter().collect();
        let mut changed = false;
        for id in checked.iter().filter(|id| !existing.contains(id)) {
            changed |= self.remove_issue(id);
        }
        changed
    }

    /// Drops the issue with id `id`, its comments and its history. Returns
    /// whether it was stored.
    pub fn remove_issue(&mut self, id: &str) -> bool {
        self.comments.remove(id);
        self.state_histories.remove(id);
        self.attachments.remove(id);
        self.issues.remove(id).is_some()
    }

    /// The newest `updatedAt` a poll of `team_id` has returned, if any.
    pub fn watermark(&self, team_id: &str) -> Option<Timestamp> {
        self.watermarks.get(team_id).copied()
    }

    pub fn workspace(&self) -> Option<&Workspace> {
        self.workspace.as_ref()
    }

    /// Takes in what a search found. Issues go through
    /// [`Store::upsert_issue`]; the rest replaces what is stored by id. The
    /// viewer and [`Store::as_of`] stay as they are.
    pub fn merge_search(&mut self, found: &SearchResults) {
        for issue in &found.issues {
            self.upsert_issue(issue.clone());
        }
        by_id(&mut self.cycles, &found.cycles, |cycle| &cycle.id);
        by_id(&mut self.teams, &found.teams, |team| &team.id);
        by_id(&mut self.labels, &found.labels, |label| &label.id);
        by_id(&mut self.members, &found.members, |member| &member.id);
    }

    /// Takes in an analytics load for `window`. Issues go through
    /// [`Store::upsert_issue`], but a copy without state history keeps the
    /// history of the stored copy of the same version, since only some loads
    /// ask for it. The rest replaces what is stored by id.
    pub fn merge_analytics(&mut self, window: AnalyticsWindow, loaded: &AnalyticsIssues) {
        self.viewer = Some(loaded.viewer.clone());
        for issue in &loaded.issues {
            let mut issue = issue.clone();
            if issue.state_history.is_empty()
                && let Some(stored) = self.issues.get(&issue.id)
                && stored.updated_at == issue.updated_at
            {
                issue.state_history.clone_from(&stored.state_history);
            }
            self.upsert_issue(issue);
        }
        by_id(&mut self.cycles, &loaded.cycles, |cycle| &cycle.id);
        by_id(&mut self.teams, &loaded.teams, |team| &team.id);
        by_id(&mut self.labels, &loaded.labels, |label| &label.id);
        by_id(&mut self.projects, &loaded.projects, |project| &project.id);
        by_id(&mut self.members, &loaded.members, |member| &member.id);
        self.set_viewer_teams(&loaded.teams);
        self.analytics_windows.push(window);
        self.as_of = Some(
            self.as_of
                .map_or(loaded.as_of, |as_of| as_of.max(loaded.as_of)),
        );
    }

    /// How far back analytics loads reach without a gap up to now, or `None`
    /// before a load that reaches now has arrived.
    pub fn analytics_since(&self) -> Option<Timestamp> {
        let mut since = self
            .analytics_windows
            .iter()
            .filter(|window| window.reaches_now())
            .map(|window| window.since)
            .min()?;
        while let Some(earlier) = self
            .analytics_windows
            .iter()
            .filter(|window| window.since < since && window.until.is_some_and(|u| u >= since))
            .map(|window| window.since)
            .min()
        {
            since = earlier;
        }
        Some(since)
    }

    /// Adds a comment Linear has stored to its issue's comments, in order,
    /// and stores its author. Comments not loaded yet stay not loaded, so the
    /// detail still loads all of them.
    pub fn add_comment(&mut self, posted: PostedComment) {
        if let Some(author) = posted.author {
            self.members.insert(author.id.clone(), author);
        }
        let comment = posted.comment;
        let Some(comments) = self.comments.get_mut(&comment.issue_id) else {
            return;
        };
        if comments.iter().any(|known| known.id == comment.id) {
            return;
        }
        comments.push(comment);
        comments.sort_by_key(|comment| comment.created_at);
    }

    /// Replaces the comments of `issue_id`.
    pub fn set_comments(&mut self, issue_id: &str, comments: Vec<Comment>) {
        self.comments.insert(issue_id.to_owned(), comments);
    }

    /// Replaces the workflow states of team `team_id`, and stores each
    /// state by id.
    pub fn set_workflow_states(&mut self, team_id: &str, states: &[WorkflowState]) {
        by_id(&mut self.workflow_states, states, |state| &state.id);
        let ids = states.iter().map(|state| state.id.clone()).collect();
        self.team_states.insert(team_id.to_owned(), ids);
    }

    /// Replaces the members of team `team_id`, and stores each member by id.
    pub fn set_team_members(&mut self, team_id: &str, members: &[Member]) {
        by_id(&mut self.members, members, |member| &member.id);
        let ids = members.iter().map(|member| member.id.clone()).collect();
        self.team_members.insert(team_id.to_owned(), ids);
    }

    /// Replaces the labels team `team_id`'s issues can carry, and stores
    /// each label by id.
    pub fn set_team_labels(&mut self, team_id: &str, labels: &[Label]) {
        by_id(&mut self.labels, labels, |label| &label.id);
        let ids = labels.iter().map(|label| label.id.clone()).collect();
        self.team_labels.insert(team_id.to_owned(), ids);
    }

    /// The labels team `team_id`'s issues can carry, if they have been
    /// loaded.
    pub fn team_labels(&self, team_id: &str) -> Option<Vec<&Label>> {
        let ids = self.team_labels.get(team_id)?;
        Some(ids.iter().filter_map(|id| self.labels.get(id)).collect())
    }

    /// Shows `edit` on the stored issue before Linear confirms it. The
    /// issue keeps its `updated_at`, so Linear's answer replaces it.
    pub fn apply_edit(&mut self, issue_id: &str, edit: &Edit) {
        if let Some(issue) = self.issues.get_mut(issue_id) {
            edit.apply_to(issue);
        }
    }

    /// Puts back the value `write` replaced, unless the field has moved on
    /// from `write.edit` since (a later edit, or newer data from Linear).
    /// Returns whether it restored anything.
    pub fn roll_back(&mut self, write: &Write) -> bool {
        let Some(issue) = self.issues.get_mut(&write.issue_id) else {
            return false;
        };
        if write.edit.field().current(issue) != write.edit {
            return false;
        }
        write.previous.apply_to(issue);
        true
    }

    /// Takes in the detail loaded for `issue_id`: its comments, state
    /// history and attachments replace what is stored, and the rest is
    /// stored by id.
    pub fn merge_detail(&mut self, issue_id: &str, detail: &IssueDetail) {
        self.set_comments(issue_id, detail.comments.clone());
        self.attachments
            .insert(issue_id.to_owned(), detail.attachments.clone());
        self.state_histories
            .insert(issue_id.to_owned(), detail.state_history.clone());
        by_id(&mut self.members, &detail.members, |member| &member.id);
        by_id(
            &mut self.workflow_states,
            &detail.workflow_states,
            |state| &state.id,
        );
        if let Some(project) = &detail.project {
            self.projects.insert(project.id.clone(), project.clone());
        }
    }

    pub fn viewer(&self) -> Option<&Viewer> {
        self.viewer.as_ref()
    }

    pub fn issue(&self, id: &str) -> Option<&Issue> {
        self.issues.get(id)
    }

    pub fn issues(&self) -> impl Iterator<Item = &Issue> {
        self.issues.values()
    }

    pub fn cycles(&self) -> impl Iterator<Item = &Cycle> {
        self.cycles.values()
    }

    pub fn teams(&self) -> impl Iterator<Item = &Team> {
        self.teams.values()
    }

    pub fn labels(&self) -> impl Iterator<Item = &Label> {
        self.labels.values()
    }

    pub fn members(&self) -> impl Iterator<Item = &Member> {
        self.members.values()
    }

    /// The comments of `issue_id`, if they have been loaded.
    pub fn comments(&self, issue_id: &str) -> Option<&[Comment]> {
        self.comments.get(issue_id).map(Vec::as_slice)
    }

    /// The workflow states of team `team_id`, if they have been loaded.
    pub fn workflow_states(&self, team_id: &str) -> Option<Vec<&WorkflowState>> {
        let ids = self.team_states.get(team_id)?;
        Some(
            ids.iter()
                .filter_map(|id| self.workflow_states.get(id))
                .collect(),
        )
    }

    /// The members of team `team_id`, if they have been loaded.
    pub fn team_members(&self, team_id: &str) -> Option<Vec<&Member>> {
        let ids = self.team_members.get(team_id)?;
        Some(ids.iter().filter_map(|id| self.members.get(id)).collect())
    }

    /// The attachments of `issue_id`, if the detail loaded them.
    pub fn attachments(&self, issue_id: &str) -> Option<&[Attachment]> {
        self.attachments.get(issue_id).map(Vec::as_slice)
    }

    /// The workflow states `issue_id` has been in, if the detail loaded them.
    pub fn state_history(&self, issue_id: &str) -> Option<&[StateSpan]> {
        self.state_histories.get(issue_id).map(Vec::as_slice)
    }

    pub fn team(&self, id: &str) -> Option<&Team> {
        self.teams.get(id)
    }

    pub fn cycle(&self, id: &str) -> Option<&Cycle> {
        self.cycles.get(id)
    }

    pub fn label(&self, id: &str) -> Option<&Label> {
        self.labels.get(id)
    }

    pub fn member(&self, id: &str) -> Option<&Member> {
        self.members.get(id)
    }

    pub fn project(&self, id: &str) -> Option<&Project> {
        self.projects.get(id)
    }

    pub fn workflow_state(&self, id: &str) -> Option<&WorkflowState> {
        self.workflow_states.get(id)
    }

    pub fn as_of(&self) -> Option<Timestamp> {
        self.as_of
    }

    /// The teams the viewer is a member of, in the order Linear lists them.
    /// Empty until Team Issues or analytics has loaded them.
    pub fn viewer_teams(&self) -> impl Iterator<Item = &Team> {
        self.viewer_teams.iter().filter_map(|id| self.teams.get(id))
    }

    /// Every workflow state loaded, of any team.
    pub fn all_workflow_states(&self) -> impl Iterator<Item = &WorkflowState> {
        self.workflow_states.values()
    }

    pub fn projects(&self) -> impl Iterator<Item = &Project> {
        self.projects.values()
    }

    /// Whether the filter options of `team_id` have loaded. A picker's
    /// member load alone does not count.
    pub fn has_team_options(&self, team_id: &str) -> bool {
        self.team_projects.contains_key(team_id)
    }

    /// The projects of `team_id`, once its options have loaded.
    pub fn team_projects(&self, team_id: &str) -> impl Iterator<Item = &Project> {
        self.team_projects
            .get(team_id)
            .into_iter()
            .flatten()
            .filter_map(|id| self.projects.get(id))
    }
}

fn by_id<T: Clone>(stored: &mut BTreeMap<String, T>, incoming: &[T], id: impl Fn(&T) -> &String) {
    for item in incoming {
        stored.insert(id(item).clone(), item.clone());
    }
}
