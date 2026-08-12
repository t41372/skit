//! Behavioral ports of the public `tests/test_tui_edit.py` contracts from Python `main@206f9ef`.
//!
//! All six contracts are observable here. Source resolution is asserted at the store boundary; the
//! editor round trips cross the real `skit tui` PTY boundary. Python's last test pokes Textual's
//! private `_drift_cache`; Rust has no corresponding cache object, so its architectural port proves
//! the stronger user-visible invariant: within the same TUI session an initially-drifted Library
//! row is re-derived after the editor repairs the source, and the final alternate-screen frame no
//! longer carries the stale drift warning.

use std::{
    fs,
    io::{Read as _, Write as _},
    path::{Path, PathBuf},
    process::Command as StdCommand,
    thread,
    time::Duration,
};

use portable_pty::{CommandBuilder, PtySize, native_pty_system};
use skit_application::EntryRepository as _;
use skit_language::{detect_candidates, write_managed_params};
use skit_store::FileStore;
use tempfile::TempDir;

fn write_entry(
    data: &Path,
    slug: &str,
    name: &str,
    kind: &str,
    mode: &str,
    source: &Path,
    payload: Option<(&str, &[u8])>,
) {
    let directory = data.join("scripts").join(slug);
    fs::create_dir_all(&directory).unwrap();
    if let Some((filename, bytes)) = payload {
        fs::write(directory.join(filename), bytes).unwrap();
    }
    let template = (kind == "command")
        .then_some("template = \"echo hi\"\n")
        .unwrap_or("");
    fs::write(
        directory.join("meta.toml"),
        format!(
            concat!(
                "schema = 1\n",
                "name = {name:?}\n",
                "kind = {kind:?}\n",
                "mode = {mode:?}\n",
                "source = {source:?}\n",
                "source_hash = \"\"\n",
                "added_at = \"2026-08-12T00:00:00Z\"\n",
                "id = \"0123456789abcdef0123456789abcdef\"\n",
                "workdir = \"origin\"\n",
                "description = \"\"\n",
                "{template}",
            ),
            name = name,
            kind = kind,
            mode = mode,
            source = source.display().to_string(),
            template = template,
        ),
    )
    .unwrap();
    FileStore::new(data).rebuild_registry().unwrap();
}

fn compile_editor(root: &Path) -> PathBuf {
    let source = root.join("editor_probe.rs");
    fs::write(
        &source,
        r##"
use std::{env, fs, io::Write as _, path::PathBuf};
fn main() {
    let target = PathBuf::from(env::args_os().nth(1).expect("editor target"));
    if let Some(capture) = env::var_os("SKIT_EDIT_CAPTURE") {
        fs::write(capture, target.to_string_lossy().as_bytes()).expect("capture editor target");
    }
    if let Some(repair) = env::var_os("SKIT_EDIT_REPAIR") {
        fs::copy(repair, &target).expect("repair editor target");
    } else if env::var_os("SKIT_EDIT_MARK").is_some() {
        let mut file = fs::OpenOptions::new().append(true).open(&target).expect("open target");
        file.write_all(b"# edited by probe\n").expect("edit target");
    }
}
"##,
    )
    .unwrap();
    let executable = root.join(editor_probe_name());
    let status = StdCommand::new("rustc")
        .arg(&source)
        .arg("-o")
        .arg(&executable)
        .status()
        .unwrap();
    assert!(status.success(), "failed to compile editor probe");
    executable
}

#[cfg(windows)]
fn editor_probe_name() -> &'static str {
    "editor-probe.exe"
}

#[cfg(not(windows))]
fn editor_probe_name() -> &'static str {
    "editor-probe"
}

