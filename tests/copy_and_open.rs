use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use insta::assert_snapshot;
use linear_rat::app::{App, Effect, Event, MyIssues, State};
use linear_rat::linear::demo;
use ratatui::backend::TestBackend;
use ratatui::style::Color;

fn app() -> App<TestBackend> {
    App::new(TestBackend::new(80, 24), State::demo()).unwrap()
}

fn press(app: &mut App<TestBackend>, keys: &str) -> Vec<Effect> {
    keys.chars()
        .flat_map(|c| {
            app.handle(Event::Key(KeyEvent::new(
                KeyCode::Char(c),
                KeyModifiers::NONE,
            )))
            .unwrap()
        })
        .collect()
}

/// The notice rows above the bottom bar, joined and trimmed.
fn notice(app: &App<TestBackend>) -> String {
    let buffer = app.backend().buffer();
    let rows = buffer.area.height;
    (rows.saturating_sub(4)..rows - 1)
        .map(|y| {
            (0..buffer.area.width)
                .map(|x| buffer[(x, y)].symbol())
                .collect::<String>()
        })
        .filter(|line| !line.starts_with(['│', '└', '┌']))
        .collect::<String>()
        .trim()
        .to_owned()
}

/// The color of the first cell of `text`, wherever it is on the screen.
fn fg_of(app: &App<TestBackend>, text: &str) -> Option<Color> {
    let buffer = app.backend().buffer();
    (0..buffer.area.height).find_map(|y| {
        let row: String = (0..buffer.area.width)
            .map(|x| buffer[(x, y)].symbol())
            .collect();
        let x = row[..row.find(text)?].chars().count();
        Some(buffer.cell((u16::try_from(x).ok()?, y))?.fg)
    })
}

#[test]
fn yi_copies_the_identifier_of_the_focused_issue() {
    let mut app = app();

    let effects = press(&mut app, "yi");

    assert_eq!(effects, vec![Effect::Copy("DES-34".to_owned())]);
    assert_eq!(notice(&app), "Copied DES-34");
}

#[test]
fn yu_copies_the_url_of_the_focused_issue() {
    let mut app = app();

    let effects = press(&mut app, "yu");

    assert_eq!(
        effects,
        vec![Effect::Copy(
            "https://linear.app/squeakworks/issue/DES-34".to_owned()
        )]
    );
    assert_eq!(notice(&app), "Copied the URL of DES-34");
}

#[test]
fn yb_copies_the_branch_name_of_the_focused_issue() {
    let mut app = app();

    let effects = press(&mut app, "yb");

    assert_eq!(
        effects,
        vec![Effect::Copy(
            "des-34-design-qa-for-the-autumn-release".to_owned()
        )]
    );
    assert_eq!(notice(&app), "Copied the branch name of DES-34");
}

#[test]
fn ym_copies_a_markdown_link_to_the_focused_issue() {
    let mut app = app();

    let effects = press(&mut app, "ym");

    assert_eq!(
        effects,
        vec![Effect::Copy(
            "[DES-34 Design QA for the autumn release](https://linear.app/squeakworks/issue/DES-34)"
                .to_owned()
        )]
    );
    assert_eq!(notice(&app), "Copied a Markdown link to DES-34");
}

#[test]
fn ym_escapes_brackets_and_backslashes_in_the_title() {
    let mut data = demo::workspace();
    let issue = data
        .issues
        .iter_mut()
        .find(|issue| issue.identifier == "DES-34")
        .unwrap();
    issue.title = r"Fix the [beta] parser \ edge case]".to_owned();
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
    let mut app = App::new(TestBackend::new(80, 24), state).unwrap();

    let effects = press(&mut app, "ym");

    assert_eq!(
        effects,
        vec![Effect::Copy(
            r"[DES-34 Fix the \[beta\] parser \\ edge case\]](https://linear.app/squeakworks/issue/DES-34)"
                .to_owned()
        )]
    );
}

#[test]
fn o_asks_to_open_the_focused_issue_in_the_browser() {
    let mut app = app();

    let effects = press(&mut app, "o");

    assert_eq!(
        effects,
        vec![Effect::OpenUrl(
            "https://linear.app/squeakworks/issue/DES-34".to_owned()
        )]
    );
    assert_eq!(notice(&app), "Opening DES-34 in the browser");
}

#[test]
fn copy_and_open_work_from_the_detail_too() {
    let mut app = app();
    app.handle(Event::Key(KeyEvent::new(
        KeyCode::Enter,
        KeyModifiers::NONE,
    )))
    .unwrap();

    let effects = press(&mut app, "yio");

    assert_eq!(
        effects,
        vec![
            Effect::Copy("DES-34".to_owned()),
            Effect::OpenUrl("https://linear.app/squeakworks/issue/DES-34".to_owned()),
        ]
    );
}

#[test]
fn nothing_is_copied_or_opened_without_a_focused_issue() {
    let mut app = App::new(TestBackend::new(80, 24), State::default()).unwrap();

    let effects = press(&mut app, "yiyuybymo");

    assert!(effects.is_empty(), "{effects:?}");
    assert_eq!(notice(&app), "");
}

#[test]
fn the_next_key_press_clears_the_copy_notice() {
    let mut app = app();

    press(&mut app, "yi");
    press(&mut app, "x");

    assert_eq!(notice(&app), "");
}

#[test]
fn an_effect_that_fails_shows_its_error_as_the_notice() {
    let mut app = app();

    app.handle(Event::EffectFailed(
        "could not open the browser with xdg-open: not found".to_owned(),
    ))
    .unwrap();

    assert_eq!(
        notice(&app),
        "could not open the browser with xdg-open: not found"
    );
}

#[test]
fn y_opens_which_key_with_the_four_copy_actions() {
    let mut app = app();

    press(&mut app, "y");

    assert_snapshot!(app.backend());
}

#[test]
fn nothing_is_copied_or_opened_on_a_screen_without_issues() {
    let mut app = app();
    press(&mut app, "2");

    let effects = press(&mut app, "yiyuybymo");

    assert!(effects.is_empty(), "{effects:?}");
    assert_eq!(notice(&app), "");
}

#[test]
fn which_key_mutes_copy_actions_without_a_focused_issue() {
    let mut app = app();
    press(&mut app, "2y");

    assert_eq!(fg_of(&app, "i  Copy the identifier"), Some(Color::DarkGray));
    assert_eq!(fg_of(&app, "Copy the identifier"), Some(Color::DarkGray));
}

#[test]
fn which_key_accents_copy_actions_with_a_focused_issue() {
    let mut app = app();
    press(&mut app, "y");

    assert_eq!(fg_of(&app, "i  Copy the identifier"), Some(Color::Cyan));
    assert_eq!(fg_of(&app, "Copy the identifier"), Some(Color::Reset));
}
