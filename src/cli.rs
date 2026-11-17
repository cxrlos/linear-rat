use clap::builder::Styles;
use clap::builder::styling::{AnsiColor, Style};
use clap::{Parser, Subcommand, ValueEnum};

use crate::auth::KeyringEntry;
use crate::open::IssueKey;

/// Headings, as `Usage:` and `Getting started:`.
const HEADING: Style = Style::new().bold().underline();
/// What is typed as written: commands, flags and the commands of examples.
const LITERAL: Style = AnsiColor::Cyan.on_default().bold();
/// What stands for a value, as `<SHELL>`.
const PLACEHOLDER: Style = Style::new().dimmed();

/// The help's styles. clap drops them when stdout is not a terminal or
/// `NO_COLOR` is set, and man pages and completions never carry them.
fn styles() -> Styles {
    Styles::styled()
        .header(HEADING)
        .usage(HEADING)
        .literal(LITERAL)
        .placeholder(PLACEHOLDER)
}

fn heading(text: &str) -> String {
    format!("{HEADING}{text}{HEADING:#}")
}

/// An example's command, styled as a literal and padded to `width` columns
/// so the explanations line up.
fn example(command: &str, width: usize) -> String {
    let padding = " ".repeat(width.saturating_sub(command.len()));
    format!("{LITERAL}{command}{LITERAL:#}{padding}")
}

fn getting_started() -> String {
    format!(
        "{}\n  \
         {}store a Linear API key in the keyring (see `lrat auth --help`)\n  \
         {}open My Issues\n  \
         {}try lrat without an account\n\n\
         LINEAR_API_KEY, when set, is used instead of the keyring.",
        heading("Getting started:"),
        example("lrat auth login", 19),
        example("lrat", 19),
        example("lrat --demo", 19),
    )
}

fn ways_to_log_in() -> String {
    format!(
        "Create a personal API key at https://linear.app/settings/account/security, then:\n  \
         {}paste the key at the prompt\n  \
         {}read it from a pipe, as in `pass linear | lrat auth login --stdin`\n  \
         {}copy it from an existing keyring entry (default linear/api-key)\n  \
         {}check where the key comes from and which workspace it opens\n\n\
         LINEAR_API_KEY, when set, wins over the keyring. `[auth] store` in config.toml picks the keyring.",
        example("lrat auth login", 33),
        example("lrat auth login --stdin", 33),
        example("lrat auth login --from-keyring", 33),
        example("lrat auth status", 33),
    )
}

#[derive(Debug, Parser)]
#[command(
    name = "lrat",
    version,
    about,
    args_conflicts_with_subcommands = true,
    styles = styles(),
    after_help = getting_started()
)]
pub struct Cli {
    /// Run without a Linear account or API key
    #[arg(long)]
    pub demo: bool,

    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Manage the Linear API key
    #[command(subcommand, after_help = ways_to_log_in())]
    Auth(AuthCommand),
    /// Print the completion script for a shell
    Completions {
        /// The shell to complete in
        shell: CompletionShell,
    },
    /// Open an issue's Linear page in the browser
    Open {
        /// The issue key, such as ENG-123, in any case
        key: IssueKey,
    },
    /// Check the API key, Linear, the terminal and the config
    ///
    /// Prints one line per check, marked ● (ok), ▲ (warning) or ✘ (failure),
    /// each warning or failure followed by how to fix it. Never prints the
    /// API key. Exits non-zero when any check fails.
    Doctor,
}

#[derive(Debug, Subcommand)]
pub enum AuthCommand {
    /// Check an API key with Linear and store it in the configured keyring
    Login {
        /// Read the key from standard input instead of prompting for it
        #[arg(long)]
        stdin: bool,
        /// Read the key from an existing OS keyring entry instead of prompting
        /// for it
        ///
        /// The entry is named SERVICE/ACCOUNT and defaults to linear/api-key.
        /// On Linux it is a Secret Service item (GNOME Keyring, KWallet) with
        /// the attributes `service` and `account`, as `secret-tool store`
        /// writes it, or `service` and `username`; on macOS it is a generic
        /// password in the Keychain with that service and account. lrat
        /// checks the key with Linear, then stores a copy under its own entry;
        /// the entry read is left as it is. For another entry, name it:
        /// `--from-keyring work/linear-key`.
        #[arg(
            long,
            value_name = "SERVICE/ACCOUNT",
            num_args = 0..=1,
            default_missing_value = KeyringEntry::DEFAULT,
            conflicts_with = "stdin",
        )]
        from_keyring: Option<KeyringEntry>,
    },
    /// Show where the API key comes from, and its workspace and viewer
    Status,
    /// Delete the API key from the configured keyring
    Logout,
}

/// A shell `lrat completions` writes a script for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum CompletionShell {
    Bash,
    Zsh,
    Fish,
}
