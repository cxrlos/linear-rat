//! The terminal guard: the one owner of raw mode and the alternate screen.

use std::future::Future;
use std::io;
use std::panic;
use std::sync::Once;
use std::sync::atomic::{AtomicBool, Ordering};

use crossterm::terminal::{self, EnterAlternateScreen, LeaveAlternateScreen};
use crossterm::{cursor, execute};
use tokio::signal::unix::{SignalKind, signal};

static ACTIVE: AtomicBool = AtomicBool::new(false);
static PANIC_HOOK: Once = Once::new();

/// Holds the terminal in raw mode on the alternate screen. Dropping it, or a
/// panic anywhere while it lives, puts the terminal back.
#[derive(Debug)]
pub struct TerminalGuard(());

impl TerminalGuard {
    pub fn enter() -> io::Result<Self> {
        PANIC_HOOK.call_once(restore_before_panic_message);
        terminal::enable_raw_mode()?;
        ACTIVE.store(true, Ordering::SeqCst);
        // From here on, an early return drops the guard and restores.
        let guard = Self(());
        execute!(io::stdout(), EnterAlternateScreen, cursor::Hide)?;
        Ok(guard)
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        restore();
    }
}

/// Leaves raw mode and the alternate screen. Safe to call more than once.
fn restore() {
    if !ACTIVE.swap(false, Ordering::SeqCst) {
        return;
    }
    if let Err(error) = terminal::disable_raw_mode() {
        tracing::warn!(%error, "could not leave raw mode");
    }
    if let Err(error) = execute!(io::stdout(), LeaveAlternateScreen, cursor::Show) {
        tracing::warn!(%error, "could not leave the alternate screen");
    }
}

fn restore_before_panic_message() {
    let print_panic = panic::take_hook();
    panic::set_hook(Box::new(move |info| {
        restore();
        tracing::error!("{info}");
        print_panic(info);
    }));
}

/// Resolves on SIGTERM or SIGHUP, so the runtime can return and the guard can
/// restore the terminal instead of the process dying in raw mode.
pub fn shutdown_signal() -> io::Result<impl Future<Output = ()>> {
    let mut terminate = signal(SignalKind::terminate())?;
    let mut hangup = signal(SignalKind::hangup())?;
    Ok(async move {
        tokio::select! {
            _ = terminate.recv() => {}
            _ = hangup.recv() => {}
        }
    })
}
