//! `lrat doctor`: the checks over gathered facts, gathering through the fake
//! client and a local fake server, and the printed report.

// clippy's allow-unwrap-in-tests does not cover helpers outside #[test] functions.
#![allow(clippy::unwrap_used)]

use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use clap::Parser;
use jiff::{SignedDuration, Timestamp};
use linear_rat::auth::{ApiKey, Credentials, KeyStore, KeyringError, KeyringStore, WorkspaceFile};
use linear_rat::cli::{Cli, Command};
use linear_rat::doctor::{self, ConfigFacts, Facts, Report, Status, Terminal, Tmux};
use linear_rat::linear::{
    ClientConfig, Diagnostics, FakeClient, HttpClient, LinearClient, RateLimit, Role, demo,
};
use serde_json::json;
use wiremock::matchers::{body_string_contains, method};
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

const SECRET: &str = "lin_api_fictional0000000000000000000000000000";

/// A keyring with nothing in it.
struct EmptyStore;

impl KeyStore for EmptyStore {
    fn kind(&self) -> KeyringStore {
        KeyringStore::Os
    }

    fn get(&self, _: &str) -> Result<Option<ApiKey>, KeyringError> {
        Ok(None)
    }

    fn set(&self, _: &str, _: &ApiKey) -> Result<(), KeyringError> {
        Ok(())
    }

    fn delete(&self, _: &str) -> Result<bool, KeyringError> {
        Ok(false)
    }
}

/// A fresh directory for one test.
fn scratch_dir() -> PathBuf {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!(
        "doctor-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::SeqCst)
    ));
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn credentials(env_key: Option<&str>) -> Credentials<EmptyStore> {
    Credentials::new(
        EmptyStore,
        WorkspaceFile::new(scratch_dir().join("keyring-workspace")),
        env_key.map(str::to_owned),
    )
}

/// The config at `config.toml` in a fresh directory, holding `text`, or no
/// file at all.
fn config(text: Option<&str>) -> ConfigFacts {
    let path = scratch_dir().join("config.toml");
    if let Some(text) = text {
        fs::write(&path, text).unwrap();
    }
    ConfigFacts::at(path)
}

fn truecolor() -> Terminal {
    Terminal {
        colorterm: Some("truecolor".to_owned()),
        term: Some("xterm-256color".to_owned()),
        tmux: None,
    }
}

async fn gather_with<C: LinearClient>(
    env_key: Option<&str>,
    client: impl FnOnce(&ApiKey) -> Result<C, linear_rat::linear::ClientError>,
    terminal: Terminal,
    config: ConfigFacts,
) -> Facts {
    doctor::gather(&credentials(env_key), client, terminal, config, || {
        demo::NOW
    })
    .await
}

/// The report's lines with the config path made stable.
fn printed(report: &Report, config_dir: Option<&str>) -> String {
    let text = report.render(false);
    match config_dir {
        Some(dir) => text.replace(dir, "<config dir>"),
        None => text,
    }
}

fn line<'a>(report: &'a Report, name: &str) -> &'a doctor::Check {
    report
        .checks
        .iter()
        .find(|check| check.name == name)
        .unwrap_or_else(|| panic!("no {name} check in {report:?}"))
}

#[tokio::test]
async fn a_valid_key_passes_every_check() {
    let config = config(None);
    let dir = config
        .path
        .as_ref()
        .unwrap()
        .parent()
        .unwrap()
        .display()
        .to_string();
    let facts = gather_with(Some(SECRET), |_| Ok(FakeClient), truecolor(), config).await;

    let report = doctor::report(&facts);

    assert!(!report.failed());
    assert!(
        report.checks.iter().all(|check| check.status == Status::Ok),
        "{report:#?}"
    );
    insta::assert_snapshot!(printed(&report, Some(&dir)));
}

#[tokio::test]
async fn a_missing_key_fails_and_skips_the_network_checks() {
    let facts = gather_with(None, |_| Ok(FakeClient), truecolor(), config(None)).await;

    let report = doctor::report(&facts);

    assert!(report.failed());
    let key = line(&report, "Key source");
    assert_eq!(key.status, Status::Fail);
    assert_eq!(key.detail, "no API key");
    assert_eq!(
        key.fix.as_deref(),
        Some("run `lrat auth login`, or set LINEAR_API_KEY")
    );
    for name in ["API key", "Rate limit", "Role", "Clock"] {
        let check = line(&report, name);
        assert_eq!(check.status, Status::Warn, "{name}");
        assert_eq!(check.detail, "skipped: no API key resolved", "{name}");
        assert!(check.fix.is_some(), "{name}");
    }
}

