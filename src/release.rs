//! The release check (ADR 0004): GitHub's latest stable lrat release, how
//! lrat was installed, and the cache that keeps the check to once a day.
//!
//! The request carries no key and no workspace data: only lrat's version in
//! the User-Agent.

use std::ffi::OsStr;
use std::fs::{self, DirBuilder};
use std::io::ErrorKind;
use std::os::unix::fs::DirBuilderExt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::Context as _;
use etcetera::{BaseStrategy, choose_base_strategy};
use jiff::{SignedDuration, Timestamp};
use reqwest::header::ACCEPT;
use semver::Version;
use serde::{Deserialize, Serialize};

use crate::cli::Cli;
use crate::config::UpdateSettings;
use crate::snapshot::write_atomically;

/// Where GitHub answers with the latest stable release. It skips drafts and
/// pre-releases.
pub const LATEST_URL: &str = "https://api.github.com/repos/cxrlos/linear-rat/releases/latest";

/// Set to any non-empty value, it turns the check off.
pub const OFF_VAR: &str = "LRAT_NO_UPDATE_CHECK";

/// How long a check holds before the next one is due.
pub const CHECK_EVERY: SignedDuration = SignedDuration::from_hours(24);

/// How long one check may take, connecting included.
const TIMEOUT: Duration = Duration::from_secs(10);

/// How lrat was installed, as the running binary's path tells it.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum Channel {
    /// A Homebrew Cellar.
    Homebrew,
    /// A system bin directory, where only the package manager installs; on
    /// Arch that is the AUR package.
    Aur,
    /// The Nix store.
    Nix,
    /// The cargo bin dir, where `cargo install`, `cargo binstall` and the
    /// shell installer all put lrat without a receipt, or anywhere else.
    #[default]
    CargoBin,
}

impl Channel {
    /// The channel of the binary at `path`, which should have its symlinks
    /// resolved, since Homebrew links its binaries out of the Cellar.
    pub fn of(path: &Path) -> Self {
        if path.starts_with("/nix/store") {
            return Self::Nix;
        }
        if path.components().any(|part| part.as_os_str() == "Cellar") {
            return Self::Homebrew;
        }
        if ["/usr/bin", "/usr/sbin", "/bin"]
            .iter()
            .any(|dir| path.parent() == Some(Path::new(dir)))
        {
            return Self::Aur;
        }
        Self::CargoBin
    }

    /// The channel of the running binary, or [`Channel::CargoBin`] when its
    /// path cannot be read.
    pub fn of_running() -> Self {
        std::env::current_exe()
            .and_then(fs::canonicalize)
            .map(|path| Self::of(&path))
            .unwrap_or_default()
    }

    /// Whether a newer release shows as a notice. The AUR and Nix ship a
    /// release when they package it, which may be later, so they never do.
    pub fn shows_notice(self) -> bool {
        matches!(self, Self::Homebrew | Self::CargoBin)
    }

    /// How to upgrade an install of this channel.
    pub fn upgrade(self) -> &'static str {
        match self {
            Self::Homebrew => "brew upgrade linear-rat",
            Self::Aur => "upgrade it through your AUR helper",
            Self::Nix => "upgrade it through Nix",
            Self::CargoBin => {
                "upgrade the way you installed it; see github.com/cxrlos/linear-rat#install"
            }
        }
    }
}

/// The version lrat runs as.
pub fn running() -> Version {
    Version::parse(env!("CARGO_PKG_VERSION")).unwrap_or_else(|_| Version::new(0, 0, 0))
}

/// The stable version a release tag names, as `v0.3.1`. A pre-release tag
/// names none, so it never counts as newer.
pub fn tag_version(tag: &str) -> Option<Version> {
    Version::parse(tag.strip_prefix('v').unwrap_or(tag))
        .ok()
        .filter(|version| version.pre.is_empty())
}

/// The GitHub page of `version`'s release.
pub fn release_url(version: &Version) -> String {
    format!("https://github.com/cxrlos/linear-rat/releases/tag/v{version}")
}

