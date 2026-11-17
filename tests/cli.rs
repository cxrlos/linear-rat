// clippy's allow-unwrap-in-tests does not cover helpers outside #[test] functions.
#![allow(clippy::unwrap_used)]

use clap::Parser;
use linear_rat::auth::KeyringEntry;
use linear_rat::cli::{AuthCommand, Cli, Command, CompletionShell};

fn parse(args: &[&str]) -> Option<Command> {
    Cli::try_parse_from(args).unwrap().command
}

#[test]
fn lrat_alone_has_no_subcommand() {
    assert!(parse(&["lrat"]).is_none());
}

#[test]
fn demo_cannot_be_combined_with_a_subcommand() {
    let error = Cli::try_parse_from(["lrat", "--demo", "auth", "status"]).unwrap_err();
    assert_eq!(
        error.kind(),
        clap::error::ErrorKind::ArgumentConflict,
        "{error}"
    );
    assert!(Cli::try_parse_from(["lrat", "auth", "logout", "--demo"]).is_err());
    assert!(Cli::try_parse_from(["lrat", "--demo"]).unwrap().demo);
}

#[test]
fn the_auth_subcommands_parse() {
    assert!(matches!(
        parse(&["lrat", "auth", "login"]),
        Some(Command::Auth(AuthCommand::Login {
            stdin: false,
            from_keyring: None
        }))
    ));
    assert!(matches!(
        parse(&["lrat", "auth", "login", "--stdin"]),
        Some(Command::Auth(AuthCommand::Login {
            stdin: true,
            from_keyring: None
        }))
    ));
    assert!(matches!(
        parse(&["lrat", "auth", "status"]),
        Some(Command::Auth(AuthCommand::Status))
    ));
    assert!(matches!(
        parse(&["lrat", "auth", "logout"]),
        Some(Command::Auth(AuthCommand::Logout))
    ));
}

fn from_keyring(args: &[&str]) -> Option<KeyringEntry> {
    match parse(args) {
        Some(Command::Auth(AuthCommand::Login { from_keyring, .. })) => from_keyring,
        other => panic!("not a login: {other:?}"),
    }
}

#[test]
fn from_keyring_without_a_value_reads_linear_api_key() {
    let entry = from_keyring(&["lrat", "auth", "login", "--from-keyring"]).unwrap();

    assert_eq!(entry.service(), "linear");
    assert_eq!(entry.account(), "api-key");
}

#[test]
fn from_keyring_with_a_value_reads_that_service_and_account() {
    let spaced =
        from_keyring(&["lrat", "auth", "login", "--from-keyring", "work/linear-key"]).unwrap();
    let joined = from_keyring(&[
        "lrat",
        "auth",
        "login",
        "--from-keyring=pass/me@example.com",
    ])
    .unwrap();

    assert_eq!((spaced.service(), spaced.account()), ("work", "linear-key"));
    assert_eq!(
        (joined.service(), joined.account()),
        ("pass", "me@example.com")
    );
}

#[test]
fn an_entry_splits_at_the_first_slash_so_the_account_may_hold_one() {
    let entry =
        from_keyring(&["lrat", "auth", "login", "--from-keyring", "linear/team/key"]).unwrap();

    assert_eq!((entry.service(), entry.account()), ("linear", "team/key"));
}

#[test]
fn an_entry_needs_both_a_service_and_an_account() {
    for bad in ["linear", "/api-key", "linear/", ""] {
        let error =
            Cli::try_parse_from(["lrat", "auth", "login", "--from-keyring", bad]).unwrap_err();
        assert_eq!(
            error.kind(),
            clap::error::ErrorKind::ValueValidation,
            "{bad}: {error}"
        );
        assert!(error.to_string().contains("SERVICE/ACCOUNT"), "{error}");
    }
}

#[test]
fn from_keyring_conflicts_with_stdin() {
    let error =
        Cli::try_parse_from(["lrat", "auth", "login", "--stdin", "--from-keyring"]).unwrap_err();

    assert_eq!(
        error.kind(),
        clap::error::ErrorKind::ArgumentConflict,
        "{error}"
    );
}

