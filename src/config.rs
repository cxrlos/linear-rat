//! The config file, `~/.config/lrat/config.toml` (the XDG config path, on
//! macOS too). `docs/config.md` documents every setting.
//!
//! ```toml
//! [ui]
//! icons = "nerd"                         # or "unicode", the default
//! wide-layout-columns = 120
//!
//! [theme]
//! name = "nord"                          # a built-in, or themes/<name>.json
//!
//! [keys.global]
//! quit = "Q"                             # replace: Q is now quit's only key
//! show-cheatsheet = { append = ["<F1>"] }
//!
//! [keys.list]
//! move-down = { prepend = ["n"] }        # n first, j still works
//! noop = ["gg", "<C-d>"]                 # these keys do nothing, and leave every hint
//!
//! [sync]
//! active-poll-seconds = 15
//! idle-poll-seconds = 60
//! pause-when-unfocused = true
//! live = false
//!
//! [analytics]
//! default-range-weeks = 12               # 4, 12 or 26
//! stale-days = 14
//! ```
//!
//! A problem is reported with its file and line, and only the section it is
//! in falls back to its defaults: a syntax error resets its whole section,
//! `[keys]` included, and a bad value does too, except that a bad binding
//! resets only its `[keys]` context table. Unknown keys and tables are
//! reported and ignored.

use std::fs;
use std::io;
use std::ops::RangeInclusive;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::Context as _;
use etcetera::BaseStrategy;
use etcetera::base_strategy::Xdg;
use toml::Spanned;
use toml::de::{DeTable, DeValue};
use toml_parser::parser::{Event, EventKind};

use crate::analytics::{StaleThreshold, WeekRange};
use crate::keys::{self, Change, Context, KeySequence, Override, Target};
use crate::ui::icons::IconSet;
use crate::ui::theme::{self, Theme};

/// Everything read from the config file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    /// The viewer's changes to the default keymap, in file order.
    pub keys: Vec<Override>,
    /// `[ui]`.
    pub ui: UiSettings,
    /// The theme `[theme] name` picks, or the default when it does not
    /// load; [`Loaded::theme`] is that theme.
    pub theme: String,
    /// `[sync]`.
    pub sync: SyncSettings,
    /// `[analytics]`.
    pub analytics: AnalyticsSettings,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            keys: Vec::new(),
            ui: UiSettings::default(),
            theme: theme::DEFAULT.to_owned(),
            sync: SyncSettings::default(),
            analytics: AnalyticsSettings::default(),
        }
    }
}

/// `[ui]`: how lrat draws.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UiSettings {
    /// `icons`: `"unicode"` (default) or `"nerd"`.
    pub icons: IconSet,
    /// `wide-layout-columns`: from this many columns on, the detail shows
    /// beside the list; below it the focused pane fills the body. 120 by
    /// default.
    pub wide_layout_columns: u16,
}

impl Default for UiSettings {
    fn default() -> Self {
        Self {
            icons: IconSet::Unicode,
            wide_layout_columns: 120,
        }
    }
}

/// `[sync]`: how lrat keeps its data fresh: the poll intervals and pause,
/// and whether live updates run alongside the poll.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SyncSettings {
    /// `active-poll-seconds`: between polls while the viewer is active, 15
    /// by default.
    pub active_poll: Duration,
    /// `idle-poll-seconds`: between polls while the viewer is idle, 60 by
    /// default. Never shorter than `active_poll`.
    pub idle_poll: Duration,
    /// `pause-when-unfocused`: stop polling while the terminal reports focus
    /// lost, true by default.
    pub pause_when_unfocused: bool,
    /// `live`: apply live updates from Linear's websocket as well, false by
    /// default.
    pub live: bool,
}

impl Default for SyncSettings {
    fn default() -> Self {
        Self {
            active_poll: Duration::from_secs(15),
            idle_poll: Duration::from_secs(60),
            pause_when_unfocused: true,
            live: false,
        }
    }
}

/// `[analytics]`: what the Analytics screen starts with. Nothing reads it
/// until that screen lands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AnalyticsSettings {
    /// `default-range-weeks`: 4, 12 (default) or 26.
    pub default_range: WeekRange,
    /// `stale-days`: days without an update that make an open issue stale,
    /// 14 by default.
    pub stale_threshold: StaleThreshold,
}

