use clap::{Parser, Subcommand};

use crate::auth::KeyringEntry;

#[derive(Debug, Parser)]
#[command(name = "lrat", version, about, args_conflicts_with_subcommands = true)]
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
    #[command(subcommand)]
    Auth(AuthCommand),
}

#[derive(Debug, Subcommand)]
pub enum AuthCommand {
    /// Check an API key with Linear and store it in the OS keyring
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
    /// Delete the API key from the OS keyring
    Logout,
}
