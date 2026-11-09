//! End-to-end tests of text editing in the terminal interface.
//!
//! Each test starts the real `skit` binary on a pseudo-terminal, types the keys a person types,
//! and reads what the terminal shows. A launch-menu test ends in the most exact record of a value
//! there is: the run itself prints the value it received. A settings test reads the saved entry.
//!
//! A paste goes through [`Session::paste`], which acts like a real terminal. A terminal wraps a
//! paste in the bracketed-paste markers only while the program has the mode on. Without the mode,
//! the terminal types the text, and a line break arrives as the Enter key.
//!
//! Unix only: the session start waits for the cursor-position question that crossterm asks only
//! on unix (see `LiveTui::spawn` in `terminal_pty.rs`).
#![cfg(unix)]

use std::{
    fs,
    path::PathBuf,
    time::{Duration, Instant},
};

use portable_pty::{CommandBuilder, PtySize};
use skit_application::EntryRepository as _;
use skit_store::FileStore;
use tempfile::TempDir;

#[path = "support/pty.rs"]
mod pty;

use pty::{AnswerQueries, PtyChild, final_terminal_screen, strip_terminal_control};

const ROWS: u16 = 30;
const COLUMNS: u16 = 100;

const ENTER: &[u8] = b"\r";
const TAB: &[u8] = b"\t";
const BACK_TAB: &[u8] = b"\x1b[Z";
const BACKSPACE: &[u8] = b"\x7f";
const DELETE: &[u8] = b"\x1b[3~";
const LEFT: &[u8] = b"\x1b[D";
const RIGHT: &[u8] = b"\x1b[C";
const UP: &[u8] = b"\x1b[A";
const HOME: &[u8] = b"\x1b[H";
const END: &[u8] = b"\x1b[F";
const SHIFT_LEFT: &[u8] = b"\x1b[1;2D";
const SHIFT_RIGHT: &[u8] = b"\x1b[1;2C";
const SHIFT_HOME: &[u8] = b"\x1b[1;2H";
const SHIFT_END: &[u8] = b"\x1b[1;2F";
const CTRL_LEFT: &[u8] = b"\x1b[1;5D";
const CTRL_RIGHT: &[u8] = b"\x1b[1;5C";
const ALT_LEFT: &[u8] = b"\x1b[1;3D";
const ALT_RIGHT: &[u8] = b"\x1b[1;3C";
const CTRL_SHIFT_LEFT: &[u8] = b"\x1b[1;6D";
const CTRL_SHIFT_RIGHT: &[u8] = b"\x1b[1;6C";
const ALT_SHIFT_LEFT: &[u8] = b"\x1b[1;4D";
const ALT_SHIFT_RIGHT: &[u8] = b"\x1b[1;4C";
const ALT_B: &[u8] = b"\x1bb";
const ALT_F: &[u8] = b"\x1bf";
const ALT_D: &[u8] = b"\x1bd";
const ALT_BACKSPACE: &[u8] = b"\x1b\x7f";
const CTRL_DELETE: &[u8] = b"\x1b[3;5~";
const ALT_DELETE: &[u8] = b"\x1b[3;3~";
const CTRL_A: &[u8] = b"\x01";
const CTRL_C: &[u8] = b"\x03";
const CTRL_D: &[u8] = b"\x04";
const CTRL_E: &[u8] = b"\x05";
const CTRL_H: &[u8] = b"\x08";
const CTRL_K: &[u8] = b"\x0b";
const CTRL_O: &[u8] = b"\x0f";
const CTRL_S: &[u8] = b"\x13";
const CTRL_U: &[u8] = b"\x15";
const CTRL_V: &[u8] = b"\x16";
const CTRL_W: &[u8] = b"\x17";
const CTRL_X: &[u8] = b"\x18";
const CTRL_Y: &[u8] = b"\x19";
const CTRL_Z: &[u8] = b"\x1a";

/// The terminal row of the first launch-menu box: title, preset line, field label, top border.
const NAME_ROW: u16 = 4;
/// The terminal column of the first character in a launch-menu box.
const BOX_COLUMN: u16 = 2;
/// The terminal row of the Name box in Entry settings.
const SETTINGS_NAME_ROW: u16 = 3;
/// The terminal column of the first character in an Entry settings box.
const SETTINGS_BOX_COLUMN: u16 = 3;

/// The clipboard programs that a local session can give a copy to.
const CLIPBOARD_PROGRAMS: [&str; 4] = ["pbcopy", "wl-copy", "xclip", "xsel"];

/// One library with one command entry, and the directories of one session.
///
/// The session finds a fake of each clipboard program first on `PATH`. A fake saves the text it
/// reads, so no test touches the real clipboard, and a test can read what the session copied.
struct Fixture {
    data: TempDir,
    state: TempDir,
    config: TempDir,
    home: TempDir,
}

