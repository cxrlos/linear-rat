//! The terminal guard: the one owner of raw mode, the alternate screen and
//! focus reporting, which it also hands to `$EDITOR` and to the shell on
//! suspend, and takes back.

use std::backtrace::Backtrace;
use std::fs::{self, DirBuilder, OpenOptions};
use std::future::Future;
use std::io::{self, Write};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::panic;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Once;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use anyhow::{Context, bail};

use crossterm::event::{DisableFocusChange, EnableFocusChange};
use crossterm::terminal::{self, EnterAlternateScreen, LeaveAlternateScreen};
use crossterm::{cursor, execute};
use futures_util::{Stream, stream};
use jiff::Timestamp;
use rustix::process::{Signal, kill_current_process_group};
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
        // From here on, an early return drops the guard and restores.
        let guard = Self(());
        take_over()?;
        Ok(guard)
    }
}

/// Raw mode, the alternate screen, focus reporting and a hidden cursor.
fn take_over() -> io::Result<()> {
    terminal::enable_raw_mode()?;
    ACTIVE.store(true, Ordering::SeqCst);
    execute!(
        io::stdout(),
        EnterAlternateScreen,
        EnableFocusChange,
        cursor::Hide
    )
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
    if let Err(error) = execute!(
        io::stdout(),
        DisableFocusChange,
        LeaveAlternateScreen,
        cursor::Show
    ) {
        tracing::warn!(%error, "could not leave the alternate screen");
    }
}

/// Opens the viewer's editor on `text` and returns the file as they saved
/// it. The editor is `$VISUAL`, else `$EDITOR`, else `vi`, run by `sh` so a
/// value with arguments works. The file is mode 0600 in a fresh directory
/// only the viewer can enter, removed afterwards. While the editor runs, the
/// terminal is out of raw mode and on the main screen; it is taken back on
/// every way out, panics included, and the caller must draw the whole screen
/// again.
pub fn edit(text: &str) -> anyhow::Result<String> {
    let editor = editor_command(std::env::var("VISUAL").ok(), std::env::var("EDITOR").ok());
    edit_with(&editor, text)
}

/// [`edit`] with the editor command `editor`.
fn edit_with(editor: &str, text: &str) -> anyhow::Result<String> {
    let dir = PrivateDir::create()?;
    let path = dir.0.join("lrat.md");
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&path)
        .with_context(|| format!("could not create {}", path.display()))?;
    file.write_all(text.as_bytes())
        .and_then(|()| file.sync_all())
        .with_context(|| format!("could not write {}", path.display()))?;
    drop(file);

    let status = {
        let _handed_over = HandedOver::begin();
        Command::new("sh")
            .arg("-c")
            .arg(format!("{editor} \"$1\""))
            .arg("sh")
            .arg(&path)
            .status()
    }
    .with_context(|| format!("could not start `{editor}`"))?;
    if !status.success() {
        bail!("`{editor}` exited with {status}");
    }
    fs::read_to_string(&path).with_context(|| format!("could not read {}", path.display()))
}

/// Writes `text` to a new file under the system temp directory that only
/// the viewer can read, named after `stem`, and returns its path. The file
/// stays: it holds what the viewer wrote that lrat could not save.
pub fn keep(text: &str, stem: &str) -> anyhow::Result<PathBuf> {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    let base = std::env::temp_dir();
    let mut attempts = 0;
    loop {
        let n = NEXT.fetch_add(1, Ordering::SeqCst);
        let path = base.join(format!("lrat-{stem}-{}-{n}.md", std::process::id()));
        // A name already taken, perhaps by another user, is skipped rather
        // than written through.
        let mut file = match OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&path)
        {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists && attempts < 100 => {
                attempts += 1;
                continue;
            }
            Err(error) => {
                return Err(error).with_context(|| format!("could not create {}", path.display()));
            }
        };
        file.write_all(text.as_bytes())
            .and_then(|()| file.sync_all())
            .with_context(|| format!("could not write {}", path.display()))?;
        return Ok(path);
    }
}

/// The editor to run: the first of `visual` and `editor` that is set and
/// not blank, else `vi`.
fn editor_command(visual: Option<String>, editor: Option<String>) -> String {
    [visual, editor]
        .into_iter()
        .flatten()
        .map(|value| value.trim().to_owned())
        .find(|value| !value.is_empty())
        .unwrap_or_else(|| "vi".to_owned())
}

/// A directory under the system temp directory that only the viewer can
/// enter, removed with what is in it on drop.
struct PrivateDir(PathBuf);

