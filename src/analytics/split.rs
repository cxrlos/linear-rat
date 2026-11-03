use std::collections::BTreeMap;

use super::lifecycle::completed_at;
use super::{Period, Unit};
use crate::linear::model::{Issue, Priority};

/// The part of the split work that falls under one key.
#[derive(Debug, Clone, PartialEq)]
pub struct Share<K> {
    pub key: K,
    pub issues: u32,
    /// In the unit passed to [`splits`].
    pub scope: f64,
}

/// Work completed in a period, split three ways.
#[derive(Debug, Clone, PartialEq)]
pub struct Splits {
    /// By project id; `None` for issues outside any project. Largest first.
    pub by_project: Vec<Share<Option<String>>>,
    /// By label id; an issue counts once under each of its labels, and under
    /// `None` when it has none. Largest first.
    pub by_label: Vec<Share<Option<String>>>,
    /// In priority order, urgent first; priorities with no issues are left out.
    pub by_priority: Vec<Share<Priority>>,
}

/// Splits the issues completed in `period` by project, label and priority.
/// Canceled issues are not completed, so they are left out.
pub fn splits(issues: &[Issue], unit: Unit, period: &Period) -> Splits {
    let mut by_project = BTreeMap::new();
    let mut by_label = BTreeMap::new();
    let mut by_priority = BTreeMap::new();
    let completed = issues
        .iter()
        .filter(|issue| completed_at(issue).is_some_and(|at| period.contains(at)));
    for issue in completed {
        let scope = unit.scope_of(issue);
        add(&mut by_project, issue.project_id.clone(), scope);
        if issue.label_ids.is_empty() {
            add(&mut by_label, None, scope);
        }
        for label in &issue.label_ids {
            add(&mut by_label, Some(label.clone()), scope);
        }
        add(&mut by_priority, issue.priority, scope);
    }
    Splits {
        by_project: largest_first(by_project),
        by_label: largest_first(by_label),
        by_priority: into_shares(by_priority),
    }
}

fn add<K: Ord>(shares: &mut BTreeMap<K, (u32, f64)>, key: K, scope: f64) {
    let (issues, total) = shares.entry(key).or_default();
    *issues += 1;
    *total += scope;
}

fn into_shares<K>(shares: BTreeMap<K, (u32, f64)>) -> Vec<Share<K>> {
    shares
        .into_iter()
        .map(|(key, (issues, scope))| Share { key, issues, scope })
        .collect()
}

/// Most issues first; ties keep key order, so the result is deterministic.
fn largest_first<K>(shares: BTreeMap<K, (u32, f64)>) -> Vec<Share<K>> {
    let mut shares = into_shares(shares);
    shares.sort_by_key(|share| std::cmp::Reverse(share.issues));
    shares
}