impl Fixture {
    /// A library whose one command entry `Demo` prints the values of its two fields.
    fn new() -> Self {
        let fixture = Self {
            data: TempDir::new().unwrap(),
            state: TempDir::new().unwrap(),
            config: TempDir::new().unwrap(),
            home: TempDir::new().unwrap(),
        };
        let directory = fixture.data.path().join("scripts/demo");
        fs::create_dir_all(&directory).unwrap();
        fs::write(
            directory.join("meta.toml"),
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
                "template = \"printf '[%s|%s]\\\\n' {name} {other}\"\n",
                "params = [\"name\", \"other\"]\n",
                "[[parameters]]\n",
                "name = \"name\"\n",
                "delivery = \"placeholder\"\n",
                "[[parameters]]\n",
                "name = \"other\"\n",
                "delivery = \"placeholder\"\n",
            ),
        )
        .unwrap();
        FileStore::new(fixture.data.path())
            .rebuild_registry()
            .unwrap();
        let bin = fixture.bin();
        fs::create_dir_all(&bin).unwrap();
        for program in CLIPBOARD_PROGRAMS {
            let path = bin.join(program);
            fs::write(
                &path,
                "#!/bin/sh\ncat > \"$0.part\" && mv \"$0.part\" \"$0.copied\"\n",
            )
            .unwrap();
            fs::set_permissions(&path, std::os::unix::fs::PermissionsExt::from_mode(0o755))
                .unwrap();
        }
        fixture
    }

    /// The directory of the fake clipboard programs.
    fn bin(&self) -> PathBuf {
        self.home.path().join("bin")
    }

    /// Wait until the fake `program` saved a copy, and return it.
    fn wait_copied(&self, program: &str) -> String {
        let path = self.bin().join(format!("{program}.copied"));
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if let Ok(text) = fs::read_to_string(&path) {
                return text;
            }
            assert!(Instant::now() < deadline, "{program} received no copy");
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    fn command_builder(&self, arguments: &[&str]) -> CommandBuilder {
        let mut command = CommandBuilder::new(PathBuf::from(env!("CARGO_BIN_EXE_skit")));
        command.args(arguments);
        command.cwd(self.home.path());
        command.env("TERM", "xterm-256color");
        command.env("NO_COLOR", "1");
        command.env("SKIT_LANG", "en");
        command.env("SKIT_DATA_DIR", self.data.path());
        command.env("SKIT_STATE_DIR", self.state.path());
        command.env("SKIT_CONFIG_DIR", self.config.path());
        command.env("HOME", self.home.path());
        // A local session, with the fake clipboard programs first. Wayland chooses `wl-copy`
        // off macOS.
        let path = std::env::var_os("PATH").unwrap_or_default();
        let mut paths = vec![self.bin()];
        paths.extend(std::env::split_paths(&path));
        command.env("PATH", std::env::join_paths(paths).unwrap());
        command.env("WAYLAND_DISPLAY", "skit-test");
        for variable in ["SSH_CONNECTION", "SSH_CLIENT", "SSH_TTY"] {
            command.env_remove(variable);
        }
        command
    }

    fn start(&self) -> Session {
        Session::start(self.command_builder(&["tui"]), "Library")
    }

    /// Open the launch menu of `Demo`, with the focus on its first box.
    fn launch_menu(&self) -> Session {
        let mut session = self.start();
        session.send(ENTER);
        session.screen_with("Run Demo");
        session
    }

    /// Open Entry settings of `Demo`, with the focus on its Name box.
    fn entry_settings(&self) -> Session {
        let mut session = self.start();
        session.send(b"p");
        session.screen_with("Entry settings · Demo");
        session
    }

    /// The saved entry `Demo`.
    fn saved(&self) -> skit_domain::EntryMeta {
        FileStore::new(self.data.path())
            .resolve("demo")
            .unwrap()
            .meta
    }
}

/// One live interface session.
struct Session {
    pty: PtyChild,
}

impl Session {
    fn start(command: CommandBuilder, ready: &str) -> Self {
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
        pty.wait_cursor_query_after(0);
        let mut session = Self { pty };
        session.screen_with(ready);
        session
    }

    /// Let the child settle, then write keys.
    fn send(&mut self, bytes: &[u8]) {
        self.pty.settle_quiet(Duration::from_millis(30));
        self.pty.write_raw(bytes);
    }

    /// Paste `text` as a terminal pastes it.
    ///
    /// A terminal wraps a paste in the bracketed-paste markers only while the program has the mode
    /// on. A program without the mode receives the paste as typed keys, and each line break then
    /// arrives as the Enter key.
    fn paste(&mut self, text: &str) {
        self.pty.settle_quiet(Duration::from_millis(30));
        let raw = self.pty.raw_after(0);
        let enabled = rfind(&raw, b"\x1b[?2004h");
        let disabled = rfind(&raw, b"\x1b[?2004l");
        let bracketed = match (enabled, disabled) {
            (Some(on), Some(off)) => on > off,
            (Some(_), None) => true,
            (None, _) => false,
        };
        if bracketed {
            let mut bytes = b"\x1b[200~".to_vec();
            bytes.extend_from_slice(text.as_bytes());
            bytes.extend_from_slice(b"\x1b[201~");
            self.pty.write_raw(&bytes);
        } else {
            let typed = text.replace("\r\n", "\r").replace('\n', "\r");
            self.pty.write_raw(typed.as_bytes());
        }
    }

    /// Wait until the screen shows `needle`, and return the screen.
    fn screen_with(&mut self, needle: &str) -> String {
        self.pty.wait_for_after_rendered(0, needle, |bytes| {
            final_terminal_screen(bytes, ROWS, COLUMNS)
        })
    }

    /// The screen as the terminal shows it now.
    fn screen(&mut self) -> String {
        self.pty.settle();
        final_terminal_screen(&self.pty.raw_after(0), ROWS, COLUMNS)
    }