fn client_for(server: &MockServer) -> HttpClient {
    let config = ClientConfig {
        endpoint: format!("{}/graphql", server.uri()),
        connect_timeout: Duration::from_millis(300),
        request_timeout: Duration::from_secs(5),
    };
    HttpClient::with_config(&ApiKey::new(SECRET).unwrap(), config).unwrap()
}

#[tokio::test]
async fn an_invalid_key_fails_with_how_to_replace_it() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(401))
        .mount(&server)
        .await;
    let client = client_for(&server);

    let facts = gather_with(Some(SECRET), |_| Ok(client), truecolor(), config(None)).await;
    let report = doctor::report(&facts);

    assert!(report.failed());
    assert_eq!(line(&report, "Key source").status, Status::Ok);
    let key = line(&report, "API key");
    assert_eq!(key.status, Status::Fail);
    assert_eq!(
        key.detail,
        "Linear rejected it: it is wrong, revoked or expired"
    );
    assert_eq!(
        key.fix.as_deref(),
        Some(
            "create a personal API key at https://linear.app/settings/account/security, then \
             put it in LINEAR_API_KEY, or unset LINEAR_API_KEY to use the OS keyring"
        )
    );
    assert_eq!(line(&report, "Role").status, Status::Warn);
    let text = report.render(false);
    assert!(!text.contains(SECRET), "{text}");
}

#[tokio::test]
async fn a_broken_config_section_fails_naming_file_and_line() {
    let config = config(Some(
        "[ui]\nicons = \"nerd\"\n\n[sync]\nactive-poll-seconds = \"often\"\n",
    ));
    let path = config.path.as_ref().unwrap().display().to_string();
    let facts = gather_with(Some(SECRET), |_| Ok(FakeClient), truecolor(), config).await;

    let report = doctor::report(&facts);

    assert!(report.failed());
    let config = line(&report, "Config");
    assert_eq!(config.status, Status::Fail);
    assert!(
        config.detail.starts_with(&format!("{path}:5: ")),
        "{}",
        config.detail
    );
    assert!(
        config.detail.contains("[sync] uses its defaults"),
        "{}",
        config.detail
    );
    assert_eq!(
        config.fix.as_deref(),
        Some(format!("edit line 5 of {path}; docs/config.md lists every setting").as_str())
    );
}

#[tokio::test]
async fn an_unknown_config_key_only_warns() {
    let config = config(Some("[ui]\ncolour = \"blue\"\n"));
    let facts = gather_with(Some(SECRET), |_| Ok(FakeClient), truecolor(), config).await;

    let report = doctor::report(&facts);

    assert!(!report.failed());
    let config = line(&report, "Config");
    assert_eq!(config.status, Status::Warn);
    assert!(
        config
            .detail
            .contains(":2: unknown key `colour` in [ui] is ignored"),
        "{}",
        config.detail
    );
}

fn reached(diagnostics: Diagnostics, now: Timestamp) -> Facts {
    Facts {
        key: Ok(linear_rat::auth::KeySource::Env),
        linear: doctor::Linear::Reached {
            viewer: linear_rat::linear::model::Viewer {
                id: "u1".to_owned(),
                name: "Remy Fromage".to_owned(),
            },
            workspace: linear_rat::linear::model::Workspace {
                id: "o1".to_owned(),
                name: "Cheddar Works".to_owned(),
                url_key: "cheddar-works".to_owned(),
            },
            diagnostics: Ok(diagnostics),
            administrable_teams: Ok(1),
        },
        terminal: truecolor(),
        config: config(None),
        now,
    }
}

fn limits(remaining: f64) -> Diagnostics {
    Diagnostics {
        role: Role {
            admin: true,
            owner: false,
            guest: false,
        },
        rate_limits: vec![RateLimit {
            kind: "requests".to_owned(),
            allowed: 5000.0,
            remaining,
            reset: Some(demo::NOW + SignedDuration::from_mins(12)),
        }],
        server_time: Some(demo::NOW),
    }
}

#[test]
fn a_low_rate_limit_warns_and_a_spent_one_fails() {
    let low = doctor::report(&reached(limits(1000.0), demo::NOW));
    let check = line(&low, "Rate limit");
    assert_eq!(check.status, Status::Warn);
    assert_eq!(
        check.detail,
        "requests 1000 of 5000 left: requests is low, so background loads slow down"
    );
    assert_eq!(
        check.fix.as_deref(),
        Some(
            "nothing to do but wait; it resets in 12 minutes, and other tools on this key spend it too"
        )
    );

    let spent = doctor::report(&reached(limits(0.0), demo::NOW));
    assert_eq!(line(&spent, "Rate limit").status, Status::Fail);
    assert!(spent.failed());
}