impl Default for AnalyticsSettings {
    fn default() -> Self {
        Self {
            default_range: WeekRange::Twelve,
            stale_threshold: StaleThreshold::default(),
        }
    }
}

/// A config, the theme it picks, and the problems found while reading them.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Loaded {
    pub config: Config,
    pub theme: Theme,
    pub errors: Vec<ConfigError>,
}

/// A problem in the config file or a theme file, located by line where
/// there is one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigError {
    pub path: PathBuf,
    pub line: Option<usize>,
    pub message: String,
}

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let path = self.path.display();
        match self.line {
            Some(line) => write!(f, "{path}:{line}: {}", self.message),
            None => write!(f, "{path}: {}", self.message),
        }
    }
}

/// `$XDG_CONFIG_HOME/lrat/config.toml`, by default `~/.config/lrat/config.toml`.
pub fn path() -> anyhow::Result<PathBuf> {
    let xdg = Xdg::new().context("finding the home directory")?;
    Ok(xdg.config_dir().join("lrat").join("config.toml"))
}

/// Where the viewer's theme files live: `themes/` beside the config file.
pub fn themes_dir(config_path: &Path) -> PathBuf {
    config_path.with_file_name("themes")
}

/// Reads the config at `path`. A missing file is the default config.
pub fn load(path: &Path) -> Loaded {
    match fs::read_to_string(path) {
        Ok(text) => parse(&text, path),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Loaded::default(),
        Err(error) => Loaded {
            errors: vec![ConfigError {
                path: path.to_owned(),
                line: None,
                message: format!("could not read it ({error}); using the default config"),
            }],
            ..Loaded::default()
        },
    }
}

/// Reads config `text` as if it came from `path`, which errors name. A
/// theme that is not built in is read from [`themes_dir`] of `path`.
pub fn parse(text: &str, path: &Path) -> Loaded {
    let mut reader = Reader {
        text,
        path,
        errors: Vec::new(),
    };
    let (document, syntax_errors) = DeTable::parse_recoverable(text);
    let headers = headers(text);
    let mut broken: Vec<String> = Vec::new();
    let mut whole_file_broken = false;
    // Recovery after one mistake often reports more after it, so only the
    // first syntax error of each section, or of the file, is kept.
    for error in syntax_errors {
        let offset = error.span().map(|span| span.start);
        let table = offset.and_then(|offset| table_at(&headers, offset));
        if whole_file_broken || table.as_ref().is_some_and(|table| broken.contains(table)) {
            continue;
        }
        let fallback = match table.as_deref() {
            None => {
                whole_file_broken = true;
                "; using the default config".to_owned()
            }
            Some("keys") => "; [keys] uses the default keys".to_owned(),
            Some(name) if SECTIONS.contains(&name) => format!("; [{name}] uses its defaults"),
            Some(_) => String::new(),
        };
        reader.error(offset, format!("{}{fallback}", error.message().trim_end()));
        broken.extend(table);
    }

    let mut config = Config::default();
    let mut theme_name = None;
    if !whole_file_broken {
        for (name, value) in document.get_ref() {
            let name_str: &str = name.get_ref();
            if broken.iter().any(|table| table == name_str) {
                continue;
            }
            match name_str {
                "keys" => config.keys = reader.keys(value),
                "ui" => config.ui = reader.section("ui", value, read_ui),
                "theme" => {
                    let read = reader.section("theme", value, read_theme);
                    if let Some((name, span)) = read.name {
                        config.theme.clone_from(&name);
                        theme_name = Some((name, span));
                    }
                }
                "sync" => {
                    let sync = reader.section("sync", value, read_sync);
                    config.sync = if sync.idle_poll < sync.active_poll {
                        reader.error_at(
                            value,
                            "`sync.idle-poll-seconds` must not be shorter than \
                             `sync.active-poll-seconds`; [sync] uses its defaults"
                                .to_owned(),
                        );
                        SyncSettings::default()
                    } else {
                        sync
                    };
                }
                "analytics" => {
                    config.analytics = reader.section("analytics", value, read_analytics);
                }
                _ => {
                    let what = match value.get_ref() {
                        DeValue::Table(_) => format!("table `[{name_str}]`"),
                        _ => format!("key `{name_str}`"),
                    };
                    reader.error_at(
                        name,
                        format!(
                            "unknown {what} is ignored; lrat reads [ui], [theme], [keys], [sync] and [analytics]"
                        ),
                    );
                }
            }
        }
    }

    let theme = match theme_name {
        Some((name, span)) => match reader.theme(&name, span) {
            Some(theme) => theme,
            None => {
                config.theme = theme::DEFAULT.to_owned();
                Theme::default()
            }
        },
        None => Theme::default(),
    };

    let mut errors = reader.errors;
    errors.sort_by_key(|error| error.line);
    Loaded {
        config,
        theme,
        errors,
    }
}