    /// The terminal grid now, for an assertion about the cursor or about one cell.
    fn grid(&mut self) -> vt100::Parser {
        self.pty.settle();
        let rows = std::num::NonZeroU16::new(ROWS).expect("the grid has rows");
        let columns = std::num::NonZeroU16::new(COLUMNS).expect("the grid has columns");
        let mut parser = vt100::Parser::new(rows, columns, 0);
        parser.process(&self.pty.raw_after(0));
        parser
    }

    /// Wait until one box row shows exactly `value` from `column` on, and return the screen.
    ///
    /// The rest of the row is the box border and blank cells, so the value must end at the first
    /// box border or at a run of three spaces.
    fn wait_box(&mut self, row: u16, column: u16, value: &str) -> String {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let shown = box_text(&self.grid(), row, column);
            if shown == value {
                return self.screen();
            }
            assert!(
                Instant::now() < deadline,
                "the box at row {row} shows {shown:?}, not {value:?}:\n{}",
                self.screen()
            );
        }
    }

    /// The row and the column of the first cell that shows `text`, after the screen shows it.
    ///
    /// The row before `text` must hold only characters one cell wide.
    fn locate(&mut self, text: &str) -> (u16, u16) {
        let _ = self.screen_with(text);
        let grid = self.grid();
        let screen = grid.screen();
        (0..ROWS)
            .find_map(|row| {
                let line = screen.contents_between(row, 0, row, COLUMNS);
                let index = line.find(text)?;
                Some((row, u16::try_from(line[..index].chars().count()).unwrap()))
            })
            .unwrap_or_else(|| panic!("the screen does not show {text:?}:\n{}", screen.contents()))
    }

    /// Wait until the hardware cursor stands on `(row, column)`.
    fn wait_cursor(&mut self, row: u16, column: u16) {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let grid = self.grid();
            let screen = grid.screen();
            if screen.cursor_position() == (row, column) && !screen.hide_cursor() {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "the cursor is at {:?} (hidden: {}), not at {:?}:\n{}",
                screen.cursor_position(),
                screen.hide_cursor(),
                (row, column),
                screen.contents()
            );
        }
    }

    /// Run the launch menu with Enter and return the line the run printed.
    fn run_output(&mut self) -> String {
        self.send(ENTER);
        let status = self.pty.wait_exit_within(Duration::from_secs(30));
        // The reader can still hold the last output of the run when the child has exited.
        self.pty.settle_quiet(Duration::from_millis(300));
        let history = strip_terminal_control(&String::from_utf8_lossy(&self.pty.raw_after(0)));
        assert_eq!(status.exit_code(), 0, "{history}");
        let start = history
            .rfind("\n[")
            .map(|index| index + 1)
            .or_else(|| history.starts_with('[').then_some(0))
            .unwrap_or_else(|| panic!("the run printed no value line:\n{history}"));
        let line = &history[start..];
        line[..line.find('\n').unwrap_or(line.len())].to_owned()
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.pty.kill();
    }
}

fn rfind(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .rposition(|window| window == needle)
}

/// The text of one box row from `column` to the box border, without trailing blank cells.
fn box_text(parser: &vt100::Parser, row: u16, column: u16) -> String {
    let text = parser.screen().contents_between(row, column, row, COLUMNS);
    let end = text.find('│').unwrap_or(text.len());
    text[..end].trim_end().to_owned()
}

// Ways this can fail: the mode is never on, so the terminal types the paste and its line break
// runs the entry; the paste keeps its later lines; a pasted Tab moves the focus; or the focus
// leaves the box after the paste.
#[test]
fn a_paste_in_a_launch_menu_box_keeps_its_first_line_and_never_runs_or_moves_the_focus() {
    let fixture = Fixture::new();
    let mut session = fixture.launch_menu();
    session.paste("Bob\tX\nCarol");
    session.pty.settle();
    let screen = session.screen();
    assert!(
        session.pty.try_exit_code().is_none() && !screen.contains("── Run Demo ──"),
        "the paste ran the entry:\n{screen}"
    );
    assert!(
        !screen.contains("Carol"),
        "the paste kept its second line:\n{screen}"
    );
    session.send(b"!");
    assert_eq!(session.run_output(), "[Bob\tX!|]");
}

// Ways this can fail: crossterm stops reading part of the way through a long paste and waits for
// the next key; or the paste arrives cut off.
#[test]
fn a_paste_over_two_kilobytes_arrives_whole_without_another_key() {
    let fixture = Fixture::new();
    let mut session = fixture.launch_menu();
    let value = format!("{}END", "a".repeat(3_000));
    session.paste(&value);
    let since = Instant::now();
    session.screen_with("aaaaEND");
    assert!(
        since.elapsed() < Duration::from_secs(5),
        "the long paste took {:?}",
        since.elapsed()
    );
    assert_eq!(session.run_output(), format!("[{value}|]"));
}

