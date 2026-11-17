//! Grouping and sorting My Issues and Results through the app: the pickers,
//! the focus kept across a regroup and the defaults from `[ui]`.

// clippy's allow-unwrap-in-tests does not cover helpers outside #[test] functions.
#![allow(clippy::unwrap_used)]

use std::path::Path;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use insta::assert_snapshot;
use ratatui::backend::TestBackend;

use linear_rat::app::{App, Arrangement, Event, GroupKey, Grouping, Sort, SortKey, State};
use linear_rat::config;
use linear_rat::linear::model::Priority;

fn sized(columns: u16, rows: u16, state: State) -> App<TestBackend> {
    App::new(TestBackend::new(columns, rows), state).unwrap()
}

fn press(app: &mut App<TestBackend>, keys: &str) {
    for c in keys.chars() {
        let code = match c {
            '\x1b' => KeyCode::Esc,
            '\n' => KeyCode::Enter,
            c => KeyCode::Char(c),
        };
        app.handle(Event::Key(KeyEvent::new(code, KeyModifiers::NONE)))
            .unwrap();
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

fn focused_id(app: &App<TestBackend>) -> String {
    app.state().focused_issue().unwrap().id.clone()
}

#[test]
fn my_issues_shows_how_it_is_grouped_and_sorted() {
    let app = sized(120, 30, State::demo());

    let title = &screen(&app)[1];
    assert!(
        title.contains("My Issues · group: workflow state · sort: active cycle, then priority"),
        "{title}"
    );
}

#[test]
fn the_group_picker_lists_every_grouping_with_the_current_one_selected() {
    let mut app = sized(120, 30, State::demo());

    press(&mut app, "vg");

    assert_snapshot!(screen(&app).join("\n"));
}

#[test]
fn focus_stays_on_the_same_issue_across_a_regroup_and_a_sort() {
    let mut app = sized(120, 30, State::demo());
    press(&mut app, "jjjjj");
    let focused = focused_id(&app);

    // 2 is priority.
    press(&mut app, "vg2");
    assert_eq!(
        app.state().my_issues.arrangement().group,
        Grouping::Priority
    );
    assert_eq!(focused_id(&app), focused);

    // 2 is updated newest first, after the list's own order.
    press(&mut app, "vs2");
    assert_eq!(
        app.state().my_issues.arrangement().sort,
        Some(Sort::new(SortKey::Updated, true))
    );
    assert_eq!(focused_id(&app), focused);
    let row = screen(&app)
        .into_iter()
        .find(|row| row.contains('▸'))
        .unwrap();
    let identifier = &app.state().focused_issue().unwrap().identifier;
    assert!(row.contains(identifier.as_str()), "{row}");
}

#[test]
fn my_issues_grouped_by_priority_and_sorted_by_updated() {
    let mut app = sized(120, 30, State::demo());

    press(&mut app, "vg2vs2");

    let keys: Vec<&GroupKey> = app
        .state()
        .my_issues
        .groups()
        .iter()
        .map(|group| &group.key)
        .collect();
    assert!(matches!(keys[0], GroupKey::Priority(_)), "{keys:?}");
    for group in app.state().my_issues.groups() {
        let updated: Vec<_> = group.issues.iter().map(|i| i.updated_at).collect();
        assert!(updated.is_sorted_by(|a, b| a >= b), "{:?}", group.key);
    }
    assert_snapshot!(screen(&app).join("\n"));
}

#[test]
fn an_empty_group_is_not_drawn() {
    let mut app = sized(120, 30, State::demo());

    press(&mut app, "vg2");

    let urgent = app
        .state()
        .my_issues
        .groups()
        .iter()
        .any(|group| group.key == GroupKey::Priority(Priority::Urgent));
    let drawn = screen(&app).iter().any(|row| row.contains("Urgent ("));
    assert_eq!(drawn, urgent);
    assert!(
        app.state()
            .my_issues
            .groups()
            .iter()
            .all(|group| !group.issues.is_empty())
    );
}

#[test]
fn defaults_from_ui_apply_at_startup() {
    let loaded = config::parse(
        "[ui]\nmy-issues-group = \"none\"\nmy-issues-sort = \"created-asc\"\n",
        Path::new("config.toml"),
    );
    let state = State::demo().with_config(&loaded);

    assert_eq!(
        state.my_issues.arrangement(),
        Arrangement {
            group: Grouping::None,
            sort: Some(Sort::new(SortKey::Created, false)),
        }
    );
    let created: Vec<_> = state.my_issues.issues().map(|i| i.created_at).collect();
    assert!(created.is_sorted());
    assert_eq!(state.team_issues.arrangement(), Arrangement::TEAM_ISSUES);
}

#[test]
fn a_bad_default_warns_and_the_list_keeps_its_usual_arrangement() {
    let loaded = config::parse(
        "[ui]\nmy-issues-group = \"label\"\n",
        Path::new("config.toml"),
    );
    let state = State::demo().with_config(&loaded);

    assert!(
        state
            .notice
            .as_deref()
            .is_some_and(|notice| notice.contains("ui.my-issues-group")),
        "{:?}",
        state.notice
    );
    assert_eq!(state.my_issues.arrangement(), Arrangement::MY_ISSUES);
}
