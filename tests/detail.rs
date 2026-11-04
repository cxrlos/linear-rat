//! The issue detail: Markdown, comments, state history and metadata, the lazy
//! load behind them, and scrolling.

// clippy's allow-unwrap-in-tests does not cover helpers outside #[test] functions.
#![allow(clippy::unwrap_used)]

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use insta::assert_snapshot;
use linear_rat::app::{App, Effect, Event, State};
use linear_rat::linear::{AssignedIssues, IssueDetail, Urgency};
use ratatui::backend::TestBackend;

const MARKDOWN: &str = "\
# Rind photos upload twice

Seen on **flaky networks** with *slow* uploads and ~~old~~ new builds. Run `burrow sync` to check.

## Steps

1. Open a shared pantry
2. Add a wheel
   - with a photo
   - without one

- [x] Reproduce on staging
- [ ] Write the fix

```
fn upload(photo: &Photo) {
    retry(photo);
}
```

> Customers noticed this first.

See the [runbook](https://wiki.squeakworks.example/runbook) and <https://linear.app/squeakworks>.

![Rind photo](https://uploads.squeakworks.example/rind.png)

熟成チーズの写真が二回アップロードされます。棚の一覧も確認してください。熟成庫の温度も記録します。

---

Thanks!";

fn key(c: char) -> Event {
    Event::Key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE))
}

fn ctrl(c: char) -> Event {
    Event::Key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL))
}

fn enter() -> Event {
    Event::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
}

/// The demo's open assigned issues, the first one in the list with
/// `description`.
fn loaded_with(description: &str) -> AssignedIssues {
    let first = first_issue_id();
    let mut loaded = AssignedIssues::demo();
    let issue = loaded
        .issues
        .iter_mut()
        .find(|issue| issue.id == first)
        .unwrap();
    issue.description = Some(description.to_owned());
    loaded
}

/// An urgent load of `issue_id`'s detail, as an explicit open asks for.
fn urgent_load(issue_id: String) -> Effect {
    Effect::LoadIssueDetail {
        issue_id,
        urgency: Urgency::Urgent,
    }
}

fn first_issue_id() -> String {
    State::demo().my_issues.focused_issue().unwrap().id.clone()
}

fn app(columns: u16, rows: u16, loaded: &AssignedIssues) -> App<TestBackend> {
    let mut state = State::default();
    state.show_assigned(loaded);
    App::new(TestBackend::new(columns, rows), state).unwrap()
}

fn send(app: &mut App<TestBackend>, events: impl IntoIterator<Item = Event>) -> Vec<Effect> {
    events
        .into_iter()
        .flat_map(|event| app.handle(event).unwrap())
        .collect()
}

/// Feeds the fake client's answer to every detail load in `effects`.
fn answer(app: &mut App<TestBackend>, effects: Vec<Effect>) {
    for effect in effects {
        if let Effect::LoadIssueDetail { issue_id, .. } = effect {
            let result = Ok(IssueDetail::demo(&issue_id));
            app.handle(Event::IssueDetailLoaded { issue_id, result })
                .unwrap();
        }
    }
}

fn screen(app: &App<TestBackend>) -> Vec<String> {
    let buffer = app.backend().buffer();
    let width = usize::from(buffer.area.width);
    buffer
        .content()
        .chunks(width)
        .map(|row| row.iter().map(|cell| cell.symbol()).collect())
        .collect()
}

/// The demo issue with the most comments and state changes together, and
/// its place in My Issues.
fn richest_detail() -> (usize, String) {
    let state = State::demo();
    let issues: Vec<_> = state
        .my_issues
        .groups()
        .iter()
        .flat_map(|group| &group.issues)
        .collect();
    let (index, issue) = issues
        .iter()
        .enumerate()
        .max_by_key(|(_, issue)| {
            let detail = IssueDetail::demo(&issue.id);
            detail.comments.len() + detail.state_history.len()
        })
        .unwrap();
    (index, issue.id.clone())
}

#[test]
fn opening_the_detail_asks_for_the_focused_issues_comments_and_history_once() {
    let mut app = app(80, 24, &AssignedIssues::demo());

    let first = send(&mut app, [enter()]);
    answer(&mut app, first.clone());
    let back = send(
        &mut app,
        [Event::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))],
    );

    assert_eq!(first, [urgent_load(first_issue_id())]);
    assert!(back.is_empty(), "{back:?}");
}

#[test]
fn reopening_the_detail_loads_it_again() {
    let mut app = app(80, 24, &AssignedIssues::demo());
    let effects = send(&mut app, [enter()]);
    answer(&mut app, effects);

    let again = send(
        &mut app,
        [
            Event::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
            enter(),
        ],
    );

    assert_eq!(again, [urgent_load(first_issue_id())]);
}

#[test]
fn a_wide_terminal_loads_the_detail_of_each_issue_the_list_focuses_in_the_background() {
    let mut app = app(200, 60, &AssignedIssues::demo());

    let effects = send(&mut app, [key('j'), key('j'), key('k')]);

    assert_eq!(effects.len(), 2, "{effects:?}");
    assert!(
        effects.iter().all(|effect| matches!(
            effect,
            Effect::LoadIssueDetail {
                urgency: Urgency::Background,
                ..
            }
        )),
        "{effects:?}"
    );
}

#[test]
fn focusing_the_detail_on_a_wide_terminal_loads_it_again_urgently() {
    let mut app = app(200, 60, &AssignedIssues::demo());
    let effects = send(&mut app, [key('j')]);
    let Some(Effect::LoadIssueDetail {
        issue_id: focused, ..
    }) = effects.first().cloned()
    else {
        panic!("moving in the list asked for no load: {effects:?}");
    };
    answer(&mut app, effects);

    let effects = send(&mut app, [key('l')]);

    assert_eq!(effects, [urgent_load(focused)]);
}

