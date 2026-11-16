//! End-to-end checks of the terminal from outside lrat: the real binary runs
//! with `--demo` in a pseudo-terminal, under empty XDG directories, and the
//! tests read the terminal's mode and output as a shell would.

// clippy's allow-unwrap-in-tests does not cover helpers outside #[test] functions.
#![allow(clippy::unwrap_used)]

use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use portable_pty::{Child, CommandBuilder, MasterPty, PtySize, SlavePty, native_pty_system};
use rustix::termios::LocalModes;

/// How long anything may take before a test gives up.
const PATIENCE: Duration = Duration::from_secs(10);
const ENTER_ALTERNATE_SCREEN: &str = "\x1b[?1049h";
const LEAVE_ALTERNATE_SCREEN: &str = "\x1b[?1049l";
/// Text the demo's first frame shows.
const DRAWN: &str = "My Issues";

/// A program running in a pseudo-terminal of 100 by 30 cells.
struct Pty {
    master: Box<dyn MasterPty + Send>,
    /// Held so the terminal outlives the program and its mode can still be
    /// read once it exits.
    _slave: Box<dyn SlavePty + Send>,
    child: Box<dyn Child + Send + Sync>,
    writer: Box<dyn Write + Send>,
    output: Arc<Mutex<Vec<u8>>>,
    home: TempDir,
}

impl Pty {
    fn lrat() -> Self {
        Self::spawn(Path::new(env!("CARGO_BIN_EXE_lrat")), &["--demo"])
    }

    /// `program` with `args`, its HOME and XDG directories fresh and empty.
    fn spawn(program: &Path, args: &[&str]) -> Self {
        Self::spawn_with(program, args, &[])
    }

    /// [`Pty::spawn`] with the environment variables `env` set too.
    fn spawn_with(program: &Path, args: &[&str], env: &[(&str, &str)]) -> Self {
        let home = TempDir::new();
        let pair = native_pty_system()
            .openpty(PtySize {
                rows: 30,
                cols: 100,
                pixel_width: 0,
                pixel_height: 0,
            })
            .unwrap();
        let mut command = CommandBuilder::new(program);
        command.args(args);
        command.env_clear();
        command.env("PATH", std::env::var_os("PATH").unwrap_or_default());
        command.env("TERM", "xterm-256color");
        command.env("HOME", &home.0);
        for (var, value) in env {
            command.env(var, value);
        }
        for (var, dir) in [
            ("XDG_CONFIG_HOME", "config"),
            ("XDG_CACHE_HOME", "cache"),
            ("XDG_STATE_HOME", "state"),
        ] {
            let path = home.0.join(dir);
            fs::create_dir(&path).unwrap();
            command.env(var, path);
        }
        let child = pair.slave.spawn_command(command).unwrap();

        let output = Arc::new(Mutex::new(Vec::new()));
        let mut reader = pair.master.try_clone_reader().unwrap();
        let sink = Arc::clone(&output);
        thread::spawn(move || {
            let mut buffer = [0; 4096];
            while let Ok(read @ 1..) = reader.read(&mut buffer) {
                sink.lock().unwrap().extend_from_slice(&buffer[..read]);
            }
        });
        let writer = pair.master.take_writer().unwrap();
        Self {
            master: pair.master,
            _slave: pair.slave,
            child,
            writer,
            output,
            home,
        }
    }

    fn pid(&self) -> u32 {
        self.child.process_id().unwrap()
    }

    fn output(&self) -> String {
        String::from_utf8_lossy(&self.output.lock().unwrap()).into_owned()
    }

    /// Everything the program wrote from byte `from` on.
    fn output_since(&self, from: usize) -> String {
        String::from_utf8_lossy(&self.output.lock().unwrap()[from..]).into_owned()
    }

    fn written(&self) -> usize {
        self.output.lock().unwrap().len()
    }

    fn press(&mut self, bytes: &[u8]) {
        self.writer.write_all(bytes).unwrap();
        self.writer.flush().unwrap();
    }

    fn signal(&self, name: &str) {
        let sent = Command::new("kill")
            .arg(format!("-{name}"))
            .arg(self.pid().to_string())
            .status()
            .unwrap();
        assert!(sent.success(), "kill -{name} failed");
    }

    /// Whether the terminal is cooked: line editing and echo on, as a shell
    /// prompt needs.
    fn cooked(&self) -> bool {
        let termios = self.master.get_termios().unwrap();
        let flags = u64::from(termios.local_flags.bits());
        let wanted = u64::from((LocalModes::ICANON | LocalModes::ECHO).bits());
        flags & wanted == wanted
    }

    /// Whether the process is stopped, as job control shows it.
    fn stopped(&self) -> bool {
        let ps = Command::new("ps")
            .args(["-o", "stat=", "-p", &self.pid().to_string()])
            .output()
            .unwrap();
        String::from_utf8_lossy(&ps.stdout)
            .trim_start()
            .starts_with('T')
    }

    fn exited(&mut self) -> bool {
        self.child.try_wait().unwrap().is_some()
    }

    /// Waits until `ready` holds, failing the test with `what` and the
    /// output so far if it does not in time.
    fn wait_until(&mut self, what: &str, mut ready: impl FnMut(&mut Self) -> bool) {
        let deadline = Instant::now() + PATIENCE;
        while !ready(self) {
            assert!(
                Instant::now() < deadline,
                "timed out waiting until {what}; output so far: {:?}",
                self.output()
            );
            thread::sleep(Duration::from_millis(10));
        }
    }

