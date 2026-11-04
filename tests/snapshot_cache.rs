//! The snapshot cache on disk: what a second launch paints from.

// clippy's allow-unwrap-in-tests does not cover helpers outside #[test] functions.
#![allow(clippy::unwrap_used)]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use jiff::Timestamp;
use linear_rat::app::Store;
use linear_rat::linear::{AssignedIssues, demo};
use linear_rat::snapshot::SnapshotCache;
use serde_json::Value;

/// A fresh, empty cache directory for one test.
fn cache_dir() -> PathBuf {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!(
        "snapshot-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::SeqCst)
    ))
}

fn demo_store() -> Store {
    let mut store = Store::default();
    store.merge(&AssignedIssues::demo());
    store
}

/// The one snapshot file in `dir`.
fn snapshot_file(dir: &PathBuf) -> PathBuf {
    let files: Vec<PathBuf> = fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
        .collect();
    assert_eq!(files.len(), 1, "{files:?}");
    files[0].clone()
}

fn rewrite(path: &PathBuf, change: impl FnOnce(&mut Value)) {
    let mut json: Value = serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap();
    change(&mut json);
    fs::write(path, json.to_string()).unwrap();
}

#[test]
fn a_saved_store_loads_back_as_the_last_session() {
    let cache = SnapshotCache::new(cache_dir());
    let store = demo_store();

    cache.save(&store, Some(demo::NOW)).unwrap();
    let saved = cache.load().unwrap();

    assert_eq!(saved.store, store);
    assert_eq!(saved.synced_at, Some(demo::NOW));
}

#[test]
fn nothing_saved_loads_nothing() {
    assert!(SnapshotCache::new(cache_dir()).load().is_none());
}

#[test]
fn the_snapshot_is_readable_only_by_the_viewer_and_leaves_no_temporary_file() {
    let dir = cache_dir();
    let cache = SnapshotCache::new(dir.clone());

    cache.save(&demo_store(), None).unwrap();
    cache.save(&demo_store(), None).unwrap();

    let mode = fs::metadata(snapshot_file(&dir))
        .unwrap()
        .permissions()
        .mode();
    assert_eq!(mode & 0o777, 0o600);
    let names: Vec<String> = fs::read_dir(&dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert!(names.iter().all(|name| !name.contains("tmp")), "{names:?}");
}

#[test]
fn the_snapshot_holds_its_version_identity_and_the_store_and_nothing_else() {
    let dir = cache_dir();
    SnapshotCache::new(dir.clone())
        .save(&demo_store(), None)
        .unwrap();

    let text = fs::read_to_string(snapshot_file(&dir)).unwrap();
    let json: Value = serde_json::from_str(&text).unwrap();
    let mut keys: Vec<&String> = json.as_object().unwrap().keys().collect();
    keys.sort();
    assert_eq!(
        keys,
        ["store", "synced_at", "version", "viewer_id", "workspace_id"]
    );
    assert!(!text.contains("lin_api_"));
}

#[test]
fn a_snapshot_of_another_schema_version_is_discarded() {
    let dir = cache_dir();
    let cache = SnapshotCache::new(dir.clone());
    cache.save(&demo_store(), None).unwrap();

    rewrite(&snapshot_file(&dir), |json| json["version"] = 999.into());

    assert!(cache.load().is_none());
}

#[test]
fn a_snapshot_whose_store_belongs_to_another_viewer_is_discarded() {
    let dir = cache_dir();
    let cache = SnapshotCache::new(dir.clone());
    cache.save(&demo_store(), None).unwrap();

    rewrite(&snapshot_file(&dir), |json| {
        json["store"]["viewer"]["id"] = "someone-else".into();
    });

    assert!(cache.load().is_none());
}

#[test]
fn a_snapshot_whose_store_belongs_to_another_workspace_is_discarded() {
    let dir = cache_dir();
    let cache = SnapshotCache::new(dir.clone());
    cache.save(&demo_store(), None).unwrap();

    rewrite(&snapshot_file(&dir), |json| {
        json["store"]["workspace"]["id"] = "workspace-other".into();
    });

    assert!(cache.load().is_none());
}

#[test]
fn a_save_that_runs_after_a_later_one_is_written_never_overwrites_it() {
    let cache = SnapshotCache::new(cache_dir());
    let older = demo_store();
    let mut newer_issues = AssignedIssues::demo();
    newer_issues.issues.truncate(1);
    let mut newer = Store::default();
    newer.merge(&newer_issues);
    assert_ne!(older, newer);

    let first = cache.next_save();
    let second = cache.next_save();
    cache.save_in_order(second, &newer, None).unwrap();
    cache.save_in_order(first, &older, None).unwrap();

    assert_eq!(cache.load().unwrap().store, newer);
}

#[test]
fn a_corrupt_snapshot_is_discarded() {
    let dir = cache_dir();
    let cache = SnapshotCache::new(dir.clone());
    cache.save(&demo_store(), None).unwrap();

    fs::write(snapshot_file(&dir), "{ not json").unwrap();

    assert!(cache.load().is_none());
}

#[test]
fn a_store_without_a_viewer_is_not_saved() {
    let dir = cache_dir();
    let cache = SnapshotCache::new(dir.clone());

    cache
        .save(&Store::default(), Some(Timestamp::UNIX_EPOCH))
        .unwrap();

    assert!(cache.load().is_none());
}

#[test]
fn each_workspace_and_viewer_has_its_own_file_and_the_last_one_saved_loads() {
    let dir = cache_dir();
    let cache = SnapshotCache::new(dir.clone());
    let mut other = AssignedIssues::demo();
    other.viewer.id = "viewer-other".to_owned();
    let mut other_store = Store::default();
    other_store.merge(&other);

    cache.save(&demo_store(), None).unwrap();
    cache.save(&other_store, None).unwrap();

    let json_files = fs::read_dir(&dir)
        .unwrap()
        .filter(|entry| {
            entry
                .as_ref()
                .unwrap()
                .path()
                .extension()
                .is_some_and(|ext| ext == "json")
        })
        .count();
    assert_eq!(json_files, 2);
    assert_eq!(cache.load().unwrap().store, other_store);
}