#[test]
fn the_role_names_itself_and_says_scopes_are_inferred() {
    let report = doctor::report(&reached(limits(4000.0), demo::NOW));

    assert_eq!(
        line(&report, "Role").detail,
        "workspace admin, administers 1 team; the key's scopes are inferred from this, \
         since Linear does not report them"
    );
}

#[test]
fn a_clock_off_by_more_than_thirty_seconds_warns() {
    let near = doctor::report(&reached(
        limits(4000.0),
        demo::NOW + SignedDuration::from_secs(30),
    ));
    let check = line(&near, "Clock");
    assert_eq!(check.status, Status::Ok);
    assert_eq!(check.detail, "30s ahead of Linear");

    let far = doctor::report(&reached(
        limits(4000.0),
        demo::NOW - SignedDuration::from_secs(95),
    ));
    let check = line(&far, "Clock");
    assert_eq!(check.status, Status::Warn);
    assert_eq!(check.detail, "95s behind Linear");
    assert!(
        check
            .fix
            .as_deref()
            .unwrap()
            .contains("timedatectl set-ntp true")
    );
}

fn terminal_report(terminal: Terminal, theme: &str) -> Report {
    let mut facts = reached(limits(4000.0), demo::NOW);
    facts.terminal = terminal;
    facts.config = config(Some(&format!("[theme]\nname = \"{theme}\"\n")));
    doctor::report(&facts)
}

#[test]
fn colors_come_from_colorterm_then_term() {
    let true_color = terminal_report(truecolor(), "nord");
    assert_eq!(
        line(&true_color, "Colors").detail,
        "24-bit (COLORTERM=truecolor)"
    );

    let xterm = Terminal {
        colorterm: None,
        term: Some("xterm-256color".to_owned()),
        tmux: None,
    };
    let plain = terminal_report(xterm.clone(), "terminal");
    let check = line(&plain, "Colors");
    assert_eq!(check.status, Status::Ok);
    assert_eq!(
        check.detail,
        "256 colors (TERM=xterm-256color, COLORTERM unset); the terminal theme needs no more"
    );

    let themed = terminal_report(xterm, "nord");
    let check = line(&themed, "Colors");
    assert_eq!(check.status, Status::Warn);
    assert!(
        check
            .fix
            .as_deref()
            .unwrap()
            .contains("COLORTERM=truecolor")
    );

    let dumb = terminal_report(Terminal::default(), "terminal");
    assert_eq!(line(&dumb, "Colors").status, Status::Warn);
}

#[test]
fn outside_tmux_copying_and_focus_events_are_not_detectable() {
    let report = terminal_report(truecolor(), "terminal");

    assert_eq!(
        line(&report, "OSC 52 copy").detail,
        "not detectable outside tmux"
    );
    assert_eq!(
        line(&report, "Focus events").detail,
        "not detectable outside tmux"
    );
}

#[test]
fn inside_tmux_its_settings_are_checked() {
    let tmux = |clipboard_problem: Option<&str>, focus_events| Terminal {
        tmux: Some(Tmux {
            clipboard_problem: clipboard_problem.map(str::to_owned),
            focus_events,
        }),
        ..truecolor()
    };

    let good = terminal_report(tmux(None, Some(true)), "terminal");
    assert_eq!(line(&good, "OSC 52 copy").status, Status::Ok);
    assert_eq!(line(&good, "Focus events").status, Status::Ok);

    let bad = terminal_report(
        tmux(
            Some(
                "tmux ignores the copy while set-clipboard is external: add `set -s set-clipboard on` to tmux.conf",
            ),
            Some(false),
        ),
        "terminal",
    );
    let copy = line(&bad, "OSC 52 copy");
    assert_eq!(copy.status, Status::Warn);
    assert!(copy.detail.contains("set-clipboard is external"));
    let focus = line(&bad, "Focus events");
    assert_eq!(focus.status, Status::Warn);
    assert_eq!(
        focus.fix.as_deref(),
        Some("add `set -g focus-events on` to tmux.conf and reload it")
    );
}

#[test]
fn every_warning_and_failure_carries_a_fix() {
    let mut facts = reached(limits(0.0), demo::NOW - SignedDuration::from_mins(5));
    facts.terminal = Terminal::default();
    facts.config = config(Some("[sync]\nactive-poll-seconds = \"often\"\n[nope]\n"));

    let report = doctor::report(&facts);

    for check in &report.checks {
        assert_eq!(check.fix.is_some(), check.status != Status::Ok, "{check:?}");
    }
}

