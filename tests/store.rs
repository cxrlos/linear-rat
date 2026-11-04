use jiff::Timestamp;
use linear_rat::app::{State, Store};
use linear_rat::linear::AssignedIssues;
use linear_rat::linear::model::Issue;

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