/// What turned the check off.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Off {
    /// `[updates] check = false`.
    Config,
    /// [`OFF_VAR`] is set.
    Env,
}

/// What turns the check off, if anything: `settings`, or `env`, the value
/// of [`OFF_VAR`], when it is not empty.
pub fn off_switch(settings: &UpdateSettings, env: Option<&OsStr>) -> Option<Off> {
    if env.is_some_and(|value| !value.is_empty()) {
        Some(Off::Env)
    } else if !settings.check {
        Some(Off::Config)
    } else {
        None
    }
}

/// Whether this run checks in the background: only the TUI on a real
/// workspace, with nothing turning the check off. `--demo` and the
/// subcommands never do; `lrat doctor` checks on its own.
pub fn checks_in_background(cli: &Cli, off: Option<Off>) -> bool {
    cli.command.is_none() && !cli.demo && off.is_none()
}

/// What the last checks found: when the last one ran, the newest release
/// it saw and the release the viewer skipped.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Record {
    pub checked_at: Option<Timestamp>,
    pub latest: Option<Version>,
    pub skipped: Option<Version>,
}

impl Record {
    /// Whether a check is due at `now`: never checked, a day after the
    /// last one, or the clock was set back past it.
    pub fn due(&self, now: Timestamp) -> bool {
        self.checked_at.is_none_or(|checked| {
            let since = now.duration_since(checked);
            since >= CHECK_EVERY || since.is_negative()
        })
    }
}

#[derive(Serialize, Deserialize)]
struct File {
    checked_at: Option<Timestamp>,
    latest: Option<String>,
    skipped: Option<String>,
}

/// The cache file of the release check, under the XDG cache dir.
#[derive(Debug, Clone)]
pub struct ReleaseCache {
    path: PathBuf,
}

impl ReleaseCache {
    /// The cache file `release.json` in `dir`.
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self {
            path: dir.into().join("release.json"),
        }
    }

    /// `$XDG_CACHE_HOME/lrat/release.json` (by default
    /// `~/.cache/lrat/release.json`, on macOS too).
    pub fn in_cache_dir() -> anyhow::Result<Self> {
        let base = choose_base_strategy().context("finding the home directory")?;
        Ok(Self::new(base.cache_dir().join("lrat")))
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// What the last checks found, or an empty record when there is no
    /// file or it cannot be read, which only makes the next check due.
    pub fn load(&self) -> Record {
        let file = match fs::read(&self.path) {
            Ok(bytes) => serde_json::from_slice::<File>(&bytes)
                .with_context(|| format!("decoding {}", self.path.display())),
            Err(error) if error.kind() == ErrorKind::NotFound => return Record::default(),
            Err(error) => Err(error).with_context(|| format!("reading {}", self.path.display())),
        };
        match file {
            Ok(file) => Record {
                checked_at: file.checked_at,
                latest: file.latest.as_deref().and_then(tag_version),
                skipped: file.skipped.as_deref().and_then(tag_version),
            },
            Err(error) => {
                tracing::warn!("discarding the release cache: {error:#}");
                Record::default()
            }
        }
    }

    pub fn save(&self, record: &Record) -> anyhow::Result<()> {
        let file = File {
            checked_at: record.checked_at,
            latest: record.latest.as_ref().map(ToString::to_string),
            skipped: record.skipped.as_ref().map(ToString::to_string),
        };
        let json = serde_json::to_vec(&file).context("encoding the release cache")?;
        if let Some(dir) = self.path.parent() {
            DirBuilder::new()
                .recursive(true)
                .mode(0o700)
                .create(dir)
                .with_context(|| format!("creating {}", dir.display()))?;
        }
        write_atomically(&self.path, &json)
    }
}

/// Where the latest release is asked for: GitHub, or a fake server in
/// tests.
#[derive(Debug, Clone)]
pub struct ReleaseSource {
    url: String,
    http: reqwest::Client,
}

#[derive(Deserialize)]
struct Latest {
    tag_name: String,
}

impl ReleaseSource {
    /// GitHub's latest release of lrat.
    pub fn github() -> anyhow::Result<Self> {
        Self::new(LATEST_URL)
    }

