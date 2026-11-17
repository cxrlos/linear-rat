//! What the API key may do in each team, as lrat has learned it. Linear has
//! no query for a key's limits and no dry run for a mutation, so the map is
//! inferred: a team starts unknown, or reachable when the viewer is a member
//! of it, a forbidden write denies its class there and a later success of
//! that class allows it again. A denial holds for [`DENIAL_HOLDS`], so a
//! key that gained access, or a refusal that was not about access, gets
//! that success.

use std::collections::BTreeMap;

use jiff::{SignedDuration, Timestamp};
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

/// How long a denial stops lrat from sending a write of its class before
/// the next press tries again.
pub const DENIAL_HOLDS: SignedDuration = SignedDuration::from_mins(15);

impl Capability {
    pub fn is_denied(self) -> bool {
        matches!(self, Capability::Denied { .. })
    }

    /// Whether it is denied and the denial still holds at `now`, the
    /// latest tick. Before the first tick every denial holds; one learned
    /// before the first tick holds until it.
    pub fn holds_at(self, now: Option<Timestamp>) -> bool {
        match (self, now) {
            (Capability::Denied { .. }, None) => true,
            (Capability::Denied { since: Some(since) }, Some(now)) => {
                now.duration_since(since) < DENIAL_HOLDS
            }
            (Capability::Denied { since: None }, Some(_))
            | (Capability::Unknown | Capability::Allowed, _) => false,
        }
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

    /// Forgets every write class denied in any team, so the next press of
    /// each tries again. Reach is left alone: loading the team's issues
    /// learns it again. Returns whether there was any to forget.
    pub fn forget_denied_writes(&mut self) -> bool {
        let mut forgot = false;
        for team in self.teams.values_mut() {
            let before = team.writes.len();
            team.writes.retain(|_, capability| !capability.is_denied());
            forgot |= team.writes.len() != before;
        }
        forgot
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
    fn a_denial_holds_until_it_lapses() {
        let later = |mins| Some(AT.checked_add(SignedDuration::from_mins(mins)).unwrap());
        let denied = Capability::Denied { since: Some(AT) };

        assert!(denied.holds_at(None));
        assert!(denied.holds_at(later(14)));
        assert!(!denied.holds_at(later(15)));
        assert!(Capability::Denied { since: None }.holds_at(None));
        assert!(!Capability::Denied { since: None }.holds_at(Some(AT)));
        assert!(!Capability::Allowed.holds_at(None));
        assert!(!Capability::Unknown.holds_at(Some(AT)));
    }

    #[test]
    fn forgetting_denied_writes_keeps_allowed_writes_and_reach() {
        let mut map = Capabilities::default();
        map.learn_write("team-eng", WriteClass::CreateIssue, Ok(()), None);
        map.learn_write(
            "team-eng",
            WriteClass::UpdateIssue,
            Err(&ErrorKind::Forbidden),
            Some(AT),
        );
        map.learn_reach("team-ops", Err(&ErrorKind::Forbidden), Some(AT));

        assert!(map.forget_denied_writes());
        assert!(!map.forget_denied_writes());

        assert_eq!(
            map.write("team-eng", WriteClass::UpdateIssue),
            Capability::Unknown
        );
        assert_eq!(
            map.write("team-eng", WriteClass::CreateIssue),
            Capability::Allowed
        );
        assert!(map.reach("team-ops").is_denied());
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