#[test]
fn login_help_names_the_supported_keychains_and_how_to_point_at_another_entry() {
    let error = Cli::try_parse_from(["lrat", "auth", "login", "--help"]).unwrap_err();
    let help = error.to_string();

    assert_eq!(error.kind(), clap::error::ErrorKind::DisplayHelp);
    assert!(
        help.contains("--from-keyring [<SERVICE/ACCOUNT>]"),
        "{help}"
    );
    assert!(help.contains("Secret Service"), "{help}");
    assert!(help.contains("Keychain"), "{help}");
    assert!(help.contains("linear/api-key"), "{help}");
}

#[test]
fn completions_takes_bash_zsh_or_fish() {
    for (name, shell) in [
        ("bash", CompletionShell::Bash),
        ("zsh", CompletionShell::Zsh),
        ("fish", CompletionShell::Fish),
    ] {
        let parsed = parse(&["lrat", "completions", name]);
        assert!(
            matches!(parsed, Some(Command::Completions { shell: s }) if s == shell),
            "{name}"
        );
    }
    let error = Cli::try_parse_from(["lrat", "completions", "powershell"]).unwrap_err();
    assert_eq!(
        error.kind(),
        clap::error::ErrorKind::InvalidValue,
        "{error}"
    );
}

fn help(args: &[&str]) -> String {
    Cli::try_parse_from(args).unwrap_err().to_string()
}

#[test]
fn top_level_help_says_how_to_log_in() {
    let help = help(&["lrat", "--help"]);
    assert!(help.contains("lrat auth login"), "{help}");
    assert!(help.contains("LINEAR_API_KEY"), "{help}");
    assert!(help.contains("lrat --demo"), "{help}");
}

#[test]
fn auth_help_shows_each_way_to_log_in() {
    let help = help(&["lrat", "auth", "--help"]);
    for way in [
        "lrat auth login",
        "lrat auth login --stdin",
        "lrat auth login --from-keyring",
        "lrat auth status",
        "linear.app/settings/account/security",
    ] {
        assert!(help.contains(way), "missing {way:?} in:\n{help}");
    }
}

/// The help of `path`, from `lrat` down, as a terminal would get it,
/// escape codes and all.
fn styled_help(path: &[&str]) -> String {
    use clap::CommandFactory;
    let mut command = Cli::command();
    command.build();
    let mut command = &mut command;
    for name in path {
        command = command.find_subcommand_mut(name).unwrap();
    }
    command.render_help().ansi().to_string()
}

const BOLD_UNDERLINED: &str = "\x1b[1m\x1b[4m";
const ACCENT: &str = "\x1b[1m\x1b[36m";
const MUTED: &str = "\x1b[2m";

#[test]
fn help_headings_are_bold_and_underlined() {
    let help = styled_help(&[]);
    for heading in ["Usage:", "Commands:", "Options:", "Getting started:"] {
        assert!(
            help.contains(&format!("{BOLD_UNDERLINED}{heading}")),
            "{heading} in {help:?}"
        );
    }
}

#[test]
fn help_accents_commands_and_flags_and_mutes_placeholders() {
    let help = styled_help(&[]);
    for literal in ["--demo", "auth", "lrat auth login", "lrat --demo"] {
        assert!(
            help.contains(&format!("{ACCENT}{literal}")),
            "{literal} in {help:?}"
        );
    }
    let login = styled_help(&["auth", "login"]);
    assert!(
        login.contains(&format!("{MUTED}<SERVICE/ACCOUNT>")),
        "{login:?}"
    );
}

#[test]
fn auth_help_accents_each_way_to_log_in() {
    let help = styled_help(&["auth"]);
    for way in ["lrat auth login --stdin", "lrat auth status"] {
        assert!(
            help.contains(&format!("{ACCENT}{way}")),
            "{way} in {help:?}"
        );
    }
}

#[test]
fn piped_help_has_no_escape_codes() {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_lrat"))
        .arg("--help")
        .env_remove("CLICOLOR_FORCE")
        .output()
        .unwrap();
    let help = String::from_utf8(output.stdout).unwrap();
    assert!(help.contains("Getting started"), "{help}");
    assert!(!help.contains('\x1b'), "{help:?}");
}