    /// The latest release as `url` answers it, in GitHub's shape.
    pub fn new(url: impl Into<String>) -> anyhow::Result<Self> {
        let http = reqwest::Client::builder()
            .timeout(TIMEOUT)
            .user_agent(concat!("lrat/", env!("CARGO_PKG_VERSION")))
            .build()
            .context("building the HTTP client")?;
        Ok(Self {
            url: url.into(),
            http,
        })
    }

    /// The version of the latest stable release.
    pub async fn latest(&self) -> anyhow::Result<Version> {
        let response = self
            .http
            .get(&self.url)
            .header(ACCEPT, "application/vnd.github+json")
            .send()
            .await
            .context("could not reach GitHub")?;
        let status = response.status();
        if !status.is_success() {
            anyhow::bail!("GitHub answered {status}");
        }
        let body = response
            .bytes()
            .await
            .context("could not read GitHub's answer")?;
        let latest: Latest =
            serde_json::from_slice(&body).context("could not read GitHub's answer")?;
        tag_version(&latest.tag_name)
            .with_context(|| format!("the release tag {} is not a version", latest.tag_name))
    }
}

/// Everything the background check needs: where to ask and where to keep
/// what it found.
#[derive(Debug, Clone)]
pub struct ReleaseCheck {
    pub source: ReleaseSource,
    pub cache: ReleaseCache,
}

#[cfg(test)]
mod tests {
    use std::ffi::OsStr;
    use std::path::Path;

    use clap::Parser;
    use jiff::Timestamp;
    use semver::Version;

    use super::*;
    use crate::cli::Cli;
    use crate::config::UpdateSettings;

    fn version(text: &str) -> Version {
        Version::parse(text).unwrap()
    }

    #[test]
    fn a_homebrew_cellar_path_is_homebrew() {
        for path in [
            "/opt/homebrew/Cellar/linear-rat/0.3.0/bin/lrat",
            "/usr/local/Cellar/linear-rat/0.3.0/bin/lrat",
            "/home/linuxbrew/.linuxbrew/Cellar/linear-rat/0.3.0/bin/lrat",
        ] {
            assert_eq!(Channel::of(Path::new(path)), Channel::Homebrew, "{path}");
        }
    }

    #[test]
    fn a_system_bin_path_is_the_aur() {
        for path in ["/usr/bin/lrat", "/usr/sbin/lrat", "/bin/lrat"] {
            assert_eq!(Channel::of(Path::new(path)), Channel::Aur, "{path}");
        }
    }

    #[test]
    fn a_nix_store_path_is_nix() {
        let path = Path::new("/nix/store/abc123-linear-rat-0.3.0/bin/lrat");
        assert_eq!(Channel::of(path), Channel::Nix);
    }

    #[test]
    fn anything_else_is_the_cargo_bin_dir() {
        for path in [
            "/home/remy/.cargo/bin/lrat",
            "/Users/remy/.cargo/bin/lrat",
            "/usr/local/bin/lrat",
            "/home/remy/bin/lrat",
            "lrat",
        ] {
            assert_eq!(Channel::of(Path::new(path)), Channel::CargoBin, "{path}");
        }
    }

    #[test]
    fn only_homebrew_and_the_cargo_bin_dir_show_the_notice() {
        assert!(Channel::Homebrew.shows_notice());
        assert!(Channel::CargoBin.shows_notice());
        assert!(!Channel::Aur.shows_notice());
        assert!(!Channel::Nix.shows_notice());
    }

    #[test]
    fn each_channel_says_how_to_upgrade() {
        assert_eq!(Channel::Homebrew.upgrade(), "brew upgrade linear-rat");
        assert_eq!(
            Channel::CargoBin.upgrade(),
            "upgrade the way you installed it; see github.com/cxrlos/linear-rat#install"
        );
        assert_eq!(Channel::Aur.upgrade(), "upgrade it through your AUR helper");
        assert_eq!(Channel::Nix.upgrade(), "upgrade it through Nix");
    }

