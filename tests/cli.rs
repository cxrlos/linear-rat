// clippy's allow-unwrap-in-tests does not cover helpers outside #[test] functions.
#![allow(clippy::unwrap_used)]

use clap::Parser;
use linear_rat::cli::{AuthCommand, Cli, Command};

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
        Some(Command::Auth(AuthCommand::Login { stdin: false }))
    ));
    assert!(matches!(
        parse(&["lrat", "auth", "login", "--stdin"]),
        Some(Command::Auth(AuthCommand::Login { stdin: true }))
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
