//! Properties of merging into the store out of order: whatever order issue
//! versions arrive in, alone or in team change batches, the newest version
//! of each issue is the one kept, and a team's watermark never goes back.

use std::collections::BTreeMap;
use std::sync::LazyLock;

use jiff::Timestamp;
use linear_rat::app::Store;
use linear_rat::linear::model::Issue;
use linear_rat::linear::{AssignedIssues, TeamChanges};
use proptest::prelude::*;
use proptest::test_runner::FileFailurePersistence;

const TEAM: &str = "team-eng";

/// Midnight UTC on 1 October 2026, `seconds` later.
fn at(seconds: i64) -> Timestamp {
    Timestamp::constant(1_790_812_800 + seconds, 0)
}

/// Version `updated` of issue `n`, built from a real demo issue. The title
/// names the version, so the test can tell which one was kept.
fn version(n: usize, updated: i64) -> Issue {
    static TEMPLATE: LazyLock<Issue> =
        LazyLock::new(|| AssignedIssues::demo().issues.swap_remove(0));
    Issue {
        id: format!("issue-{n}"),
        identifier: format!("ENG-{n}"),
        title: format!("issue {n} as of {updated}"),
        team_id: TEAM.to_owned(),
        updated_at: at(updated),
        ..TEMPLATE.clone()
    }
}

/// Up to six issues with one to five versions each, every version of an
/// issue with its own `updatedAt`, in a random order.
fn shuffled_versions() -> impl Strategy<Value = Vec<Issue>> {
    proptest::collection::vec(proptest::collection::btree_set(0i64..100_000, 1..=5), 1..=6)
        .prop_map(|issues| {
            issues
                .into_iter()
                .enumerate()
                .flat_map(|(n, times)| times.into_iter().map(move |updated| version(n, updated)))
                .collect::<Vec<_>>()
        })
        .prop_shuffle()
}

/// The newest version of each issue among `versions`, by id.
fn newest(versions: &[Issue]) -> BTreeMap<String, Issue> {
    let mut newest: BTreeMap<String, Issue> = BTreeMap::new();
    for issue in versions {
        let kept = newest
            .entry(issue.id.clone())
            .or_insert_with(|| issue.clone());
        if issue.updated_at > kept.updated_at {
            *kept = issue.clone();
        }
    }
    newest
}

fn stored(store: &Store) -> BTreeMap<String, Issue> {
    store
        .issues()
        .map(|issue| (issue.id.clone(), issue.clone()))
        .collect()
}

/// One poll of the team bringing `issues`, its watermark the newest of them.
fn batch(issues: &[Issue], as_of: i64) -> TeamChanges {
    TeamChanges {
        team_id: TEAM.to_owned(),
        issues: issues.to_vec(),
        trashed: Vec::new(),
        cycles: Vec::new(),
        labels: Vec::new(),
        members: Vec::new(),
        teams: Vec::new(),
        newest: issues.iter().map(|issue| issue.updated_at).max(),
        as_of: at(as_of),
    }
}

/// `versions` cut into consecutive batches at `cuts`, sorted offsets.
fn batches(versions: &[Issue], cuts: &[usize]) -> Vec<TeamChanges> {
    let mut bounds = vec![0];
    bounds.extend(cuts.iter().map(|&cut| cut.min(versions.len())));
    bounds.push(versions.len());
    bounds.sort_unstable();
    bounds
        .windows(2)
        .enumerate()
        .map(|(n, pair)| batch(&versions[pair[0]..pair[1]], 200_000 + n as i64))
        .collect()
}

proptest! {
    #![proptest_config(ProptestConfig {
        failure_persistence: Some(Box::new(FileFailurePersistence::Direct(
            "proptest-regressions/store_properties.txt",
        ))),
        ..ProptestConfig::default()
    })]

    #[test]
    fn upserts_in_any_order_keep_the_newest_version_of_each_issue(
        versions in shuffled_versions(),
    ) {
        let mut store = Store::default();

        for issue in &versions {
            store.upsert_issue(issue.clone());
        }

        prop_assert_eq!(stored(&store), newest(&versions));
    }

    #[test]
    fn team_batches_in_any_order_keep_the_newest_version_and_never_lower_the_watermark(
        versions in shuffled_versions(),
        cuts in proptest::collection::vec(0usize..30, 0..6),
    ) {
        let mut store = Store::default();
        let mut watermark: Option<Timestamp> = None;

        for changes in batches(&versions, &cuts) {
            store.apply(&changes);
            let now = store.watermark(TEAM);
            prop_assert!(now >= watermark, "the watermark went from {watermark:?} to {now:?}");
            watermark = now;
        }

        prop_assert_eq!(stored(&store), newest(&versions));
        let newest_change = versions.iter().map(|issue| issue.updated_at).max();
        prop_assert_eq!(store.watermark(TEAM), newest_change);
    }
}
