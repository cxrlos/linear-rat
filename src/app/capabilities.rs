//! What the API key may do in each team, as lrat has learned it. Linear has
//! no query for a key's limits and no dry run for a mutation, so the map is
//! inferred: a team starts unknown, or reachable when the viewer is a member
//! of it, a forbidden write denies its class there and a later success of
//! that class allows it again.

use std::collections::BTreeMap;

use jiff::Timestamp;
use serde::{Deserialize, Serialize};

use crate::linear::ErrorKind;

/// The kinds of write a key can be limited in, one capability each.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum WriteClass {
    UpdateIssue,
    CreateIssue,
    CreateComment,
    ArchiveOrTrash,
    Relations,
}

impl WriteClass {
    /// What the class does, after "cannot", as `edit issues`.
    pub fn verb(self) -> &'static str {
        match self {
            WriteClass::UpdateIssue => "edit issues",
            WriteClass::CreateIssue => "create issues",
            WriteClass::CreateComment => "comment on issues",
            WriteClass::ArchiveOrTrash => "archive or trash issues",
            WriteClass::Relations => "change issue relations",
        }
    }
}

/// Whether the key may do something.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Capability {
    /// Nothing learned yet.
    #[default]
    Unknown,
    Allowed,
    /// Linear refused it; `since` is when lrat learned that, `None` before
    /// the first tick.
    Denied {
        since: Option<Timestamp>,
    },
}

impl Capability {
    pub fn is_denied(self) -> bool {
        matches!(self, Capability::Denied { .. })
    }
}

/// One team's entry: whether the key can reach the team at all, and each
/// write class there.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
struct TeamCapabilities {
    reach: Capability,
    writes: BTreeMap<WriteClass, Capability>,
}

/// The capability map, by team id.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Capabilities {
    teams: BTreeMap<String, TeamCapabilities>,
}

impl Capabilities {
    /// Whether the key can reach team `team_id`.
    pub fn reach(&self, team_id: &str) -> Capability {
        self.teams
            .get(team_id)
            .map_or(Capability::Unknown, |team| team.reach)
    }

    /// Whether the key may do `class` in team `team_id`. A team it cannot
    /// reach denies every class.
    pub fn write(&self, team_id: &str, class: WriteClass) -> Capability {
        let Some(team) = self.teams.get(team_id) else {
            return Capability::Unknown;
        };
        if team.reach.is_denied() {
            return team.reach;
        }
        team.writes.get(&class).copied().unwrap_or_default()
    }

    /// Learns from a write of `class` in `team_id` that ended `outcome`: a
    /// forbidden failure denies the class from `at`, a success allows it,
    /// and any other failure says nothing about it.
    pub fn learn_write(
        &mut self,
        team_id: &str,
        class: WriteClass,
        outcome: Result<(), &ErrorKind>,
        at: Option<Timestamp>,
    ) {
        let learned = match outcome {
            Ok(()) => Capability::Allowed,
            Err(ErrorKind::Forbidden) => Capability::Denied { since: at },
            Err(_) => return,
        };
        let team = self.teams.entry(team_id.to_owned()).or_default();
        team.writes.insert(class, learned);
        if learned == Capability::Allowed {
            team.reach = Capability::Allowed;
        }
    }

    /// Learns from reading team `team_id` that ended `outcome`, as
    /// [`Capabilities::learn_write`] does for a write.
    pub fn learn_reach(
        &mut self,
        team_id: &str,
        outcome: Result<(), &ErrorKind>,
        at: Option<Timestamp>,
    ) {
        let learned = match outcome {
            Ok(()) => Capability::Allowed,
            Err(ErrorKind::Forbidden) => Capability::Denied { since: at },
            Err(_) => return,
        };
        self.teams.entry(team_id.to_owned()).or_default().reach = learned;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const AT: Timestamp = Timestamp::constant(1_793_696_400, 0);

    #[test]
    fn a_forbidden_write_denies_its_class_and_a_success_allows_it_again() {
        let mut map = Capabilities::default();
        assert_eq!(
            map.write("team-eng", WriteClass::UpdateIssue),
            Capability::Unknown
        );

        map.learn_write(
            "team-eng",
            WriteClass::UpdateIssue,
            Err(&ErrorKind::Forbidden),
            Some(AT),
        );
        assert_eq!(
            map.write("team-eng", WriteClass::UpdateIssue),
            Capability::Denied { since: Some(AT) }
        );
        assert_eq!(
            map.write("team-eng", WriteClass::CreateComment),
            Capability::Unknown
        );
        assert_eq!(
            map.write("team-des", WriteClass::UpdateIssue),
            Capability::Unknown
        );

        map.learn_write("team-eng", WriteClass::UpdateIssue, Ok(()), None);
        assert_eq!(
            map.write("team-eng", WriteClass::UpdateIssue),
            Capability::Allowed
        );
    }

    #[test]
    fn other_failures_teach_nothing() {
        let mut map = Capabilities::default();

        map.learn_write(
            "team-eng",
            WriteClass::CreateIssue,
            Err(&ErrorKind::Offline),
            Some(AT),
        );

        assert_eq!(map, Capabilities::default());
    }

    #[test]
    fn a_team_out_of_reach_denies_every_write() {
        let mut map = Capabilities::default();

        map.learn_reach("team-eng", Err(&ErrorKind::Forbidden), Some(AT));

        assert!(map.reach("team-eng").is_denied());
        assert!(map.write("team-eng", WriteClass::Relations).is_denied());
    }
}