impl PrivateDir {
    fn create() -> anyhow::Result<Self> {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let base = std::env::temp_dir();
        let mut attempts = 0;
        loop {
            let n = NEXT.fetch_add(1, Ordering::SeqCst);
            let path = base.join(format!("lrat-{}-{n}", std::process::id()));
            // A name already taken, perhaps by another user, is skipped
            // rather than reused.
            match DirBuilder::new().mode(0o700).create(&path) {
                Ok(()) => return Ok(Self(path)),
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists && attempts < 100 => {
                    attempts += 1;
                }
                Err(error) => {
                    return Err(error)
                        .with_context(|| format!("could not create {}", path.display()));
                }
            }
        }
    }
}

impl Drop for PrivateDir {
    fn drop(&mut self) {
        if let Err(error) = fs::remove_dir_all(&self.0) {
            tracing::warn!(%error, path = %self.0.display(), "could not remove the editor's directory");
        }
    }
}

/// The terminal handed to another program: out of raw mode and on the main
/// screen. Dropping it takes the terminal back, if the guard held it before.
struct HandedOver {
    held: bool,
}

impl HandedOver {
    fn begin() -> Self {
        let held = ACTIVE.load(Ordering::SeqCst);
        restore();
        Self { held }
    }
}

impl Drop for HandedOver {
    fn drop(&mut self) {
        if self.held
            && let Err(error) = take_over()
        {
            tracing::warn!(%error, "could not take the terminal back");
        }
    }
}

/// Suspends lrat as a shell's job: hands the terminal back as for the
/// editor, stops lrat's process group with SIGSTOP, and once something
/// continues it, such as `fg`, takes the terminal back. The caller must draw
/// the whole screen again. Raw mode turns Ctrl-Z into a key press, so this is
/// the only way lrat stops with the terminal restored.
pub fn suspend() -> io::Result<()> {
    let _handed_over = HandedOver::begin();
    // The whole group, so a wrapper such as `cargo run` stops too and the
    // shell gets the terminal back.
    kill_current_process_group(Signal::STOP)?;
    Ok(())
}

/// Takes the terminal again if the guard holds it, for after a stop lrat did
/// not ask for, such as `kill -STOP`, in case the shell reset the terminal
/// meanwhile. The caller must draw the whole screen again.
pub fn resume() -> io::Result<()> {
    if ACTIVE.load(Ordering::SeqCst) {
        take_over()?;
    }
    Ok(())
}

/// What job control signals ask of lrat. Neither ends the runtime.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobControl {
    /// SIGTSTP from outside, as `kill -TSTP` sends: [`suspend`], as the key
    /// does.
    Suspend,
    /// SIGCONT: lrat runs again, so it [`resume`]s and draws everything.
    Continue,
}

/// SIGTSTP and SIGCONT as [`JobControl`]. Listening replaces the default
/// stop on SIGTSTP, which would leave the terminal in raw mode.
pub fn job_control() -> io::Result<impl Stream<Item = JobControl>> {
    let each = |kind, what| {
        Ok::<_, io::Error>(stream::unfold(
            signal(kind)?,
            move |mut signal| async move { signal.recv().await.map(|()| (what, signal)) },
        ))
    };
    Ok(stream::select(
        Box::pin(each(
            SignalKind::from_raw(Signal::TSTP.as_raw()),
            JobControl::Suspend,
        )?),
        Box::pin(each(
            SignalKind::from_raw(Signal::CONT.as_raw()),
            JobControl::Continue,
        )?),
    ))
}

/// Restores the terminal before the panic message prints, then saves a crash
/// report and names it last, so it is the line the viewer sees.
fn restore_before_panic_message() {
    let print_panic = panic::take_hook();
    panic::set_hook(Box::new(move |info| {
        restore();
        tracing::error!("{info}");
        print_panic(info);
        let report = crash_report(&info.to_string(), &Backtrace::force_capture().to_string());
        let saved = crate::logging::state_dir()
            .and_then(|dir| Ok(save_crash_report(&dir, &report, Timestamp::now())?));
        match saved {
            Ok(path) => eprintln!("lrat crashed; the crash report is in {}", path.display()),
            Err(error) => eprintln!("lrat crashed and could not save a crash report: {error:#}"),
        }
    }));
}

/// What a crash report says: the version, the panic's message and place,
/// and the backtrace. Nothing else goes in: not the config, not the
/// environment, not the key.
fn crash_report(panic: &str, backtrace: &str) -> String {
    format!(
        "lrat {} crashed.\n\n{panic}\n\nBacktrace:\n{backtrace}\n",
        env!("CARGO_PKG_VERSION")
    )
}

/// Writes `report` to a new file, readable only by the viewer, in `dir`,
/// named for the time `at`, and returns its path.
fn save_crash_report(dir: &Path, report: &str, at: Timestamp) -> io::Result<PathBuf> {
    fs::create_dir_all(dir)?;
    let stamp = at.strftime("%Y%m%dT%H%M%SZ");
    let pid = std::process::id();
    let mut n = 0;
    loop {
        let path = dir.join(format!("crash-{stamp}-{pid}-{n}.txt"));
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&path)
        {
            Ok(mut file) => {
                file.write_all(report.as_bytes())?;
                return Ok(path);
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists && n < 100 => n += 1,
            Err(error) => return Err(error),
        }
    }
}