// Ways this can fail: a chord moves the caret to the wrong place or not at all; Alt+B and Alt+F
// arrive with the ALT modifier and nothing reads it; the caret stands on the last character of
// the value instead of after it.
#[test]
fn launch_menu_caret_moves_by_character_word_and_line() {
    let fixture = Fixture::new();
    let mut session = fixture.launch_menu();
    session.send(b"one two three");
    let at = |offset: u16| BOX_COLUMN + offset;
    session.wait_cursor(NAME_ROW, at(13));
    for (keys, offset) in [
        (HOME, 0),
        (END, 13),
        (CTRL_A, 0),
        (CTRL_E, 13),
        (ALT_B, 8),
        (ALT_B, 4),
        (ALT_F, 8),
        (ALT_F, 13),
        (CTRL_LEFT, 8),
        (CTRL_RIGHT, 13),
        (ALT_LEFT, 8),
        (ALT_RIGHT, 13),
        (LEFT, 12),
        (RIGHT, 13),
    ] {
        session.send(keys);
        session.wait_cursor(NAME_ROW, at(offset));
    }
    session.send(CTRL_A);
    session.send(b">");
    assert_eq!(session.run_output(), "[>one two three|]");
}

// Ways this can fail: a delete chord removes the wrong span; Ctrl+U clears the whole line
// instead of the text before the caret; Ctrl+D, Alt+D, or Alt+Delete do nothing.
#[test]
fn launch_menu_deletes_by_character_word_and_line() {
    let fixture = Fixture::new();
    let mut session = fixture.launch_menu();
    let step = |session: &mut Session, keys: &[u8], value: &str| {
        session.send(keys);
        session.wait_box(NAME_ROW, BOX_COLUMN, value);
    };
    step(
        &mut session,
        b"alpha beta gamma delta",
        "alpha beta gamma delta",
    );
    step(&mut session, CTRL_W, "alpha beta gamma");
    step(&mut session, ALT_BACKSPACE, "alpha beta");
    step(&mut session, CTRL_A, "alpha beta");
    step(&mut session, ALT_D, "beta");
    step(&mut session, b"one two ", "one two beta");
    step(&mut session, CTRL_DELETE, "one two");
    step(&mut session, CTRL_A, "one two");
    step(&mut session, ALT_DELETE, "two");
    step(&mut session, END, "two");
    step(&mut session, b"x y z", "two x y z");
    step(&mut session, LEFT, "two x y z");
    step(&mut session, LEFT, "two x y z");
    step(&mut session, CTRL_U, " z");
    step(&mut session, b"keep", "keep z");
    step(&mut session, CTRL_K, "keep");
    step(&mut session, b"s", "keeps");
    step(&mut session, LEFT, "keeps");
    step(&mut session, CTRL_D, "keep");
    step(&mut session, CTRL_H, "kee");
    step(&mut session, BACKSPACE, "ke");
    step(&mut session, DELETE, "ke");
    step(&mut session, b"y", "key");
    assert_eq!(session.run_output(), "[key|]");
}

// Ways this can fail: Shift does not select; typing inserts beside the selection instead of
// replacing it; Backspace or Delete removes one character instead of the selection; the word
// selections stop at the wrong place; the selection is not drawn.
#[test]
fn launch_menu_selection_by_keyboard_is_drawn_and_typing_or_deleting_replaces_it() {
    let fixture = Fixture::new();
    let mut session = fixture.launch_menu();
    session.send(b"hello world");
    session.wait_box(NAME_ROW, BOX_COLUMN, "hello world");
    for _ in 0..5 {
        session.send(SHIFT_LEFT);
    }
    session.wait_cursor(NAME_ROW, BOX_COLUMN + 6);
    let grid = session.grid();
    for column in 6..11 {
        let cell = grid
            .screen()
            .cell(NAME_ROW, BOX_COLUMN + column)
            .unwrap()
            .clone();
        assert!(
            cell.inverse(),
            "the selected cell {column} is not drawn as a selection"
        );
    }
    let before = grid
        .screen()
        .cell(NAME_ROW, BOX_COLUMN + 4)
        .unwrap()
        .clone();
    assert!(
        !before.inverse(),
        "a cell outside the selection is drawn as selected"
    );

    let step = |session: &mut Session, keys: &[u8], value: &str| {
        session.send(keys);
        session.wait_box(NAME_ROW, BOX_COLUMN, value);
    };
    step(&mut session, b"there", "hello there");
    step(&mut session, SHIFT_HOME, "hello there");
    step(&mut session, b"X", "X");
    step(&mut session, b"abc def", "Xabc def");
    step(&mut session, CTRL_SHIFT_LEFT, "Xabc def");
    step(&mut session, BACKSPACE, "Xabc");
    step(&mut session, ALT_SHIFT_LEFT, "Xabc");
    step(&mut session, DELETE, "");
    step(&mut session, b"start middle end", "start middle end");
    step(&mut session, HOME, "start middle end");
    step(&mut session, CTRL_SHIFT_RIGHT, "start middle end");
    step(&mut session, ALT_SHIFT_RIGHT, "start middle end");
    step(&mut session, b"<", "<end");
    step(&mut session, SHIFT_RIGHT, "<end");
    step(&mut session, b"E", "<End");
    step(&mut session, SHIFT_END, "<End");
    step(&mut session, b"!", "<E!");
    assert_eq!(session.run_output(), "[<E!|]");
}

