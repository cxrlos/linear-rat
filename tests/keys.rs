use std::path::Path;

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use insta::assert_snapshot;
use linear_rat::app::{App, Effect, Event, State};
use linear_rat::config;
use ratatui::backend::TestBackend;
use ratatui::style::Color;

/// Everything a viewer can override, once each.
const OVERRIDES: &str = r#"
[keys.global]
quit = "Q"
show-cheatsheet = { append = ["<F1>"] }

[keys.list]
move-down = { prepend = ["n"] }
move-to-bottom = { append = ["gb"] }
noop = ["gg", "k"]
"#;

fn app_with(config: &str) -> App<TestBackend> {
    let loaded = config::parse(config, Path::new("config.toml"));
    App::new(
        TestBackend::new(80, 24),
        State::default().with_config(&loaded),
    )
    .unwrap()
}

fn app() -> App<TestBackend> {
    app_with("")
}

fn key(c: char) -> Event {
    Event::Key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE))
}

fn esc() -> Event {
    Event::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
}

fn press(app: &mut App<TestBackend>, keys: &str) -> Vec<Effect> {
    keys.chars()
        .flat_map(|c| app.handle(key(c)).unwrap())
        .collect()
}

fn row(app: &App<TestBackend>, y: u16) -> String {
    let buffer = app.backend().buffer();
    (0..buffer.area.width)
        .map(|x| buffer[(x, y)].symbol())
        .collect()
}

/// The color `text` starts with on row `y`.
fn fg_of(app: &App<TestBackend>, y: u16, text: &str) -> Option<Color> {
    let line = row(app, y);
    let x = line[..line.find(text)?].chars().count();
    let cell = app.backend().buffer().cell((u16::try_from(x).ok()?, y))?;
    Some(cell.fg)
}

#[test]
fn the_bottom_bar_lists_the_keys_of_the_issue_list() {
    let app = app();

    assert_snapshot!(format!("{}\n{}", row(&app, 22), row(&app, 23)));
}

#[test]
fn the_bottom_bar_lines_up_moves_and_actions_in_two_rows() {
    let app = app();

    assert_eq!(
        row(&app, 22).trim_end(),
        " Move │ j down     │ k up   │ gg top    │ G bottom  │ C-d/C-u page │ h/l pane"
    );
    assert_eq!(
        row(&app, 23).trim_end(),
        " Do   │ Enter open │ y yank │ o browser │ Esc close │ ? keys       │ q quit"
    );
}

#[test]
fn a_bottom_bar_too_wide_for_columns_lists_each_row_after_its_label() {
    let loaded = config::parse("", Path::new("config.toml"));
    let app = App::new(
        TestBackend::new(70, 24),
        State::default().with_config(&loaded),
    )
    .unwrap();

    assert_eq!(
        row(&app, 22).trim_end(),
        " Move │ j down  k up  gg top  G bottom  C-d/C-u page  h/l pane"
    );
    assert_eq!(
        row(&app, 23).trim_end(),
        " Do   │ Enter open  y yank  o browser  Esc close  ? keys  q quit"
    );
}

#[test]
fn the_bottom_bar_mutes_unavailable_actions_and_accents_the_rest() {
    let app = app();

    assert_eq!(fg_of(&app, 23, "Esc"), Some(Color::DarkGray));
    assert_eq!(fg_of(&app, 22, "j"), Some(Color::Cyan));
}

#[test]
fn the_bottom_bar_accents_esc_and_the_moves_that_scroll_while_the_detail_is_open() {
    let mut app = App::new(TestBackend::new(80, 24), State::demo()).unwrap();

    app.handle(Event::Key(KeyEvent::new(
        KeyCode::Enter,
        KeyModifiers::NONE,
    )))
    .unwrap();

    assert_eq!(fg_of(&app, 23, "Esc"), Some(Color::Cyan));
    assert_eq!(fg_of(&app, 22, "j"), Some(Color::Cyan), "more below");
    assert_eq!(fg_of(&app, 22, "k"), Some(Color::DarkGray), "at the top");
}

#[test]
fn each_tab_shows_the_key_that_opens_it() {
    let app = app();

    assert_eq!(
        row(&app, 0).trim_end(),
        " [1] My Issues │ [2] Team Issues │ [3] Analytics"
    );
}

#[test]
fn a_tab_shows_its_overridden_key_and_none_when_unbound() {
    let app = app_with(
        r#"
[keys.global]
show-team-issues = "t"
noop = ["3"]
"#,
    );

    assert_eq!(
        row(&app, 0).trim_end(),
        " [1] My Issues │ [t] Team Issues │ Analytics"
    );
}

#[test]
fn a_tab_leaves_out_a_key_the_screen_binds_to_something_else() {
    let app = app_with(
        r#"
[keys.list]
move-down = "2"
"#,
    );

    assert_eq!(
        row(&app, 0).trim_end(),
        " [1] My Issues │ Team Issues │ [3] Analytics"
    );
}