fn run_tui_edit(
    data: &Path,
    state: &Path,
    config: &Path,
    editor: &Path,
    capture: &Path,
    repair: Option<&Path>,
) -> (u32, Vec<u8>) {
    let pair = native_pty_system()
        .openpty(PtySize {
            rows: 30,
            cols: 120,
            pixel_width: 0,
            pixel_height: 0,
        })
        .unwrap();
    let mut command = CommandBuilder::new(PathBuf::from(env!("CARGO_BIN_EXE_skit")));
    command.arg("tui");
    command.env("TERM", "xterm-256color");
    command.env("SKIT_LANG", "en");
    command.env("SKIT_DATA_DIR", data);
    command.env("SKIT_STATE_DIR", state);
    command.env("SKIT_CONFIG_DIR", config);
    command.env("VISUAL", editor);
    command.env("EDITOR", editor);
    command.env("SKIT_EDIT_CAPTURE", capture);
    if let Some(repair) = repair {
        command.env("SKIT_EDIT_REPAIR", repair);
    } else {
        command.env("SKIT_EDIT_MARK", "1");
    }
    let mut child = pair.slave.spawn_command(command).unwrap();
    drop(pair.slave);

    let mut reader = pair.master.try_clone_reader().unwrap();
    let drain = thread::spawn(move || {
        let mut bytes = Vec::new();
        reader.read_to_end(&mut bytes).unwrap();
        bytes
    });
    let mut writer = pair.master.take_writer().unwrap();

    // Crossterm asks for the cursor position during terminal setup. Answer it before sending user
    // input, then give each public interaction enough time to complete its synchronous host effect.
    thread::sleep(Duration::from_millis(60));
    writer.write_all(b"\x1b[1;1R").unwrap();
    writer.flush().unwrap();
    thread::sleep(Duration::from_millis(180));
    writer.write_all(b"e").unwrap();
    writer.flush().unwrap();
    thread::sleep(Duration::from_millis(450));
    writer.write_all(b"q").unwrap();
    writer.flush().unwrap();

    let status = child.wait().unwrap();
    drop(writer);
    (status.exit_code(), drain.join().unwrap())
}

fn tui_edit(
    data: &Path,
    state: &Path,
    config: &Path,
    editor: &Path,
    capture: &Path,
) -> (u32, String) {
    let (code, bytes) = run_tui_edit(data, state, config, editor, capture, None);
    (code, String::from_utf8_lossy(&bytes).into_owned())
}

/// Minimal terminal-state interpreter for the crossterm operations Ratatui uses in these tests.
///
/// A raw PTY log contains every old frame, so `!log.contains("script changed")` would be a weak and
/// incorrect cache-invalidation assertion. This model follows cursor placement, screen/line erasure,
/// and the alternate-screen lifetime, and snapshots the frame that was visible immediately before
/// `LeaveAlternateScreen`. It also remembers whether any earlier rendered frame contained the drift
/// sentence, proving the fixture really exercised stale-state removal.
struct TerminalState {
    width: usize,
    height: usize,
    row: usize,
    col: usize,
    saved: (usize, usize),
    cells: Vec<Vec<char>>,
    last_alt: Option<Vec<Vec<char>>>,
    saw_drift: bool,
}

impl TerminalState {
    fn new(width: usize, height: usize) -> Self {
        Self {
            width,
            height,
            row: 0,
            col: 0,
            saved: (0, 0),
            cells: vec![vec![' '; width]; height],
            last_alt: None,
            saw_drift: false,
        }
    }

    fn clear(&mut self) {
        self.cells.iter_mut().for_each(|row| row.fill(' '));
        self.row = 0;
        self.col = 0;
    }

    fn row_contains_drift(&self, row: usize) -> bool {
        self.cells
            .get(row)
            .map(|cells| cells.iter().collect::<String>().contains("script changed"))
            .unwrap_or(false)
    }

    fn put(&mut self, character: char) {
        if self.row >= self.height {
            return;
        }
        if self.col >= self.width {
            self.col = 0;
            self.row = self.row.saturating_add(1);
            if self.row >= self.height {
                return;
            }
        }
        self.cells[self.row][self.col] = character;
        self.col = self.col.saturating_add(1);
        self.saw_drift |= self.row_contains_drift(self.row);
    }

    fn param(params: &str, index: usize, default: usize) -> usize {
        params
            .trim_start_matches('?')
            .split(';')
            .nth(index)
            .filter(|value| !value.is_empty())
            .and_then(|value| value.parse().ok())
            .unwrap_or(default)
    }