    /// Waits for lrat to draw the demo in raw mode.
    fn wait_for_first_frame(&mut self) {
        self.wait_until("lrat draws the demo", |pty| pty.output().contains(DRAWN));
        assert!(!self.cooked(), "lrat drew without raw mode");
    }

    fn quit(&mut self) {
        self.press(b"q");
        self.wait_until("lrat exits", Self::exited);
    }
}

impl Drop for Pty {
    fn drop(&mut self) {
        // A failed test must not leave a stopped lrat behind.
        if !self.exited() {
            let _ = self.child.kill();
        }
    }
}

/// A fresh directory under the system temp directory, removed on drop.
struct TempDir(PathBuf);

impl TempDir {
    fn new() -> Self {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let n = NEXT.fetch_add(1, Ordering::SeqCst);
        let path = std::env::temp_dir().join(format!("lrat-pty-{}-{n}", std::process::id()));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn quitting_leaves_the_terminal_cooked_on_the_main_screen() {
    let mut pty = Pty::lrat();
    pty.wait_for_first_frame();

    pty.quit();

    assert!(pty.cooked());
    let output = pty.output();
    assert!(output.rfind(LEAVE_ALTERNATE_SCREEN) > output.rfind(ENTER_ALTERNATE_SCREEN));
}

#[test]
fn sigterm_while_drawing_leaves_the_terminal_cooked() {
    let mut pty = Pty::lrat();
    pty.wait_until("lrat starts drawing", |pty| {
        pty.output().contains(ENTER_ALTERNATE_SCREEN)
    });

    pty.signal("TERM");

    pty.wait_until("lrat exits", Pty::exited);
    assert!(pty.cooked());
    assert!(pty.output().contains(LEAVE_ALTERNATE_SCREEN));
}

/// Suspends lrat by `suspend`, checks the shell would get a cooked terminal,
/// continues it and checks it takes the terminal back and draws it all.
fn suspend_and_continue(suspend: impl FnOnce(&mut Pty)) {
    let mut pty = Pty::lrat();
    pty.wait_for_first_frame();
    let before = pty.written();

    suspend(&mut pty);

    pty.wait_until("lrat stops", |pty| pty.stopped());
    assert!(pty.cooked());
    assert!(pty.output_since(before).contains(LEAVE_ALTERNATE_SCREEN));

    let stopped_at = pty.written();
    pty.signal("CONT");

    pty.wait_until("lrat draws again", |pty| {
        pty.output_since(stopped_at).contains(DRAWN)
    });
    assert!(!pty.stopped());
    assert!(!pty.cooked(), "lrat came back without raw mode");
    assert!(
        pty.output_since(stopped_at)
            .contains(ENTER_ALTERNATE_SCREEN)
    );
    pty.quit();
    assert!(pty.cooked());
}

#[test]
fn ctrl_z_stops_lrat_with_the_terminal_cooked_and_sigcont_brings_it_back() {
    suspend_and_continue(|pty| pty.press(b"\x1a"));
}

#[test]
fn sigtstp_from_outside_does_what_ctrl_z_does() {
    suspend_and_continue(|pty| pty.signal("TSTP"));
}

/// Set to make [`panics_inside_the_guard_when_asked`] panic.
const FORCE_PANIC: &str = "LRAT_PTY_FORCE_PANIC";
const PANIC_MESSAGE: &str = "forced panic to check that the terminal is restored";
/// Not a real key: it must never reach a crash report.
const SENTINEL_KEY: &str = "lin_api_sentinel_never_in_a_crash_report";

/// Passes at once unless [`FORCE_PANIC`] is set, when it panics with the
/// terminal guard held, as `examples/force_panic.rs` does. The panic test
/// runs it in a pseudo-terminal from this same test binary, so it is never
/// older than the code it checks.
#[test]
fn panics_inside_the_guard_when_asked() {
    if std::env::var_os(FORCE_PANIC).is_none() {
        return;
    }
    let _guard = linear_rat::terminal::TerminalGuard::enter().unwrap();
    panic!("{PANIC_MESSAGE}");
}

#[test]
fn a_panic_restores_the_terminal_then_names_its_crash_report() {
    let this_test_binary = std::env::current_exe().unwrap();
    let mut pty = Pty::spawn_with(
        &this_test_binary,
        &[
            "panics_inside_the_guard_when_asked",
            "--exact",
            "--nocapture",
            "--test-threads=1",
        ],
        &[(FORCE_PANIC, "1"), ("LINEAR_API_KEY", SENTINEL_KEY)],
    );

    pty.wait_until("the panic names its crash report", |pty| {
        pty.exited() && pty.output().contains("crash report is in")
    });

    assert!(pty.cooked());
    let output = pty.output();
    let named = output.rfind("crash report is in").unwrap();
    assert!(output.rfind(LEAVE_ALTERNATE_SCREEN) < Some(named));
    let state = pty.home.0.join("state").join("lrat");
    let reports: Vec<PathBuf> = fs::read_dir(&state)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.to_string_lossy().contains("crash-"))
        .collect();
    assert_eq!(reports.len(), 1, "{reports:?}");
    let shown = output[named..]
        .lines()
        .next()
        .unwrap()
        .replace(['\r', '\n'], "");
    assert!(
        shown.ends_with(&*reports[0].to_string_lossy()),
        "{shown:?} does not name {}",
        reports[0].display()
    );
    let report = fs::read_to_string(&reports[0]).unwrap();
    assert!(report.contains(PANIC_MESSAGE));
    assert!(report.contains("Backtrace:"));
    assert!(!report.contains(SENTINEL_KEY));
    assert!(!output.contains(SENTINEL_KEY));
}
