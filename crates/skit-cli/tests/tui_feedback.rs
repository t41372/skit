//! End-to-end tests of the feedback that a person sees in the terminal interface.
//!
//! Each test starts the real `skit` binary on a pseudo-terminal, types the keys a person types,
//! and reads the screen that the terminal shows. The screen is a replay of every byte from the
//! start of the session, so a cell that a later frame left alone still reads correctly.
//!
//! Unix only: the session start waits for the cursor-position question that crossterm asks only
//! on unix (see `LiveTui::spawn` in `terminal_pty.rs`).
#![cfg(unix)]

use std::{
    fs,
    os::unix::fs::PermissionsExt as _,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use portable_pty::{CommandBuilder, PtySize};
use skit_application::form_state::FormStateService;
use skit_domain::parameters::{ParamDecl, ParameterDelivery};
use skit_store::{FileFormStateStore, FileStore};
use tempfile::TempDir;

#[path = "support/pty.rs"]
mod pty;

use pty::{AnswerQueries, PtyChild, final_terminal_screen, strip_terminal_control};

const ROWS: u16 = 30;
const COLUMNS: u16 = 120;
const CTRL_BACKSLASH: &[u8] = b"\x1c";
const CTRL_C: &[u8] = b"\x03";
const CTRL_R: &[u8] = b"\x12";
const CTRL_S: &[u8] = b"\x13";
const DELETE: &[u8] = b"\x1b[3~";
const DOWN: &[u8] = b"\x1b[B";
const ESCAPE: &[u8] = b"\x1b";
const TAB: &[u8] = b"\t";
const BACKSPACE: &[u8] = b"\x7f";

/// A script that prints [`RUNNING`] when it starts, then waits until a key stops it.
///
/// The command line shows `$((6*7))`, so only the output of the script holds the result. The wait
/// is a loop of short sleeps. dash handles Ctrl+C only after its current child ends, so a key that
/// comes after the `echo` and before a long `sleep` starts would let the whole `sleep` run.
///
/// bash and zsh ignore Ctrl+\ (`SIGQUIT`) themselves, and bash is `sh` on macOS. Without the trap,
/// the key would stop only the current `sleep`, and the loop would start the next one. The trap
/// ends the script with 131, the code of a command that `SIGQUIT` stops, on each shell and at each
/// moment of the key.
const LONG_RUN: &str = "trap 'exit 131' QUIT; echo ready-$((6*7)); while :; do sleep 1; done";
const RUNNING: &str = "ready-42";

/// The time that a person usually needs to press a key after the first line of a run shows.
///
/// skit takes the interrupt keys over just after the script starts (see
/// `crates/skit-runtime/src/interrupts.rs`). Until then, a key also stops skit. That moment is
/// shorter than a millisecond, or about 20 milliseconds on a host that is very busy. A person
/// needs at least about 100 milliseconds to see the line and press a key, but a test can press at
/// once.
const REACTION: Duration = Duration::from_millis(300);

/// One library with one entry, and the directories of one session.
struct Fixture {
    data: TempDir,
    state: TempDir,
    config: TempDir,
    home: TempDir,
}

impl Fixture {
    /// The empty directories of one session.
    fn directories() -> Self {
        Self {
            data: TempDir::new().unwrap(),
            state: TempDir::new().unwrap(),
            config: TempDir::new().unwrap(),
            home: TempDir::new().unwrap(),
        }
    }

    /// A library whose one shell entry `hello` is a copy of `body`, added with `skit add`.
    fn script(body: &str) -> Self {
        let fixture = Self::directories();
        let source = fixture.home.path().join("hello.sh");
        fs::write(&source, body).unwrap();
        fixture.add(&source, &[]);
        fixture
    }

    /// Add `source` to the library with `skit add` and the `flags`.
    fn add(&self, source: &Path, flags: &[&str]) {
        let mut arguments = vec!["add", source.to_str().unwrap()];
        arguments.extend(flags);
        arguments.push("--no-input");
        let added = self.output(&arguments);
        assert!(added.status.success(), "{added:?}");
    }

    /// Run `skit` with `args` outside a terminal, in the directories of this session.
    fn output(&self, args: &[&str]) -> std::process::Output {
        std::process::Command::new(env!("CARGO_BIN_EXE_skit"))
            .args(args)
            .current_dir(self.home.path())
            .env("NO_COLOR", "1")
            .env("SKIT_LANG", "en")
            .env("SKIT_DATA_DIR", self.data.path())
            .env("SKIT_STATE_DIR", self.state.path())
            .env("SKIT_CONFIG_DIR", self.config.path())
            .env("HOME", self.home.path())
            .output()
            .unwrap()
    }

    /// A library whose one command entry `Demo` runs `template` through `sh -c`.
    fn command(template: &str) -> Self {
        let fixture = Self::directories();
        let parameter = if template.contains("{name}") {
            concat!(
                "params = [\"name\"]\n",
                "[[parameters]]\n",
                "name = \"name\"\n",
                "delivery = \"placeholder\"\n",
            )
        } else {
            "params = []\n"
        };
        let directory = fixture.data.path().join("scripts/demo");
        fs::create_dir_all(&directory).unwrap();
        fs::write(
            directory.join("meta.toml"),
            format!(
                concat!(
                    "schema = 1\n",
                    "name = \"Demo\"\n",
                    "kind = \"command\"\n",
                    "mode = \"copy\"\n",
                    "source = \"\"\n",
                    "source_hash = \"\"\n",
                    "added_at = \"2026-08-08T00:00:00Z\"\n",
                    "id = \"0123456789abcdef0123456789abcdef\"\n",
                    "workdir = \"invoke\"\n",
                    "description = \"\"\n",
                    "template = {template:?}\n",
                    "{parameter}",
                ),
                template = template,
                parameter = parameter,
            ),
        )
        .unwrap();
        FileStore::new(fixture.data.path())
            .rebuild_registry()
            .unwrap();
        fixture
    }

    fn after_run(self, value: &str) -> Self {
        fs::write(
            self.config.path().join("config.toml"),
            format!("after_run = {value:?}\n"),
        )
        .unwrap();
        self
    }

    fn command_builder(&self) -> CommandBuilder {
        self.skit(&["tui"])
    }

    /// The `skit` binary with `args`, in the directories of this session.
    fn skit(&self, args: &[&str]) -> CommandBuilder {
        let mut command = CommandBuilder::new(PathBuf::from(env!("CARGO_BIN_EXE_skit")));
        command.args(args);
        command.cwd(self.home.path());
        command.env("TERM", "xterm-256color");
        command.env("NO_COLOR", "1");
        command.env("SKIT_LANG", "en");
        command.env("SKIT_DATA_DIR", self.data.path());
        command.env("SKIT_STATE_DIR", self.state.path());
        command.env("SKIT_CONFIG_DIR", self.config.path());
        command.env("HOME", self.home.path());
        command
    }

    fn start(&self) -> Session {
        Session::start(self.command_builder())
    }
}

/// One live interface session.
struct Session {
    pty: PtyChild,
    rows: u16,
    columns: u16,
}

impl Session {
    fn start(command: CommandBuilder) -> Self {
        Self::start_sized(command, ROWS, COLUMNS)
    }

    fn start_sized(command: CommandBuilder, rows: u16, columns: u16) -> Self {
        let mut pty = PtyChild::spawn(
            command,
            PtySize {
                rows,
                cols: columns,
                pixel_width: 0,
                pixel_height: 0,
            },
            AnswerQueries::On,
        );
        pty.wait_cursor_query_after(0);
        let mut session = Self { pty, rows, columns };
        session.screen_with("Library");
        session
    }

    /// Let the child settle, then write.
    fn send(&mut self, bytes: &[u8]) {
        self.pty.settle_quiet(Duration::from_millis(30));
        self.pty.send(bytes);
    }

    /// Write a key whose host effect gives the terminal to a run or an editor.
    ///
    /// The session leaves its screen for the effect. When it comes back, it asks where the cursor
    /// is, so the question marks the end of the effect. Any other effect stays on the screen and
    /// asks nothing: send its key with [`Self::send`] and wait for the screen.
    fn send_effect_key(&mut self, bytes: &[u8]) {
        let checkpoint = self.pty.checkpoint();
        self.send(bytes);
        self.pty.wait_cursor_query_after(checkpoint);
    }

    /// Wait until the screen shows `needle`, and return the screen.
    fn screen_with(&mut self, needle: &str) -> String {
        let (rows, columns) = (self.rows, self.columns);
        self.pty.wait_for_after_rendered(0, needle, |bytes| {
            final_terminal_screen(bytes, rows, columns)
        })
    }

    /// Wait until the screen shows `present` and does not show `absent`, and return the screen.
    ///
    /// A frame can arrive in more than one read, so a wait for one new word alone can return
    /// while an old word is still on the screen.
    fn screen_without(&mut self, present: &str, absent: &str) -> String {
        const READY: &str = "SCREEN-READY";
        let (rows, columns) = (self.rows, self.columns);
        self.pty.wait_for_after_rendered(0, READY, |bytes| {
            let screen = final_terminal_screen(bytes, rows, columns);
            if screen.contains(present) && !screen.contains(absent) {
                READY.to_owned()
            } else {
                String::new()
            }
        });
        self.screen()
    }

    /// The screen as the terminal shows it now.
    fn screen(&mut self) -> String {
        self.pty.settle();
        final_terminal_screen(&self.pty.raw_after(0), self.rows, self.columns)
    }

    /// The terminal grid, for an assertion about one cell's attributes.
    fn grid(&mut self) -> vt100::Parser {
        self.pty.settle();
        let rows = std::num::NonZeroU16::new(self.rows).expect("the grid has rows");
        let columns = std::num::NonZeroU16::new(self.columns).expect("the grid has columns");
        let mut parser = vt100::Parser::new(rows, columns, 0);
        parser.process(&self.pty.raw_after(0));
        parser
    }

    /// Wait until the cell at `row` and `column` passes `check`, and return the cell.
    ///
    /// A busy machine can send a frame after a quiet moment, so one settle can read the frame
    /// before it.
    fn cell_when(
        &mut self,
        (row, column): (u16, u16),
        what: &str,
        check: impl Fn(&vt100::Cell) -> bool,
    ) -> vt100::Cell {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let grid = self.grid();
            let cell = grid
                .screen()
                .cell(row, column)
                .expect("the cell is on the screen")
                .clone();
            if check(&cell) {
                return cell;
            }
            assert!(
                Instant::now() < deadline,
                "the cell never showed {what}: {cell:?}"
            );
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    fn wait_exit(&mut self) -> (u32, String) {
        let status = self.pty.wait_exit_within(Duration::from_secs(30));
        // The last lines can still be on their way from the terminal when the child exits.
        self.pty.settle();
        let history = strip_terminal_control(&String::from_utf8_lossy(&self.pty.raw_after(0)));
        (status.exit_code(), history)
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.pty.kill();
    }
}

/// The row and column of the first cell of `needle` on the screen.
fn position(parser: &vt100::Parser, needle: &str) -> (u16, u16) {
    let screen = parser.screen();
    for row in 0..ROWS {
        let text = screen.contents_between(row, 0, row, COLUMNS);
        if let Some(byte) = text.find(needle) {
            return (row, u16::try_from(text[..byte].chars().count()).unwrap());
        }
    }
    panic!(
        "the screen does not show {needle:?}:\n{}",
        screen.contents()
    );
}

/// The footer line under the chips, without its border.
///
/// It is the whole status only when the status fits on one row. Use [`footer_row_with`] for a
/// status that can wrap.
fn status_row(screen: &str) -> String {
    screen
        .lines()
        .rev()
        .find(|line| line.starts_with('│'))
        .unwrap_or_default()
        .trim_matches(|character| character == '│' || character == ' ')
        .to_owned()
}

/// The footer row that holds `words`, without its border.
///
/// A status longer than one row wraps onto a second row, for example when it names a long
/// temporary path. Its glyph and its first words stay on the first row.
fn footer_row_with(screen: &str, words: &str) -> String {
    screen
        .lines()
        .find(|line| line.starts_with('│') && line.contains(words))
        .unwrap_or_default()
        .trim_matches(|character| character == '│' || character == ' ')
        .to_owned()
}

// Ways this can fail: the reload says nothing; the receipt has no ✓; it never clears, so the
// entry count stays hidden; or it clears before a person can read it.
#[test]
fn a_reload_shows_a_receipt_that_clears_itself_and_brings_the_entry_count_back() {
    let fixture = Fixture::command("echo done");
    let mut session = fixture.start();
    assert_eq!(status_row(&session.screen_with("1/1 entry")), "1/1 entry");

    session.send(CTRL_R);
    let shown = session.screen_with("✓ Library reloaded (1 entry)");
    let since = Instant::now();
    assert_eq!(status_row(&shown), "✓ Library reloaded (1 entry)");

    let cleared = session.screen_with("1/1 entry");
    let stayed = since.elapsed();
    assert_eq!(status_row(&cleared), "1/1 entry");
    assert!(
        stayed >= Duration::from_secs(3),
        "the receipt cleared after {stayed:?}, before a person can read it"
    );
}

/// Sleep until `instant`. Return at once when it is past.
fn sleep_until(instant: Instant) {
    std::thread::sleep(instant.saturating_duration_since(Instant::now()));
}

// Ways this can fail: a second receipt with the same words keeps the time of the first one, so it
// clears before a person can read it or never shows; or it never clears.
#[test]
fn a_repeated_receipt_gets_its_own_time() {
    let fixture = Fixture::command("echo done");
    let mut session = fixture.start();
    session.send(CTRL_R);
    session.screen_with("✓ Library reloaded (1 entry)");
    let first = Instant::now();

    // A second reload, late in the time of the first receipt, gives a receipt with the same words.
    sleep_until(first + Duration::from_secs(3));
    session.send(CTRL_R);
    let second = Instant::now();

    // The time of the first receipt is over. The second receipt still shows.
    sleep_until(first + Duration::from_millis(4_500));
    let shown = session.screen();
    assert_eq!(
        status_row(&shown),
        "✓ Library reloaded (1 entry)",
        "{shown}"
    );

    // It clears itself at the end of its own time.
    session.screen_with("1/1 entry");
    let stayed = second.elapsed();
    assert!(
        stayed >= Duration::from_secs(3),
        "the second receipt cleared after {stayed:?}"
    );
}

// Ways this can fail: the run has no header, a failed run prints no exit code, or the session
// that the run ends exits with 0 instead of the code of the run.
#[test]
fn a_failed_run_that_ends_the_session_prints_its_frame_and_returns_its_exit_code() {
    let fixture = Fixture::command("exit 3");
    let mut session = fixture.start();
    session.send(b"\r");
    session.screen_with("Run Demo");
    session.send(b"\r");
    let (code, history) = session.wait_exit();
    assert_eq!(code, 3, "{history}");
    let header = history.find("── Run Demo ──").expect("the run header");
    let command = history.find("→ ").expect("the command line");
    let failure = history
        .find("✗ Demo exited with code 3")
        .expect("the failure line");
    assert!(header < command && command < failure, "{history}");
    assert_eq!(history.matches("exited with code").count(), 1, "{history}");
    assert!(!history.contains("✓ Demo finished"), "{history}");
}

// Ways this can fail: the session that a successful run ends prints no result line, so the
// shell shows the output of the script and nothing that says the run finished; the line comes
// before the output; it goes out twice; or the session exits with a code other than 0.
#[test]
fn a_successful_run_that_ends_the_session_prints_its_result_line_and_exits_with_zero() {
    let fixture = Fixture::command("echo done");
    let mut session = fixture.start();
    session.send(b"\r");
    session.screen_with("Run Demo");
    session.send(b"\r");
    let (code, history) = session.wait_exit();
    assert_eq!(code, 0, "{history}");
    let header = history.find("── Run Demo ──").expect("the run header");
    // The command line also holds the word, so the last one is the output of the script.
    let output = history.rfind("done").expect("the output of the script");
    let finished = history.find("✓ Demo finished").expect("the result line");
    assert!(header < output && output < finished, "{history}");
    assert_eq!(history.matches("✓ Demo finished").count(), 1, "{history}");
    assert!(!history.contains("exited with code"), "{history}");
}

// Ways this can fail: a run that returns to the Library says nothing about the outcome, the
// failure has no ✗, or the failure never clears.
#[test]
fn a_run_that_stays_reports_its_outcome_and_a_failure_waits_for_a_key() {
    let fixture = Fixture::command("exit 3").after_run("stay");
    let mut session = fixture.start();
    session.send(b"\r");
    session.screen_with("Run Demo");
    session.send_effect_key(b"\r");
    let failed = session.screen_with("✗ Demo exited with code 3");
    assert_eq!(status_row(&failed), "✗ Demo exited with code 3");

    // A failure has no time limit. The next key clears it, and the entry count comes back.
    session.send(DOWN);
    let cleared = session.screen_with("1/1 entry");
    assert_eq!(status_row(&cleared), "1/1 entry");
}

#[test]
fn a_successful_run_that_stays_shows_a_receipt() {
    let fixture = Fixture::command("echo done").after_run("stay");
    let mut session = fixture.start();
    session.send(b"\r");
    session.screen_with("Run Demo");
    session.send_effect_key(b"\r");
    let finished = session.screen_with("✓ Demo finished");
    assert_eq!(status_row(&finished), "✓ Demo finished");
}

// Ways this can fail: the key stops skit together with the script, so the session that must stay
// is gone and nothing reports how the run ended; skit ignores the key and the script runs on; or
// the session comes back without the outcome of the run.
#[test]
fn an_interrupt_key_ends_the_run_and_a_session_that_stays_reports_it() {
    for (key, code) in [(CTRL_C, 130), (CTRL_BACKSLASH, 131)] {
        let fixture = Fixture::command(LONG_RUN).after_run("stay");
        let mut session = fixture.start();
        session.send(b"\r");
        session.screen_with("Run Demo");
        session.send(b"\r");
        session.screen_with(RUNNING);
        std::thread::sleep(REACTION);
        // The terminal sends the signal of the key to every process of the run.
        session.send_effect_key(key);
        let outcome = format!("✗ Demo exited with code {code}");
        let interrupted = session.screen_with(&outcome);
        assert_eq!(status_row(&interrupted), outcome, "{interrupted}");

        // The session is still open: the next key clears the failure.
        session.send(DOWN);
        let cleared = session.screen_with("1/1 entry");
        assert_eq!(status_row(&cleared), "1/1 entry");
    }
}

// Ways this can fail: Ctrl+C stops skit together with the script, so the shell shows the output of
// the script and no word about how the run ended; or the session returns a code that is not the
// code of the run.
#[test]
fn ctrl_c_ends_the_run_and_a_session_that_ends_prints_the_result_line() {
    let fixture = Fixture::command(LONG_RUN);
    let mut session = fixture.start();
    session.send(b"\r");
    session.screen_with("Run Demo");
    session.send(b"\r");
    session.screen_with(RUNNING);
    std::thread::sleep(REACTION);
    session.send(CTRL_C);
    let (code, history) = session.wait_exit();
    assert_eq!(code, 130, "{history}");
    let output = history.rfind(RUNNING).expect("the output of the script");
    let failure = history
        .find("✗ Demo exited with code 130")
        .expect("the result line");
    assert!(output < failure, "{history}");
    assert_eq!(history.matches("exited with code").count(), 1, "{history}");
}

// Ways this can fail: Ctrl+C stops skit together with the script, so the preset that the run asked
// for is lost (version 0.4 keeps it: `src/skit/cli.py:3222-3228`), stderr says nothing about the
// run, or skit returns a code that is not the code of the run.
#[test]
fn ctrl_c_ends_a_skit_run_that_still_saves_its_preset_and_reports_its_code() {
    let fixture = Fixture::command("echo ready-$((6*7))-{name}; while :; do sleep 1; done");
    let command = fixture.skit(&[
        "run",
        "demo",
        "--set",
        "name=x",
        "--save-preset",
        "prod",
        "--no-input",
    ]);
    let mut pty = PtyChild::spawn(
        command,
        PtySize {
            rows: ROWS,
            cols: COLUMNS,
            pixel_width: 0,
            pixel_height: 0,
        },
        AnswerQueries::On,
    );
    pty.wait_for_after(0, "ready-42-x");
    std::thread::sleep(REACTION);
    pty.send(CTRL_C);
    let (code, output) = pty.finish();
    let history = strip_terminal_control(&output);
    assert_eq!(code, 130, "{history}");
    assert!(history.contains("Run exited with code 130"), "{history}");
    let saved = FormStateService::new(FileFormStateStore::new(fixture.state.path()))
        .load(&skit_domain::Slug::parse("demo").unwrap())
        .presets;
    assert_eq!(
        saved
            .get("prod")
            .and_then(|values| values.get("name"))
            .map(String::as_str),
        Some("x"),
        "{history}"
    );
}

// Ways this can fail: the error names no program, gives no next step, or shows the internal
// lower-case text.
#[test]
fn a_missing_program_names_the_program_and_the_next_step() {
    let fixture = Fixture::command("echo done");
    let mut command = fixture.command_builder();
    let empty = TempDir::new().unwrap();
    command.env("PATH", empty.path());
    let mut session = Session::start(command);
    session.send(b"\r");
    session.screen_with("Run Demo");
    session.send_effect_key(b"\r");
    let refused = session.screen_with("✗ sh is not on PATH. Install it, then run again.");
    assert!(
        !refused.contains("required program was not found"),
        "{refused}"
    );
}

// Ways this can fail: the dialog hides its keys, Enter does nothing or removes while the focus
// ring is on Keep, or Tab does not move the ring.
#[test]
fn the_remove_dialog_shows_its_keys_and_enter_acts_on_the_focused_chip() {
    let fixture = Fixture::command("echo done");
    let mut session = fixture.start();
    session.send(DELETE);
    let dialog = session.screen_with("Esc Keep");
    assert!(dialog.contains("y Remove"), "{dialog}");

    // The ring opens on Keep, so Enter keeps the entry.
    session.send(b"\r");
    let kept = session.screen_without("1/1 entry", "Confirm removal");
    assert!(kept.contains("Demo"), "{kept}");

    session.send(DELETE);
    session.screen_with("Esc Keep");
    session.send(TAB);
    session.send(b"\r");
    let removed = session.screen_with("✓ Entry removed");
    assert!(!removed.contains("Demo"), "{removed}");
}

/// A library whose lock file path holds a directory.
///
/// Each write that takes the library lock fails, also for root. Each read continues without the
/// lock, so the session starts and every screen opens.
fn locked_library() -> Fixture {
    let fixture = Fixture::command("echo done");
    let lock = fixture.data.path().join("registry.native.lock");
    // The registry rebuild of the fixture made the lock file.
    let _ = fs::remove_file(&lock);
    fs::create_dir(&lock).unwrap();
    fixture
}

/// The first words of the error of a write to a [`locked_library`].
///
/// The error then names the lock path, so its length comes from the temporary directory of the
/// host. A long error wraps onto a second row, and the footer cuts what does not fit there. These
/// words always start the first row.
const LOCK_ERROR: &str = "could not open";

// Ways this can fail: Health shows no word about the failed rebuild, so Ctrl+R seems to do
// nothing; a short terminal has no room for the error; or the key that acknowledges the error
// clears it before the screen showed it.
#[test]
fn a_failed_health_rebuild_shows_its_error_until_the_next_key() {
    // A short terminal shows the error on the last row, and a taller one shows it in a box.
    for rows in [12, ROWS] {
        let fixture = locked_library();
        let mut session = Session::start_sized(fixture.command_builder(), rows, COLUMNS);
        session.send(b"h");
        session.screen_with("Health check");
        session.send(CTRL_R);
        let failed = session.screen_with(LOCK_ERROR);
        let error = failed
            .lines()
            .find(|line| line.contains(LOCK_ERROR))
            .unwrap_or_default()
            .trim_matches(|character| character == '│' || character == ' ');
        assert!(error.starts_with("✗ could not open"), "{failed}");
        assert!(failed.contains("Health check"), "{failed}");

        // Esc acknowledges the error and closes Health.
        session.send(ESCAPE);
        session.screen_without("1/1 entry", LOCK_ERROR);
    }
}

// Ways this can fail: the dialog stays open with no word about the failed removal, so the key
// seems to do nothing; or the key that acknowledges the error clears it before the screen showed
// it.
#[test]
fn a_failed_removal_shows_its_error_under_the_dialog() {
    let fixture = locked_library();
    let mut session = fixture.start();
    session.send(DELETE);
    session.screen_with("Esc Keep");
    session.send(TAB);
    session.send(b"\r");
    let failed = session.screen_with(LOCK_ERROR);
    assert!(
        footer_row_with(&failed, LOCK_ERROR).starts_with("✗ could not open"),
        "{failed}"
    );
    assert!(
        failed.contains("y Remove"),
        "the dialog stays open: {failed}"
    );

    // Esc acknowledges the error and keeps the entry.
    session.send(ESCAPE);
    let library = session.screen_without("1/1 entry", LOCK_ERROR);
    assert!(library.contains("Demo"), "{library}");
}

// Ways this can fail: the guard hides its keys, Enter throws the work away while the ring is on
// Keep editing, or Tab does not move the ring.
#[test]
fn the_discard_guard_shows_its_keys_and_enter_acts_on_the_focused_chip() {
    let fixture = Fixture::command("echo done");
    let mut session = fixture.start();
    session.send(b"p");
    session.screen_with("Entry settings ·");
    session.send(b"x");
    session.send(ESCAPE);
    let guard = session.screen_with("Esc Keep editing");
    assert!(guard.contains("y Discard"), "{guard}");

    // The ring opens on Keep editing, so Enter goes back to the edits.
    session.send(b"\r");
    session.screen_without("Entry settings ·", "Discard unsaved changes?");

    session.send(ESCAPE);
    session.screen_with("Esc Keep editing");
    session.send(TAB);
    session.send(b"\r");
    session.screen_without("1/1 entry", "Entry settings ·");
}

// Ways this can fail: the pointer over a chip changes nothing, a press shows no pressed look, or
// the pressed look stays after the press moves away and ends.
#[test]
fn a_footer_chip_shows_the_pointer_over_it_and_a_press_until_the_release() {
    let fixture = Fixture::command("echo done");
    let mut session = fixture.start();
    session.screen_with("? Help");
    let (row, column) = position(&session.grid(), "? Help");
    let label = column + 2;
    let before = session.grid();
    let idle = before.screen().cell(row, label).unwrap().clone();
    assert!(!idle.underline(), "an idle chip label is not underlined");

    // SGR 1006: motion with no button is 35; columns and rows count from 1.
    session.send(format!("\x1b[<35;{};{}M", label + 1, row + 1).as_bytes());
    session.pty.settle();
    let hovered = session.grid();
    let cell = hovered.screen().cell(row, label).unwrap();
    assert!(cell.underline() && cell.bold(), "hover: {cell:?}");

    session.send(format!("\x1b[<0;{};{}M", label + 1, row + 1).as_bytes());
    session.pty.settle();
    let pressed = session.grid();
    let cell = pressed.screen().cell(row, label).unwrap();
    assert_ne!(cell.inverse(), idle.inverse(), "press: {cell:?}");

    // A release on another cell activates nothing and ends the pressed look.
    session.send(b"\x1b[<0;1;1m");
    session.pty.settle();
    let released = session.grid();
    let cell = released.screen().cell(row, label).unwrap();
    assert_eq!(cell.inverse(), idle.inverse(), "release: {cell:?}");
    assert!(
        !session.screen().contains("Close"),
        "the release opened Help"
    );
}

// Ways this can fail: a drag inside a pressed chip ends the press, but no frame shows the end, so
// the chip looks pressed after the button is up; or the release after the drag opens Help.
#[test]
fn a_drag_inside_a_pressed_chip_ends_the_pressed_look_at_once() {
    let fixture = Fixture::command("echo done");
    let mut session = fixture.start();
    session.screen_with("? Help");
    let (row, column) = position(&session.grid(), "? Help");
    // The H of Help, and the next cell, which is in the same chip.
    let (label, next) = (column + 2, column + 3);
    let idle = session.grid().screen().cell(row, label).unwrap().clone();

    // SGR 1006: columns and rows count from 1. Motion with no button is 35, a press of the
    // primary button is 0, and motion with the primary button down is 32.
    session.send(format!("\x1b[<35;{};{}M", label + 1, row + 1).as_bytes());
    session.cell_when((row, label), "the hover look", |cell| {
        cell.underline() && cell.bold()
    });
    session.send(format!("\x1b[<0;{};{}M", label + 1, row + 1).as_bytes());
    session.cell_when((row, label), "the pressed look", |cell| {
        cell.inverse() != idle.inverse()
    });

    // The drag ends the press. The pointer is still on the chip, so the chip shows the hover look.
    session.send(format!("\x1b[<32;{};{}M", next + 1, row + 1).as_bytes());
    let dragged = session.cell_when((row, label), "the end of the pressed look", |cell| {
        cell.inverse() == idle.inverse()
    });
    assert!(dragged.underline() && dragged.bold(), "drag: {dragged:?}");

    session.send(format!("\x1b[<0;{};{}m", next + 1, row + 1).as_bytes());
    let released = session.grid();
    let cell = released.screen().cell(row, label).unwrap();
    assert_eq!(cell.inverse(), idle.inverse(), "release: {cell:?}");
    assert!(
        !session.screen().contains("Close"),
        "the release after the drag opened Help"
    );
}

// Ways this can fail: a search that starts with "." shows hidden files whatever the setting is,
// but the chip says that Ctrl+H shows or hides them now, so a press changes nothing that the
// person can see; or the key or a click on the chip does not change what the chip then says.
#[test]
fn the_hidden_files_chip_says_what_it_changes_during_a_search_that_starts_with_a_dot() {
    let fixture = Fixture::command("echo done");
    fs::write(fixture.home.path().join(".env"), "KEY=value\n").unwrap();
    let mut session = fixture.start();
    session.send(b"a");
    session.screen_with("Previous field");
    session.send(b"\x0f");
    session.screen_with("[Ctrl+H] Show hidden files");

    // The search shows the hidden file. The setting applies after the search.
    session.send(b".env");
    let screen = session.screen_with("[Ctrl+H] Show hidden files after this search");
    assert!(screen.contains("· .env"), "{screen}");
    session.send(b"\x08");
    let screen = session.screen_with("[Ctrl+H] Hide hidden files after this search");
    assert!(screen.contains("· .env"), "{screen}");

    // Esc clears the search, and the setting now shows the hidden file.
    session.send(ESCAPE);
    let screen = session.screen_without("[Ctrl+H] Hide hidden files", "after this search");
    assert!(screen.contains("· .env"), "{screen}");

    // A click on the chip hides it.
    let (row, column) = position(&session.grid(), "[Ctrl+H]");
    session.send(
        format!(
            "\x1b[<0;{column};{row}M\x1b[<0;{column};{row}m",
            column = column + 1,
            row = row + 1
        )
        .as_bytes(),
    );
    session.screen_without("[Ctrl+H] Show hidden files", "· .env");
}

/// The footer rows of an Add screen: every row under the bottom border of its panel.
fn add_footer(screen: &str) -> Vec<&str> {
    let lines = screen.lines().collect::<Vec<_>>();
    let border = lines
        .iter()
        .rposition(|line| line.starts_with('╰'))
        .unwrap_or_else(|| panic!("the Add panel has no bottom border:\n{screen}"));
    lines[border + 1..].to_vec()
}

// Ways this can fail: the Add footer shows the navigation keys without words, so a person must
// guess what Tab does there; or the words push a chip below the last row of a 60-column terminal,
// so a chip that the screen advertises is out of view.
#[test]
fn the_add_footer_names_its_navigation_keys_and_keeps_every_chip_in_view_at_60_columns() {
    let fixture = Fixture::command("echo done");
    fs::write(
        fixture.home.path().join("hello.sh"),
        "NAME=world\necho \"Hello, $NAME\"\n",
    )
    .unwrap();
    let mut session = Session::start_sized(fixture.command_builder(), ROWS, 60);
    session.send(b"a");
    let source = session.screen_with("Previous field");
    let footer = add_footer(&source).join("\n");
    for chip in [
        "[Enter] Continue",
        "[Esc] Cancel",
        "[Ctrl+N] Write a script…",
        "[Ctrl+P] Draft a prompt…",
        "[Tab/↓] Next field",
        "[Shift+Tab/↑] Previous field",
    ] {
        assert!(footer.contains(chip), "{chip} is out of view:\n{source}");
    }

    session.send(b"hello.sh");
    session.screen_with("hello.sh");
    session.send(b"\r");
    session.screen_with("Tick");
    // The focus moves to the checkbox of NAME, where the footer has the most chips.
    session.send(TAB);
    session.send(TAB);
    session.send(TAB);
    let review = session.screen_with("[Space] Toggle");
    let footer = add_footer(&review).join("\n");
    for chip in [
        "[Ctrl+S] Add",
        "[Esc] Cancel",
        "[Space] Toggle",
        "[Ctrl+E] Edit script",
        "[Tab/↓] Next field",
        "[Shift+Tab/↑] Previous field",
    ] {
        assert!(footer.contains(chip), "{chip} is out of view:\n{review}");
    }
}

/// Permission bits for one path, given back as `0o755` when the guard ends.
struct ModeGuard(PathBuf);

impl ModeGuard {
    fn set(path: &Path, mode: u32) -> Self {
        fs::set_permissions(path, fs::Permissions::from_mode(mode)).unwrap();
        Self(path.to_owned())
    }
}

impl Drop for ModeGuard {
    fn drop(&mut self) {
        let _ = fs::set_permissions(&self.0, fs::Permissions::from_mode(0o755));
    }
}

/// Report whether this user can make a file in `directory`. Root can, whatever the mode is.
fn can_write_into(directory: &Path) -> bool {
    let probe = directory.join(".probe");
    let written = fs::write(&probe, b"").is_ok();
    let _ = fs::remove_file(&probe);
    written
}

// Ways this can fail: the warning that skit kept a draft it could not remove shows with a ✓ and
// clears itself, so a person who looked away never learns about the draft; its words run
// together; or the next key does not clear it.
#[test]
fn an_add_that_cannot_remove_its_draft_reports_a_warning_until_the_next_key() {
    let fixture = Fixture::command("echo done");
    let drafts = fixture.data.path().join("drafts");
    fs::create_dir(&drafts).unwrap();
    let draft = drafts.join("skit-new-ship.sh");
    fs::write(&draft, "#!/bin/sh\necho drafted\n").unwrap();
    // The commit writes the entry, and then the cleanup must move the draft out of a directory
    // that it cannot write.
    let _mode = ModeGuard::set(&drafts, 0o555);
    if can_write_into(&drafts) {
        // Root writes into every directory, so this case needs an ordinary user.
        return;
    }
    let mut session = fixture.start();
    session.send(b"a");
    session.screen_with("Add an entry");
    let path = draft.display().to_string();
    session.send(path.as_bytes());
    session.screen_with(&path);
    session.send(b"\r");
    session.screen_with("Add skit-new-ship");
    session.send(CTRL_S);
    // The warning names the drafts path, so it wraps onto a second row.
    let first_row = |screen: &str| {
        screen
            .lines()
            .find(|line| line.contains("Entry added"))
            .unwrap_or_default()
            .trim_matches(|character| character == '│' || character == ' ')
            .to_owned()
    };
    let added = session.screen_with("Entry added");
    let warning = first_row(&added);
    assert!(
        warning.starts_with("⚠ Entry added — warning: could not create ")
            && warning.contains("drafts"),
        "{added}"
    );

    // A warning has no time limit.
    std::thread::sleep(Duration::from_millis(4_500));
    assert_eq!(first_row(&session.screen()), warning);
    session.send(DOWN);
    let cleared = session.screen_with("2/2 entries");
    assert_eq!(status_row(&cleared), "2/2 entries");
    assert!(
        draft.exists(),
        "skit keeps the draft that it could not remove"
    );
}

// Ways this can fail: the launch menu takes an empty extra-arguments box as no answer, so the run
// gets the remembered arguments again (version 0.4 runs with the box as it is:
// `src/skit/tui_form.py:808-815`); skit says that it reuses them; the run also loses the value of
// the field; or the run goes without the arguments, but skit still remembers them, so the next run
// gets them again.
#[test]
fn an_emptied_extra_arguments_box_runs_without_the_remembered_arguments_and_forgets_them() {
    // The command line shows `$((6*7))`, so only the output of the run holds `said-42`.
    let fixture = Fixture::command("echo said-$((6*7))-{name}");
    let first = fixture.output(&[
        "run",
        "demo",
        "--set",
        "name=world",
        "--no-input",
        "--",
        "eee",
    ]);
    assert!(
        String::from_utf8_lossy(&first.stdout).contains("said-42-world eee"),
        "{first:?}"
    );

    let mut session = fixture.start();
    session.send(b"\r");
    session.screen_with("Run Demo");
    // The name box has the focus. The next box shows the remembered arguments.
    session.screen_with("eee");
    session.send(TAB);
    session.send(BACKSPACE);
    session.send(BACKSPACE);
    session.send(BACKSPACE);
    session.screen_without("Run Demo", "eee");
    session.send(b"\r");
    let (code, history) = session.wait_exit();
    assert_eq!(code, 0, "{history}");
    // The screens of the launch menu showed the arguments, so read only the run.
    let run = &history[history.find("── Run Demo ──").expect("the run header")..];
    assert!(run.contains("said-42-world"), "{run}");
    assert!(!run.contains("eee"), "{run}");
    assert!(!run.contains("Reusing your last arguments"), "{run}");

    let next = fixture.output(&["run", "demo", "--no-input"]);
    let (stdout, stderr) = (
        String::from_utf8_lossy(&next.stdout),
        String::from_utf8_lossy(&next.stderr),
    );
    assert!(stdout.contains("said-42-world"), "{next:?}");
    assert!(!stdout.contains("eee"), "{next:?}");
    assert!(!stderr.contains("Reusing your last arguments"), "{next:?}");
}

// Ways this can fail: the preset picker shows a blank row for the last values, or a choice of a
// preset changes the fields without a word.
#[test]
fn the_preset_picker_names_the_last_values() {
    let fixture = Fixture::command("echo {name}");
    let mut parameter = ParamDecl::new("name");
    parameter.delivery = ParameterDelivery::Placeholder;
    FormStateService::new(FileFormStateStore::new(fixture.state.path()))
        .save_preset(
            &skit_domain::Slug::parse("demo").unwrap(),
            "friendly",
            &[parameter],
            &std::collections::BTreeMap::from([("name".to_owned(), "Ada".to_owned())]),
        )
        .unwrap();
    let mut session = fixture.start();
    session.send(b"\r");
    session.screen_with("Run Demo");
    let form = session.screen_with("↩ last values");
    assert!(form.contains("Preset"), "{form}");
}

// Ways this can fail: the notice stays until the next key, so a person who changed their mind
// sees a stale warning; or the wait for it quits the session.
#[test]
fn the_quit_notice_clears_itself_without_a_key() {
    let fixture = Fixture::command("echo done");
    let mut session = fixture.start();
    session.send(b"\x03");
    session.screen_with("Press Ctrl+C again to quit");
    let gone = session
        .pty
        .wait_for_after_rendered(0, "NOTICE_GONE", |bytes| {
            if final_terminal_screen(bytes, ROWS, COLUMNS).contains("again to quit") {
                String::new()
            } else {
                "NOTICE_GONE".to_owned()
            }
        });
    assert_eq!(gone, "NOTICE_GONE");
    assert!(session.pty.try_exit_code().is_none(), "the wait quit skit");
}

/// The bytes that leave the alternate screen. The terminal then shows the shell again.
const LEAVE_ALTERNATE_SCREEN: &[u8] = b"\x1b[?1049l";
/// The question for the cursor position. The answer comes back over the input, so each question
/// waits for one round trip of the terminal link.
const CURSOR_QUESTION: &[u8] = b"\x1b[6n";

/// Report whether `bytes` holds `needle`.
fn holds(bytes: &[u8], needle: &[u8]) -> bool {
    bytes.windows(needle.len()).any(|window| window == needle)
}

/// A stand-in for an editor: it saves the file that it gets.
///
/// It writes the bytes back as it found them, as an editor does when a person saves with no
/// change. When `change` exists, it adds a line instead.
fn saving_editor(directory: &Path, change: &Path) -> PathBuf {
    let editor = directory.join("editor.sh");
    fs::write(
        &editor,
        format!(
            concat!(
                "#!/bin/sh\n",
                "if [ -e '{change}' ]; then\n",
                "  printf 'echo changed\\n' >> \"$1\"\n",
                "else\n",
                "  saved=$(cat \"$1\"; printf x)\n",
                "  printf '%s' \"${{saved%x}}\" > \"$1\"\n",
                "fi\n",
            ),
            change = change.display(),
        ),
    )
    .unwrap();
    fs::set_permissions(&editor, fs::Permissions::from_mode(0o755)).unwrap();
    editor
}

// Ways this can fail: an editor that saves the file with no change still gets "✓ Source saved";
// the interface prints the report of `skit edit` ("Saved hello." and the hint that names
// `skit params`) into the shell, where the person sees it after skit ends; a real change gets no
// receipt; or the editor key does not give the terminal to the editor.
#[test]
fn the_editor_key_reports_a_saved_source_only_when_the_editor_changed_it() {
    const BODY: &str = "echo hello\n";
    let fixture = Fixture::script(BODY);
    let change = fixture.home.path().join("change");
    let editor = saving_editor(fixture.home.path(), &change);
    let stored = fixture.data.path().join("scripts/hello/script.sh");
    let mut command = fixture.command_builder();
    command.env("VISUAL", &editor);
    let mut session = Session::start(command);

    // The editor saves the file as it was.
    let before = session.pty.checkpoint();
    session.send_effect_key(b"e");
    let unchanged = session.screen_with("No changes to hello.");
    assert_eq!(
        status_row(&unchanged),
        "No changes to hello.",
        "{unchanged}"
    );
    assert!(
        holds(&session.pty.raw_after(before), LEAVE_ALTERNATE_SCREEN),
        "the editor did not get the terminal"
    );
    assert_eq!(fs::read_to_string(&stored).unwrap(), BODY);

    // The editor adds a line.
    fs::write(&change, "").unwrap();
    session.send_effect_key(b"e");
    let saved = session.screen_with("✓ Source saved");
    assert_eq!(status_row(&saved), "✓ Source saved", "{saved}");
    assert_eq!(
        fs::read_to_string(&stored).unwrap(),
        format!("{BODY}echo changed\n")
    );

    session.send(b"q");
    let (code, history) = session.wait_exit();
    assert_eq!(code, 0, "{history}");
    assert!(!history.contains("Saved hello."), "{history}");
    assert!(!history.contains("parameter drift"), "{history}");
}

// Ways this can fail: skit cannot read the original of a reference entry before or after the
// editor, and the interface reports "No changes" for an edit that changed the file, or a save that
// skit could not see; or the report clears itself before the person reads it.
#[test]
fn the_editor_key_says_so_when_it_cannot_read_the_source_to_compare() {
    let fixture = Fixture::directories();
    // A short directory keeps the status on one row on every host.
    let originals = tempfile::Builder::new().tempdir_in("/tmp").unwrap();
    let original = originals.path().join("locked.sh");
    fs::write(&original, "echo locked\n").unwrap();
    fixture.add(&original, &["--ref"]);
    // The owner can write the original but cannot read it.
    let _mode = ModeGuard::set(&original, 0o200);
    if fs::read(&original).is_ok() {
        // Root reads every file, so this case needs an ordinary user.
        return;
    }
    let change = fixture.home.path().join("change");
    fs::write(&change, "").unwrap();
    let editor = saving_editor(fixture.home.path(), &change);
    let mut command = fixture.command_builder();
    command.env("VISUAL", &editor);
    let mut session = Session::start(command);

    session.send_effect_key(b"e");
    let unknown = session.screen_with("so it cannot tell whether locked changed.");
    let status = unknown
        .lines()
        .find(|line| line.contains("cannot tell"))
        .unwrap_or_default()
        .trim_matches(|character| character == '│' || character == ' ')
        .to_owned();
    assert!(
        status.starts_with("⚠ skit could not read ") && status.contains("/locked.sh, so it"),
        "{unknown}"
    );
    assert!(!unknown.contains("No changes"), "{unknown}");
    assert!(!unknown.contains("Source saved"), "{unknown}");

    // A warning has no time limit.
    std::thread::sleep(Duration::from_millis(4_500));
    assert!(
        session
            .screen()
            .contains("cannot tell whether locked changed.")
    );
}

// Ways this can fail: work that needs no terminal of its own leaves the alternate screen, so the
// shell shows through, and the screen stays blank until skit asks where the cursor is and draws
// again. Over SSH, that question waits for one round trip of the link. Opening the launch menu,
// counting the matches of a pattern after each typed key, saving a preset, reloading, adding an
// entry, and removing one are all such work. Or a removal prints its receipt into the shell,
// where the person sees it only after skit ends.
#[test]
fn quick_host_work_keeps_the_interface_on_the_screen() {
    let fixture = Fixture::command("echo {name}").after_run("stay");
    fs::write(fixture.home.path().join("hello.sh"), "echo hello\n").unwrap();
    let mut session = fixture.start();
    let start = session.pty.checkpoint();

    session.send(b"\r");
    session.screen_with("Run Demo");
    session.send(b"*.sh");
    session.screen_with("✓ matches 1 file(s)");
    session.send(CTRL_S);
    session.screen_with("Preset name");
    session.send(b"shells\r");
    session.screen_with("✓ Preset \"shells\" saved.");
    session.send(ESCAPE);
    session.screen_without("1/1 entry", "Run Demo");
    session.send(CTRL_R);
    session.screen_with("✓ Library reloaded (1 entry)");
    session.send(b"a");
    session.screen_with("Add an entry");
    session.send(b"hello.sh");
    session.screen_with("hello.sh");
    session.send(b"\r");
    session.screen_with("Add hello");
    session.send(CTRL_S);
    session.screen_with("✓ Entry added");
    session.send(DELETE);
    session.screen_with("Esc Keep");
    session.send(TAB);
    session.send(b"\r");
    session.screen_with("✓ Entry removed");

    let shown = session.pty.raw_after(start);
    assert!(
        !holds(&shown, LEAVE_ALTERNATE_SCREEN),
        "skit left the alternate screen:\n{}",
        String::from_utf8_lossy(&shown)
    );
    assert!(
        !holds(&shown, CURSOR_QUESTION),
        "skit waited for the cursor position:\n{}",
        String::from_utf8_lossy(&shown)
    );
    let history = strip_terminal_control(&String::from_utf8_lossy(&shown));
    assert!(!history.contains("Removed: "), "{history}");
}
