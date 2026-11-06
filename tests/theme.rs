use std::collections::HashSet;
use std::path::Path;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use insta::assert_debug_snapshot;
use linear_rat::app::{App, Event, State};
use linear_rat::config;
use linear_rat::linear::model::{Priority, StateType};
use linear_rat::ui::icons::IconSet;
use linear_rat::ui::theme::{Theme, contrast_ratio};
use ratatui::backend::TestBackend;
use ratatui::style::Color;

/// WCAG 2's minimum for body text.
const TEXT_MIN_CONTRAST: f64 = 4.5;
/// WCAG 2's minimum for large text and interface parts: muted text, borders,
/// glyphs and the accent.
const UI_MIN_CONTRAST: f64 = 3.0;

const STATE_TYPES: [StateType; 6] = [
    StateType::Triage,
    StateType::Backlog,
    StateType::Unstarted,
    StateType::Started,
    StateType::Completed,
    StateType::Canceled,
];

const PRIORITIES: [Priority; 5] = [
    Priority::Urgent,
    Priority::High,
    Priority::Medium,
    Priority::Low,
    Priority::None,
];

fn demo_with(config: &str, columns: u16) -> App<TestBackend> {
    let loaded = config::parse(config, Path::new("config.toml"));
    assert_eq!(loaded.errors, vec![], "{config}");
    App::new(
        TestBackend::new(columns, 24),
        State::demo().with_config(&loaded),
    )
    .unwrap()
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

#[test]
fn there_are_the_default_and_ten_base16_built_in_themes() {
    let names: Vec<&str> = Theme::built_in_names().collect();

    assert_eq!(
        names,
        [
            "terminal",
            "catppuccin-mocha",
            "dracula",
            "everforest-dark",
            "gruvbox-dark",
            "kanagawa-wave",
            "nord",
            "one-dark",
            "rose-pine",
            "solarized-dark",
            "tokyo-night",
        ]
    );
    for name in names {
        assert!(Theme::built_in(name).is_some(), "{name} does not load");
    }
}

#[test]
fn every_base16_built_in_theme_keeps_its_roles_legible_on_its_background() {
    for name in Theme::built_in_names().skip(1) {
        let Some(theme) = Theme::built_in(name) else {
            panic!("{name} does not load");
        };
        let ratio = |color: Color| contrast_ratio(color, theme.background).unwrap_or(0.0);

        assert!(ratio(theme.text) >= TEXT_MIN_CONTRAST, "{name} text");
        let mut roles = vec![
            ("muted", theme.muted),
            ("accent", theme.accent),
            ("error", theme.error),
            ("border", theme.border),
        ];
        roles.extend(STATE_TYPES.map(|state_type| ("state", theme.state(state_type))));
        roles.extend(PRIORITIES.map(|priority| ("priority", theme.priority(priority))));
        for (role, color) in roles {
            assert!(
                ratio(color) >= UI_MIN_CONTRAST,
                "{name} {role}: {:.2}",
                ratio(color)
            );
        }
    }
}

#[test]
fn contrast_ratio_matches_the_wcag_extremes() {
    let black = Color::Rgb(0, 0, 0);
    let white = Color::Rgb(255, 255, 255);

    assert_eq!(contrast_ratio(black, white), Some(21.0));
    assert_eq!(contrast_ratio(white, white), Some(1.0));
    assert_eq!(contrast_ratio(Color::Cyan, white), None);
}

#[test]
fn every_built_in_theme_renders_the_demo_my_issues_screen() {
    for name in Theme::built_in_names() {
        let app = demo_with(&format!("[theme]\nname = \"{name}\"\n"), 80);

        assert_debug_snapshot!(format!("my_issues_{name}"), app.backend().buffer());
    }
}

#[test]
fn a_base16_theme_paints_its_background_and_the_default_keeps_the_terminals() {
    let nord = demo_with("[theme]\nname = \"nord\"\n", 80);
    let terminal = demo_with("", 80);

    let nord_cells = nord.backend().buffer().content();
    assert!(
        nord_cells
            .iter()
            .all(|cell| cell.bg == Color::Rgb(0x2e, 0x34, 0x40)),
        "a cell of the nord theme is not on base00"
    );
    assert!(
        terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .all(|cell| cell.bg == Color::Reset)
    );
}

#[test]
fn popups_keep_the_theme_background() {
    let mut app = demo_with("[theme]\nname = \"nord\"\n", 80);

    app.handle(Event::Key(KeyEvent::new(
        KeyCode::Char('?'),
        KeyModifiers::NONE,
    )))
    .unwrap();

    assert!(screen(&app).iter().any(|row| row.contains("Keys")));
    let base00 = Color::Rgb(0x2e, 0x34, 0x40);
    assert!(
        app.backend()
            .buffer()
            .content()
            .iter()
            .all(|cell| cell.bg == base00)
    );
}

#[test]
fn the_wide_layout_starts_at_the_configured_width() {
    let wide = demo_with("[ui]\nwide-layout-columns = 80\n", 80);
    let narrow = demo_with("[ui]\nwide-layout-columns = 200\n", 120);

    assert!(screen(&wide)[1].contains("DES-34"), "{:#?}", screen(&wide));
    assert_eq!(screen(&wide)[1].matches(['┐', '┓']).count(), 2);
    assert_eq!(screen(&narrow)[1].matches(['┐', '┓']).count(), 1);
}

#[test]
fn the_nerd_font_set_draws_its_glyphs() {
    let app = demo_with("[ui]\nicons = \"nerd\"\n", 80);

    let rows = screen(&app);
    assert!(rows[2].starts_with("│\u{f042} Started (9)"), "{}", rows[2]);
    assert!(
        rows[3].contains("\u{f042} In Progress \u{f102} High"),
        "{}",
        rows[3]
    );
}

#[test]
fn no_two_state_types_or_priorities_share_a_glyph_in_either_icon_set() {
    for icons in IconSet::ALL {
        let states: HashSet<&str> = STATE_TYPES.iter().map(|&s| icons.state(s)).collect();
        let priorities: HashSet<&str> = PRIORITIES.iter().map(|&p| icons.priority(p)).collect();

        assert_eq!(states.len(), STATE_TYPES.len(), "{icons:?} state glyphs");
        assert_eq!(
            priorities.len(),
            PRIORITIES.len(),
            "{icons:?} priority glyphs"
        );
        assert!(
            states
                .iter()
                .chain(&priorities)
                .all(|glyph| !glyph.trim().is_empty()),
            "{icons:?} has a blank glyph"
        );
    }
}