// Ways this can fail: Ctrl+C with a selection arms the quit instead of copying, or sends nothing
// to the system clipboard; Ctrl+C without a selection no longer arms the quit; Ctrl+V pastes
// nothing; Ctrl+X keeps the text; undo or redo does nothing or skips a step.
#[test]
fn launch_menu_copy_cut_paste_undo_and_redo() {
    let fixture = Fixture::new();
    let mut session = fixture.launch_menu();
    let step = |session: &mut Session, keys: &[u8], value: &str| {
        session.send(keys);
        session.wait_box(NAME_ROW, BOX_COLUMN, value);
    };
    step(&mut session, b"copy me", "copy me");
    step(&mut session, SHIFT_HOME, "copy me");
    let copied = session.pty.checkpoint();
    session.send(CTRL_C);
    session.pty.settle();
    let after_copy = session.pty.raw_after(copied);
    assert!(
        rfind(&after_copy, b"\x1b]52;c;Y29weSBtZQ==").is_some(),
        "the copy sent nothing to the system clipboard: {:?}",
        String::from_utf8_lossy(&after_copy)
    );
    let screen = session.screen();
    assert!(
        !screen.contains("Press Ctrl+C again to quit"),
        "a copy armed the quit:\n{screen}"
    );
    step(&mut session, END, "copy me");
    step(&mut session, b" ", "copy me");
    step(&mut session, CTRL_V, "copy me copy me");
    step(&mut session, CTRL_Z, "copy me");
    step(&mut session, CTRL_Y, "copy me copy me");
    step(&mut session, CTRL_Z, "copy me");
    step(&mut session, CTRL_Z, "copy me");
    step(&mut session, CTRL_Y, "copy me");
    step(&mut session, CTRL_Y, "copy me copy me");
    step(&mut session, SHIFT_HOME, "copy me copy me");
    step(&mut session, CTRL_X, "");
    session.send(TAB);
    session.send(CTRL_V);
    session.wait_box(NAME_ROW + 5, BOX_COLUMN, "copy me copy me");
    session.send(BACK_TAB);
    step(&mut session, b"no selection", "no selection");
    session.send(CTRL_C);
    session.screen_with("Press Ctrl+C again to quit");
    assert_eq!(session.run_output(), "[no selection|copy me copy me]");
}

// Ways this can fail: a local copy reaches only OSC 52, which Terminal.app ignores and iTerm2 and
// tmux block by default, so a paste in another program gets the old text; the clipboard program
// receives other text.
#[test]
fn a_local_copy_also_reaches_the_clipboard_program_of_the_platform() {
    let fixture = Fixture::new();
    let mut session = fixture.launch_menu();
    session.send(b"copy me");
    session.wait_box(NAME_ROW, BOX_COLUMN, "copy me");
    session.send(SHIFT_HOME);
    session.send(CTRL_C);
    let program = if cfg!(target_os = "macos") {
        "pbcopy"
    } else {
        "wl-copy"
    };
    assert_eq!(fixture.wait_copied(program), "copy me");
    session.send(b"!");
    assert_eq!(session.run_output(), "[!|]");
}

// Ways this can fail: a click places the caret in the wrong cell; a drag selects nothing; a
// double click selects nothing or the wrong span.
#[test]
fn launch_menu_selection_by_mouse_click_drag_and_double_click() {
    let fixture = Fixture::new();
    let mut session = fixture.launch_menu();
    session.send(b"alpha beta gamma");
    session.wait_box(NAME_ROW, BOX_COLUMN, "alpha beta gamma");
    // SGR 1006 reports count columns and rows from 1.
    let x = |offset: u16| BOX_COLUMN + offset + 1;
    let y = NAME_ROW + 1;
    let press = |column: u16| format!("\x1b[<0;{column};{y}M");
    let release = |column: u16| format!("\x1b[<0;{column};{y}m");
    let drag = |column: u16| format!("\x1b[<32;{column};{y}M");

    session.send(
        format!(
            "{}{}{}{}",
            press(x(7)),
            release(x(7)),
            press(x(7)),
            release(x(7))
        )
        .as_bytes(),
    );
    session.send(b"B");
    session.wait_box(NAME_ROW, BOX_COLUMN, "alpha B gamma");

    session.send(format!("{}{}{}", press(x(0)), drag(x(5)), release(x(5))).as_bytes());
    session.send(CTRL_X);
    session.wait_box(NAME_ROW, BOX_COLUMN, " B gamma");

    session.send(format!("{}{}", press(x(2)), release(x(2))).as_bytes());
    session.wait_cursor(NAME_ROW, BOX_COLUMN + 2);
    session.send(b"!");
    session.wait_box(NAME_ROW, BOX_COLUMN, " B! gamma");
    assert_eq!(session.run_output(), "[ B! gamma|]");
}

// Ways this can fail: `skit run` draws its launch menu in a terminal loop of its own, and that loop
// does not count the second press of a double click, so the press only moves the caret.
#[test]
fn the_skit_run_launch_menu_selects_a_word_with_a_double_click() {
    let fixture = Fixture::new();
    let mut session = Session::start(fixture.command_builder(&["run", "demo"]), "Run Demo");
    session.send(b"alpha beta");
    let (row, column) = session.locate("alpha beta");
    // SGR 1006 reports count columns and rows from 1, so this is the second letter of `alpha`.
    let x = column + 2;
    let y = row + 1;
    let click = format!("\x1b[<0;{x};{y}M\x1b[<0;{x};{y}m");
    session.send(format!("{click}{click}").as_bytes());
    session.send(b"X");
    session.wait_box(row, column, "X beta");
    assert_eq!(session.run_output(), "[X beta|]");
}

