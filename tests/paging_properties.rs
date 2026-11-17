//! Properties of stitching a Relay connection's pages: any split of a list
//! into pages stitches back to the list, and a cursor Linear repeats ends
//! the walk instead of looping.

use std::collections::BTreeMap;
use std::sync::LazyLock;

use linear_rat::linear::AssignedIssues;
use linear_rat::linear::model::Issue;
use linear_rat::linear::paging::{Next, RepeatedCursor, Stitch};
use proptest::prelude::*;
use proptest::test_runner::FileFailurePersistence;

/// One page as Linear sends it: its issues and its `pageInfo`.
#[derive(Debug, Clone)]
struct Page {
    issues: Vec<Issue>,
    has_next_page: bool,
    end_cursor: Option<String>,
}

/// `count` distinct issues, built from a real demo issue.
fn issues(count: usize) -> Vec<Issue> {
    static TEMPLATE: LazyLock<Issue> =
        LazyLock::new(|| AssignedIssues::demo().issues.swap_remove(0));
    let template = &*TEMPLATE;
    (0..count)
        .map(|n| Issue {
            id: format!("issue-{n}"),
            identifier: format!("ENG-{n}"),
            ..template.clone()
        })
        .collect()
}

/// `list` split at `cuts` (sorted offsets into it), each page pointing at
/// the next by a fresh cursor and the last one saying there is no more.
/// Pages may be empty.
fn split(list: &[Issue], cuts: &[usize]) -> Vec<Page> {
    let mut bounds = vec![0];
    bounds.extend(cuts.iter().copied());
    bounds.push(list.len());
    let last = bounds.len() - 2;
    bounds
        .windows(2)
        .enumerate()
        .map(|(n, pair)| Page {
            issues: list[pair[0]..pair[1]].to_vec(),
            has_next_page: n < last,
            end_cursor: Some(format!("cursor-{n}")),
        })
        .collect()
}

/// Walks `pages` the way the client does: the first page, then whichever
/// page the cursor names, until the stitch stops. A server that never
/// stops is cut off after a generous number of requests.
fn walk(pages: &[Page], limit: usize) -> (Result<(Vec<Page>, bool), RepeatedCursor>, usize) {
    // A cursor names the page after the first one that gave it.
    let mut after: BTreeMap<&str, usize> = BTreeMap::new();
    for (n, page) in pages.iter().enumerate() {
        if let Some(cursor) = page.end_cursor.as_deref() {
            after.entry(cursor).or_insert(n + 1);
        }
    }
    let mut stitch = Stitch::new(limit);
    let mut at = 0;
    let mut requests = 0;
    while requests <= 2 * pages.len() + 2 {
        requests += 1;
        let page = pages[at].clone();
        let (more, cursor) = (page.has_next_page, page.end_cursor.clone());
        match stitch.add(page, more, cursor.as_deref()) {
            Err(repeated) => return (Err(repeated), requests),
            Ok(Next::Done { cut }) => return (Ok((stitch.into_pages(), cut)), requests),
            Ok(Next::After(cursor)) => at = after[cursor.as_str()],
        }
    }
    panic!("the walk was still going after {requests} requests");
}

fn ids(issues: &[Issue]) -> Vec<&str> {
    issues.iter().map(|issue| issue.id.as_str()).collect()
}

/// A list of up to 40 issues and up to 8 cut points into it.
fn list_and_cuts() -> impl Strategy<Value = (Vec<Issue>, Vec<usize>)> {
    (0usize..40).prop_flat_map(|count| {
        let cuts = proptest::collection::vec(0..=count, 0..8).prop_map(|mut cuts| {
            cuts.sort_unstable();
            cuts
        });
        (Just(issues(count)), cuts)
    })
}

proptest! {
    #![proptest_config(ProptestConfig {
        failure_persistence: Some(Box::new(FileFailurePersistence::Direct(
            "proptest-regressions/paging_properties.txt",
        ))),
        ..ProptestConfig::default()
    })]

    #[test]
    fn any_split_stitches_back_to_the_whole_list((list, cuts) in list_and_cuts()) {
        let pages = split(&list, &cuts);

        let (result, requests) = walk(&pages, usize::MAX);

        let (stitched, cut) = result.expect("fresh cursors never repeat");
        let flat: Vec<Issue> = stitched.into_iter().flat_map(|page| page.issues).collect();
        prop_assert_eq!(ids(&flat), ids(&list));
        prop_assert!(!cut);
        prop_assert_eq!(requests, pages.len());
    }

    #[test]
    fn a_limit_keeps_the_first_pages_and_says_more_were_left(
        (list, cuts) in list_and_cuts(),
        limit in 1usize..6,
    ) {
        let pages = split(&list, &cuts);

        let (result, _) = walk(&pages, limit);

        let (stitched, cut) = result.expect("fresh cursors never repeat");
        prop_assert_eq!(stitched.len(), pages.len().min(limit));
        prop_assert_eq!(cut, pages.len() > limit);
        let kept: Vec<Issue> = pages[..stitched.len()]
            .iter()
            .flat_map(|page| page.issues.clone())
            .collect();
        let flat: Vec<Issue> = stitched.into_iter().flat_map(|page| page.issues).collect();
        prop_assert_eq!(ids(&flat), ids(&kept));
    }

    #[test]
    fn a_repeated_cursor_stops_the_walk(
        (list, cuts) in list_and_cuts(),
        picks in (any::<prop::sample::Index>(), any::<prop::sample::Index>()),
    ) {
        let mut pages = split(&list, &cuts);
        prop_assume!(pages.len() >= 2);
        // A page before the last that says there is more, but hands back
        // the cursor of itself or of a page before it.
        let late = 1 + picks.0.index(pages.len() - 1);
        let early = picks.1.index(late);
        let repeated = pages[early].end_cursor.clone();
        pages[late].has_next_page = true;
        pages[late].end_cursor = repeated;

        let (result, requests) = walk(&pages, usize::MAX);

        prop_assert_eq!(result.map(|_| ()), Err(RepeatedCursor));
        prop_assert_eq!(requests, late + 1);
    }

    #[test]
    fn more_pages_without_a_cursor_stops_the_walk((list, cuts) in list_and_cuts()) {
        let mut pages = split(&list, &cuts);
        let last = pages.len() - 1;
        pages[last].has_next_page = true;
        pages[last].end_cursor = None;

        let (result, _) = walk(&pages, usize::MAX);

        prop_assert_eq!(result.map(|_| ()), Err(RepeatedCursor));
    }
}
