//! End-to-end palette census of the Ratatui interface.
//!
//! Each case starts the real `skit` binary on a pseudo-terminal, opens one screen, and replays the
//! terminal output through `vt100`. The census records every visible cell with its foreground,
//! background, and attributes. Every run writes each census to `CARGO_TARGET_TMPDIR/tui-census/`,
//! and each census must be byte-identical to the committed file in `tests/tui_census/`. Set
//! `SKIT_BLESS_TUI_CENSUS=1` to replace the committed files.
//!
//! The child gets an empty environment plus the exact variables each case names, so a variable in
//! the developer's shell cannot change a census.
//!
//! Unix only: the handshake that `PtyChild` uses to start a Ratatui session is a Unix PTY protocol
//! (see `terminal_pty.rs`).
#![cfg(unix)]

use std::{
    env,
    fmt::Write as _,
    fs,
    path::{Path, PathBuf},
    time::Duration,
};

use portable_pty::{CommandBuilder, PtySize};
use skit_store::{FileConfigStore, FileStore};
use tempfile::TempDir;

#[path = "support/pty.rs"]
mod pty;

use pty::{AnswerQueries, PtyChild, strip_terminal_control};

const ROWS: u16 = 30;
const COLUMNS: u16 = 100;
/// A word that the library footer shows once the first frame is complete.
///
/// Every needle is one word: Ratatui moves the cursor over a default-style space instead of
/// writing it, so a needle with a space can miss text that the screen shows.
const LIBRARY_READY: &str = "entries";

/// One screen of the census: the keys that open it from the library, and text that only the new
/// frame draws.
struct Screen {
    name: &'static str,
    keys: &'static [u8],
    needle: &'static str,
}

const SCREENS: [Screen; 8] = [
    Screen {
        name: "library",
        keys: b"",
        needle: LIBRARY_READY,
    },
    Screen {
        name: "library-detail-hidden",
        keys: b"\t",
        needle: "Kind",
    },
    Screen {
        name: "run-form",
        keys: b"\r",
        needle: "Preset:",
    },
    Screen {
        name: "help",
        keys: b"?",
        needle: "Rerun",
    },
    Screen {
        name: "add",
        keys: b"a",
        needle: "script,",
    },
    Screen {
        name: "entry-settings",
        keys: b"p",
        needle: "Renaming",
    },
    Screen {
        name: "remove",
        keys: b"\x1b[3~",
        needle: "removal",
    },
    Screen {
        name: "search",
        keys: b"/",
        needle: "list",
    },
];

/// Terminal variables for one census environment, added to the fixed base environment.
struct Environment {
    name: &'static str,
    variables: &'static [(&'static str, &'static str)],
}

const TRUECOLOR: Environment = Environment {
    name: "truecolor",
    variables: &[("COLORTERM", "truecolor")],
};
const NO_COLORTERM: Environment = Environment {
    name: "no-colorterm",
    variables: &[],
};
const NO_COLOR: Environment = Environment {
    name: "no-color",
    variables: &[("COLORTERM", "truecolor"), ("NO_COLOR", "1")],
};
const EMPTY_NO_COLOR: Environment = Environment {
    name: "empty-no-color",
    variables: &[("COLORTERM", "truecolor"), ("NO_COLOR", "")],
};

struct Fixture {
    data: TempDir,
    state: TempDir,
    config: TempDir,
    home: TempDir,
}

impl Fixture {
    fn new() -> Self {
        let fixture = Self {
            data: TempDir::new().unwrap(),
            state: TempDir::new().unwrap(),
            config: TempDir::new().unwrap(),
            home: TempDir::new().unwrap(),
        };
        FileConfigStore::new(fixture.config.path().to_path_buf())
            .mark_mirror_configured()
            .unwrap();
        write_command_entry(
            fixture.data.path(),
            "alpha",
            "Alpha",
            "0123456789abcdef0123456789abcdef",
            true,
        );
        write_command_entry(
            fixture.data.path(),
            "beta",
            "Beta",
            "fedcba9876543210fedcba9876543210",
            false,
        );
        FileStore::new(fixture.data.path())
            .rebuild_registry()
            .unwrap();
        fixture
    }

