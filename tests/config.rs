use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use linear_rat::analytics::{StaleThreshold, WeekRange};
use linear_rat::config::{self, Loaded};
use linear_rat::ui::icons::IconSet;
use linear_rat::ui::theme::Theme;
use ratatui::style::Color;

fn parse(text: &str) -> Loaded {
    config::parse(text, Path::new("config.toml"))
}

fn messages(loaded: &Loaded) -> Vec<String> {
    loaded.errors.iter().map(ToString::to_string).collect()
}

/// A fresh config directory for one test, holding `config.toml` and the
/// given theme files.
fn config_dir(test: &str, config: &str, themes: &[(&str, &str)]) -> std::io::Result<PathBuf> {
    let dir = std::env::temp_dir().join(format!("lrat-{test}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(dir.join("themes"))?;
    fs::write(dir.join("config.toml"), config)?;
    for (name, json) in themes {
        fs::write(dir.join("themes").join(name), json)?;
    }
    Ok(dir)
}

const MINE: &str = r#"{
  "name": "Mine",
  "base00": "101010", "base01": "202020", "base02": "303030", "base03": "404040",
  "base04": "b0b0b0", "base05": "e0e0e0", "base06": "f0f0f0", "base07": "ffffff",
  "base08": "ff5555", "base09": "ffaa55", "base0A": "ffff55", "base0B": "55ff55",
  "base0C": "55ffff", "base0D": "5555ff", "base0E": "ff55ff", "base0F": "aa5500"
}"#;

#[test]
fn an_empty_config_is_every_documented_default() {
    let loaded = parse("");

    assert_eq!(loaded.errors, vec![]);
    let config = loaded.config;
    assert_eq!(config.ui.icons, IconSet::Unicode);
    assert_eq!(config.ui.wide_layout_columns, 120);
    assert_eq!(config.theme, "terminal");
    assert_eq!(config.sync.active_poll, Duration::from_secs(15));
    assert_eq!(config.sync.idle_poll, Duration::from_secs(60));
    assert!(config.sync.pause_when_unfocused);
    assert!(!config.sync.live);
    assert_eq!(config.analytics.default_range, WeekRange::Twelve);
    assert_eq!(config.analytics.stale_threshold, StaleThreshold::days(14));
    assert_eq!(loaded.theme, Theme::default());
}

#[test]
fn ui_icons_picks_the_nerd_font_set() {
    let loaded = parse("[ui]\nicons = \"nerd\"\n");

    assert_eq!(loaded.errors, vec![]);
    assert_eq!(loaded.config.ui.icons, IconSet::Nerd);
}

#[test]
fn every_section_reads_its_settings() {
    let loaded = parse(
        r#"
[ui]
icons = "nerd"
wide-layout-columns = 100

[theme]
name = "nord"

[sync]
active-poll-seconds = 30
idle-poll-seconds = 300
pause-when-unfocused = false
live = true

[analytics]
default-range-weeks = 26
stale-days = 30
"#,
    );

    assert_eq!(messages(&loaded), Vec::<String>::new());
    let config = loaded.config;
    assert_eq!(config.ui.icons, IconSet::Nerd);
    assert_eq!(config.ui.wide_layout_columns, 100);
    assert_eq!(config.theme, "nord");
    assert_eq!(config.sync.active_poll, Duration::from_secs(30));
    assert_eq!(config.sync.idle_poll, Duration::from_secs(300));
    assert!(!config.sync.pause_when_unfocused);
    assert!(config.sync.live);
    assert_eq!(config.analytics.default_range, WeekRange::TwentySix);
    assert_eq!(config.analytics.stale_threshold, StaleThreshold::days(30));
    assert_eq!(Some(loaded.theme), Theme::built_in("nord"));
}

#[test]
fn a_bad_value_leaves_only_its_section_at_the_defaults() {
    let loaded = parse(
        r#"
[ui]
icons = "nerd"

[sync]
active-poll-seconds = 30
live = "yes"

[analytics]
default-range-weeks = 8
stale-days = 30
"#,
    );

    assert_eq!(
        messages(&loaded),
        vec![
            "config.toml:7: `sync.live` must be true or false; [sync] uses its defaults",
            "config.toml:10: `analytics.default-range-weeks` must be 4, 12 or 26; \
             [analytics] uses its defaults",
        ]
    );
    assert_eq!(loaded.config.ui.icons, IconSet::Nerd);
    assert_eq!(loaded.config.sync.active_poll, Duration::from_secs(15));
    assert_eq!(
        loaded.config.analytics.stale_threshold,
        StaleThreshold::days(14)
    );
}

#[test]
fn a_syntax_error_breaks_only_its_section() {
    let loaded = parse(
        r#"
[ui]
icons = "nerd"
wide-layout-columns = 100 100

[sync]
live = true
"#,
    );

    assert_eq!(loaded.errors.len(), 1, "{:?}", messages(&loaded));
    let message = messages(&loaded).join("");
    assert!(message.starts_with("config.toml:4: "), "{message}");
    assert!(message.ends_with("; [ui] uses its defaults"), "{message}");
    assert_eq!(loaded.config.ui.icons, IconSet::Unicode);
    assert!(loaded.config.sync.live);
}

#[test]
fn a_repeated_table_is_the_one_that_falls_back() {
    let loaded = parse(
        r#"
[ui]
icons = "nerd"

[theme]
name = "nord"

[ui]
wide-layout-columns = 90
"#,
    );

    let message = messages(&loaded).join("\n");
    assert!(message.starts_with("config.toml:8: "), "{message}");
    assert!(message.ends_with("; [ui] uses its defaults"), "{message}");
    assert_eq!(loaded.config.theme, "nord");
    assert_eq!(loaded.theme, Theme::built_in("nord").unwrap());
    assert_eq!(loaded.config.ui.icons, IconSet::Unicode);
    assert_eq!(loaded.config.ui.wide_layout_columns, 120);
}

#[test]
fn a_syntax_error_in_one_keys_context_resets_every_context() {
    let loaded = parse(
        r#"
[keys.global]
quit = "Q"

[keys.list]
move-down = ["n" "j"]

[sync]
live = true
"#,
    );

    let message = messages(&loaded).join("\n");
    assert!(message.starts_with("config.toml:6: "), "{message}");
    assert!(
        message.ends_with("; [keys] uses the default keys"),
        "{message}"
    );
    assert!(loaded.config.keys.is_empty());
    assert!(loaded.config.sync.live);
}

#[test]
fn a_header_inside_a_multi_line_string_is_not_a_table() {
    let loaded = parse(
        r#"
[sync]
live = true

[analytics]
note = """
[sync]
"""
stale-days = 30 30
"#,
    );

    let message = messages(&loaded).join("\n");
    assert!(message.starts_with("config.toml:9: "), "{message}");
    assert!(
        message.ends_with("; [analytics] uses its defaults"),
        "{message}"
    );
    assert!(loaded.config.sync.live);
    assert_eq!(
        loaded.config.analytics.stale_threshold,
        StaleThreshold::default()
    );
}

#[test]
fn a_quoted_table_name_is_the_table_it_names() {
    let loaded = parse(
        r#"
[sync]
live = true

["ui"]
icons = "nerd" "nerd"
"#,
    );

    let message = messages(&loaded).join("\n");
    assert!(message.starts_with("config.toml:6: "), "{message}");
    assert!(message.ends_with("; [ui] uses its defaults"), "{message}");
    assert!(loaded.config.sync.live);
    assert_eq!(loaded.config.ui.icons, IconSet::Unicode);
}

#[test]
fn a_syntax_error_before_any_table_uses_the_default_config() {
    let loaded = parse("live = \n\n[sync]\nlive = true\n");

    let message = messages(&loaded).join("\n");
    assert!(message.starts_with("config.toml:1: "), "{message}");
    assert!(message.ends_with("; using the default config"), "{message}");
    assert!(!loaded.config.sync.live);
}

#[test]
fn an_unknown_key_is_reported_and_the_rest_of_its_section_applies() {
    let loaded = parse("[sync]\nlive = true\npoll = 3\n");

    assert_eq!(
        messages(&loaded),
        vec!["config.toml:3: unknown key `poll` in [sync] is ignored"]
    );
    assert!(loaded.config.sync.live);
}

#[test]
fn a_misspelt_table_is_reported_instead_of_ignored() {
    let loaded = parse("[key.global]\nquit = \"Q\"\n\n[ui]\nicons = \"nerd\"\n");

    assert_eq!(
        messages(&loaded),
        vec![
            "config.toml:1: unknown table `[key]` is ignored; lrat reads [ui], [theme], \
             [keys], [sync] and [analytics]"
        ]
    );
    assert_eq!(loaded.config.keys, vec![]);
    assert_eq!(loaded.config.ui.icons, IconSet::Nerd);
}

#[test]
fn an_idle_poll_shorter_than_the_active_one_is_rejected() {
    let loaded = parse("[sync]\nactive-poll-seconds = 60\nidle-poll-seconds = 30\n");

    assert_eq!(
        messages(&loaded),
        vec![
            "config.toml:1: `sync.idle-poll-seconds` must not be shorter than \
             `sync.active-poll-seconds`; [sync] uses its defaults"
        ]
    );
    assert_eq!(loaded.config.sync.active_poll, Duration::from_secs(15));
}

#[test]
fn an_out_of_range_number_names_the_range() {
    let loaded = parse("[analytics]\nstale-days = 0\n");

    assert_eq!(
        messages(&loaded),
        vec![
            "config.toml:2: `analytics.stale-days` must be a whole number from 1 to 365; \
             [analytics] uses its defaults"
        ]
    );
}

#[test]
fn a_user_theme_file_is_picked_up_by_name() -> std::io::Result<()> {
    let dir = config_dir(
        "user-theme",
        "[theme]\nname = \"mine\"\n",
        &[("mine.json", MINE)],
    )?;

    let loaded = config::load(&dir.join("config.toml"));
    fs::remove_dir_all(&dir)?;

    assert_eq!(loaded.errors, vec![]);
    assert_eq!(loaded.config.theme, "mine");
    assert_eq!(loaded.theme.background, Color::Rgb(0x10, 0x10, 0x10));
    assert_eq!(loaded.theme.text, Color::Rgb(0xe0, 0xe0, 0xe0));
    assert_eq!(loaded.theme.accent, Color::Rgb(0x55, 0xff, 0xff));
    Ok(())
}

#[test]
fn a_broken_theme_file_falls_back_to_the_default_naming_file_and_line() -> std::io::Result<()> {
    let broken = MINE.replace("\"55ffff\"", "\"55fffg\"");
    let dir = config_dir(
        "broken-theme",
        "[theme]\nname = \"mine\"\n",
        &[("mine.json", &broken)],
    )?;

    let loaded = config::load(&dir.join("config.toml"));
    fs::remove_dir_all(&dir)?;

    let theme_file = dir.join("themes").join("mine.json");
    assert_eq!(
        messages(&loaded),
        vec![format!(
            "{}:6: `55fffg` is not a hex color such as \"1e1e2e\"; using the terminal theme",
            theme_file.display()
        )]
    );
    assert_eq!(loaded.config.theme, "terminal");
    assert_eq!(loaded.theme, Theme::default());
    Ok(())
}

#[test]
fn a_theme_that_is_neither_built_in_nor_a_file_is_reported() {
    let loaded = config::parse(
        "[theme]\nname = \"nope\"\n",
        Path::new("/nonexistent/lrat/config.toml"),
    );

    assert_eq!(
        messages(&loaded),
        vec![
            "/nonexistent/lrat/config.toml:2: no theme `nope`: it is not built in and there \
             is no /nonexistent/lrat/themes/nope.json; using the terminal theme"
        ]
    );
    assert_eq!(loaded.theme, Theme::default());
}

#[test]
fn a_theme_name_cannot_reach_outside_the_themes_directory() {
    let loaded = parse("[theme]\nname = \"../secrets\"\n");

    let message = messages(&loaded).join("\n");
    assert!(
        message.starts_with("config.toml:2: `theme.name` must be a theme name"),
        "{message}"
    );
    assert_eq!(loaded.theme, Theme::default());
}
