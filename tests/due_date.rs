//! The due-date prompt's input: every form it accepts, `clear`, and the
//! reasons it gives for input it does not.

// clippy's allow-unwrap-in-tests does not cover helpers outside #[test] functions.
#![allow(clippy::unwrap_used)]

use jiff::civil::{Date, date};
use linear_rat::app::parse_due_date;

/// A Wednesday.
const TODAY: Date = date(2026, 11, 18);

fn parsed(input: &str) -> Option<Date> {
    parse_due_date(input, TODAY).unwrap()
}

#[test]
fn a_full_date_is_taken_as_written() {
    assert_eq!(parsed("2026-12-24"), Some(date(2026, 12, 24)));
    assert_eq!(parsed("  2025-01-02 "), Some(date(2025, 1, 2)));
}

#[test]
fn today_and_tomorrow_count_from_today() {
    assert_eq!(parsed("today"), Some(TODAY));
    assert_eq!(parsed("Tomorrow"), Some(date(2026, 11, 19)));
}

#[test]
fn plus_days_and_weeks_count_from_today() {
    assert_eq!(parsed("+3d"), Some(date(2026, 11, 21)));
    assert_eq!(parsed("+0d"), Some(TODAY));
    assert_eq!(parsed("+2w"), Some(date(2026, 12, 2)));
    assert_eq!(parsed("+15D"), Some(date(2026, 12, 3)));
}

#[test]
fn a_weekday_is_the_next_one_never_today() {
    assert_eq!(parsed("fri"), Some(date(2026, 11, 20)));
    assert_eq!(parsed("friday"), Some(date(2026, 11, 20)));
    assert_eq!(parsed("thu"), Some(date(2026, 11, 19)));
    assert_eq!(parsed("wed"), Some(date(2026, 11, 25)));
    assert_eq!(parsed("Wednesday"), Some(date(2026, 11, 25)));
    assert_eq!(parsed("mon"), Some(date(2026, 11, 23)));
    assert_eq!(parsed("tue"), Some(date(2026, 11, 24)));
    assert_eq!(parsed("sat"), Some(date(2026, 11, 21)));
    assert_eq!(parsed("sun"), Some(date(2026, 11, 22)));
}

#[test]
fn clear_removes_the_due_date() {
    assert_eq!(parse_due_date("clear", TODAY), Ok(None));
    assert_eq!(parse_due_date(" CLEAR ", TODAY), Ok(None));
}

#[test]
fn a_date_that_does_not_exist_says_so() {
    let error = parse_due_date("2026-02-30", TODAY).unwrap_err();

    assert!(error.contains("2026-02-30"), "{error}");
}

#[test]
fn anything_else_names_the_forms_it_takes() {
    for input in [
        "",
        "next week",
        "+3",
        "+d",
        "+3m",
        "26-11-20",
        "2026/11/20",
        "frid",
    ] {
        let error = parse_due_date(input, TODAY).unwrap_err();
        assert!(error.contains("YYYY-MM-DD"), "{input:?}: {error}");
    }
}

#[test]
fn an_offset_too_far_ahead_is_refused() {
    assert!(parse_due_date("+999999w", TODAY).is_err());
}
