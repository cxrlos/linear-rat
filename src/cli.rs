use clap::{Parser, Subcommand, ValueEnum};

use crate::auth::KeyringEntry;
use crate::open::IssueKey;

#[derive(Debug, Parser)]
#[command(
    name = "lrat",
    version,
    about,
    args_conflicts_with_subcommands = true,
    after_help = "Getting started:\n  \
        lrat auth login    store a Linear API key in the keyring (see `lrat auth --help`)\n  \
        lrat               open My Issues\n  \
        lrat --demo        try lrat without an account\n\n\
        LINEAR_API_KEY, when set, is used instead of the keyring."
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
    #[command(
        subcommand,
        after_help = "Create a personal API key at https://linear.app/settings/account/security, then:\n  \
            lrat auth login                  paste the key at the prompt\n  \
            lrat auth login --stdin          read it from a pipe, as in `pass linear | lrat auth login --stdin`\n  \
            lrat auth login --from-keyring   copy it from an existing keyring entry (default linear/api-key)\n  \
            lrat auth status                 check where the key comes from and which workspace it opens\n\n\
            LINEAR_API_KEY, when set, wins over the keyring. `[auth] store` in config.toml picks the keyring."
    )]
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