    fn spawn(&self, environment: &Environment) -> PtyChild {
        let mut command = CommandBuilder::new(PathBuf::from(env!("CARGO_BIN_EXE_skit")));
        command.env_clear();
        command.cwd(self.home.path());
        // No screen in this census reads PATH, and a fixed value keeps a later one stable.
        command.env("PATH", "/nonexistent-skit-census-path");
        command.env("HOME", self.home.path());
        command.env("TERM", "xterm-256color");
        command.env("SKIT_LANG", "en");
        command.env("SKIT_DATA_DIR", self.data.path());
        command.env("SKIT_STATE_DIR", self.state.path());
        command.env("SKIT_CONFIG_DIR", self.config.path());
        for (key, value) in environment.variables {
            command.env(key, value);
        }
        // A coverage run finds the child's profile through these variables. They do not change a
        // frame.
        for (key, value) in env::vars_os() {
            let key_text = key.to_string_lossy();
            if key_text == "LLVM_PROFILE_FILE" || key_text.starts_with("CARGO_LLVM_COV") {
                command.env(key, value);
            }
        }
        PtyChild::spawn(
            command,
            PtySize {
                rows: ROWS,
                cols: COLUMNS,
                pixel_width: 0,
                pixel_height: 0,
            },
            AnswerQueries::On,
        )
    }

    /// Open `screen` in a new session and return every byte the session wrote.
    ///
    /// The snapshot happens while the interface still runs, because `vt100` replays the alternate
    /// screen only until the session leaves it.
    fn capture(&self, environment: &Environment, screen: &Screen) -> Vec<u8> {
        let mut child = self.spawn(environment);
        child.wait_cursor_query_after(0);
        wait_for_text(&mut child, 0, LIBRARY_READY);
        child.settle();
        if !screen.keys.is_empty() {
            let checkpoint = child.checkpoint();
            child.send(screen.keys);
            wait_for_text(&mut child, checkpoint, screen.needle);
            child.settle();
        }
        let raw = child.raw_after(0);
        quit(&mut child);
        raw
    }
}

/// End the session the way a person does: two Ctrl+C presses.
///
/// A killed child never writes its coverage profile, so the census would add no coverage.
fn quit(child: &mut PtyChild) {
    child.send(&[0x03]);
    child.settle();
    child.send(&[0x03]);
    let status = child.wait_exit_within(Duration::from_secs(10));
    assert!(
        status.success(),
        "the session did not end cleanly: {}",
        status.exit_code()
    );
}

fn wait_for_text(child: &mut PtyChild, checkpoint: usize, needle: &str) {
    child.wait_for_after_rendered(checkpoint, needle, |bytes| {
        strip_terminal_control(&String::from_utf8_lossy(bytes))
    });
}

fn write_command_entry(data: &Path, slug: &str, name: &str, id: &str, with_parameter: bool) {
    let directory = data.join("scripts").join(slug);
    fs::create_dir_all(&directory).unwrap();
    let (template, parameters) = if with_parameter {
        (
            "echo {name}",
            concat!(
                "params = [\"name\"]\n",
                "[[parameters]]\n",
                "name = \"name\"\n",
                "delivery = \"placeholder\"\n",
                "required = true\n",
            ),
        )
    } else {
        ("echo done", "params = []\n")
    };
    let mut meta = String::new();
    writeln!(meta, "schema = 1").unwrap();
    writeln!(meta, "name = {name:?}").unwrap();
    writeln!(meta, "kind = \"command\"").unwrap();
    writeln!(meta, "mode = \"copy\"").unwrap();
    writeln!(meta, "source = \"\"").unwrap();
    writeln!(meta, "source_hash = \"\"").unwrap();
    writeln!(meta, "added_at = \"2026-08-08T00:00:00Z\"").unwrap();
    writeln!(meta, "id = {id:?}").unwrap();
    writeln!(meta, "workdir = \"invoke\"").unwrap();
    writeln!(meta, "description = \"A census fixture.\"").unwrap();
    writeln!(meta, "template = {template:?}").unwrap();
    meta.push_str(parameters);
    fs::write(directory.join("meta.toml"), meta).unwrap();
}