// Ways this can fail: the caret counts characters instead of terminal cells; a move or a delete
// splits a wide character, an emoji, or a combined character.
#[test]
fn launch_menu_caret_and_deletion_respect_wide_characters_and_graphemes() {
    let fixture = Fixture::new();
    let mut session = fixture.launch_menu();
    session.send("日本語🙂ab".as_bytes());
    session.wait_cursor(NAME_ROW, BOX_COLUMN + 10);
    session.send(LEFT);
    session.send(LEFT);
    session.wait_cursor(NAME_ROW, BOX_COLUMN + 8);
    session.send(LEFT);
    session.wait_cursor(NAME_ROW, BOX_COLUMN + 6);
    session.send(SHIFT_LEFT);
    session.wait_cursor(NAME_ROW, BOX_COLUMN + 4);
    session.send("語".as_bytes());
    session.wait_box(NAME_ROW, BOX_COLUMN, "日本語🙂ab");
    // A terminal draws an emoji ZWJ sequence in as many cells as it likes, so this case uses a
    // combined character and a plain emoji. The editor tests cover the ZWJ sequence.
    session.send(END);
    session.send("e\u{301}🙂z".as_bytes());
    session.send(LEFT);
    session.send(BACKSPACE);
    session.send(BACKSPACE);
    session.wait_box(NAME_ROW, BOX_COLUMN, "日本語🙂abz");
    assert_eq!(session.run_output(), "[日本語🙂abz|]");
}

// Ways this can fail: a delete joins an emoji with a zero-width joiner and a second emoji into one
// grapheme and leaves the caret inside it, so the typed character splits the joined emoji.
#[test]
fn a_delete_that_joins_two_graphemes_types_after_the_joined_one() {
    let fixture = Fixture::new();
    let mut session = fixture.launch_menu();
    session.paste("\u{1f469}\u{200d} \u{1f4bb}");
    session.send(HOME);
    session.send(RIGHT);
    session.send(DELETE);
    session.send(b"x");
    assert_eq!(session.run_output(), "[\u{1f469}\u{200d}\u{1f4bb}x|]");
}

// Ways this can fail: a click in the multi-line box keeps the column of an earlier Up, so the
// next Up goes to that old column and the typed character lands at the end of the line.
#[test]
fn a_click_in_the_multi_line_box_sets_the_column_of_the_next_up() {
    let fixture = Fixture::new();
    let mut session = fixture.entry_settings();
    session.send(TAB);
    let first = SETTINGS_NAME_ROW + 4;
    session.paste("abcdef\nabcdef\nabcdef");
    session.wait_cursor(first + 2, SETTINGS_BOX_COLUMN + 6);
    session.send(UP);
    session.wait_cursor(first + 1, SETTINGS_BOX_COLUMN + 6);
    // SGR 1006 reports count columns and rows from 1, so this is the cell of `c` on line 3.
    let x = SETTINGS_BOX_COLUMN + 2 + 1;
    let y = first + 2 + 1;
    session.send(format!("\x1b[<0;{x};{y}M\x1b[<0;{x};{y}m").as_bytes());
    session.wait_cursor(first + 2, SETTINGS_BOX_COLUMN + 2);
    session.send(UP);
    session.send(b"X");
    session.wait_box(first + 1, SETTINGS_BOX_COLUMN, "abXcdef");
    session.send(CTRL_S);
    session.screen_with("Settings saved");
    assert_eq!(fixture.saved().description, "abcdef\nabXcdef\nabcdef");
}

// Ways this can fail: an editing chord in the Add path box does nothing or reaches a screen
// command; the selection or the clipboard does not work there.
#[test]
fn the_add_path_box_uses_every_editing_key_group() {
    let fixture = Fixture::new();
    let mut session = Session::start(fixture.command_builder(&["add"]), "Source path");
    let row = 3;
    let step = |session: &mut Session, keys: &[u8], value: &str| {
        session.send(keys);
        session.wait_box(row, BOX_COLUMN, value);
    };
    // Words follow Unicode word boundaries: a slash separates two words, and `two.py` is one.
    step(&mut session, b"/tmp/one/two.py", "/tmp/one/two.py");
    step(&mut session, ALT_B, "/tmp/one/two.py");
    step(&mut session, CTRL_W, "/tmp/two.py");
    step(&mut session, CTRL_A, "/tmp/two.py");
    step(&mut session, ALT_F, "/tmp/two.py");
    step(&mut session, ALT_D, "/two.py");
    step(&mut session, CTRL_E, "/two.py");
    step(&mut session, SHIFT_LEFT, "/two.py");
    step(&mut session, SHIFT_LEFT, "/two.py");
    step(&mut session, CTRL_X, "/two.");
    step(&mut session, CTRL_V, "/two.py");
    step(&mut session, CTRL_V, "/two.pypy");
    step(&mut session, CTRL_Z, "/two.py");
    step(&mut session, CTRL_Y, "/two.pypy");
    step(&mut session, LEFT, "/two.pypy");
    step(&mut session, LEFT, "/two.pypy");
    step(&mut session, CTRL_U, "py");
    step(&mut session, CTRL_Z, "/two.pypy");
    step(&mut session, CTRL_A, "/two.pypy");
    step(&mut session, CTRL_D, "two.pypy");
    step(&mut session, RIGHT, "two.pypy");
    step(&mut session, RIGHT, "two.pypy");
    step(&mut session, RIGHT, "two.pypy");
    step(&mut session, CTRL_K, "two");
    let screen = session.screen();
    assert!(
        screen.contains("Add an entry"),
        "a key left the Add screen:\n{screen}"
    );
}

