use jiff::Timestamp;
use linear_rat::app::{State, Store};
use linear_rat::linear::model::Issue;
use linear_rat::linear::{AssignedIssues, TeamChanges};

/// Midnight UTC on day `day` of October 2026.
fn october(day: i64) -> Timestamp {
    Timestamp::constant(1_790_812_800 + (day - 1) * 86_400, 0)
}

/// The first demo issue, as of `updated_at`, with `title`.
fn issue(updated_at: Timestamp, title: &str) -> Issue {
    let mut issue = AssignedIssues::demo().issues[0].clone();
    issue.updated_at = updated_at;
    issue.title = title.to_owned();
    issue
}

#[test]
fn a_newer_issue_replaces_the_stored_one() {
    let mut store = Store::default();
    store.upsert_issue(issue(october(1), "old"));

    let replaced = store.upsert_issue(issue(october(2), "new"));

    assert!(replaced);
    assert_eq!(store.issues().next().unwrap().title, "new");
}

#[test]
fn an_older_issue_never_overwrites_a_newer_one() {
    let mut store = Store::default();
    store.upsert_issue(issue(october(2), "new"));

    let replaced = store.upsert_issue(issue(october(1), "old"));

    assert!(!replaced);
    assert_eq!(store.issues().next().unwrap().title, "new");
}

#[test]
fn an_issue_as_new_as_the_stored_one_replaces_it() {
    let mut store = Store::default();
    store.upsert_issue(issue(october(2), "first"));

    let replaced = store.upsert_issue(issue(october(2), "second"));

    assert!(replaced);
    assert_eq!(store.issues().next().unwrap().title, "second");
}

#[test]
fn merging_a_load_keeps_every_entity_by_id() {
    let loaded = AssignedIssues::demo();
    let mut store = Store::default();

    store.merge(&loaded);
    store.merge(&loaded);

    assert_eq!(store.issues().count(), loaded.issues.len());
    assert_eq!(store.cycles().count(), loaded.cycles.len());
    assert_eq!(store.teams().count(), loaded.teams.len());
    assert_eq!(store.viewer(), Some(&loaded.viewer));
}

#[test]
fn my_issues_is_built_from_the_store() {
    let state = State::demo();

    let shown: usize = state
        .my_issues
        .groups()
        .iter()
        .map(|group| group.issues.len())
        .sum();

    assert_eq!(
        state.store.issues().count(),
        AssignedIssues::demo().issues.len()
    );
    assert!(shown > 0);
    assert!(shown <= state.store.issues().count());
}

fn changes(team_id: &str, issues: Vec<Issue>) -> TeamChanges {
    TeamChanges {
        team_id: team_id.to_owned(),
        newest: issues.iter().map(|issue| issue.updated_at).max(),
        issues,
        trashed: Vec::new(),
        cycles: Vec::new(),
        labels: Vec::new(),
        members: Vec::new(),
        teams: Vec::new(),
        as_of: october(31),
    }
}

#[test]
fn a_poll_moves_the_store_time_forward_and_never_back() {
    let mut store = Store::default();
    let mut later = changes("team-eng", Vec::new());
    later.as_of = october(20);
    let mut earlier = changes("team-eng", Vec::new());
    earlier.as_of = october(10);

    store.apply(&later);
    store.apply(&earlier);

    assert_eq!(store.as_of(), Some(october(20)));
}

#[test]
fn a_poll_upserts_its_issues_and_moves_the_team_watermark() {
    let mut store = Store::default();
    store.upsert_issue(issue(october(1), "old"));
    let team = store.issues().next().unwrap().team_id.clone();

    let changed = store.apply(&changes(&team, vec![issue(october(3), "new")]));

    assert!(changed);
    assert_eq!(store.issues().next().unwrap().title, "new");
    assert_eq!(store.watermark(&team), Some(october(3)));
}

#[test]
fn a_poll_that_brings_back_what_is_stored_changes_nothing() {
    let mut store = Store::default();
    store.upsert_issue(issue(october(2), "same"));
    let team = store.issues().next().unwrap().team_id.clone();
    store.apply(&changes(&team, vec![issue(october(2), "same")]));

    let changed = store.apply(&changes(&team, vec![issue(october(2), "same")]));

    assert!(!changed);
}

#[test]
fn a_watermark_never_moves_back() {
    let mut store = Store::default();
    store.apply(&changes("team-eng", vec![issue(october(5), "newer")]));

    store.apply(&changes("team-eng", vec![issue(october(2), "older")]));

    assert_eq!(store.watermark("team-eng"), Some(october(5)));
    assert_eq!(store.issues().next().unwrap().title, "newer");
}

#[test]
fn a_trashed_issue_leaves_the_store() {
    let mut store = Store::default();
    store.upsert_issue(issue(october(1), "doomed"));
    let stored = store.issues().next().unwrap().clone();
    let mut trashed = changes(&stored.team_id, Vec::new());
    trashed.trashed = vec![stored.id.clone()];
    trashed.newest = Some(october(2));

    assert!(store.apply(&trashed));

    assert_eq!(store.issue(&stored.id), None);
    assert_eq!(store.watermark(&stored.team_id), Some(october(2)));
}

#[test]
fn reconciliation_drops_checked_issues_that_no_longer_exist() {
    let mut store = Store::default();
    store.merge(&AssignedIssues::demo());
    let ids: Vec<String> = store.issues().map(|issue| issue.id.clone()).collect();
    let (gone, kept) = ids.split_at(2);

    let changed = store.retain_existing(&ids, kept);

    assert!(changed);
    assert!(gone.iter().all(|id| store.issue(id).is_none()));
    assert_eq!(store.issues().count(), kept.len());
}

#[test]
fn reconciliation_keeps_issues_it_did_not_check() {
    let mut store = Store::default();
    store.merge(&AssignedIssues::demo());
    let before = store.issues().count();

    let changed = store.retain_existing(&[], &[]);

    assert!(!changed);
    assert_eq!(store.issues().count(), before);
}

#[test]
fn a_load_remembers_the_workspace() {
    let mut store = Store::default();

    store.merge(&AssignedIssues::demo());

    assert_eq!(
        store.workspace().map(|workspace| workspace.id.as_str()),
        Some("workspace-squeakworks")
    );
}
