//! Adding and removing labels and changing the estimate through the app
//! driver: the multi-select label picker and the add/remove deltas it sends,
//! the estimate picker following the team's scale, and the rollback when
//! Linear rejects either edit.

// clippy's allow-unwrap-in-tests does not cover helpers outside #[test] functions.
#![allow(clippy::unwrap_used)]

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use insta::assert_snapshot;
use linear_rat::app::{
    Action, App, Edit, Editable, Effect, Event, Modal, State, Write, estimate_scale,
};
use linear_rat::keys;
use linear_rat::linear::model::{Estimation, Issue, Label};
use linear_rat::linear::{AssignedIssues, FakeClient, LinearClient, Patch, demo};
use ratatui::backend::TestBackend;

/// The issue My Issues focuses first in the demo, in team DES.
const FIRST: &str = "DES-34";
const DES: &str = "team-des";

fn app() -> App<TestBackend> {
    App::new(TestBackend::new(80, 24), State::demo()).unwrap()
}

/// The demo app with team DES estimating on `estimation`.
fn app_estimating(estimation: Estimation) -> App<TestBackend> {
    let mut app = app();
    let mut loaded = AssignedIssues::demo();
    for team in &mut loaded.teams {
        if team.id == DES {
            team.estimation = estimation;
        }
    }
    app.handle(Event::MyIssuesLoaded(Ok(loaded))).unwrap();
    app
}

fn key(code: KeyCode) -> Event {
    Event::Key(KeyEvent::new(code, KeyModifiers::NONE))
}