    fn erase_line(&mut self, mode: usize) {
        if self.row >= self.height {
            return;
        }
        match mode {
            1 => self.cells[self.row][..self.col.min(self.width)].fill(' '),
            2 => self.cells[self.row].fill(' '),
            _ => self.cells[self.row][self.col.min(self.width)..].fill(' '),
        }
    }

    fn erase_display(&mut self, mode: usize) {
        match mode {
            2 | 3 => self.clear(),
            1 => {
                for row in 0..self.row.min(self.height) {
                    self.cells[row].fill(' ');
                }
                if self.row < self.height {
                    self.cells[self.row][..self.col.min(self.width)].fill(' ');
                }
            }
            _ => {
                if self.row < self.height {
                    self.cells[self.row][self.col.min(self.width)..].fill(' ');
                    for row in self.row.saturating_add(1)..self.height {
                        self.cells[row].fill(' ');
                    }
                }
            }
        }
    }

    fn csi(&mut self, params: &str, final_byte: u8) {
        match final_byte {
            b'H' | b'f' => {
                self.row = Self::param(params, 0, 1)
                    .saturating_sub(1)
                    .min(self.height.saturating_sub(1));
                self.col = Self::param(params, 1, 1)
                    .saturating_sub(1)
                    .min(self.width.saturating_sub(1));
            }
            b'A' => self.row = self.row.saturating_sub(Self::param(params, 0, 1)),
            b'B' => {
                self.row = self
                    .row
                    .saturating_add(Self::param(params, 0, 1))
                    .min(self.height.saturating_sub(1))
            }
            b'C' => {
                self.col = self
                    .col
                    .saturating_add(Self::param(params, 0, 1))
                    .min(self.width.saturating_sub(1))
            }
            b'D' => self.col = self.col.saturating_sub(Self::param(params, 0, 1)),
            b'G' => {
                self.col = Self::param(params, 0, 1)
                    .saturating_sub(1)
                    .min(self.width.saturating_sub(1))
            }
            b'd' => {
                self.row = Self::param(params, 0, 1)
                    .saturating_sub(1)
                    .min(self.height.saturating_sub(1))
            }
            b'J' => self.erase_display(Self::param(params, 0, 0)),
            b'K' => self.erase_line(Self::param(params, 0, 0)),
            b's' => self.saved = (self.row, self.col),
            b'u' => (self.row, self.col) = self.saved,
            b'h' if params == "?1049" => self.clear(),
            b'l' if params == "?1049" => self.last_alt = Some(self.cells.clone()),
            // SGR, cursor visibility, bracketed paste, mouse modes, cursor query and other terminal
            // modes do not alter the text grid relevant to this contract.
            _ => {}
        }
    }

    fn feed(&mut self, bytes: &[u8]) {
        let mut index = 0;
        while index < bytes.len() {
            match bytes[index] {
                b'\x1b' if bytes.get(index + 1) == Some(&b'[') => {
                    let start = index + 2;
                    let mut end = start;
                    while end < bytes.len() && !(0x40..=0x7e).contains(&bytes[end]) {
                        end += 1;
                    }
                    if end >= bytes.len() {
                        break;
                    }
                    let params = String::from_utf8_lossy(&bytes[start..end]);
                    self.csi(&params, bytes[end]);
                    index = end + 1;
                }
                b'\x1b' if bytes.get(index + 1) == Some(&b']') => {
                    // OSC: consume through BEL or ST (ESC backslash).
                    index += 2;
                    while index < bytes.len() {
                        if bytes[index] == 0x07 {
                            index += 1;
                            break;
                        }
                        if bytes[index] == 0x1b && bytes.get(index + 1) == Some(&b'\\') {
                            index += 2;
                            break;
                        }
                        index += 1;
                    }
                }
                b'\x1b' => {
                    // DECSC/DECRC and charset selectors are the only non-CSI escapes crossterm may
                    // place here that affect cursor interpretation.
                    match bytes.get(index + 1).copied() {
                        Some(b'7') => {
                            self.saved = (self.row, self.col);
                            index += 2;
                        }
                        Some(b'8') => {
                            (self.row, self.col) = self.saved;
                            index += 2;
                        }
                        Some(b'(' | b')') => index = (index + 3).min(bytes.len()),
                        Some(_) => index += 2,
                        None => break,
                    }
                }
                b'\r' => {
                    self.col = 0;
                    index += 1;
                }
                b'\n' => {
                    self.row = self
                        .row
                        .saturating_add(1)
                        .min(self.height.saturating_sub(1));
                    index += 1;
                }
                0x08 => {
                    self.col = self.col.saturating_sub(1);
                    index += 1;
                }
                byte if byte < 0x20 || byte == 0x7f => index += 1,
                _ => {
                    let Ok(text) = std::str::from_utf8(&bytes[index..]) else {
                        // Decode exactly one UTF-8 scalar by trying the legal prefix lengths. PTY
                        // output from skit is UTF-8; incomplete tail bytes are simply ignored.
                        let mut decoded = None;
                        for length in 1..=4 {
                            if index + length <= bytes.len()
                                && let Ok(piece) =
                                    std::str::from_utf8(&bytes[index..index + length])
                                && let Some(character) = piece.chars().next()
                                && character.len_utf8() == length
                            {
                                decoded = Some((character, length));
                                break;
                            }
                        }
                        if let Some((character, length)) = decoded {
                            self.put(character);
                            index += length;
                        } else {
                            index += 1;
                        }
                        continue;
                    };
                    let Some(character) = text.chars().next() else {
                        break;
                    };
                    self.put(character);
                    index += character.len_utf8();
                }
            }
        }
    }

