//! Everything lrat has loaded from Linear, by id. Loads, polls, live events
//! and write confirmations all merge into it, in any order, and screens are
//! built from it.

use std::collections::BTreeMap;

use jiff::Timestamp;

use crate::linear::AssignedIssues;
use crate::linear::model::{Comment, Cycle, Issue, Label, Member, Team, Viewer};

#[derive(Debug, Default, Clone, PartialEq)]
pub struct Store {
    viewer: Option<Viewer>,
    issues: BTreeMap<String, Issue>,
    cycles: BTreeMap<String, Cycle>,
    teams: BTreeMap<String, Team>,
    labels: BTreeMap<String, Label>,
    members: BTreeMap<String, Member>,
    /// Comments by issue id, oldest first.
    comments: BTreeMap<String, Vec<Comment>>,
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

    /// Replaces the comments of `issue_id`.
    pub fn set_comments(&mut self, issue_id: &str, comments: Vec<Comment>) {
        self.comments.insert(issue_id.to_owned(), comments);
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

    pub fn as_of(&self) -> Option<Timestamp> {
        self.as_of
    }
}

fn by_id<T: Clone>(stored: &mut BTreeMap<String, T>, incoming: &[T], id: impl Fn(&T) -> &String) {
    for item in incoming {
        stored.insert(id(item).clone(), item.clone());
    }
}