/// The top-level tables lrat reads.
const SECTIONS: [&str; 5] = ["ui", "theme", "keys", "sync", "analytics"];

/// Where each table header starts and the top-level table it opens, in
/// file order. TOML's own parser finds them, so a header-like line inside a
/// string is not one and a quoted name such as `["ui"]` is the table it
/// names.
fn headers(text: &str) -> Vec<(usize, String)> {
    let source = toml_parser::Source::new(text);
    let tokens = source.lex().into_vec();
    let mut events: Vec<Event> = Vec::new();
    toml_parser::parser::parse_document(&tokens, &mut events, &mut ());
    let mut headers = Vec::new();
    let mut open = None;
    for event in &events {
        match event.kind() {
            EventKind::StdTableOpen | EventKind::ArrayTableOpen => {
                open = Some(event.span().start())
            }
            EventKind::SimpleKey => {
                if let (Some(start), Some(raw)) = (open.take(), source.get(event)) {
                    let mut name = String::new();
                    raw.decode_key(&mut name, &mut ());
                    headers.push((start, name));
                }
            }
            EventKind::StdTableClose | EventKind::ArrayTableClose | EventKind::Newline => {
                open = None;
            }
            _ => {}
        }
    }
    headers
}

/// The top-level table of the last header starting at or before `offset`,
/// so a syntax error breaks only the section it is in, and a repeated
/// header breaks the table it repeats. `None` before any header.
fn table_at(headers: &[(usize, String)], offset: usize) -> Option<String> {
    headers
        .iter()
        .rev()
        .find(|(start, _)| *start <= offset)
        .map(|(_, name)| name.clone())
}

/// One setting read from its value, or why the value is wrong. `None` for a
/// key the section does not have.
type Setting = Option<Result<(), String>>;

#[derive(Default)]
struct ThemeSection {
    name: Option<(String, usize)>,
}

fn read_ui(ui: &mut UiSettings, key: &str, value: &Spanned<DeValue>) -> Setting {
    Some(match key {
        "icons" => match value.get_ref().as_str().and_then(IconSet::from_name) {
            Some(icons) => {
                ui.icons = icons;
                Ok(())
            }
            None => Err("`ui.icons` must be \"unicode\" or \"nerd\"".to_owned()),
        },
        "wide-layout-columns" => integer(value, "ui.wide-layout-columns", 0..=u64::from(u16::MAX))
            .map(|columns| ui.wide_layout_columns = u16::try_from(columns).unwrap_or(u16::MAX)),
        _ => return None,
    })
}

fn read_theme(theme: &mut ThemeSection, key: &str, value: &Spanned<DeValue>) -> Setting {
    if key != "name" {
        return None;
    }
    Some(match value.get_ref().as_str() {
        Some(name)
            if !name.is_empty()
                && name
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') =>
        {
            theme.name = Some((name.to_owned(), value.span().start));
            Ok(())
        }
        _ => Err(format!(
            "`theme.name` must be a theme name such as \"nord\"; built in: {}",
            Theme::built_in_names().collect::<Vec<_>>().join(", ")
        )),
    })
}

fn read_sync(sync: &mut SyncSettings, key: &str, value: &Spanned<DeValue>) -> Setting {
    const SECONDS: RangeInclusive<u64> = 5..=3600;
    Some(match key {
        "active-poll-seconds" => integer(value, "sync.active-poll-seconds", SECONDS)
            .map(|seconds| sync.active_poll = Duration::from_secs(seconds)),
        "idle-poll-seconds" => integer(value, "sync.idle-poll-seconds", SECONDS)
            .map(|seconds| sync.idle_poll = Duration::from_secs(seconds)),
        "pause-when-unfocused" => boolean(value, "sync.pause-when-unfocused")
            .map(|pause| sync.pause_when_unfocused = pause),
        "live" => boolean(value, "sync.live").map(|live| sync.live = live),
        _ => return None,
    })
}

