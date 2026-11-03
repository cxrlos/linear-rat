use clap::{Parser, Subcommand};

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
    },
    /// Show where the API key comes from, and its workspace and viewer
    Status,
    /// Delete the API key from the OS keyring
    Logout,
}