// Ways this can fail: the Search box of the file picker drops a terminal paste; Alt+B types the
// letter b instead of moving the caret; Ctrl+U does nothing; Ctrl+C and Ctrl+V do not reach the
// session clipboard there.
#[test]
fn the_file_picker_search_box_takes_a_paste_and_every_editing_key_group() {
    let fixture = Fixture::new();
    let mut session = Session::start(fixture.command_builder(&["add"]), "Source path");
    session.send(CTRL_O);
    let (row, column) = session.locate("Search");
    let (row, column) = (row + 1, column);
    session.paste("alpha\nlater line");
    session.wait_box(row, column, "alpha");
    let step = |session: &mut Session, keys: &[u8], value: &str| {
        session.send(keys);
        session.wait_box(row, column, value);
    };
    step(&mut session, b" beta", "alpha beta");
    step(&mut session, ALT_B, "alpha beta");
    step(&mut session, CTRL_U, "beta");
    step(&mut session, CTRL_Z, "alpha beta");
    step(&mut session, SHIFT_RIGHT, "alpha beta");
    step(&mut session, SHIFT_RIGHT, "alpha beta");
    step(&mut session, SHIFT_RIGHT, "alpha beta");
    step(&mut session, SHIFT_RIGHT, "alpha beta");
    let copied = session.pty.checkpoint();
    session.send(CTRL_C);
    session.pty.settle();
    let after_copy = session.pty.raw_after(copied);
    assert!(
        rfind(&after_copy, b"\x1b]52;c;YmV0YQ==").is_some(),
        "the copy sent nothing to the system clipboard: {:?}",
        String::from_utf8_lossy(&after_copy)
    );
    step(&mut session, RIGHT, "alpha beta");
    step(&mut session, CTRL_V, "alpha betabeta");
    // The box text ends at the last character, so the `!` shows the space that Ctrl+W keeps.
    step(&mut session, CTRL_W, "alpha");
    step(&mut session, b"!", "alpha !");
    let screen = session.screen();
    assert!(
        screen.contains("Search") && !screen.contains("Press Ctrl+C again to quit"),
        "a key left the picker or armed the quit:\n{screen}"
    );
}

// Ways this can fail: the search box ignores an editing chord; Ctrl+U clears the whole query
// instead of the text before the caret; undo does not bring the query back.
#[test]
fn the_search_box_uses_every_editing_key_group() {
    let fixture = Fixture::new();
    let mut session = fixture.start();
    session.send(b"/");
    let row = 1;
    let column = 1;
    let step = |session: &mut Session, keys: &[u8], value: &str| {
        session.send(keys);
        session.wait_box(row, column, value);
    };
    step(&mut session, b"old demo", "old demo");
    step(&mut session, ALT_B, "old demo");
    step(&mut session, CTRL_U, "demo");
    step(&mut session, CTRL_E, "demo");
    step(&mut session, b" x", "demo x");
    step(&mut session, ALT_BACKSPACE, "demo");
    step(&mut session, BACKSPACE, "demo");
    step(&mut session, SHIFT_HOME, "demo");
    step(&mut session, CTRL_X, "");
    step(&mut session, CTRL_Z, "demo");
    step(&mut session, CTRL_Y, "");
    step(&mut session, CTRL_V, "demo");
    let screen = session.screen();
    assert!(
        screen.contains("Search"),
        "a key left the search:\n{screen}"
    );
}

// Ways this can fail: the settings box drops Ctrl chords before the box sees them; Home or End at
// the edge of the box scrolls the page; the save keeps a value the keys did not make.
#[test]
fn an_entry_settings_box_owns_its_editing_keys_and_never_scrolls_the_page() {
    let fixture = Fixture::new();
    let mut session = fixture.entry_settings();
    let step = |session: &mut Session, keys: &[u8], value: &str| {
        session.send(keys);
        session.wait_box(SETTINGS_NAME_ROW, SETTINGS_BOX_COLUMN, value);
    };
    step(&mut session, END, "Demo");
    step(&mut session, END, "Demo");
    let screen = session.screen();
    assert!(
        screen
            .lines()
            .next()
            .unwrap_or_default()
            .contains("Entry settings · Demo")
            && screen.contains("Basics"),
        "End at the end of the box scrolled the page:\n{screen}"
    );
    session.wait_cursor(SETTINGS_NAME_ROW, SETTINGS_BOX_COLUMN + 4);
    step(&mut session, HOME, "Demo");
    step(&mut session, HOME, "Demo");
    step(&mut session, CTRL_E, "Demo");
    step(&mut session, b" tool", "Demo tool");
    step(&mut session, ALT_B, "Demo tool");
    step(&mut session, CTRL_K, "Demo");
    step(&mut session, CTRL_A, "Demo");
    step(&mut session, b"My ", "My Demo");
    step(&mut session, SHIFT_END, "My Demo");
    step(&mut session, CTRL_X, "My");
    step(&mut session, CTRL_Z, "My Demo");
    step(&mut session, CTRL_E, "My Demo");
    step(&mut session, CTRL_W, "My");
    step(&mut session, CTRL_Y, "My");
    step(&mut session, b"tool", "My tool");
    session.send(CTRL_S);
    session.screen_with("Settings saved");
    assert_eq!(fixture.saved().name, "My tool");
}

