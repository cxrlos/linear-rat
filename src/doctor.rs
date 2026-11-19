//! `lrat doctor`: one line per check of the key, Linear, the terminal, the
//! config and the version, each warning or failure followed by how to fix
//! it.
//!
//! [`gather`] collects [`Facts`] through the [`LinearClient`] trait and an
//! injected environment; [`report`] turns them into checks and is pure.

use std::fmt::Write as _;
use std::path::PathBuf;
use std::process::{Command, Stdio};

use jiff::{SignedDuration, Timestamp};
use semver::Version;

use crate::auth::{self, ApiKey, AuthError, Credentials, KEY_VAR, KeySource, KeyStore};
use crate::clipboard;
use crate::config::{self, ConfigError, Loaded};
use crate::linear::model::{Viewer, Workspace};
use crate::linear::{ClientError, Diagnostics, LinearClient};
use crate::release::{self, Channel, Off, ReleaseSource};
use crate::ui::theme;

/// A clock further than this from Linear's is reported.
const CLOCK_TOLERANCE: SignedDuration = SignedDuration::from_secs(30);

/// How a check came out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Ok,
    Warn,
    Fail,
}

impl Status {
    fn marker(self) -> &'static str {
        match self {
            Self::Ok => "●",
            Self::Warn => "▲",
            Self::Fail => "✘",
        }
    }

    fn color(self) -> &'static str {
        match self {
            Self::Ok => "\x1b[32m",
            Self::Warn => "\x1b[33m",
            Self::Fail => "\x1b[31m",
        }
    }
}

/// One line of the report. Every warning and failure carries a fix.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Check {
    pub status: Status,
    pub name: &'static str,
    pub detail: String,
    pub fix: Option<String>,
}

impl Check {
    fn ok(name: &'static str, detail: impl Into<String>) -> Self {
        Self {
            status: Status::Ok,
            name,
            detail: detail.into(),
            fix: None,
        }
    }

    fn warn(name: &'static str, detail: impl Into<String>, fix: impl Into<String>) -> Self {
        Self {
            status: Status::Warn,
            name,
            detail: detail.into(),
            fix: Some(fix.into()),
        }
    }

    fn fail(name: &'static str, detail: impl Into<String>, fix: impl Into<String>) -> Self {
        Self {
            status: Status::Fail,
            name,
            detail: detail.into(),
            fix: Some(fix.into()),
        }
    }
}

/// Every check, in the order printed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Report {
    pub checks: Vec<Check>,
}

impl Report {
    /// Whether any check failed, which makes `lrat doctor` exit non-zero.
    pub fn failed(&self) -> bool {
        self.checks.iter().any(|check| check.status == Status::Fail)
    }

    /// The report as printed: one line per check, its fix indented below it.
    /// With `color`, the markers are green, yellow and red.
    pub fn render(&self, color: bool) -> String {
        let width = self
            .checks
            .iter()
            .map(|check| check.name.chars().count())
            .max()
            .unwrap_or_default();
        let mut out = String::new();
        for check in &self.checks {
            let marker = check.status.marker();
            let marker = if color {
                format!("{}{marker}\x1b[0m", check.status.color())
            } else {
                marker.to_owned()
            };
            let _ = writeln!(
                out,
                "{marker} {:width$}  {}",
                check.name,
                check.detail,
                width = width
            );
            if let Some(fix) = &check.fix {
                let _ = writeln!(out, "  {:width$}  fix: {fix}", "", width = width);
            }
        }
        out
    }
}

/// Whether `lrat doctor` colors its output: only on a terminal, and only
/// while `NO_COLOR` is unset or empty.
pub fn use_color(stdout_is_terminal: bool, no_color: Option<&str>) -> bool {
    stdout_is_terminal && no_color.is_none_or(str::is_empty)
}

/// Everything the checks look at.
#[derive(Debug)]
pub struct Facts {
    /// Where the key comes from, or why there is none.
    pub key: Result<KeySource, AuthError>,
    pub linear: Linear,
    pub terminal: Terminal,
    pub config: ConfigFacts,
    pub version: VersionFacts,
    /// The local clock right after Linear's dated diagnostics response.
    pub now: Timestamp,
}

/// What Linear said about the key.
#[derive(Debug)]
pub enum Linear {
    /// No key resolved, so Linear was not asked.
    NotAsked,
    /// Linear refused the key.
    Rejected,
    /// The viewer could not be loaded for another reason.
    Unreachable(String),
    Reached {
        viewer: Viewer,
        workspace: Workspace,
        diagnostics: Result<Diagnostics, String>,
        /// How many teams the viewer can administer.
        administrable_teams: Result<usize, String>,
    },
}