    fn final_text(&self) -> String {
        self.last_alt
            .as_ref()
            .unwrap_or(&self.cells)
            .iter()
            .map(|row| row.iter().collect::<String>())
            .collect::<Vec<_>>()
            .join("\n")
    }
}

#[test]
fn test_editable_source_copy_mode_points_at_the_stored_copy() {
    let data = TempDir::new().unwrap();
    let original = data.path().join("original.py");
    fs::write(&original, b"print('original')\n").unwrap();
    write_entry(
        data.path(),
        "a",
        "a",
        "python",
        "copy",
        &original,
        Some(("script.py", b"print(1)\n")),
    );

    let store = FileStore::new(data.path());
    let entry = store.resolve("a").unwrap();
    assert_eq!(
        store.payload_path(&entry).unwrap(),
        data.path().join("scripts/a/script.py")
    );
}

#[test]
fn test_editable_source_reference_mode_points_at_the_original() {
    let data = TempDir::new().unwrap();
    let original = data.path().join("orig.py");
    fs::write(&original, b"print(1)\n").unwrap();
    write_entry(
        data.path(),
        "r",
        "r",
        "python",
        "reference",
        &original,
        None,
    );

    let store = FileStore::new(data.path());
    let entry = store.resolve("r").unwrap();
    assert_eq!(store.payload_path(&entry).unwrap(), original);
}

#[test]
fn test_editable_source_command_entry_has_none() {
    let data = TempDir::new().unwrap();
    let state = TempDir::new().unwrap();
    let config = TempDir::new().unwrap();
    write_entry(
        data.path(),
        "c",
        "c",
        "command",
        "copy",
        Path::new(""),
        None,
    );

    let store = FileStore::new(data.path());
    let entry = store.resolve("c").unwrap();
    assert!(
        store.payload_path(&entry).is_err(),
        "a command template must not masquerade as an editable file"
    );

    let mut command = assert_cmd::cargo::cargo_bin_cmd!("skit");
    command
        .env("SKIT_DATA_DIR", data.path())
        .env("SKIT_STATE_DIR", state.path())
        .env("SKIT_CONFIG_DIR", config.path())
        .env("SKIT_LANG", "en")
        .args(["edit", "c", "--no-input"])
        .assert()
        .code(2)
        .stderr(predicates::str::contains(
            "does not have an editable source",
        ));
}

