//! Everything lrat has loaded from Linear, by id. Loads, polls, live events
//! and write confirmations all merge into it, in any order, and screens are
//! built from it.

use std::collections::{BTreeMap, HashSet};

use jiff::Timestamp;
use serde::{Deserialize, Serialize};

use super::edit::{Edit, Write};
use crate::linear::model::{
    Comment, Cycle, Issue, Label, Member, Project, StateSpan, Team, Viewer, WorkflowState,
    Workspace,
};
use crate::linear::{AssignedIssues, IssueDetail, SearchResults, TeamChanges};

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
    /// The ids of a team's members, by team id, once loaded.
    team_members: BTreeMap<String, Vec<String>>,
    /// Workflow state history by issue id, oldest first, as the detail
    /// loaded it. Kept apart from the issues so an upsert from a list,
    /// which carries no history, does not drop it.
    state_histories: BTreeMap<String, Vec<StateSpan>>,
    /// When the newest data was loaded, which decides the active cycle.
    as_of: Option<Timestamp>,
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
        self.as_of = Some(
            self.as_of
                .map_or(loaded.as_of, |as_of| as_of.max(loaded.as_of)),
        );
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

    fn remove_issue(&mut self, id: &str) -> bool {
        self.comments.remove(id);
        self.state_histories.remove(id);
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

    /// Takes in the detail loaded for `issue_id`: its comments and state
    /// history replace what is stored, and the rest is stored by id.
    pub fn merge_detail(&mut self, issue_id: &str, detail: &IssueDetail) {
        self.set_comments(issue_id, detail.comments.clone());
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
}

fn by_id<T: Clone>(stored: &mut BTreeMap<String, T>, incoming: &[T], id: impl Fn(&T) -> &String) {
    for item in incoming {
        stored.insert(id(item).clone(), item.clone());
    }
}