/// The terminal, as its environment describes it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Terminal {
    pub colorterm: Option<String>,
    pub term: Option<String>,
    /// `None` outside tmux.
    pub tmux: Option<Tmux>,
}

/// What tmux says about the settings lrat relies on.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Tmux {
    /// Why tmux would not pass an OSC 52 copy on, as
    /// [`clipboard::tmux_problem`] says.
    pub clipboard_problem: Option<String>,
    /// tmux's `focus-events`, or `None` when tmux could not be asked.
    pub focus_events: Option<bool>,
}

/// The config file and what loading it found.
#[derive(Debug)]
pub struct ConfigFacts {
    /// Where the file is, or why that is unknown.
    pub path: Result<PathBuf, String>,
    pub exists: bool,
    pub loaded: Loaded,
}

/// The running version and the latest release, asked for live, bypassing
/// the release cache.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VersionFacts {
    pub running: Version,
    pub channel: Channel,
    pub latest: Result<Version, NotChecked>,
}

/// Why the latest release is unknown.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NotChecked {
    Off(Off),
    /// The check failed, with why.
    Failed(String),
}

impl VersionFacts {
    /// Asks `source` for the latest release, unless `off` turned the check
    /// off, for an install through `channel`.
    pub async fn check(source: &ReleaseSource, off: Option<Off>, channel: Channel) -> Self {
        let latest = match off {
            Some(off) => Err(NotChecked::Off(off)),
            None => source
                .latest()
                .await
                .map_err(|error| NotChecked::Failed(format!("{error:#}"))),
        };
        Self {
            running: release::running(),
            channel,
            latest,
        }
    }
}

/// Collects the facts: the key from `credentials`, Linear through the client
/// `connect` makes, the terminal from `var`, the config and `version`. `now` is read
/// right after the diagnostics response, the last request, so the clock
/// offset leaves out the time spent on the other requests.
pub async fn gather<S, C>(
    credentials: &Credentials<S>,
    connect: impl FnOnce(&ApiKey) -> Result<C, ClientError>,
    terminal: Terminal,
    config: ConfigFacts,
    version: VersionFacts,
    now: impl FnOnce() -> Timestamp,
) -> Facts
where
    S: KeyStore,
    C: LinearClient,
{
    let (key, linear) = match credentials.resolve() {
        Ok((api_key, source)) => (Ok(source), ask_linear(&api_key, connect).await),
        Err(error) => (Err(error), Linear::NotAsked),
    };
    Facts {
        key,
        linear,
        terminal,
        config,
        version,
        now: now(),
    }
}

async fn ask_linear<C: LinearClient>(
    key: &ApiKey,
    connect: impl FnOnce(&ApiKey) -> Result<C, ClientError>,
) -> Linear {
    let client = match connect(key) {
        Ok(client) => client,
        Err(error) => return Linear::Unreachable(chain(&error)),
    };
    let (viewer, workspace) = match client.viewer().await {
        Ok(found) => found,
        Err(ClientError::InvalidKey) => return Linear::Rejected,
        Err(error) => return Linear::Unreachable(chain(&error)),
    };
    let administrable_teams = client
        .administrable_teams()
        .await
        .map(|teams| teams.len())
        .map_err(|error| chain(&error));
    // Last, so the caller reads the local clock right after the response whose
    // Date header the clock check compares it with.
    let diagnostics = client.diagnostics().await.map_err(|error| chain(&error));
    Linear::Reached {
        viewer,
        workspace,
        diagnostics,
        administrable_teams,
    }
}

impl Terminal {
    /// The terminal described by `var`, asking tmux about its settings when
    /// `TMUX` is set.
    pub fn from_env(var: impl Fn(&str) -> Option<String>) -> Self {
        let tmux = var("TMUX").map(|_| Tmux {
            clipboard_problem: clipboard::tmux_problem(),
            focus_events: tmux_option("focus-events").map(|value| value == "on"),
        });
        Self {
            colorterm: var("COLORTERM"),
            term: var("TERM"),
            tmux,
        }
    }
}