/// Types `keys`, one character each, and returns every effect asked for.
fn press(app: &mut App<TestBackend>, keys: &str) -> Vec<Effect> {
    keys.chars()
        .flat_map(|c| app.handle(key(KeyCode::Char(c))).unwrap())
        .collect()
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

fn shows(app: &App<TestBackend>, text: &str) -> bool {
    screen(app).iter().any(|line| line.contains(text))
}

/// The lines inside the open picker's border below the filter, trimmed.
fn picker_lines(app: &App<TestBackend>) -> Vec<String> {
    let lines = screen(app);
    let Some(top) = lines.iter().position(|line| line.contains(" of DES-")) else {
        return Vec::new();
    };
    let start = lines[top].chars().position(|c| c == '┌').unwrap();
    lines[top + 2..]
        .iter()
        .take_while(|line| line.chars().nth(start) == Some('│'))
        .map(|line| {
            let inner: String = line
                .chars()
                .skip(start + 1)
                .take_while(|&c| c != '│')
                .collect();
            inner.trim().to_owned()
        })
        .collect()
}

fn issue(app: &App<TestBackend>, identifier: &str) -> Issue {
    app.state()
        .store
        .issues()
        .find(|issue| issue.identifier == identifier)
        .cloned()
        .unwrap()
}

/// When the demo workspace last updated the first issue.
fn demo_updated_at() -> jiff::Timestamp {
    demo::workspace()
        .issues
        .into_iter()
        .find(|issue| issue.identifier == FIRST)
        .unwrap()
        .updated_at
}

fn labels_loaded(team_id: &str) -> Event {
    Event::TeamLabelsLoaded {
        team_id: team_id.to_owned(),
        result: Ok(demo::team_options(&demo::workspace(), team_id).labels),
    }
}

/// The only effect, which must be a write.
fn only_write(effects: Vec<Effect>) -> Write {
    match effects.as_slice() {
        [Effect::UpdateIssue(write)] => write.clone(),
        other => panic!("expected exactly one write, got {other:?}"),
    }
}

/// Linear's answer to `write`, as the fake client gives it, on a thread of
/// its own so each answer starts from the demo workspace.
fn accepted(write: &Write) -> Event {
    let issue = std::thread::scope(|scope| {
        scope
            .spawn(|| {
                tokio::runtime::Builder::new_current_thread()
                    .build()
                    .unwrap()
                    .block_on(FakeClient.update_issue(&write.issue_id, &write.update()))
            })
            .join()
            .unwrap()
    })
    .unwrap();
    Event::IssueUpdated {
        write: write.clone(),
        result: Ok(Box::new(issue)),
    }
}

fn rejected(write: &Write) -> Event {
    Event::IssueUpdated {
        write: write.clone(),
        result: Err("Linear returned errors: Entity not found (INVALID_INPUT)".to_owned()),
    }
}

/// The label picker of the first issue, with the team's labels loaded.
fn label_picker() -> App<TestBackend> {
    let mut app = app();
    press(&mut app, "L");
    app.handle(labels_loaded(DES)).unwrap();
    app
}

#[test]
fn l_opens_the_label_picker_and_asks_for_the_teams_labels() {
    let mut app = app();

    let effects = press(&mut app, "L");

    assert_eq!(
        effects,
        vec![Effect::LoadTeamLabels {
            team_id: DES.to_owned()
        }]
    );
    assert!(shows(&app, "Labels of DES-34"));
    assert!(shows(&app, "Loading…"));
}

#[test]
fn the_label_picker_marks_the_issues_labels_among_the_team_and_workspace_labels() {
    let app = label_picker();

    assert_eq!(
        picker_lines(&app),
        [
            "1 [ ] brand",
            "2 [ ] bug",
            "3 [ ] feature",
            "4 [x] improvement",
            "5 [ ] research",
        ]
    );
    assert_snapshot!(app.backend());
}

#[test]
fn tab_and_numbers_toggle_entries_and_enter_sends_only_the_change() {
    let mut app = label_picker();

    app.handle(key(KeyCode::Tab)).unwrap();
    press(&mut app, "4");
    press(&mut app, "5");
    let write = only_write(app.handle(key(KeyCode::Enter)).unwrap());

    let update = write.update();
    assert_eq!(
        update.added_label_ids,
        ["team-des-label-brand", "team-des-label-research"]
    );
    assert_eq!(update.removed_label_ids, ["label-improvement"]);
    assert_eq!(update.state_id, None);
    assert_eq!(update.estimate, Patch::Unchanged);
    assert_eq!(
        issue(&app, FIRST).label_ids,
        ["team-des-label-brand", "team-des-label-research"]
    );
    assert!(app.state().modals.top().is_none());
}

#[test]
fn typing_filters_the_label_picker_and_numbers_toggle_what_is_left() {
    let mut app = label_picker();

    press(&mut app, "res1");

    assert_eq!(picker_lines(&app), ["1 [x] research"]);
}

#[test]
fn labels_the_picker_does_not_list_are_left_alone() {
    let mut app = app();
    let id = issue(&app, FIRST).id;
    let mut other = issue(&app, FIRST);
    other.label_ids.push("team-ops-label-infra".to_owned());
    other.updated_at += jiff::SignedDuration::from_secs(1);
    let mut loaded = AssignedIssues::demo();
    for issue in &mut loaded.issues {
        if issue.id == id {
            *issue = other.clone();
        }
    }
    app.handle(Event::MyIssuesLoaded(Ok(loaded))).unwrap();
    press(&mut app, "L");
    app.handle(labels_loaded(DES)).unwrap();

    press(&mut app, "3");
    let write = only_write(app.handle(key(KeyCode::Enter)).unwrap());

    let update = write.update();
    assert_eq!(update.added_label_ids, ["label-feature"]);
    assert!(update.removed_label_ids.is_empty());
    assert_eq!(
        issue(&app, FIRST).label_ids,
        ["label-improvement", "team-ops-label-infra", "label-feature"]
    );
}

#[test]
fn marks_made_before_the_labels_load_survive_the_load() {
    let mut app = app();
    press(&mut app, "L");
    let before = picker_lines(&app);
    let position = before
        .iter()
        .position(|line| line.ends_with("[x] improvement"))
        .unwrap();
    press(&mut app, &(position + 1).to_string());
    app.handle(labels_loaded(DES)).unwrap();

    assert!(
        picker_lines(&app).contains(&"4 [ ] improvement".to_owned()),
        "{:?}",
        picker_lines(&app)
    );
}

#[test]
fn reopening_the_label_picker_lists_the_loaded_labels_and_asks_again() {
    let mut app = label_picker();
    app.handle(key(KeyCode::Esc)).unwrap();

    let effects = press(&mut app, "L");

    assert_eq!(
        effects,
        vec![Effect::LoadTeamLabels {
            team_id: DES.to_owned()
        }]
    );
    assert_eq!(picker_lines(&app).len(), 5, "{:?}", picker_lines(&app));
    assert!(!shows(&app, "Loading…"));
}

#[test]
fn labels_created_or_deleted_in_linear_show_once_the_picker_reloads() {
    let mut app = label_picker();
    app.handle(key(KeyCode::Esc)).unwrap();
    press(&mut app, "L");
    press(&mut app, "1");
    let mut labels = demo::team_options(&demo::workspace(), DES).labels;
    labels.retain(|label| label.name != "research");
    labels.push(Label {
        id: "team-des-label-regression".to_owned(),
        team_id: Some(DES.to_owned()),
        name: "regression".to_owned(),
    });

    app.handle(Event::TeamLabelsLoaded {
        team_id: DES.to_owned(),
        result: Ok(labels),
    })
    .unwrap();

    assert_eq!(
        picker_lines(&app),
        [
            "1 [x] brand",
            "2 [ ] bug",
            "3 [ ] feature",
            "4 [x] improvement",
            "5 [ ] regression",
        ]
    );
}

#[test]
fn saving_the_labels_the_issue_has_sends_nothing() {
    let mut app = label_picker();

    press(&mut app, "44");
    let effects = app.handle(key(KeyCode::Enter)).unwrap();

    assert!(effects.is_empty());
    assert!(app.state().modals.top().is_none());
}

#[test]
fn a_label_edit_keeps_linears_answer() {
    let mut app = label_picker();
    press(&mut app, "1");
    let write = only_write(app.handle(key(KeyCode::Enter)).unwrap());

    let effects = app.handle(accepted(&write)).unwrap();

    assert!(effects.is_empty());
    assert_eq!(
        issue(&app, FIRST).label_ids,
        ["label-improvement", "team-des-label-brand"]
    );
}

#[test]
fn a_rejected_label_edit_restores_the_labels_with_the_error() {
    let mut app = label_picker();
    press(&mut app, "12");
    let write = only_write(app.handle(key(KeyCode::Enter)).unwrap());

    app.handle(rejected(&write)).unwrap();

    assert_eq!(issue(&app, FIRST).label_ids, ["label-improvement"]);
    assert!(shows(&app, "Could not change the labels of DES-34"));
}

#[test]
fn the_label_picker_says_when_the_labels_could_not_load() {
    let mut app = app();
    press(&mut app, "L");

    app.handle(Event::TeamLabelsLoaded {
        team_id: DES.to_owned(),
        result: Err("the network is down".to_owned()),
    })
    .unwrap();

    assert!(shows(&app, "Could not load: the network is down"));
    assert!(shows(&app, "Could not load the labels of DES"));
}

#[test]
fn each_scale_lists_its_points_smallest_first() {
    let points = |estimation| -> Vec<u32> {
        estimate_scale(estimation)
            .iter()
            .map(|&(points, _)| points)
            .collect()
    };

    assert_eq!(points(Estimation::Exponential), [1, 2, 4, 8, 16]);
    assert_eq!(points(Estimation::Fibonacci), [1, 2, 3, 5, 8]);
    assert_eq!(points(Estimation::Linear), [1, 2, 3, 4, 5]);
    assert_eq!(points(Estimation::TShirt), [1, 2, 3, 5, 8]);
    assert!(points(Estimation::NotUsed).is_empty());
}

#[test]
fn the_estimate_picker_shows_the_teams_scale() {
    let cases = [
        (
            Estimation::Linear,
            ["1 point", "2 points", "3 points", "4 points", "5 points"],
        ),
        (
            Estimation::Fibonacci,
            ["1 point", "2 points", "3 points", "5 points", "8 points"],
        ),
        (Estimation::TShirt, ["XS", "S", "M", "L", "XL"]),
    ];
    // DES-34 has 3 points, which the exponential scale does not have, so it
    // is listed in its place.
    let mut app = app_estimating(Estimation::Exponential);
    press(&mut app, "e");
    assert_eq!(
        picker_lines(&app),
        [
            "1 1 point",
            "2 2 points",
            "3 3 points ✓",
            "4 4 points",
            "5 8 points",
            "6 16 points",
            "7 No estimate"
        ]
    );
    for (estimation, scale) in cases {
        let mut app = app_estimating(estimation);

        let effects = press(&mut app, "e");

        assert!(effects.is_empty(), "{estimation:?}: {effects:?}");
        let names: Vec<String> = picker_lines(&app)
            .iter()
            .map(|line| line[2..].trim_end_matches(" ✓").to_owned())
            .collect();
        let mut expected: Vec<String> = scale.iter().map(|&name| name.to_owned()).collect();
        expected.push("No estimate".to_owned());
        assert_eq!(names, expected, "{estimation:?}");
    }
}

#[test]
fn the_estimate_picker_marks_the_issues_estimate() {
    let mut app = app();

    press(&mut app, "e");

    assert_snapshot!(app.backend());
}

#[test]
fn an_estimate_off_the_scale_is_listed_in_its_place() {
    let mut app = app_estimating(Estimation::TShirt);
    let mut loaded = AssignedIssues::demo();
    for team in &mut loaded.teams {
        if team.id == DES {
            team.estimation = Estimation::TShirt;
        }
    }
    for issue in &mut loaded.issues {
        if issue.identifier == FIRST {
            issue.estimate = Some(13.0);
            issue.updated_at += jiff::SignedDuration::from_secs(1);
        }
    }
    app.handle(Event::MyIssuesLoaded(Ok(loaded))).unwrap();

    press(&mut app, "e");

    assert_eq!(
        picker_lines(&app),
        [
            "1 XS",
            "2 S",
            "3 M",
            "4 L",
            "5 XL",
            "6 XXL ✓",
            "7 No estimate"
        ]
    );
}

#[test]
fn without_estimates_the_action_is_unavailable_and_opens_nothing() {
    let mut app = app_estimating(Estimation::NotUsed);
    let spec = keys::spec(Action::Edit(Editable::Estimate)).unwrap();
    assert!(!(spec.available)(app.state()));

    let effects = press(&mut app, "e");

    assert!(effects.is_empty());
    assert!(app.state().modals.top().is_none());
    let with = app_estimating(Estimation::Linear);
    assert!((spec.available)(with.state()));
}

#[test]
fn the_cheatsheet_mutes_the_estimate_action_without_estimates() {
    let style_of = |estimation| {
        let mut app = App::new(TestBackend::new(80, 60), State::demo()).unwrap();
        let mut loaded = AssignedIssues::demo();
        for team in &mut loaded.teams {
            team.estimation = estimation;
        }
        app.handle(Event::MyIssuesLoaded(Ok(loaded))).unwrap();
        press(&mut app, "?");
        assert!(matches!(
            app.state().modals.top(),
            Some(Modal::Cheatsheet { .. })
        ));
        let buffer = app.backend().buffer();
        let width = usize::from(buffer.area.width);
        let rows: Vec<String> = screen(&app);
        let row = rows
            .iter()
            .position(|line| line.contains("Change the estimate"))
            .unwrap();
        let column = rows[row].find("Change the estimate").unwrap();
        let column = rows[row][..column].chars().count();
        (
            buffer.content()[row * width + column].style(),
            app.state().theme.muted,
        )
    };

    let (without, muted) = style_of(Estimation::NotUsed);
    let (with, _) = style_of(Estimation::Linear);

    assert_eq!(without.fg, Some(muted));
    assert_ne!(with.fg, Some(muted));
}

#[test]
fn an_estimate_edit_sends_whole_points_and_keeps_linears_answer() {
    let mut app = app();
    press(&mut app, "e");
    let write = only_write(press(&mut app, "5"));

    assert_eq!(write.edit, Edit::Estimate(Some(5)));
    assert_eq!(write.previous, Edit::Estimate(Some(3)));
    assert_eq!(write.update().estimate, Patch::Set(5));
    assert_eq!(issue(&app, FIRST).estimate, Some(5.0));
    app.handle(accepted(&write)).unwrap();
    let stored = issue(&app, FIRST);
    assert_eq!(stored.estimate, Some(5.0));
    assert!(stored.updated_at > demo_updated_at());
}

#[test]
fn no_estimate_clears_the_estimate() {
    let mut app = app();
    press(&mut app, "e");

    let write = only_write(press(&mut app, "6"));

    assert_eq!(write.update().estimate, Patch::Clear);
    assert_eq!(issue(&app, FIRST).estimate, None);
}

#[test]
fn a_rejected_estimate_edit_restores_the_estimate_with_the_error() {
    let mut app = app();
    let before = issue(&app, FIRST).estimate;
    press(&mut app, "e");
    let write = only_write(press(&mut app, "6"));

    app.handle(rejected(&write)).unwrap();

    assert_eq!(issue(&app, FIRST).estimate, before);
    assert!(shows(&app, "Could not change the estimate of DES-34"));
}