#[test]
fn test_edit_opens_editor_and_reports() {
    let data = TempDir::new().unwrap();
    let state = TempDir::new().unwrap();
    let config = TempDir::new().unwrap();
    let tools = TempDir::new().unwrap();
    let original = data.path().join("original.py");
    fs::write(&original, b"print('original')\n").unwrap();
    write_entry(
        data.path(),
        "a",
        "a",
        "python",
        "copy",
        &original,
        Some(("script.py", b"print(1)\n")),
    );
    let editor = compile_editor(tools.path());
    let capture = tools.path().join("capture.txt");

    let (code, output) = tui_edit(data.path(), state.path(), config.path(), &editor, &capture);
    assert_eq!(code, 0, "{output}");
    assert_eq!(
        fs::read_to_string(&capture).unwrap(),
        data.path()
            .join("scripts/a/script.py")
            .display()
            .to_string()
    );
    assert!(
        fs::read_to_string(data.path().join("scripts/a/script.py"))
            .unwrap()
            .contains("# edited by probe")
    );
    // Keep the Python-visible receipt, not the Rust implementation's current generic success copy.
    // If Rust prints only "Source saved", this is a deliberate parity finding.
    assert!(output.contains("Edited a."), "{output}");
}

#[test]
fn test_edit_command_entry_reports_no_source() {
    let data = TempDir::new().unwrap();
    let state = TempDir::new().unwrap();
    let config = TempDir::new().unwrap();
    let tools = TempDir::new().unwrap();
    write_entry(
        data.path(),
        "c",
        "c",
        "command",
        "copy",
        Path::new(""),
        None,
    );
    let editor = compile_editor(tools.path());
    let capture = tools.path().join("capture.txt");

    let (_code, output) = tui_edit(data.path(), state.path(), config.path(), &editor, &capture);
    assert!(!capture.exists(), "the editor ran for a command entry");
    assert!(output.contains("no editable source"), "{output}");
}

#[test]
fn test_edit_invalidates_the_drift_cache() {
    let data = TempDir::new().unwrap();
    let state = TempDir::new().unwrap();
    let config = TempDir::new().unwrap();
    let tools = TempDir::new().unwrap();
    let original = data.path().join("original.py");
    let base = "CITY = 'Taipei'\nprint(CITY)\n";
    fs::write(&original, base).unwrap();

    let declarations = detect_candidates("python", base);
    assert_eq!(declarations.len(), 1, "fixture must expose exactly CITY");
    assert_eq!(declarations[0].name, "CITY");
    let clean = write_managed_params("python", base, &declarations).unwrap();
    let drifted = clean.replacen("CITY = 'Taipei'", "CITY = 42", 1);
    assert_ne!(
        drifted, clean,
        "fixture did not create a type/default drift"
    );

    write_entry(
        data.path(),
        "a",
        "a",
        "python",
        "copy",
        &original,
        Some(("script.py", drifted.as_bytes())),
    );
    let store = FileStore::new(data.path());
    let entry = store.resolve("a").unwrap();
    let before = skit_store::library_surface(&store, state.path(), config.path()).unwrap();
    assert!(
        before.details.get(&entry.slug).unwrap().drifted,
        "fixture must begin with a real drift warning"
    );

    let repair = tools.path().join("clean.py");
    fs::write(&repair, clean.as_bytes()).unwrap();
    let editor = compile_editor(tools.path());
    let capture = tools.path().join("capture.txt");
    let (code, bytes) = run_tui_edit(
        data.path(),
        state.path(),
        config.path(),
        &editor,
        &capture,
        Some(&repair),
    );
    assert_eq!(
        code,
        0,
        "TUI did not exit cleanly: {}",
        String::from_utf8_lossy(&bytes)
    );
    assert_eq!(
        fs::read_to_string(data.path().join("scripts/a/script.py")).unwrap(),
        clean,
        "the editor probe did not actually repair the drifted source"
    );

    let mut terminal = TerminalState::new(120, 30);
    terminal.feed(&bytes);
    assert!(
        terminal.saw_drift,
        "the same TUI session never rendered the fixture's initial drift warning"
    );
    let final_frame = terminal.final_text();
    assert!(
        !final_frame.contains("script changed"),
        "the post-editor Library frame reused stale drift state instead of re-deriving it:\n{final_frame}"
    );

    // Independently pin the returned store projection so a parser bug cannot turn a stale TUI into
    // a false success: the source now has no drift according to the same public Library projection
    // the TUI reload consumes.
    let after = skit_store::library_surface(&store, state.path(), config.path()).unwrap();
    assert!(
        !after.details.get(&entry.slug).unwrap().drifted,
        "the repaired source still projects as drifted"
    );
}