/// tmux's global `option`, or `None` when tmux cannot be asked.
fn tmux_option(option: &str) -> Option<String> {
    let output = Command::new("tmux")
        .args(["show-options", "-gv", option])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

impl ConfigFacts {
    /// Loads the config from its XDG path with the loader lrat starts with.
    pub fn load() -> Self {
        match config::path() {
            Ok(path) => Self::at(path),
            Err(error) => Self {
                path: Err(format!("{error:#}")),
                exists: false,
                loaded: Loaded::default(),
            },
        }
    }

    /// Loads the config at `path`.
    pub fn at(path: PathBuf) -> Self {
        Self {
            exists: path.exists(),
            loaded: config::load(&path),
            path: Ok(path),
        }
    }
}

/// The checks `facts` make, in the order printed.
pub fn report(facts: &Facts) -> Report {
    let mut checks = vec![key_source(&facts.key)];
    checks.extend(linear_checks(facts));
    checks.extend(terminal_checks(
        &facts.terminal,
        &facts.config.loaded.config.theme,
    ));
    checks.extend(config_checks(&facts.config));
    checks.push(version(&facts.version));
    Report { checks }
}

fn key_source(key: &Result<KeySource, AuthError>) -> Check {
    const NAME: &str = "Key source";
    match key {
        Ok(source) => Check::ok(NAME, source.to_string()),
        Err(AuthError::NoKey) => Check::fail(
            NAME,
            "no API key",
            format!("run `lrat auth login`, or set {KEY_VAR}"),
        ),
        Err(error @ AuthError::MalformedEnv(_)) => Check::fail(
            NAME,
            error.to_string(),
            format!("put a personal API key in {KEY_VAR}, or unset it to use the OS keyring"),
        ),
        Err(error @ AuthError::Keyring(_)) => Check::fail(
            NAME,
            chain(error),
            format!("unlock the OS keyring and try again, or set {KEY_VAR}"),
        ),
        Err(error) => Check::fail(
            NAME,
            chain(error),
            format!("run `lrat auth login`, or set {KEY_VAR}"),
        ),
    }
}

fn linear_checks(facts: &Facts) -> Vec<Check> {
    const KEY: &str = "API key";
    const NETWORK: [&str; 3] = ["Rate limit", "Role", "Clock"];
    let skipped = |why: &str, fix: &str| {
        NETWORK
            .into_iter()
            .map(|name| Check::warn(name, format!("skipped: {why}"), fix))
            .collect::<Vec<_>>()
    };
    let (first, rest) = match &facts.linear {
        Linear::NotAsked => (
            Check::warn(
                KEY,
                "skipped: no API key resolved",
                "fix the key source, then run `lrat doctor` again",
            ),
            skipped(
                "no API key resolved",
                "fix the key source, then run `lrat doctor` again",
            ),
        ),
        Linear::Rejected => {
            let fix = match &facts.key {
                Ok(KeySource::Env) => format!(
                    "create a personal API key at {}, then put it in {KEY_VAR}, or unset \
                     {KEY_VAR} to use the OS keyring",
                    auth::KEYS_URL
                ),
                _ => format!(
                    "create a personal API key at {}, then run `lrat auth login`",
                    auth::KEYS_URL
                ),
            };
            (
                Check::fail(
                    KEY,
                    "Linear rejected it: it is wrong, revoked or expired",
                    fix,
                ),
                skipped(
                    "Linear rejected the API key",
                    "replace the API key, then run `lrat doctor` again",
                ),
            )
        }
        Linear::Unreachable(error) => (
            Check::fail(
                KEY,
                format!("could not check it with Linear: {error}"),
                "check the network connection and any proxy, then run `lrat doctor` again",
            ),
            skipped(
                "Linear could not be reached",
                "check the network connection, then run `lrat doctor` again",
            ),
        ),
        Linear::Reached {
            viewer,
            workspace,
            diagnostics,
            administrable_teams,
        } => (
            Check::ok(
                KEY,
                format!(
                    "valid for {} ({}), viewer {}",
                    workspace.name, workspace.url_key, viewer.name
                ),
            ),
            vec![
                rate_limit(diagnostics, facts.now),
                role(diagnostics, administrable_teams),
                clock(diagnostics, facts.now),
            ],
        ),
    };
    let mut checks = vec![first];
    checks.extend(rest);
    checks
}

fn rate_limit(diagnostics: &Result<Diagnostics, String>, now: Timestamp) -> Check {
    const NAME: &str = "Rate limit";
    let limits = match diagnostics {
        Ok(diagnostics) => &diagnostics.rate_limits,
        Err(error) => return unread(NAME, error),
    };
    if limits.is_empty() {
        return Check::ok(NAME, "Linear reported no limits");
    }
    let detail = limits
        .iter()
        .map(|limit| {
            format!(
                "{} {:.0} of {:.0} left",
                limit.kind, limit.remaining, limit.allowed
            )
        })
        .collect::<Vec<_>>()
        .join(", ");
    let resets = |limit: &crate::linear::RateLimit| match limit.reset {
        Some(reset) => format!("it resets {}", from_now(reset, now)),
        None => "it resets within the hour".to_owned(),
    };
    if let Some(spent) = limits.iter().find(|limit| limit.remaining <= 0.0) {
        return Check::fail(
            NAME,
            format!("{detail}: {} is used up", spent.kind),
            format!(
                "wait until {}, and stop other tools that use this key",
                resets(spent)
            ),
        );
    }
    // The budget governor slows background loads below a quarter of a limit.
    if let Some(low) = limits
        .iter()
        .find(|limit| limit.remaining < limit.allowed / 4.0)
    {
        return Check::warn(
            NAME,
            format!(
                "{detail}: {} is low, so background loads slow down",
                low.kind
            ),
            format!(
                "nothing to do but wait; {}, and other tools on this key spend it too",
                resets(low)
            ),
        );
    }
    Check::ok(NAME, detail)
}

fn role(
    diagnostics: &Result<Diagnostics, String>,
    administrable_teams: &Result<usize, String>,
) -> Check {
    const NAME: &str = "Role";
    let role = match diagnostics {
        Ok(diagnostics) => diagnostics.role,
        Err(error) => return unread(NAME, error),
    };
    let teams = match administrable_teams {
        Ok(teams) => *teams,
        Err(error) => return unread(NAME, error),
    };
    let name = if role.owner {
        "workspace owner"
    } else if role.admin {
        "workspace admin"
    } else if role.guest {
        "guest"
    } else {
        "member"
    };
    let teams = match teams {
        1 => "1 team".to_owned(),
        n => format!("{n} teams"),
    };
    Check::ok(
        NAME,
        format!(
            "{name}, administers {teams}; the key's scopes are inferred from this, \
             since Linear does not report them"
        ),
    )
}

fn clock(diagnostics: &Result<Diagnostics, String>, now: Timestamp) -> Check {
    const NAME: &str = "Clock";
    let server_time = match diagnostics {
        Ok(diagnostics) => diagnostics.server_time,
        Err(error) => return unread(NAME, error),
    };
    let Some(server_time) = server_time else {
        return Check::warn(
            NAME,
            "not measured: Linear's response had no readable Date header",
            "run `lrat doctor` again",
        );
    };
    let offset = now.duration_since(server_time);
    let seconds = offset.as_secs();
    let ahead = match seconds {
        0 => "in step with Linear".to_owned(),
        s if s > 0 => format!("{s}s ahead of Linear"),
        s => format!("{}s behind Linear", -s),
    };
    if offset.abs() > CLOCK_TOLERANCE {
        Check::warn(
            NAME,
            ahead,
            "turn on network time sync: `timedatectl set-ntp true` on Linux, or Set time \
             automatically under Date & Time on macOS",
        )
    } else {
        Check::ok(NAME, ahead)
    }
}

fn unread(name: &'static str, error: &str) -> Check {
    Check::warn(
        name,
        format!("could not read it from Linear: {error}"),
        "check the network connection, then run `lrat doctor` again",
    )
}

fn terminal_checks(terminal: &Terminal, theme_name: &str) -> Vec<Check> {
    vec![
        colors(terminal, theme_name),
        osc52(terminal),
        focus_events(terminal),
    ]
}

fn colors(terminal: &Terminal, theme_name: &str) -> Check {
    const NAME: &str = "Colors";
    let colorterm = terminal.colorterm.as_deref().unwrap_or_default();
    if matches!(colorterm, "truecolor" | "24bit") {
        return Check::ok(NAME, format!("24-bit (COLORTERM={colorterm})"));
    }
    let term = terminal.term.as_deref().unwrap_or_default();
    if term.is_empty() || term == "dumb" {
        let shown = if term.is_empty() { "unset" } else { term };
        return Check::warn(
            NAME,
            format!("TERM is {shown}, so colors may not show"),
            "run lrat in a terminal emulator, which sets TERM, such as xterm-256color",
        );
    }
    let depth = if term.contains("256color") {
        "256 colors"
    } else {
        "16 colors"
    };
    let detail = format!("{depth} (TERM={term}, COLORTERM unset)");
    if theme_name == theme::DEFAULT {
        return Check::ok(
            NAME,
            format!("{detail}; the {} theme needs no more", theme::DEFAULT),
        );
    }
    Check::warn(
        NAME,
        format!("{detail}; the {theme_name} theme is drawn in 24-bit color"),
        format!(
            "if the terminal has 24-bit color, set COLORTERM=truecolor; otherwise use the \
             {} theme",
            theme::DEFAULT
        ),
    )
}

fn osc52(terminal: &Terminal) -> Check {
    const NAME: &str = "OSC 52 copy";
    match &terminal.tmux {
        None => Check::ok(NAME, "not detectable outside tmux"),
        Some(Tmux {
            clipboard_problem: None,
            ..
        }) => Check::ok(NAME, "tmux passes it on (set-clipboard on)"),
        Some(Tmux {
            clipboard_problem: Some(problem),
            ..
        }) => Check::warn(
            NAME,
            problem.clone(),
            "add `set -s set-clipboard on` to tmux.conf and reload it",
        ),
    }
}

fn focus_events(terminal: &Terminal) -> Check {
    const NAME: &str = "Focus events";
    match terminal.tmux.as_ref().map(|tmux| tmux.focus_events) {
        None => Check::ok(NAME, "not detectable outside tmux"),
        Some(None) => Check::ok(NAME, "not detectable: tmux could not be asked"),
        Some(Some(true)) => Check::ok(NAME, "tmux passes them on (focus-events on)"),
        Some(Some(false)) => Check::warn(
            NAME,
            "tmux holds them back (focus-events off), so lrat cannot tell when it is unfocused",
            "add `set -g focus-events on` to tmux.conf and reload it",
        ),
    }
}

fn config_checks(config: &ConfigFacts) -> Vec<Check> {
    const NAME: &str = "Config";
    let path = match &config.path {
        Ok(path) => path,
        Err(error) => {
            return vec![Check::warn(
                NAME,
                format!("no config path: {error}; using the defaults"),
                "set HOME, or XDG_CONFIG_HOME",
            )];
        }
    };
    if config.loaded.errors.is_empty() {
        let detail = if config.exists {
            format!("{}: no problems", path.display())
        } else {
            format!("{}: no file, using the defaults", path.display())
        };
        return vec![Check::ok(NAME, detail)];
    }
    config
        .loaded
        .errors
        .iter()
        .map(|error| {
            let fix = match error.line {
                Some(line) => format!(
                    "edit line {line} of {}; docs/config.md lists every setting",
                    error.path.display()
                ),
                None => format!(
                    "edit {}; docs/config.md lists every setting",
                    error.path.display()
                ),
            };
            if is_ignored(error) {
                Check::warn(NAME, error.to_string(), fix)
            } else {
                Check::fail(NAME, error.to_string(), fix)
            }
        })
        .collect()
}

/// Whether the loader skipped something unknown and kept the rest, rather
/// than falling back to defaults: the warnings among its errors.
fn is_ignored(error: &ConfigError) -> bool {
    error.message.contains("is ignored") || error.message.starts_with("unknown context")
}

fn version(facts: &VersionFacts) -> Check {
    const NAME: &str = "Version";
    match &facts.latest {
        Ok(latest) if *latest > facts.running => Check::warn(
            NAME,
            format!("lrat {latest} is available"),
            facts.channel.upgrade(),
        ),
        Ok(_) => Check::ok(NAME, format!("lrat {} is the latest", facts.running)),
        Err(NotChecked::Off(Off::Config)) => Check::warn(
            NAME,
            "not checked: [updates] check is false",
            "set `check = true` under [updates] in config.toml to check",
        ),
        Err(NotChecked::Off(Off::Env)) => Check::warn(
            NAME,
            format!("not checked: {} is set", release::OFF_VAR),
            format!("unset {} to check", release::OFF_VAR),
        ),
        Err(NotChecked::Failed(error)) => Check::warn(
            NAME,
            format!("not checked: {error}"),
            "check the network connection, then run `lrat doctor` again",
        ),
    }
}

/// When `then` is, from `now`, in whole minutes.
fn from_now(then: Timestamp, now: Timestamp) -> String {
    let minutes = then.duration_since(now).as_secs().div_euclid(60).max(0);
    match minutes {
        0 => "within a minute".to_owned(),
        1 => "in 1 minute".to_owned(),
        n => format!("in {n} minutes"),
    }
}

/// An error with its causes, as `{:#}` prints an `anyhow` chain.
fn chain(error: &dyn std::error::Error) -> String {
    let mut text = error.to_string();
    let mut source = error.source();
    while let Some(cause) = source {
        let _ = write!(text, ": {cause}");
        source = cause.source();
    }
    text
}
