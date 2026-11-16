//! Shell completion scripts, generated from the clap definition in `cli.rs`.

use clap::CommandFactory;
use clap_complete::Shell;

use crate::cli::{Cli, CompletionShell};

/// The name the scripts complete: the binary, not the crate.
const BIN_NAME: &str = "lrat";

/// The completion script for `shell`.
pub fn script(shell: CompletionShell) -> Vec<u8> {
    let shell = match shell {
        CompletionShell::Bash => Shell::Bash,
        CompletionShell::Zsh => Shell::Zsh,
        CompletionShell::Fish => Shell::Fish,
    };
    // Built in memory: clap_complete panics when its writer fails, and stdout
    // fails on a closed pipe.
    let mut script = Vec::new();
    clap_complete::generate(shell, &mut Cli::command(), BIN_NAME, &mut script);
    script
}