/// Resolves on SIGINT, SIGTERM or SIGHUP, so the runtime can return and the
/// guard can restore the terminal instead of the process dying in raw mode.
/// Raw mode turns Ctrl-C into a key press, so SIGINT only comes from outside,
/// as with `kill -INT`.
pub fn shutdown_signal() -> io::Result<impl Future<Output = ()>> {
    let mut interrupt = signal(SignalKind::interrupt())?;
    let mut terminate = signal(SignalKind::terminate())?;
    let mut hangup = signal(SignalKind::hangup())?;
    Ok(async move {
        tokio::select! {
            _ = interrupt.recv() => {}
            _ = terminate.recv() => {}
            _ = hangup.recv() => {}
        }
    })
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt;

    use super::*;

    #[test]
    fn the_editor_is_visual_then_editor_then_vi() {
        let some = |value: &str| Some(value.to_owned());

        assert_eq!(
            editor_command(some("code --wait"), some("nano")),
            "code --wait"
        );
        assert_eq!(editor_command(some("  "), some("nano")), "nano");
        assert_eq!(editor_command(None, some(" hx ")), "hx");
        assert_eq!(editor_command(None, None), "vi");
    }

    #[test]
    fn the_editor_gets_the_text_and_what_it_saves_comes_back() {
        let editor = r#"f() { tr a-z A-Z < "$1" > "$1.up" && mv "$1.up" "$1"; }; f"#;

        let saved = edit_with(editor, "a comment\n").unwrap();

        assert_eq!(saved, "A COMMENT\n");
    }

    #[test]
    fn kept_text_lands_in_a_file_only_the_viewer_can_read() {
        use std::os::unix::fs::PermissionsExt;

        let path = keep("Title\n\nBody\n", "DES-34").unwrap();
        let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        let text = fs::read_to_string(&path).unwrap();
        fs::remove_file(&path).unwrap();

        assert_eq!(mode, 0o600);
        assert_eq!(text, "Title\n\nBody\n");
        assert!(
            path.file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("lrat-DES-34-")
        );
    }

    #[test]
    fn the_file_is_only_readable_by_the_viewer() {
        let editor = r#"f() { ls -ld "$1" "$(dirname "$1")" | cut -c1-10 > "$1"; }; f"#;

        let saved = edit_with(editor, "").unwrap();

        let modes: Vec<&str> = saved.lines().collect();
        assert!(modes.contains(&"-rw-------"), "{saved}");
        assert!(modes.contains(&"drwx------"), "{saved}");
    }

    #[test]
    fn an_editor_that_fails_is_an_error() {
        let error = edit_with("false", "draft").unwrap_err();

        assert!(
            format!("{error:#}").contains("`false` exited with"),
            "{error:#}"
        );
    }

    #[test]
    fn a_crash_report_is_saved_where_only_the_viewer_can_read_it() {
        let dir = PrivateDir::create().unwrap();
        let state = dir.0.join("state");
        let at: Timestamp = "2026-11-16T09:30:05Z".parse().unwrap();

        let path = save_crash_report(&state, "panicked at src/x.rs:1:1:\nboom", at).unwrap();

        let name = path.file_name().unwrap().to_str().unwrap();
        assert!(
            name.starts_with("crash-20261116T093005Z-") && name.ends_with(".txt"),
            "{name}"
        );
        assert_eq!(path.parent(), Some(state.as_path()));
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "panicked at src/x.rs:1:1:\nboom"
        );
        let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }

    #[test]
    fn two_crashes_in_one_second_keep_both_reports() {
        let dir = PrivateDir::create().unwrap();
        let at: Timestamp = "2026-11-16T09:30:05Z".parse().unwrap();

        let first = save_crash_report(&dir.0, "one", at).unwrap();
        let second = save_crash_report(&dir.0, "two", at).unwrap();

        assert_ne!(first, second);
        assert_eq!(fs::read_to_string(first).unwrap(), "one");
    }

    #[test]
    fn the_report_names_the_version_the_panic_and_the_backtrace() {
        let report = crash_report("panicked at src/x.rs:1:1:\nboom", "0: main");

        assert!(report.starts_with(&format!("lrat {} crashed.\n", env!("CARGO_PKG_VERSION"))));
        assert!(report.contains("panicked at src/x.rs:1:1:\nboom\n"));
        assert!(report.ends_with("Backtrace:\n0: main\n"));
    }

    #[test]
    fn the_private_directory_goes_away_when_dropped() {
        let dir = PrivateDir::create().unwrap();
        let path = dir.0.clone();
        fs::write(path.join("lrat.md"), "x").unwrap();

        drop(dir);

        assert!(!path.exists());
    }
}