#[test]
fn color_only_on_a_terminal_without_no_color() {
    assert!(doctor::use_color(true, None));
    assert!(doctor::use_color(true, Some("")));
    assert!(!doctor::use_color(true, Some("1")));
    assert!(!doctor::use_color(false, None));
}

#[test]
fn a_colored_report_colors_only_the_markers() {
    let report = doctor::report(&reached(limits(1000.0), demo::NOW));

    let text = report.render(true);

    assert!(text.contains("\x1b[32m●\x1b[0m Key source"), "{text:?}");
    assert!(text.contains("\x1b[33m▲\x1b[0m Rate limit"), "{text:?}");
}

#[test]
fn doctor_help_describes_the_command() {
    let error = Cli::try_parse_from(["lrat", "doctor", "--help"]).unwrap_err();

    assert_eq!(error.kind(), clap::error::ErrorKind::DisplayHelp);
    let help = error.to_string();
    assert!(
        help.contains("Check the API key, Linear, the terminal and the config"),
        "{help}"
    );
    assert!(
        help.contains("Exits non-zero when any check fails"),
        "{help}"
    );
    assert!(matches!(
        Cli::try_parse_from(["lrat", "doctor"]).unwrap().command,
        Some(Command::Doctor)
    ));
}

#[tokio::test]
async fn the_real_client_reads_the_role_rate_limits_and_date() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(body_string_contains("query Diagnostics"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("date", "Mon, 02 Nov 2026 15:00:07 GMT")
                .set_body_json(json!({
                    "data": {
                        "viewer": { "admin": false, "owner": false, "guest": true },
                        "rateLimitStatus": { "limits": [{
                            "type": "requestLimit",
                            "allowedAmount": 5000.0,
                            "remainingAmount": 4321.0,
                            "requestedAmount": 1.0,
                            "period": 3600000.0,
                            "reset": 1793632320000.0
                        }] }
                    }
                })),
        )
        .expect(1)
        .mount(&server)
        .await;

    let diagnostics = client_for(&server).diagnostics().await.unwrap();

    assert_eq!(
        diagnostics,
        Diagnostics {
            role: Role {
                admin: false,
                owner: false,
                guest: true,
            },
            rate_limits: vec![RateLimit {
                kind: "requestLimit".to_owned(),
                allowed: 5000.0,
                remaining: 4321.0,
                reset: Some(Timestamp::constant(1_793_632_320, 0)),
            }],
            server_time: Some(Timestamp::constant(1_793_631_607, 0)),
        }
    );
}

/// Answers each request and records which query it was, so a test can see
/// where the local clock was read among them.
struct Recording {
    log: Arc<Mutex<Vec<&'static str>>>,
}

impl Respond for Recording {
    fn respond(&self, request: &Request) -> ResponseTemplate {
        let body = String::from_utf8_lossy(&request.body);
        let (name, data) = if body.contains("query Viewer") {
            (
                "viewer",
                json!({
                    "viewer": { "id": "u1", "name": "Ada" },
                    "organization": { "id": "o1", "name": "Acme", "urlKey": "acme" }
                }),
            )
        } else if body.contains("query Diagnostics") {
            (
                "diagnostics",
                json!({
                    "viewer": { "admin": false, "owner": false, "guest": false },
                    "rateLimitStatus": { "limits": [] }
                }),
            )
        } else {
            (
                "administrableTeams",
                json!({ "administrableTeams": {
                    "nodes": [],
                    "pageInfo": { "hasNextPage": false, "endCursor": null }
                } }),
            )
        };
        self.log.lock().unwrap().push(name);
        ResponseTemplate::new(200)
            .insert_header("date", "Mon, 02 Nov 2026 15:00:07 GMT")
            .set_body_json(json!({ "data": data }))
    }
}

#[tokio::test]
async fn the_clock_is_read_right_after_the_dated_response() {
    let server = MockServer::start().await;
    let log = Arc::new(Mutex::new(Vec::new()));
    Mock::given(method("POST"))
        .respond_with(Recording { log: log.clone() })
        .mount(&server)
        .await;
    let client = client_for(&server);

    let clock_log = log.clone();
    doctor::gather(
        &credentials(Some(SECRET)),
        |_| Ok(client),
        truecolor(),
        config(None),
        move || {
            clock_log.lock().unwrap().push("now");
            demo::NOW
        },
    )
    .await;

    let log = log.lock().unwrap();
    let now = log.iter().position(|step| *step == "now").unwrap();
    assert_eq!(log[now - 1], "diagnostics", "{log:?}");
}