/// The screen that `raw` leaves on a terminal of the census size.
fn replay(raw: &[u8]) -> vt100::Parser {
    let mut parser = vt100::Parser::new(ROWS, COLUMNS, 0);
    parser.process(raw);
    assert!(
        parser.screen().alternate_screen(),
        "the session left the alternate screen before the snapshot"
    );
    parser
}

fn color_name(color: vt100::Color) -> String {
    match color {
        vt100::Color::Default => "default".to_owned(),
        vt100::Color::Idx(index) => format!("idx:{index}"),
        vt100::Color::Rgb(red, green, blue) => format!("#{red:02x}{green:02x}{blue:02x}"),
    }
}

fn attributes(cell: &vt100::Cell) -> Vec<&'static str> {
    [
        (cell.bold(), "bold"),
        (cell.dim(), "dim"),
        (cell.italic(), "italic"),
        (cell.underline(), "underline"),
        (cell.inverse(), "inverse"),
    ]
    .into_iter()
    .filter_map(|(set, name)| set.then_some(name))
    .collect()
}

/// One run of cells in a row that share foreground, background, and attributes.
#[derive(PartialEq)]
struct Segment {
    text: String,
    foreground: String,
    background: String,
    attributes: Vec<&'static str>,
}

fn segments(screen: &vt100::Screen, row: u16) -> Vec<Segment> {
    let mut segments: Vec<Segment> = Vec::new();
    for column in 0..COLUMNS {
        let cell = screen.cell(row, column).unwrap();
        if cell.is_wide_continuation() {
            continue;
        }
        let text = if cell.has_contents() {
            cell.contents()
        } else {
            " "
        };
        let foreground = color_name(cell.fgcolor());
        let background = color_name(cell.bgcolor());
        let attributes = attributes(cell);
        match segments.last_mut() {
            Some(last)
                if last.foreground == foreground
                    && last.background == background
                    && last.attributes == attributes =>
            {
                last.text.push_str(text);
            }
            _ => segments.push(Segment {
                text: text.to_owned(),
                foreground,
                background,
                attributes,
            }),
        }
    }
    segments
}

fn json_string(text: &str) -> String {
    serde_json::to_string(text).unwrap()
}

/// The census document for one screen: one JSON line per terminal row.
fn census(raw: &[u8], screen: &str, environment: &str) -> String {
    let parser = replay(raw);
    let mut out = String::new();
    writeln!(out, "{{").unwrap();
    writeln!(out, "  \"screen\": {},", json_string(screen)).unwrap();
    writeln!(out, "  \"environment\": {},", json_string(environment)).unwrap();
    writeln!(out, "  \"columns\": {COLUMNS},").unwrap();
    writeln!(out, "  \"rows\": [").unwrap();
    for row in 0..ROWS {
        let cells = segments(parser.screen(), row)
            .iter()
            .map(|segment| {
                let attributes = segment
                    .attributes
                    .iter()
                    .map(|name| json_string(name))
                    .collect::<Vec<_>>()
                    .join(", ");
                format!(
                    "{{\"text\": {}, \"fg\": {}, \"bg\": {}, \"attrs\": [{attributes}]}}",
                    json_string(&segment.text),
                    json_string(&segment.foreground),
                    json_string(&segment.background),
                )
            })
            .collect::<Vec<_>>()
            .join(", ");
        let separator = if row + 1 == ROWS { "" } else { "," };
        writeln!(out, "    [{cells}]{separator}").unwrap();
    }
    writeln!(out, "  ]").unwrap();
    writeln!(out, "}}").unwrap();
    out
}