    #[test]
    fn a_tag_is_its_version_with_or_without_a_v() {
        assert_eq!(tag_version("v0.3.1"), Some(version("0.3.1")));
        assert_eq!(tag_version("0.3.1"), Some(version("0.3.1")));
        assert_eq!(tag_version("nightly"), None);
        assert_eq!(tag_version("v0.4.0-rc.1"), None);
    }

    #[test]
    fn the_release_page_is_the_tag_on_github() {
        assert_eq!(
            release_url(&version("0.3.1")),
            "https://github.com/cxrlos/linear-rat/releases/tag/v0.3.1"
        );
    }

    #[test]
    fn the_running_version_is_the_crate_version() {
        assert_eq!(running(), version(env!("CARGO_PKG_VERSION")));
    }

    #[test]
    fn the_config_or_any_non_empty_env_value_switches_the_check_off() {
        let on = UpdateSettings { check: true };
        let off = UpdateSettings { check: false };
        assert_eq!(off_switch(&on, None), None);
        assert_eq!(off_switch(&on, Some(OsStr::new(""))), None);
        assert_eq!(off_switch(&on, Some(OsStr::new("1"))), Some(Off::Env));
        assert_eq!(off_switch(&on, Some(OsStr::new("no"))), Some(Off::Env));
        assert_eq!(off_switch(&off, None), Some(Off::Config));
    }

    fn cli(args: &[&str]) -> Cli {
        Cli::try_parse_from(args).unwrap()
    }

    #[test]
    fn only_the_tui_on_a_real_workspace_checks_in_the_background() {
        assert!(checks_in_background(&cli(&["lrat"]), None));
        assert!(!checks_in_background(&cli(&["lrat"]), Some(Off::Config)));
        assert!(!checks_in_background(&cli(&["lrat"]), Some(Off::Env)));
        assert!(!checks_in_background(&cli(&["lrat", "--demo"]), None));
        for headless in [
            &["lrat", "auth", "status"][..],
            &["lrat", "completions", "zsh"],
            &["lrat", "open", "ENG-1"],
            &["lrat", "doctor"],
        ] {
            assert!(!checks_in_background(&cli(headless), None), "{headless:?}");
        }
    }

    #[test]
    fn a_check_is_due_a_day_after_the_last_one() {
        let checked: Timestamp = "2026-11-18T09:00:00Z".parse().unwrap();
        let at = |text: &str| text.parse::<Timestamp>().unwrap();
        let record = Record {
            checked_at: Some(checked),
            ..Record::default()
        };
        assert!(Record::default().due(checked));
        assert!(!record.due(at("2026-11-18T09:00:01Z")));
        assert!(!record.due(at("2026-11-19T08:59:59Z")));
        assert!(record.due(at("2026-11-19T09:00:00Z")));
        // A clock set back past the last check does not wait a day more.
        assert!(record.due(at("2026-11-17T09:00:00Z")));
    }

    fn scratch_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("lrat-release-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn the_cache_keeps_the_check_time_latest_and_skipped_versions() {
        let cache = ReleaseCache::new(scratch_dir("round-trip"));
        let record = Record {
            checked_at: Some("2026-11-18T09:00:00Z".parse().unwrap()),
            latest: Some(version("0.3.1")),
            skipped: Some(version("0.3.1")),
        };

        cache.save(&record).unwrap();

        assert_eq!(cache.load(), record);
        let json = std::fs::read_to_string(cache.path()).unwrap();
        assert_eq!(
            json,
            r#"{"checked_at":"2026-11-18T09:00:00Z","latest":"0.3.1","skipped":"0.3.1"}"#
        );
    }

    #[test]
    fn a_missing_or_broken_cache_is_an_empty_record() {
        let cache = ReleaseCache::new(scratch_dir("broken"));
        assert_eq!(cache.load(), Record::default());

        std::fs::create_dir_all(cache.path().parent().unwrap()).unwrap();
        std::fs::write(cache.path(), "not json").unwrap();
        assert_eq!(cache.load(), Record::default());
    }
}