#[test]
fn the_bottom_bar_shows_overridden_keys() {
    let app = app_with(OVERRIDES);

    assert_snapshot!(format!("{}\n{}", row(&app, 22), row(&app, 23)));
}

#[test]
fn question_mark_opens_the_cheatsheet_for_the_issue_list() {
    let mut app = app();

    press(&mut app, "?");

    assert_snapshot!(app.backend());
}

#[test]
fn the_cheatsheet_shows_overridden_keys() {
    let mut app = app_with(OVERRIDES);

    press(&mut app, "?");
    assert_snapshot!(app.backend());

    press(&mut app, &"j".repeat(30));
    assert_snapshot!(
        "the_cheatsheet_shows_overridden_keys_scrolled",
        app.backend()
    );
}

fn screen(app: &App<TestBackend>) -> String {
    (0..24).map(|y| row(app, y) + "\n").collect()
}

#[test]
fn the_cheatsheet_fits_inside_the_frame_and_scrolls_to_the_rest() {
    let mut app = app();

    press(&mut app, "?");
    assert!(
        row(&app, 1).starts_with("┌My Issues───"),
        "{}",
        screen(&app)
    );
    assert!(!screen(&app).contains("Quit lrat"));

    press(&mut app, &"j".repeat(30));
    assert!(
        screen(&app).contains("q       Quit lrat"),
        "{}",
        screen(&app)
    );
    assert!(!screen(&app).contains("Issue list"));
    assert!(row(&app, 20).contains("└ Esc close"), "{}", screen(&app));

    press(&mut app, &"k".repeat(30));
    assert!(row(&app, 3).contains("Issue list"), "{}", screen(&app));
}

#[test]
fn the_bottom_bar_of_the_cheatsheet_accents_only_the_ways_it_can_scroll() {
    let mut app = app();

    press(&mut app, "?");
    assert_eq!(fg_of(&app, 22, "j"), Some(Color::Cyan));
    assert_eq!(fg_of(&app, 22, "k"), Some(Color::DarkGray));

    press(&mut app, &"j".repeat(30));
    assert_eq!(fg_of(&app, 22, "j"), Some(Color::DarkGray));
    assert_eq!(fg_of(&app, 22, "k"), Some(Color::Cyan));
}

#[test]
fn esc_closes_the_cheatsheet() {
    let mut app = app();
    let before = app.backend().buffer().clone();

    press(&mut app, "?");
    app.handle(esc()).unwrap();

    assert_eq!(app.backend().buffer(), &before);
}

#[test]
fn the_cheatsheet_takes_keys_away_from_the_screen() {
    let mut app = app();

    let effects = press(&mut app, "?q2");

    assert!(effects.is_empty());
    assert!(row(&app, 1).starts_with("┌My Issues"));
}

#[test]
fn a_chord_prefix_opens_which_key_with_its_continuations() {
    let mut app = app();

    press(&mut app, "g");

    assert_snapshot!(app.backend());
}

#[test]
fn which_key_shows_overridden_continuations() {
    let mut app = app_with(OVERRIDES);

    press(&mut app, "g");

    assert_snapshot!(app.backend());
}

#[test]
fn completing_a_chord_closes_which_key() {
    let mut app = app();
    let before = app.backend().buffer().clone();

    press(&mut app, "gg");

    assert_eq!(app.backend().buffer(), &before);
}

#[test]
fn esc_cancels_a_chord() {
    let mut app = app();
    let before = app.backend().buffer().clone();

    press(&mut app, "g");
    let effects = app.handle(esc()).unwrap();

    assert!(effects.is_empty());
    assert_eq!(app.backend().buffer(), &before);
}

#[test]
fn a_key_that_continues_no_chord_cancels_it_without_running() {
    let mut app = app();
    let before = app.backend().buffer().clone();

    let effects = press(&mut app, "gq");

    assert!(effects.is_empty());
    assert_eq!(app.backend().buffer(), &before);
}

#[test]
fn a_replaced_key_runs_its_action_and_the_old_key_does_not() {
    let mut app = app_with(OVERRIDES);

    assert!(press(&mut app, "q").is_empty());
    assert_eq!(press(&mut app, "Q"), vec![Effect::Quit]);
}

#[test]
fn a_prepended_key_wins_over_a_default_on_the_same_key() {
    let mut app = app_with("[keys.global]\nquit = { prepend = [\"?\"] }\n");

    assert_eq!(press(&mut app, "?"), vec![Effect::Quit]);
}

#[test]
fn an_appended_key_loses_to_a_default_on_the_same_key() {
    let mut app = app_with("[keys.global]\nquit = { append = [\"?\"] }\n");

    assert!(press(&mut app, "?").is_empty());
    assert!(row(&app, 2).contains("Keys"));
}

