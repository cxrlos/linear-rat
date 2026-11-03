//! Checks that a panic restores the terminal before its message prints. Run
//! `cargo run --example force_panic`: the message should appear on the normal
//! screen, and `stty -a` afterwards should show `icanon` and `echo`.

use std::io;

use linear_rat::terminal::TerminalGuard;

fn main() -> io::Result<()> {
    let _terminal = TerminalGuard::enter()?;
    panic!("forced panic to check that the terminal is restored");
}