fn read_analytics(
    analytics: &mut AnalyticsSettings,
    key: &str,
    value: &Spanned<DeValue>,
) -> Setting {
    Some(match key {
        "default-range-weeks" => {
            let range = match integer(value, "analytics.default-range-weeks", 4..=26) {
                Ok(4) => Some(WeekRange::Four),
                Ok(12) => Some(WeekRange::Twelve),
                Ok(26) => Some(WeekRange::TwentySix),
                _ => None,
            };
            match range {
                Some(range) => {
                    analytics.default_range = range;
                    Ok(())
                }
                None => Err("`analytics.default-range-weeks` must be 4, 12 or 26".to_owned()),
            }
        }
        "stale-days" => integer(value, "analytics.stale-days", 1..=365).map(|days| {
            analytics.stale_threshold =
                StaleThreshold::days(u32::try_from(days).unwrap_or(u32::MAX));
        }),
        _ => return None,
    })
}

fn integer(value: &Spanned<DeValue>, key: &str, range: RangeInclusive<u64>) -> Result<u64, String> {
    value
        .get_ref()
        .as_integer()
        .and_then(|integer| u64::from_str_radix(integer.as_str(), integer.radix()).ok())
        .filter(|number| range.contains(number))
        .ok_or_else(|| {
            format!(
                "`{key}` must be a whole number from {} to {}",
                range.start(),
                range.end()
            )
        })
}

fn boolean(value: &Spanned<DeValue>, key: &str) -> Result<bool, String> {
    value
        .get_ref()
        .as_bool()
        .ok_or_else(|| format!("`{key}` must be true or false"))
}

struct Reader<'a> {
    text: &'a str,
    path: &'a Path,
    errors: Vec<ConfigError>,
}

/// An error already recorded in [`Reader::errors`].
struct Reported;

