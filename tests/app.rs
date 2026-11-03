use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use insta::assert_snapshot;
use linear_rat::app::{Action, App, Effect, Event, MyIssues, State, perform};
use linear_rat::linear::demo;
use ratatui::backend::TestBackend;

fn app() -> App<TestBackend> {
    sized(80, 24)
}

fn sized(columns: u16, rows: u16) -> App<TestBackend> {
    App::new(TestBackend::new(columns, rows), State::demo()).unwrap()
}

fn key(c: char) -> Event {
    Event::Key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE))
}

fn ctrl(c: char) -> Event {
    Event::Key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL))
}

fn press(code: KeyCode) -> Event {
    Event::Key(KeyEvent::new(code, KeyModifiers::NONE))
}

fn send(app: &mut App<TestBackend>, events: impl IntoIterator<Item = Event>) {
    for event in events {
        app.handle(event).unwrap();
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

/// The row of the focused issue, marked with `▸`.
fn focused_row(app: &App<TestBackend>) -> String {
    screen(app)
        .into_iter()
        .find(|line| line.contains('▸'))
        .unwrap_or_default()
}

#[test]
fn opens_on_my_issues_with_the_three_screen_tabs() {
    let app = app();

    assert_snapshot!(app.backend());
}

#[test]
fn redraws_at_the_new_size_after_a_resize() {
    let mut app = app();

    let frames = app.frames_drawn();
    app.backend_mut().resize(200, 60);
    let effects = app
        .handle(Event::Resize {
            columns: 200,
            rows: 60,
        })
        .unwrap();

    assert!(effects.is_empty());
    assert_eq!(app.frames_drawn(), frames + 1);
    assert_snapshot!(app.backend());
}

#[test]
fn q_asks_to_quit() {
    let mut app = app();

    assert_eq!(app.handle(key('q')).unwrap(), vec![Effect::Quit]);
}

#[test]
fn draws_nothing_when_an_event_changes_nothing() {
    let mut app = app();
    let frames = app.frames_drawn();

    app.handle(key('x')).unwrap();
    app.handle(key('k')).unwrap();
    app.handle(key('q')).unwrap();

    assert_eq!(app.frames_drawn(), frames);
}

/// The groups on a My Issues list: each header without its count, the count
/// it shows, and the rows under it.
fn groups(lines: &[String]) -> Vec<(String, usize, Vec<String>)> {
    let mut groups: Vec<(String, usize, Vec<String>)> = Vec::new();
    for line in lines {
        let text = line.trim_matches(|c| c == '│' || c == ' ');
        if text.starts_with('└') {
            break;
        }
        let header = text.strip_suffix(')').and_then(|t| t.split_once(" ("));
        if let Some((name, Ok(count))) = header.map(|(name, count)| (name, count.parse())) {
            groups.push((name.to_owned(), count, Vec::new()));
        } else if let Some((_, _, rows)) = groups.last_mut()
            && !text.is_empty()
            && !text.starts_with('┌')
        {
            rows.push(text.to_owned());
        }
    }
    groups
}

#[test]
fn my_issues_groups_open_work_by_state_type_in_order_with_counts() {
    let lines = screen(&sized(100, 60));

    let groups = groups(&lines);
    let names: Vec<&str> = groups.iter().map(|(name, ..)| name.as_str()).collect();
    assert_eq!(
        names,
        ["◐ Started", "○ Unstarted", "◌ Backlog", "◇ Triage"],
        "{lines:#?}"
    );
    for (name, count, rows) in &groups {
        assert_eq!(*count, rows.len(), "{name}");
    }
    for closed in ["Done", "Canceled", "Duplicate"] {
        assert!(lines.iter().all(|line| !line.contains(closed)), "{closed}");
    }
}

#[test]
fn active_cycle_issues_come_first_within_each_group() {
    let lines = screen(&sized(100, 60));

    let in_active_cycle: Vec<Vec<bool>> = groups(&lines)
        .into_iter()
        .map(|(_, _, rows)| rows.iter().map(|row| row.ends_with("Cycle 7")).collect())
        .collect();

    assert!(
        in_active_cycle
            .iter()
            .any(|group| group.contains(&true) && group.contains(&false)),
        "{lines:#?}"
    );
    for group in in_active_cycle {
        assert!(group.is_sorted_by(|a, b| a >= b), "{group:?}");
    }
}

#[test]
fn j_and_k_move_the_focus_through_the_list() {
    let mut app = app();
    let first = focused_row(&app);

    send(&mut app, [key('j'), key('j')]);
    let third = focused_row(&app);
    send(&mut app, [key('k')]);
    let second = focused_row(&app);

    assert_ne!(first, third);
    assert_ne!(second, third);
    assert_ne!(second, first);
    send(&mut app, [key('k')]);
    assert_eq!(focused_row(&app), first);
}

#[test]
fn shift_g_scrolls_to_the_last_issue() {
    let mut app = app();

    send(&mut app, [key('G')]);

    assert_snapshot!(app.backend());
}

#[test]
fn move_to_top_returns_to_the_first_issue() {
    let first = focused_row(&app());
    let mut state = State::demo();

    perform(&mut state, Action::MoveToBottom);
    let update = perform(&mut state, Action::MoveToTop);

    assert!(update.redraw);
    let app = App::new(TestBackend::new(80, 24), state).unwrap();
    assert_eq!(focused_row(&app), first);
}

#[test]
fn ctrl_d_and_ctrl_u_move_half_a_page() {
    let mut app = app();
    let first = focused_row(&app);

    send(&mut app, [ctrl('d')]);
    let lines = screen(&app);
    let row = lines.iter().position(|line| line.contains('▸')).unwrap();
    send(&mut app, [ctrl('u')]);

    assert!(row >= 10, "{lines:#?}");
    assert_eq!(focused_row(&app), first);
}

#[test]
fn enter_opens_the_detail_full_width_on_a_narrow_terminal() {
    let mut app = app();

    send(&mut app, [key('j'), press(KeyCode::Enter)]);

    assert_snapshot!(app.backend());
}

#[test]
fn esc_returns_from_the_detail_to_the_list() {
    let mut app = app();
    send(&mut app, [key('j')]);
    let list = screen(&app);

    send(&mut app, [press(KeyCode::Enter), press(KeyCode::Esc)]);

    assert_eq!(screen(&app), list);
}

#[test]
fn a_wide_terminal_shows_the_focused_issue_beside_the_list() {
    let mut app = sized(200, 60);

    send(&mut app, [key('j'), key('j')]);

    assert_snapshot!(app.backend());
}

#[test]
fn l_and_h_move_focus_between_the_panes_on_a_wide_terminal() {
    let mut app = sized(200, 60);
    let list_focused = screen(&app);

    send(&mut app, [key('l')]);
    let detail_focused = screen(&app);
    send(&mut app, [key('j')]);
    assert_eq!(
        screen(&app),
        detail_focused,
        "j moves nothing in the detail"
    );
    send(&mut app, [key('h')]);

    assert_ne!(detail_focused, list_focused);
    assert_eq!(screen(&app), list_focused);
}

#[test]
fn the_detail_pane_has_focus_after_l_on_a_wide_terminal() {
    let mut app = sized(200, 60);

    send(&mut app, [key('l')]);

    assert_snapshot!(app.backend());
}

#[test]
fn moving_up_keeps_the_rows_above_in_view() {
    let mut app = app();
    send(&mut app, [key('G')]);
    let lines = screen(&app);
    let bottom = lines.iter().position(|line| line.contains('▸')).unwrap();
    let above = lines[bottom - 3].clone();

    send(&mut app, [key('k'), key('k'), key('k')]);
    let lines = screen(&app);

    let focused = lines.iter().position(|line| line.contains('▸')).unwrap();
    assert_eq!(focused, bottom - 3, "{lines:#?}");
    assert_eq!(lines[focused].replacen('▸', " ", 1), above, "{lines:#?}");
}

#[test]
fn moving_back_to_the_top_shows_the_first_group_header() {
    let mut app = app();
    let top = screen(&app);

    send(&mut app, [key('G')]);
    send(
        &mut app,
        [ctrl('u'), ctrl('u'), ctrl('u'), ctrl('u'), ctrl('u')],
    );

    assert_eq!(screen(&app), top);
}

#[test]
fn only_issues_in_the_active_cycle_come_first() {
    let mut data = demo::workspace();
    let open = |data: &demo::DemoWorkspace| {
        MyIssues::new(
            &data.viewer,
            &data.issues,
            &data.cycles,
            &data.labels,
            &data.members,
            &data.teams,
            demo::NOW,
        )
    };
    let first = open(&data).groups()[0].issues[0].clone();
    let past_cycle = data
        .cycles
        .iter()
        .find(|cycle| cycle.team_id == first.team_id && cycle.ends_at <= demo::NOW)
        .unwrap()
        .id
        .clone();
    let issue = data
        .issues
        .iter_mut()
        .find(|issue| issue.id == first.id)
        .unwrap();
    issue.cycle_id = Some(past_cycle);

    let my_issues = open(&data);
    let group = &my_issues.groups()[0];
    let identifiers: Vec<&str> = group
        .issues
        .iter()
        .map(|issue| issue.identifier.as_str())
        .collect();
    let moved = group
        .issues
        .iter()
        .position(|issue| issue.id == first.id)
        .unwrap();
    let active = group
        .issues
        .iter()
        .filter(|issue| my_issues.is_in_active_cycle(issue))
        .count();
    assert!(active > 0, "{identifiers:?}");
    assert!(moved >= active, "{identifiers:?}");
}

#[test]
fn the_detail_leaves_out_the_estimate_for_a_team_without_estimates() {
    let mut app = sized(200, 60);
    assert!(
        screen(&app).iter().any(|line| line.contains("Estimate")),
        "the first issue's team uses estimates"
    );

    while !focused_row(&app).contains("OPS-") {
        send(&mut app, [key('j')]);
    }

    let lines = screen(&app);
    assert!(
        lines.iter().all(|line| !line.contains("Estimate")),
        "{lines:#?}"
    );
}

#[test]
fn the_list_leaves_out_estimates_for_a_team_without_estimates() {
    let mut data = demo::workspace();
    let ops = data
        .teams
        .iter()
        .find(|team| team.key == "OPS")
        .unwrap()
        .id
        .clone();
    let issue = data
        .issues
        .iter_mut()
        .find(|issue| {
            issue.team_id == ops
                && issue.assignee_id.as_deref() == Some(data.viewer.id.as_str())
                && issue.state.state_type.is_open()
        })
        .unwrap();
    issue.estimate = Some(8.0);
    let identifier = issue.identifier.clone();
    let state = State {
        my_issues: MyIssues::new(
            &data.viewer,
            &data.issues,
            &data.cycles,
            &data.labels,
            &data.members,
            &data.teams,
            demo::NOW,
        ),
        ..State::demo()
    };

    let app = App::new(TestBackend::new(200, 60), state).unwrap();

    let lines = screen(&app);
    let row = lines
        .iter()
        .find(|line| line.contains(&format!(" {identifier} ")))
        .unwrap();
    let list_part: String = row.chars().take(120).collect();
    assert!(!list_part.contains(" 8 "), "{row}");
}

#[test]
fn an_empty_list_says_no_open_issues_are_assigned() {
    let app = App::new(TestBackend::new(80, 24), State::default()).unwrap();

    assert_snapshot!(app.backend());
}