#[test]
fn markdown_renders_in_the_detail_on_a_narrow_terminal() {
    let mut app = app(80, 24, &loaded_with(MARKDOWN));

    send(&mut app, [enter()]);

    assert_snapshot!(app.backend());
}

#[test]
fn the_rest_of_the_markdown_shows_after_scrolling() {
    let mut app = app(80, 24, &loaded_with(MARKDOWN));

    send(&mut app, [enter(), ctrl('d'), ctrl('d')]);

    assert_snapshot!(app.backend());
}

#[test]
fn markdown_renders_in_the_detail_on_a_wide_terminal() {
    let mut app = app(200, 60, &loaded_with(MARKDOWN));

    let effects = send(&mut app, [key('l')]);
    answer(&mut app, effects);

    assert_snapshot!(app.backend());
}

#[test]
fn a_cjk_line_wraps_inside_the_pane() {
    let cjk = "熟成チーズの写真が二回アップロードされます。".repeat(4);
    let mut app = app(80, 24, &loaded_with(&cjk));

    send(&mut app, [enter()]);

    let lines = screen(&app);
    let rows: Vec<&String> = lines.iter().filter(|line| line.contains('熟')).collect();
    assert!(rows.len() >= 2, "{lines:#?}");
    for row in rows {
        assert!(row.ends_with('│'), "cut off: {row}");
    }
}

#[test]
fn comments_and_history_show_once_loaded_on_a_narrow_terminal() {
    let (index, _) = richest_detail();
    let mut app = app(80, 24, &AssignedIssues::demo());
    send(&mut app, (0..index).map(|_| key('j')));

    let effects = send(&mut app, [enter()]);
    answer(&mut app, effects);
    send(&mut app, [key('G')]);

    assert_snapshot!(app.backend());
}

#[test]
fn comments_and_history_show_once_loaded_on_a_wide_terminal() {
    let (index, _) = richest_detail();
    let mut app = app(200, 60, &AssignedIssues::demo());

    let effects = send(&mut app, (0..index).map(|_| key('j')));
    answer(&mut app, effects);

    assert_snapshot!(app.backend());
}

#[test]
fn a_failed_load_says_why_in_the_detail() {
    let mut app = app(80, 24, &AssignedIssues::demo());
    let effects = send(&mut app, [enter()]);

    for effect in effects {
        if let Effect::LoadIssueDetail { issue_id, .. } = effect {
            let result = Err("timed out waiting for Linear's answer".to_owned());
            app.handle(Event::IssueDetailLoaded { issue_id, result })
                .unwrap();
        }
    }

    let lines = screen(&app).join("\n");
    assert!(
        lines.contains("Did not load: timed out waiting for Linear's answer"),
        "{lines}"
    );
}

fn long_description() -> String {
    (1..=40)
        .map(|n| format!("Paragraph {n} of a long description."))
        .collect::<Vec<_>>()
        .join("\n\n")
}

#[test]
fn a_long_description_scrolls_within_the_pane() {
    let mut app = app(80, 24, &loaded_with(&long_description()));
    send(&mut app, [enter()]);
    let top = screen(&app);

    send(&mut app, [ctrl('d')]);
    assert_snapshot!(app.backend());

    send(&mut app, [key('g'), key('g')]);
    assert_eq!(screen(&app), top);
}

#[test]
fn j_and_k_scroll_the_detail_one_line() {
    let mut app = app(80, 24, &loaded_with(&long_description()));
    send(&mut app, [enter()]);
    let top = screen(&app);

    send(&mut app, [key('j')]);
    let down = screen(&app);
    send(&mut app, [key('k')]);

    assert_eq!(down[3], top[4], "every detail line moved up one row");
    assert_eq!(screen(&app), top);
}

#[test]
fn shift_g_scrolls_to_the_end_and_no_further() {
    let mut app = app(80, 24, &loaded_with(&long_description()));
    send(&mut app, [enter()]);

    send(&mut app, [key('G')]);
    let bottom = screen(&app);
    let frames = app.frames_drawn();
    send(&mut app, [key('j'), ctrl('d')]);

    assert_eq!(app.frames_drawn(), frames, "nothing left to scroll");
    assert_eq!(screen(&app), bottom);
    assert!(
        bottom.iter().any(|line| line.contains("Updated")),
        "{bottom:#?}"
    );
}

#[test]
fn the_detail_keeps_its_scroll_when_its_issue_updates() {
    let loaded = loaded_with(&long_description());
    let mut app = app(200, 60, &loaded);
    send(&mut app, [key('l'), ctrl('d')]);
    let scrolled = screen(&app);

    let mut newer = loaded.clone();
    let issue = newer
        .issues
        .iter_mut()
        .find(|issue| issue.id == first_issue_id())
        .unwrap();
    issue.updated_at = issue
        .updated_at
        .checked_add(jiff::SignedDuration::from_mins(5))
        .unwrap();
    issue.title = "A newer title".to_owned();
    send(&mut app, [Event::MyIssuesLoaded(Ok(newer))]);

    let lines = screen(&app);
    let detail_top: Vec<String> = lines
        .iter()
        .map(|line| line.chars().skip(121).take(78).collect())
        .collect();
    let before: Vec<String> = scrolled
        .iter()
        .map(|line| line.chars().skip(121).take(78).collect())
        .collect();
    assert!(before[3].contains("Paragraph 12"), "{scrolled:#?}");
    assert_eq!(detail_top[2..10], before[2..10], "{lines:#?}");
}