impl Reader<'_> {
    /// Reads the settings of table `[name]` with `read`. Any bad value
    /// leaves the whole section at its defaults; unknown keys are only
    /// reported.
    fn section<T: Default>(
        &mut self,
        name: &str,
        value: &Spanned<DeValue>,
        read: fn(&mut T, &str, &Spanned<DeValue>) -> Setting,
    ) -> T {
        let DeValue::Table(entries) = value.get_ref() else {
            self.error_at(
                value,
                format!("`{name}` must be a table, as in `[{name}]`; [{name}] uses its defaults"),
            );
            return T::default();
        };
        let mut section = T::default();
        let mut broken = false;
        for (key, value) in entries {
            let key_str: &str = key.get_ref();
            match read(&mut section, key_str, value) {
                Some(Ok(())) => {}
                Some(Err(message)) => {
                    self.error_at(value, format!("{message}; [{name}] uses its defaults"));
                    broken = true;
                }
                None => {
                    self.error_at(
                        key,
                        format!("unknown key `{key_str}` in [{name}] is ignored"),
                    );
                }
            }
        }
        if broken { T::default() } else { section }
    }

    /// The theme called `name`, set at byte `offset` of the config: a
    /// built-in, or a Base16 file in the themes directory. `None`, with the
    /// problem reported, when neither loads.
    fn theme(&mut self, name: &str, offset: usize) -> Option<Theme> {
        if let Some(theme) = Theme::built_in(name) {
            return Some(theme);
        }
        let path = themes_dir(self.path).join(format!("{name}.json"));
        match fs::read_to_string(&path) {
            Ok(json) => match Theme::from_base16_json(&json) {
                Ok(theme) => Some(theme),
                Err(error) => {
                    self.errors.push(ConfigError {
                        path,
                        line: error.line,
                        message: format!("{error}; using the {} theme", theme::DEFAULT),
                    });
                    None
                }
            },
            Err(error) => {
                let why = if error.kind() == io::ErrorKind::NotFound {
                    format!("it is not built in and there is no {}", path.display())
                } else {
                    format!("could not read {} ({error})", path.display())
                };
                self.error(
                    Some(offset),
                    format!(
                        "no theme `{name}`: {why}; using the {} theme",
                        theme::DEFAULT
                    ),
                );
                None
            }
        }
    }

    fn keys(&mut self, keys: &Spanned<DeValue>) -> Vec<Override> {
        let DeValue::Table(contexts) = keys.get_ref() else {
            self.error_at(
                keys,
                "`keys` must be a table, as in `[keys.list]`".to_owned(),
            );
            return Vec::new();
        };
        let mut overrides = Vec::new();
        for (name, table) in contexts {
            let Some(context) = Context::from_name(name.get_ref()) else {
                self.error_at(
                    name,
                    format!(
                        "unknown context `[keys.{name}]`; expected global, list, detail or overlay"
                    ),
                );
                continue;
            };
            let before = self.errors.len();
            let found = self.context(context, table);
            if self.errors.len() == before {
                overrides.extend(found);
            } else {
                for error in &mut self.errors[before..] {
                    error.message.push_str(&format!(
                        "; [keys.{}] uses the default keys",
                        context.name()
                    ));
                }
            }
        }
        overrides
    }

    fn context(&mut self, context: Context, table: &Spanned<DeValue>) -> Vec<Override> {
        let DeValue::Table(entries) = table.get_ref() else {
            self.error_at(table, format!("`keys.{}` must be a table", context.name()));
            return Vec::new();
        };
        let mut overrides = Vec::new();
        for (id, value) in entries {
            let id_str: &str = id.get_ref();
            let target = if id_str == "noop" {
                Target::Noop
            } else {
                match keys::by_id(id_str) {
                    Some(spec) if spec.contexts.contains(&context) => Target::Action(spec.action),
                    Some(_) => {
                        self.error_at(
                            id,
                            format!("`{id_str}` does not apply in [keys.{}]", context.name()),
                        );
                        continue;
                    }
                    None => {
                        self.error_at(id, format!("unknown action `{id_str}`"));
                        continue;
                    }
                }
            };
            let mut add = |change, keys| {
                overrides.push(Override {
                    context,
                    target,
                    change,
                    keys,
                });
            };
            match value.get_ref() {
                DeValue::Table(changes) if target != Target::Noop => {
                    for (change, keys) in changes {
                        let change = match change.get_ref().as_ref() {
                            "prepend" => Change::Prepend,
                            "append" => Change::Append,
                            other => {
                                self.error_at(
                                    change,
                                    format!("`{other}` is not `prepend` or `append`"),
                                );
                                continue;
                            }
                        };
                        if let Ok(keys) = self.sequences(keys) {
                            add(change, keys);
                        }
                    }
                }
                _ => {
                    if let Ok(keys) = self.sequences(value) {
                        add(Change::Replace, keys);
                    }
                }
            }
        }
        overrides
    }

    fn sequences(&mut self, value: &Spanned<DeValue>) -> Result<Vec<KeySequence>, Reported> {
        match value.get_ref() {
            DeValue::String(_) => Ok(vec![self.sequence(value)?]),
            DeValue::Array(items) => items.iter().map(|item| self.sequence(item)).collect(),
            _ => Err(self.error_at(value, "expected a key or a list of keys".to_owned())),
        }
    }

    fn sequence(&mut self, value: &Spanned<DeValue>) -> Result<KeySequence, Reported> {
        let Some(notation) = value.get_ref().as_str() else {
            return Err(self.error_at(
                value,
                "expected a key such as \"j\" or \"<C-d>\"".to_owned(),
            ));
        };
        notation
            .parse()
            .map_err(|error| self.error_at(value, format!("{error}")))
    }

    fn error_at<T>(&mut self, spanned: &Spanned<T>, message: String) -> Reported {
        self.error(Some(spanned.span().start), message)
    }

    fn error(&mut self, offset: Option<usize>, message: String) -> Reported {
        let line = offset.map(|offset| {
            let before = self.text.get(..offset).unwrap_or(self.text);
            before.matches('\n').count() + 1
        });
        self.errors.push(ConfigError {
            path: self.path.to_owned(),
            line,
            message,
        });
        Reported
    }
}