// Ways this can fail: the multi-line box keeps no hardware cursor, so an input method opens its
// window in the wrong place; a wide character moves the cursor by one cell; a line chord, a
// selection, the clipboard, or undo works differently from a single-line box; a pasted CRLF
// keeps its carriage return.
#[test]
fn the_multi_line_description_box_edits_like_a_single_line_box_and_keeps_the_ime_cursor() {
    let fixture = Fixture::new();
    let mut session = fixture.entry_settings();
    session.send(TAB);
    let first = SETTINGS_NAME_ROW + 4;
    session.send(b"first line");
    session.wait_cursor(first, SETTINGS_BOX_COLUMN + 10);
    session.send(ENTER);
    session.send("second 日本".as_bytes());
    session.wait_cursor(first + 1, SETTINGS_BOX_COLUMN + 11);
    session.send(LEFT);
    session.wait_cursor(first + 1, SETTINGS_BOX_COLUMN + 9);
    session.send(CTRL_A);
    session.wait_cursor(first + 1, SETTINGS_BOX_COLUMN);
    session.send(SHIFT_END);
    session.send(CTRL_X);
    session.wait_box(first + 1, SETTINGS_BOX_COLUMN, "");
    session.send(UP);
    session.wait_cursor(first, SETTINGS_BOX_COLUMN);
    session.send(END);
    session.send(b" ");
    session.send(CTRL_V);
    session.wait_box(first, SETTINGS_BOX_COLUMN, "first line second 日本");
    session.send(CTRL_A);
    session.send(ALT_F);
    session.send(ALT_F);
    session.wait_cursor(first, SETTINGS_BOX_COLUMN + 11);
    session.send(CTRL_W);
    session.wait_box(first, SETTINGS_BOX_COLUMN, "first second 日本");
    session.send(CTRL_Z);
    session.wait_box(first, SETTINGS_BOX_COLUMN, "first line second 日本");
    session.send(CTRL_Y);
    session.wait_box(first, SETTINGS_BOX_COLUMN, "first second 日本");
    session.send(b"\x1b[B");
    session.paste("\r\nthird\r\nfourth");
    session.wait_box(first + 3, SETTINGS_BOX_COLUMN, "fourth");
    session.send(CTRL_S);
    session.screen_with("Settings saved");
    assert_eq!(
        fixture.saved().description,
        "first second 日本\n\nthird\nfourth"
    );
}

// Ways this can fail: Help names no editing key, so a person must remember them.
#[test]
fn help_lists_the_editing_keys() {
    let fixture = Fixture::new();
    let mut session = fixture.start();
    session.send(b"?");
    let help = session.screen_with("Editing text");
    assert!(
        help.contains("Ctrl+A, Home"),
        "Help does not name Ctrl+A:\n{help}"
    );
    session.send(END);
    let end = session.screen_with("Ctrl+Y");
    // The Search box of the file picker keeps Ctrl+H for its hidden-files chip, and Help says so.
    for expected in [
        "Ctrl+Z",
        "Ctrl+X",
        "Ctrl+V",
        "Undo",
        "Redo",
        "Show or hide hidden files in the file picker",
    ] {
        assert!(
            end.contains(expected),
            "Help does not name {expected}:\n{end}"
        );
    }
}

// Ways this can fail: the mode stays on while a run owns the terminal, so the run receives the
// paste markers; or the mode does not come back after the run.
#[test]
fn bracketed_paste_is_off_while_a_run_has_the_terminal_and_on_again_after_it() {
    let fixture = Fixture::new();
    fs::write(
        fixture.config.path().join("config.toml"),
        "after_run = \"stay\"\n",
    )
    .unwrap();
    let mut session = fixture.launch_menu();
    session.send(b"ran");
    session.wait_box(NAME_ROW, BOX_COLUMN, "ran");
    let before = session.pty.checkpoint();
    session.send(ENTER);
    session.pty.wait_cursor_query_after(before);
    session.screen_with("Demo finished");
    let raw = session.pty.raw_after(before);
    let off = rfind(&raw, b"\x1b[?2004l").expect("the mode goes off before the run");
    let output = rfind(&raw, b"[ran|]").expect("the run output");
    let on = rfind(&raw, b"\x1b[?2004h").expect("the mode comes back after the run");
    assert!(
        off < output && output < on,
        "{:?}",
        String::from_utf8_lossy(&raw)
    );
}

// Ways this can fail: the caret stands on the last character of a long value, so the last
// character hides under the caret and a typed character seems to replace it.
#[test]
fn a_long_value_keeps_one_free_cell_for_the_caret() {
    let fixture = Fixture::new();
    let mut session = fixture.launch_menu();
    // The box is as wide as the terminal less the panel border and the box border.
    let content = COLUMNS - 4;
    let value = format!("{}Z", "x".repeat(usize::from(content) + 23));
    session.send(value.as_bytes());
    let last = BOX_COLUMN + content - 1;
    session.wait_cursor(NAME_ROW, last);
    let grid = session.grid();
    let screen = grid.screen();
    assert_eq!(screen.cell(NAME_ROW, last - 1).unwrap().contents(), "Z");
    assert_eq!(screen.cell(NAME_ROW, last).unwrap().contents(), "");
}