/// Write the artifact, then compare it with the committed census. Return a failure line, if any.
fn check_census(file: &str, actual: &str) -> Option<String> {
    let artifact = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("tui-census")
        .join(file);
    fs::create_dir_all(artifact.parent().unwrap()).unwrap();
    fs::write(&artifact, actual).unwrap();
    let committed = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/tui_census")
        .join(file);
    if env::var_os("SKIT_BLESS_TUI_CENSUS").is_some() {
        fs::create_dir_all(committed.parent().unwrap()).unwrap();
        fs::write(&committed, actual).unwrap();
        return None;
    }
    match fs::read_to_string(&committed) {
        Ok(expected) if expected == actual => None,
        Ok(_) => Some(format!(
            "{file} differs from {}; the new census is {}",
            committed.display(),
            artifact.display()
        )),
        Err(error) => Some(format!(
            "{file}: cannot read {} ({error}); the new census is {}",
            committed.display(),
            artifact.display()
        )),
    }
}

fn run_census(environment: &Environment) {
    let fixture = Fixture::new();
    let failures = SCREENS
        .iter()
        .filter_map(|screen| {
            let raw = fixture.capture(environment, screen);
            let file = format!("{}.{}.json", screen.name, environment.name);
            check_census(&file, &census(&raw, screen.name, environment.name))
        })
        .collect::<Vec<_>>();
    assert!(
        failures.is_empty(),
        "census mismatch (set SKIT_BLESS_TUI_CENSUS=1 to accept):\n{}",
        failures.join("\n")
    );
}

#[test]
fn census_with_truecolor() {
    run_census(&TRUECOLOR);
}

#[test]
fn census_without_colorterm() {
    run_census(&NO_COLORTERM);
}

#[test]
fn census_with_no_color() {
    run_census(&NO_COLOR);
}

/// Every cell of the selected library row that shows `name`, and every cell on the screen.
fn library_cells(environment: &Environment) -> (Vec<vt100::Cell>, Vec<vt100::Cell>) {
    let fixture = Fixture::new();
    let raw = fixture.capture(environment, &SCREENS[0]);
    let parser = replay(&raw);
    let screen = parser.screen();
    let row = (0..ROWS)
        .find(|row| {
            screen
                .contents_between(*row, 0, *row, COLUMNS)
                .starts_with("│Alpha")
        })
        .expect("the library shows the Alpha row");
    let name = (1..=5)
        .map(|column| screen.cell(row, column).unwrap().clone())
        .collect();
    let all = (0..ROWS)
        .flat_map(|row| (0..COLUMNS).map(move |column| (row, column)))
        .map(|(row, column)| screen.cell(row, column).unwrap().clone())
        .collect();
    (name, all)
}

fn assert_no_color_contract(environment: &Environment) {
    let (name, all) = library_cells(environment);
    let colored = all
        .iter()
        .filter(|cell| {
            cell.fgcolor() != vt100::Color::Default || cell.bgcolor() != vt100::Color::Default
        })
        .count();
    let unbolded = name.iter().filter(|cell| !cell.bold()).count();
    assert!(
        colored == 0 && unbolded == 0,
        "{}: {colored} cells carry a color and {unbolded} of the 5 selected-row name cells lost bold; \
         version 0.4 (Textual NoColor filter) drops every color and keeps bold",
        environment.name
    );
}

/// Version 0.4 applies Textual's `NoColor` filter: every color becomes the default, and bold,
/// dim, and reverse stay. The selected library row is bold in version 0.4.
#[test]
fn no_color_drops_every_color_and_keeps_the_selected_row_bold() {
    assert_no_color_contract(&NO_COLOR);
}

/// Version 0.4's interface counts a present but empty `NO_COLOR` as set (Textual 8.2.8
/// `app.py:614`).
#[test]
fn empty_no_color_also_drops_every_color() {
    assert_no_color_contract(&EMPTY_NO_COLOR);
}