#[test]
fn a_noop_key_does_nothing() {
    let mut app = app_with("[keys.global]\nnoop = [\"q\"]\n");
    let frames = app.frames_drawn();

    assert!(press(&mut app, "q").is_empty());
    assert_eq!(app.frames_drawn(), frames);
}

#[test]
fn number_keys_switch_screens() {
    let mut app = app();

    press(&mut app, "2");
    assert!(row(&app, 1).starts_with("┌Team Issues"));

    press(&mut app, "3");
    assert!(row(&app, 1).starts_with("┌Analytics"));

    press(&mut app, "1");
    assert!(row(&app, 1).starts_with("┌My Issues"));
}

#[test]
fn the_number_key_of_the_current_screen_draws_nothing() {
    let mut app = app();
    let frames = app.frames_drawn();

    press(&mut app, "1");

    assert_eq!(app.frames_drawn(), frames);
}

#[test]
fn a_list_binding_shadows_a_global_one_only_where_there_is_a_list() {
    let mut app = app_with("[keys.global]\nquit = \"<C-d>\"\n");
    let ctrl_d = || Event::Key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL));

    assert!(app.handle(ctrl_d()).unwrap().is_empty());
    press(&mut app, "3");
    assert_eq!(app.handle(ctrl_d()).unwrap(), vec![Effect::Quit]);
}

#[test]
fn shifted_letters_match_their_uppercase_binding() {
    let mut app = app_with(OVERRIDES);
    let shift_q = Event::Key(KeyEvent::new(KeyCode::Char('Q'), KeyModifiers::SHIFT));

    assert_eq!(app.handle(shift_q).unwrap(), vec![Effect::Quit]);
}

#[test]
fn shift_tab_matches_its_config_notation() {
    let mut app = app_with("[keys.global]\nquit = \"<S-Tab>\"\n");
    let back_tab = Event::Key(KeyEvent::new(KeyCode::BackTab, KeyModifiers::SHIFT));

    assert_eq!(app.handle(back_tab).unwrap(), vec![Effect::Quit]);
}

#[test]
fn key_releases_are_ignored() {
    let mut app = app();
    let mut release = KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE);
    release.kind = KeyEventKind::Release;

    assert!(app.handle(Event::Key(release)).unwrap().is_empty());
}

#[test]
fn a_malformed_keys_section_is_reported_with_file_and_line() {
    let app = app_with("[keys.list]\nmove-down = [\"n\"\nmove-up = \"e\"\n");

    assert_snapshot!(app.backend());
}

#[test]
fn the_default_keymap_works_after_a_malformed_config() {
    let mut app = app_with("[keys.list]\nmove-down = [\"n\"\n");

    press(&mut app, "?");
    assert!(row(&app, 2).contains("Keys"));
    app.handle(esc()).unwrap();
    assert_eq!(press(&mut app, "q"), vec![Effect::Quit]);
}

#[test]
fn a_broken_context_keeps_its_defaults_and_the_others_still_apply() {
    let config = "\
[keys.global]
quit = \"Q\"

[keys.list]
move-down = { prepend = [\"n\"] }
mvoe-up = \"e\"
";
    let mut app = app_with(config);

    assert_snapshot!(row(&app, 21));
    assert!(row(&app, 22).starts_with(" Move │ j down"));
    assert_eq!(press(&mut app, "Q"), vec![Effect::Quit]);
}

#[test]
fn every_problem_is_reported_and_the_notice_counts_the_rest() {
    let config = "\
[keys.list]
move-down = \"<Nope>\"
quit = \"x\"

[keys.everywhere]
";
    let loaded = config::parse(config, Path::new("config.toml"));
    let messages: Vec<String> = loaded.errors.iter().map(ToString::to_string).collect();

    assert_snapshot!(messages.join("\n"));
    let app = App::new(
        TestBackend::new(80, 24),
        State::default().with_config(&loaded),
    )
    .unwrap();
    let notice = row(&app, 20) + &row(&app, 21);
    assert!(notice.starts_with("config.toml:2: `<Nope>` is not a key"));
    assert!(notice.trim_end().ends_with("(and 2 more; see the log)"));
}

#[test]
fn the_next_key_press_clears_the_notice() {
    let mut app = app_with("[keys]\nlist = 3\n");

    assert!(row(&app, 21).starts_with("config.toml:2:"));
    press(&mut app, "x");
    assert!(row(&app, 22).starts_with(" Move │ j down"));
    assert!(row(&app, 21).starts_with("└"));
}

#[test]
fn a_missing_config_file_is_the_default_config() {
    let loaded = config::load(Path::new("/nonexistent/lrat/config.toml"));

    assert_eq!(loaded, config::Loaded::default());
}
