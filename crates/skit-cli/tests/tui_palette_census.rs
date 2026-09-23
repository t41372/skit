//! End-to-end palette census of the Ratatui interface.
//!
//! Each case starts the real `skit` binary on a pseudo-terminal, opens one screen with the keys a
//! person types, and replays the terminal output through `vt100`. The census records every visible
//! cell with its foreground, background, and attributes. Every run writes each census to
//! `CARGO_TARGET_TMPDIR/tui-census/`, and each census must be byte-identical to the committed file
//! in `tests/tui_census/`. Set `SKIT_BLESS_TUI_CENSUS=1` to replace the committed files.
//!
//! The child gets an empty environment plus the exact variables each case names, so a variable in
//! the developer's shell cannot change a census. The fixture lives under `/tmp`, so its path has
//! the same length on macOS and Linux, and the census replaces the random part of that path.
//!
//! Unix only: the handshake that `PtyChild` uses to start a Ratatui session is a Unix PTY protocol
//! (see `terminal_pty.rs`).
#![cfg(unix)]

use std::{
    env,
    fmt::Write as _,
    fs,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use portable_pty::{CommandBuilder, PtySize};
use skit_store::{FileConfigStore, FileStore};
use tempfile::TempDir;

#[path = "support/pty.rs"]
mod pty;

use pty::{AnswerQueries, PtyChild, TerminalColors};

const ROWS: u16 = 30;
const COLUMNS: u16 = 100;
/// Text that the library shows once its first frame is drawn.
const LIBRARY_READY: &str = "Library";
/// How long one step may take to show its needle on a loaded host.
const STEP_BUDGET: Duration = Duration::from_secs(30);
/// The fixture root that every census shows in place of the random temporary directory. It has
/// the same length as the real root (`/tmp/.tmp` and six random characters).
const ROOT_TOKEN: &str = "/tmp/.tmpCENSUS";

const DOWN: &[u8] = b"\x1b[B";
const TO_BETA: &[u8] = DOWN;
// The library lists command entries first, then Python, then prompts.
const TO_GAMMA: &[u8] = b"\x1b[B\x1b[B";
const TO_ETA: &[u8] = b"\x1b[B\x1b[B\x1b[B";
const TO_ZETA: &[u8] = b"\x1b[B\x1b[B\x1b[B\x1b[B";
const CTRL_R: &[u8] = b"\x12";
const CTRL_C: &[u8] = b"\x03";
const CTRL_O: &[u8] = b"\x0f";
const CTRL_S: &[u8] = b"\x13";
const CTRL_T: &[u8] = b"\x14";
const DELETE: &[u8] = b"\x1b[3~";
const ESCAPE: &[u8] = b"\x1b";
const F2: &[u8] = b"\x1bOQ";
const PAGE_DOWN: &[u8] = b"\x1b[6~";

/// One key press or typed text, and a word that the screen shows only after it. An empty needle
/// waits for the terminal to go quiet instead.
struct Step {
    keys: &'static [u8],
    needle: &'static str,
}

const fn step(keys: &'static [u8], needle: &'static str) -> Step {
    Step { keys, needle }
}

/// One screen of the census: the steps that open it from the library, the terminal size, and the
/// working directory when it is not the fixture home.
struct Screen {
    name: &'static str,
    rows: u16,
    columns: u16,
    steps: &'static [Step],
    cwd: Option<&'static str>,
}

const fn screen(name: &'static str, steps: &'static [Step]) -> Screen {
    Screen {
        name,
        rows: ROWS,
        columns: COLUMNS,
        steps,
        cwd: None,
    }
}

const OPEN_ALPHA_RUN: Step = step(b"\r", "Preset:");
const OPEN_GAMMA: Step = step(TO_GAMMA, "Typed");
const OPEN_GAMMA_RUN: Step = step(b"\r", "whole");
const OPEN_SETTINGS: Step = step(b"p", "Renaming");
const OPEN_PREFERENCES: Step = step(b",", "Interface");
const OPEN_ADD: Step = step(b"a", "script,");

const SCREENS: &[Screen] = &[
    screen("library", &[]),
    screen("library-detail-hidden", &[step(b"\t", "Kind")]),
    screen("library-beta", &[step(TO_BETA, "failed")]),
    screen("library-gamma", &[OPEN_GAMMA]),
    screen("library-python", &[step(TO_ETA, "Python")]),
    Screen {
        name: "library-narrow",
        rows: 12,
        columns: 44,
        steps: &[],
        cwd: None,
    },
    screen("search", &[step(b"/", "list")]),
    screen("search-typed", &[step(b"/", "list"), step(b"al", "1/5")]),
    screen("quit-notice", &[step(CTRL_C, "again")]),
    screen("help", &[step(b"?", "Rerun")]),
    screen("run-form", &[OPEN_ALPHA_RUN]),
    screen("run-required", &[OPEN_ALPHA_RUN, step(b"\r", "required.")]),
    screen("run-typed", &[OPEN_GAMMA, OPEN_GAMMA_RUN]),
    screen(
        "run-bad-number",
        &[
            OPEN_GAMMA,
            OPEN_GAMMA_RUN,
            step(b"\x7fabc", "abc"),
            step(b"\r", "typed"),
        ],
    ),
    screen(
        "run-path-suggestion",
        &[
            OPEN_GAMMA,
            OPEN_GAMMA_RUN,
            step(b"\t\t\t", ""),
            step(b"da", "ta.csv"),
        ],
    ),
    // The menu shows the working directory as a fixed path. The resolved fixture path is longer
    // on macOS and wraps there, so this screen runs from `/`.
    Screen {
        cwd: Some("/"),
        ..screen(
            "run-token-menu",
            &[OPEN_ALPHA_RUN, step(CTRL_T, "run-time")],
        )
    },
    // A filter that matches no environment variable leaves the list empty, as version 0.4 does.
    Screen {
        cwd: Some("/"),
        ..screen(
            "run-environment-empty",
            &[
                OPEN_ALPHA_RUN,
                step(CTRL_T, "run-time"),
                step(b"\x1b[B\x1b[B\x1b[B\x1b[B\x1b[B", ""),
                step(b"\r", "filter"),
                step(b"zzqq", "zzqq"),
            ],
        )
    },
    screen("run-preset-name", &[OPEN_ALPHA_RUN, step(CTRL_S, "preset")]),
    screen(
        "run-preset-name-empty",
        &[OPEN_ALPHA_RUN, step(CTRL_S, "preset"), step(b"\r", "")],
    ),
    screen(
        "run-prompt",
        &[step(TO_ZETA, "prompt."), step(b"\r", "Runner")],
    ),
    screen("rename", &[step(F2, "Rename")]),
    screen("remove", &[step(DELETE, "removal")]),
    screen("entry-settings", &[OPEN_SETTINGS]),
    screen(
        "entry-settings-name-focus",
        &[OPEN_SETTINGS, step(b"\t", "")],
    ),
    screen("entry-settings-typed", &[OPEN_GAMMA, OPEN_SETTINGS]),
    screen(
        "entry-settings-prompt",
        &[step(TO_ZETA, "prompt."), OPEN_SETTINGS],
    ),
    // Page Down scrolls to the key hint that opens a new agent, below the runner choice.
    screen(
        "entry-settings-prompt-runner",
        &[
            step(TO_ZETA, "prompt."),
            OPEN_SETTINGS,
            step(PAGE_DOWN, "New agent…"),
        ],
    ),
    screen(
        "entry-settings-discard",
        &[OPEN_SETTINGS, step(b"x", ""), step(ESCAPE, "Discard")],
    ),
    screen("presets", &[step(b"s", "None")]),
    screen("preferences", &[OPEN_PREFERENCES]),
    // On a short terminal the Editor box scrolls the language select half out of view, so the
    // select is drawn clipped.
    Screen {
        name: "preferences-clipped-select",
        rows: 10,
        columns: COLUMNS,
        steps: &[OPEN_PREFERENCES, step(b"\t", "")],
        cwd: None,
    },
    screen(
        "preferences-agents",
        &[OPEN_PREFERENCES, step(b"\t\t\t\t\t", "claude")],
    ),
    screen(
        "preferences-new-agent",
        &[
            OPEN_PREFERENCES,
            step(b"\t\t\t\t\t\t", ""),
            step(b"\r", "aider"),
        ],
    ),
    screen(
        "preferences-agent-skill",
        &[
            OPEN_PREFERENCES,
            step(b"\t\t\t\t\t\t\t", ""),
            step(b"\r", "(user)"),
        ],
    ),
    screen(
        "preferences-language",
        &[
            OPEN_PREFERENCES,
            step(b"\t\t\t\t\t\t\t\t\t\t\t\t\t", ""),
            step(b"\r", "zh-TW"),
        ],
    ),
    screen("health", &[step(b"D", "Issues")]),
    screen("health-rebuilt", &[step(b"D", "Issues"), step(CTRL_R, "")]),
    screen("add", &[OPEN_ADD]),
    screen(
        "add-missing-source",
        &[
            OPEN_ADD,
            step(b"/nonexistent/file.sh", ""),
            step(b"\r", "found:"),
        ],
    ),
    screen("add-file-picker", &[OPEN_ADD, step(CTRL_O, "directory")]),
    screen(
        "add-file-picker-selection",
        &[OPEN_ADD, step(CTRL_O, "directory"), step(DOWN, "")],
    ),
    screen(
        "add-review",
        &[OPEN_ADD, step(b"hello.sh", ""), step(b"\r", "Tick")],
    ),
    screen(
        "add-editor-error",
        &[OPEN_ADD, step(b"\t\t\t\t", ""), step(b"\r", "editor")],
    ),
];

/// One census environment: the terminal variables added to the fixed base environment, the
/// `theme` setting in `config.toml`, and the default colors the terminal reports when asked.
struct Environment {
    name: &'static str,
    variables: &'static [(&'static str, &'static str)],
    theme: &'static str,
    colors: TerminalColors,
}

const TRUECOLOR: Environment = Environment {
    name: "truecolor",
    variables: &[("COLORTERM", "truecolor")],
    theme: "skit",
    colors: TerminalColors::Unknown,
};
const NO_COLORTERM: Environment = Environment {
    name: "no-colorterm",
    variables: &[],
    theme: "skit",
    colors: TerminalColors::Unknown,
};
const NO_COLOR: Environment = Environment {
    name: "no-color",
    variables: &[("COLORTERM", "truecolor"), ("NO_COLOR", "1")],
    theme: "skit",
    colors: TerminalColors::Unknown,
};
/// A terminal that names no color count: Rich 15.0.0 picks the 16-color system.
const BASIC_TERM: Environment = Environment {
    name: "basic-term",
    variables: &[("TERM", "xterm")],
    theme: "skit",
    colors: TerminalColors::Unknown,
};
/// The terminal theme on a dark terminal that answers OSC 10 and OSC 11.
const TERMINAL_DARK: Environment = Environment {
    name: "terminal-dark",
    variables: &[("COLORTERM", "truecolor")],
    theme: "terminal",
    colors: TerminalColors::Dark,
};
/// The terminal theme on a light terminal.
const TERMINAL_LIGHT: Environment = Environment {
    name: "terminal-light",
    variables: &[("COLORTERM", "truecolor")],
    theme: "terminal",
    colors: TerminalColors::Light,
};
/// The terminal theme on a terminal that answers only DA1, so the background stays unknown.
const TERMINAL_UNKNOWN: Environment = Environment {
    name: "terminal-unknown",
    variables: &[("COLORTERM", "truecolor")],
    theme: "terminal",
    colors: TerminalColors::Unknown,
};
/// The terminal theme with `NO_COLOR`.
const TERMINAL_NO_COLOR: Environment = Environment {
    name: "terminal-no-color",
    variables: &[("COLORTERM", "truecolor"), ("NO_COLOR", "1")],
    theme: "terminal",
    colors: TerminalColors::Dark,
};
const EMPTY_NO_COLOR: Environment = Environment {
    name: "empty-no-color",
    variables: &[("COLORTERM", "truecolor"), ("NO_COLOR", "")],
    theme: "skit",
    colors: TerminalColors::Unknown,
};

struct Fixture {
    root: TempDir,
}

impl Fixture {
    fn new(environment: &Environment) -> Self {
        let fixture = Self {
            root: TempDir::new_in("/tmp").unwrap(),
        };
        for directory in ["data", "state", "config", "home"] {
            fs::create_dir(fixture.root.path().join(directory)).unwrap();
        }
        let config = FileConfigStore::new(fixture.path("config"));
        config.mark_mirror_configured().unwrap();
        // An empty theme leaves `config.toml` without the key, as a fresh install has it.
        if !environment.theme.is_empty() {
            config.set("theme", environment.theme).unwrap();
        }
        let data = fixture.path("data");
        write_command_entry(
            &data,
            "alpha",
            "Alpha",
            "0123456789abcdef0123456789abcdef",
            true,
        );
        write_command_entry(
            &data,
            "beta",
            "Beta",
            "fedcba9876543210fedcba9876543210",
            false,
        );
        write_typed_entry(&data);
        write_prompt_entry(&data);
        write_python_entry(&data);
        FileStore::new(&data).rebuild_registry().unwrap();
        // A timestamp that is not RFC 3339 shows as written, so the census does not depend on
        // the clock.
        write_last_run(&fixture.path("state"), "alpha", 0);
        write_last_run(&fixture.path("state"), "beta", 2);
        // A run that recorded no exit status.
        fs::write(
            fixture.path("state").join("values").join("gamma.toml"),
            "[last_run]\nat = \"yesterday\"\n",
        )
        .unwrap();
        let home = fixture.path("home");
        fs::write(home.join("data.csv"), "a,b\n").unwrap();
        fs::write(
            home.join("hello.sh"),
            "#!/bin/sh\n# Greet someone.\nNAME=\"world\"\necho \"hello $NAME\"\n",
        )
        .unwrap();
        // Agent directories make the Agent Skill picker list real targets.
        fs::create_dir(home.join(".claude")).unwrap();
        fs::create_dir(home.join(".codex")).unwrap();
        fixture
    }

    fn path(&self, name: &str) -> PathBuf {
        self.root.path().join(name)
    }

    /// The fixture root as the child sees it, then as the operating system resolves it.
    ///
    /// macOS resolves `/tmp` to `/private/tmp`, so a path that the child reads from its working
    /// directory is longer there. The resolved form comes first, because it contains the other.
    fn root_spellings(&self) -> Vec<String> {
        let given = self.root.path().to_string_lossy().into_owned();
        let resolved = fs::canonicalize(self.root.path())
            .unwrap()
            .to_string_lossy()
            .into_owned();
        assert_eq!(
            given.chars().count(),
            ROOT_TOKEN.chars().count(),
            "the fixture root {given} does not match the census token length"
        );
        if resolved == given {
            vec![given]
        } else {
            vec![resolved, given]
        }
    }

    fn spawn(&self, environment: &Environment, screen: &Screen) -> PtyChild {
        let mut command = CommandBuilder::new(PathBuf::from(env!("CARGO_BIN_EXE_skit")));
        command.env_clear();
        command.cwd(screen.cwd.map_or_else(|| self.path("home"), PathBuf::from));
        // A fixed PATH that finds nothing keeps every health and agent check the same on every
        // host.
        command.env("PATH", "/nonexistent-skit-census-path");
        command.env("HOME", self.path("home"));
        command.env("TERM", "xterm-256color");
        command.env("SKIT_LANG", "en");
        command.env("SKIT_DATA_DIR", self.path("data"));
        command.env("SKIT_STATE_DIR", self.path("state"));
        command.env("SKIT_CONFIG_DIR", self.path("config"));
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
        let mut child = PtyChild::spawn(
            command,
            PtySize {
                rows: screen.rows,
                cols: screen.columns,
                pixel_width: 0,
                pixel_height: 0,
            },
            AnswerQueries::On,
        );
        child.set_terminal_colors(environment.colors);
        child
    }

    /// Open `screen` in a new session and return every byte the session wrote.
    ///
    /// The snapshot happens while the interface still runs, because `vt100` replays the alternate
    /// screen only until the session leaves it.
    fn capture(&self, environment: &Environment, screen: &Screen) -> Vec<u8> {
        let mut child = self.spawn(environment, screen);
        child.wait_cursor_query_after(0);
        wait_for_screen(&mut child, screen, LIBRARY_READY);
        for step in screen.steps {
            child.send(step.keys);
            if step.needle.is_empty() {
                child.settle();
            } else {
                wait_for_screen(&mut child, screen, step.needle);
            }
        }
        let raw = child.raw_after(0);
        quit(&mut child);
        raw
    }
}

/// End the session the way a person does: two Ctrl+C presses.
///
/// A screen that already shows the quit notice ends at the first press. A killed child never
/// writes its coverage profile, so the census would add no coverage.
fn quit(child: &mut PtyChild) {
    child.send(CTRL_C);
    child.settle();
    // The child may be gone already. A failed write then only confirms it.
    let _ = child.try_send(CTRL_C);
    let status = child.wait_exit_within(Duration::from_secs(10));
    assert!(
        status.success(),
        "the session did not end cleanly: {}",
        status.exit_code()
    );
}

/// Wait until the replayed screen shows `needle`, then until the terminal goes quiet.
///
/// The replay applies cursor moves. Ratatui skips a cell that did not change, so the raw bytes of
/// a word can be split even when the screen shows the whole word.
fn wait_for_screen(child: &mut PtyChild, screen: &Screen, needle: &str) {
    let deadline = Instant::now() + STEP_BUDGET;
    loop {
        child.settle();
        let raw = child.raw_after(0);
        let mut parser = vt100::Parser::new(screen.rows, screen.columns, 0);
        parser.process(&raw);
        let shown = parser.screen().contents();
        if shown.contains(needle) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "{}: the screen never showed {needle:?}:\n{shown}",
            screen.name
        );
    }
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
fn write_typed_entry(data: &Path) {
    let directory = data.join("scripts").join("gamma");
    fs::create_dir_all(&directory).unwrap();
    fs::write(
        directory.join("meta.toml"),
        concat!(
            "schema = 1\n",
            "name = \"Gamma\"\n",
            "kind = \"command\"\n",
            "mode = \"copy\"\n",
            "source = \"\"\n",
            "source_hash = \"\"\n",
            "added_at = \"2026-08-08T00:00:00Z\"\n",
            "id = \"00112233445566778899aabbccddeeff\"\n",
            "workdir = \"invoke\"\n",
            "description = \"Typed parameters.\"\n",
            "template = \"echo {count} {mode} {loud} {target} {token}\"\n",
            "params = [\"count\", \"mode\", \"loud\", \"target\", \"token\"]\n",
            "[[parameters]]\n",
            "name = \"count\"\n",
            "delivery = \"placeholder\"\n",
            "type = \"int\"\n",
            "default = 3\n",
            "help = \"How many times\"\n",
            "[[parameters]]\n",
            "name = \"mode\"\n",
            "delivery = \"placeholder\"\n",
            "type = \"choice\"\n",
            "choices = [\"fast\", \"slow\"]\n",
            "default = \"fast\"\n",
            "[[parameters]]\n",
            "name = \"loud\"\n",
            "delivery = \"placeholder\"\n",
            "type = \"bool\"\n",
            "default = false\n",
            "[[parameters]]\n",
            "name = \"target\"\n",
            "delivery = \"placeholder\"\n",
            "type = \"path\"\n",
            "[[parameters]]\n",
            "name = \"token\"\n",
            "delivery = \"placeholder\"\n",
            "type = \"str\"\n",
            "secret = true\n",
            "env_source = \"CENSUS_TOKEN\"\n",
        ),
    )
    .unwrap();
}
fn write_prompt_entry(data: &Path) {
    let directory = data.join("scripts").join("zeta");
    fs::create_dir_all(&directory).unwrap();
    fs::write(directory.join("prompt.md"), "Summarize the notes.\n").unwrap();
    fs::write(
        directory.join("meta.toml"),
        concat!(
            "schema = 1\n",
            "name = \"Zeta\"\n",
            "kind = \"prompt\"\n",
            "mode = \"copy\"\n",
            "source = \"prompt.md\"\n",
            "source_hash = \"\"\n",
            "added_at = \"2026-08-08T00:00:00Z\"\n",
            "id = \"ffeeddccbbaa99887766554433221100\"\n",
            "workdir = \"invoke\"\n",
            "description = \"A prompt.\"\n",
            "interpolate = true\n",
        ),
    )
    .unwrap();
}
/// A Python entry. The census PATH has no uv, so health and the detail pane report it.
fn write_python_entry(data: &Path) {
    let directory = data.join("scripts").join("eta");
    fs::create_dir_all(&directory).unwrap();
    fs::write(directory.join("eta.py"), "print(\"eta\")\n").unwrap();
    fs::write(
        directory.join("meta.toml"),
        concat!(
            "schema = 1\n",
            "name = \"Eta\"\n",
            "kind = \"python\"\n",
            "mode = \"copy\"\n",
            "source = \"eta.py\"\n",
            "source_hash = \"\"\n",
            "added_at = \"2026-08-08T00:00:00Z\"\n",
            "id = \"99887766554433221100ffeeddccbbaa\"\n",
            "workdir = \"invoke\"\n",
            "description = \"A Python script.\"\n",
            "params = []\n",
        ),
    )
    .unwrap();
}

fn write_last_run(state: &Path, slug: &str, exit: i64) {
    let directory = state.join("values");
    fs::create_dir_all(&directory).unwrap();
    fs::write(
        directory.join(format!("{slug}.toml")),
        format!("[last_run]\nat = \"yesterday\"\nexit = {exit}\n"),
    )
    .unwrap();
}

/// The screen that `raw` leaves on a terminal of the given size.
fn replay(raw: &[u8], rows: u16, columns: u16) -> vt100::Parser {
    let mut parser = vt100::Parser::new(rows, columns, 0);
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

/// One visible cell: its text and the style it shows.
#[derive(Clone, PartialEq)]
struct CellView {
    text: String,
    foreground: String,
    background: String,
    attributes: Vec<&'static str>,
}

impl CellView {
    fn same_style(&self, other: &Self) -> bool {
        self.foreground == other.foreground
            && self.background == other.background
            && self.attributes == other.attributes
    }
}

fn row_cells(screen: &vt100::Screen, row: u16) -> Vec<CellView> {
    let (_, columns) = screen.size();
    (0..columns)
        .map(|column| screen.cell(row, column).unwrap())
        .filter(|cell| !cell.is_wide_continuation())
        .map(|cell| CellView {
            text: if cell.has_contents() {
                cell.contents().to_owned()
            } else {
                " ".to_owned()
            },
            foreground: color_name(cell.fgcolor()),
            background: color_name(cell.bgcolor()),
            attributes: attributes(cell),
        })
        .collect()
}

/// Replace every `pattern` in a row with `token`, and keep the row width.
///
/// A longer pattern leaves fewer spaces before the next item on the row. The census returns those
/// spaces at the first space after the replaced word, where the host with the shorter path has
/// them.
fn replace_in_row(cells: &mut Vec<CellView>, pattern: &str, token: &str) {
    let pattern = pattern.chars().map(String::from).collect::<Vec<_>>();
    let token = token.chars().map(String::from).collect::<Vec<_>>();
    assert!(token.len() <= pattern.len());
    let mut start = 0;
    while start + pattern.len() <= cells.len() {
        let found = pattern
            .iter()
            .enumerate()
            .all(|(offset, text)| cells[start + offset].text == *text);
        if !found {
            start += 1;
            continue;
        }
        let style = cells[start].clone();
        let replacement = token
            .iter()
            .map(|text| CellView {
                text: text.clone(),
                ..style.clone()
            })
            .collect::<Vec<_>>();
        cells.splice(start..start + pattern.len(), replacement);
        let slack = pattern.len() - token.len();
        if slack > 0 {
            let gap = (start + token.len()..cells.len())
                .find(|&index| cells[index].text == " ")
                .unwrap_or(cells.len());
            let filler = cells.get(gap).cloned().unwrap_or(CellView {
                text: " ".to_owned(),
                ..style
            });
            for _ in 0..slack {
                cells.insert(gap, filler.clone());
            }
        }
        start += token.len();
    }
}

fn json_string(text: &str) -> String {
    serde_json::to_string(text).unwrap()
}

/// The census document for one screen: one JSON line per terminal row.
fn census(raw: &[u8], screen: &Screen, environment: &str, roots: &[String]) -> String {
    let parser = replay(raw, screen.rows, screen.columns);
    let mut out = String::new();
    writeln!(out, "{{").unwrap();
    writeln!(out, "  \"screen\": {},", json_string(screen.name)).unwrap();
    writeln!(out, "  \"environment\": {},", json_string(environment)).unwrap();
    writeln!(out, "  \"columns\": {},", screen.columns).unwrap();
    writeln!(out, "  \"rows\": [").unwrap();
    for row in 0..screen.rows {
        let mut cells = row_cells(parser.screen(), row);
        for root in roots {
            replace_in_row(&mut cells, root, ROOT_TOKEN);
        }
        let mut segments: Vec<CellView> = Vec::new();
        for cell in cells {
            match segments.last_mut() {
                Some(last) if last.same_style(&cell) => last.text.push_str(&cell.text),
                _ => segments.push(cell),
            }
        }
        let line = segments
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
        let separator = if row + 1 == screen.rows { "" } else { "," };
        writeln!(out, "    [{line}]{separator}").unwrap();
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
    let fixture = Fixture::new(environment);
    let roots = fixture.root_spellings();
    let failures = SCREENS
        .iter()
        .flat_map(|screen| {
            let raw = fixture.capture(environment, screen);
            let file = format!("{}.{}.json", screen.name, environment.name);
            let mut failures = Vec::new();
            if environment.theme == "terminal" {
                let parser = replay(&raw, screen.rows, screen.columns);
                failures.extend(terminal_rule_breaks(parser.screen(), screen.name));
            }
            failures.extend(check_census(
                &file,
                &census(&raw, screen, environment.name, &roots),
            ));
            failures
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

#[test]
fn census_on_a_basic_terminal() {
    run_census(&BASIC_TERM);
}

#[test]
fn census_terminal_theme_on_a_dark_terminal() {
    run_census(&TERMINAL_DARK);
}

#[test]
fn census_terminal_theme_on_a_light_terminal() {
    run_census(&TERMINAL_LIGHT);
}

#[test]
fn census_terminal_theme_with_an_unknown_background() {
    run_census(&TERMINAL_UNKNOWN);
}

#[test]
fn census_terminal_theme_with_no_color() {
    run_census(&TERMINAL_NO_COLOR);
}

/// The rules of the terminal theme (`docs/design/terminal-palette.md`), checked on every cell.
///
/// A site that bypasses the theme shows up here: the terminal theme uses only the default
/// colors, the 16 ANSI colors as hues, and reverse video for any background.
fn terminal_rule_breaks(screen: &vt100::Screen, name: &str) -> Vec<String> {
    let (rows, columns) = screen.size();
    let mut breaks = std::collections::BTreeSet::new();
    for row in 0..rows {
        for column in 0..columns {
            let cell = screen.cell(row, column).unwrap();
            let text = cell.contents();
            let foreground = cell.fgcolor();
            let background = cell.bgcolor();
            let at = format!("{name} r{row}c{column} {text:?}");
            for color in [foreground, background] {
                match color {
                    vt100::Color::Rgb(..) => {
                        breaks.insert(format!("{at}: 24-bit color {}", color_name(color)));
                    }
                    vt100::Color::Idx(index) if index >= 16 => {
                        breaks.insert(format!("{at}: 256-color index {index}"));
                    }
                    _ => {}
                }
            }
            if matches!(foreground, vt100::Color::Idx(0 | 7 | 8 | 15)) {
                breaks.insert(format!(
                    "{at}: black, white, or gray text {}",
                    color_name(foreground)
                ));
            }
            if background != vt100::Color::Default && !cell.inverse() {
                breaks.insert(format!("{at}: background without reverse"));
            }
            if foreground != vt100::Color::Default
                && !cell.inverse()
                && text.chars().any(char::is_alphanumeric)
            {
                breaks.insert(format!("{at}: a hue on readable text"));
            }
        }
    }
    breaks.into_iter().take(12).collect()
}

/// Every cell of the selected library row that shows `Alpha`, and every cell on the screen.
fn library_cells(environment: &Environment) -> (Vec<vt100::Cell>, Vec<vt100::Cell>) {
    let fixture = Fixture::new(environment);
    let library = &SCREENS[0];
    let raw = fixture.capture(environment, library);
    let parser = replay(&raw, library.rows, library.columns);
    let screen = parser.screen();
    let row = (0..library.rows)
        .find(|row| {
            screen
                .contents_between(*row, 0, *row, library.columns)
                .starts_with("│Alpha")
        })
        .expect("the library shows the Alpha row");
    let name = (1..=5)
        .map(|column| screen.cell(row, column).unwrap().clone())
        .collect();
    let all = (0..library.rows)
        .flat_map(|row| (0..library.columns).map(move |column| (row, column)))
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

/// The screen that `screen` leaves in `environment`.
fn shown(environment: &Environment, name: &str) -> vt100::Parser {
    let fixture = Fixture::new(environment);
    let screen = SCREENS
        .iter()
        .find(|screen| screen.name == name)
        .expect("a census screen with this name");
    let raw = fixture.capture(environment, screen);
    replay(&raw, screen.rows, screen.columns)
}

/// The cells that show the first occurrence of `needle` on the screen.
fn cells_of(parser: &vt100::Parser, needle: &str) -> Vec<vt100::Cell> {
    let screen = parser.screen();
    let (rows, columns) = screen.size();
    for row in 0..rows {
        let text = screen.contents_between(row, 0, row, columns);
        if let Some(byte) = text.find(needle) {
            let column = u16::try_from(text[..byte].chars().count()).unwrap();
            let width = u16::try_from(needle.chars().count()).unwrap();
            return (column..column + width)
                .map(|column| screen.cell(row, column).unwrap().clone())
                .collect();
        }
    }
    panic!(
        "the screen does not show {needle:?}:\n{}",
        screen.contents()
    );
}

fn describe(cells: &[vt100::Cell]) -> String {
    cells
        .iter()
        .map(|cell| {
            format!(
                "{}:{}/{}{}{}{}",
                cell.contents(),
                color_name(cell.fgcolor()),
                color_name(cell.bgcolor()),
                if cell.bold() { "/bold" } else { "" },
                if cell.dim() { "/dim" } else { "" },
                if cell.inverse() { "/inverse" } else { "" }
            )
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Version 0.4 draws body text in the terminal's default foreground (`theme.py`,
/// `foreground="ansi_default"`). The port drew it in bright white, which vanishes on a light
/// background.
#[test]
fn skit_theme_body_text_uses_the_default_foreground() {
    let run = shown(&TRUECOLOR, "run-form");
    let label = cells_of(&run, "Preset:");
    assert!(
        label
            .iter()
            .all(|cell| cell.fgcolor() == vt100::Color::Default),
        "run form label: {}",
        describe(&label)
    );
}

/// Version 0.4 draws hints with `[dim]` on the default foreground (`tui.py:538-552`). The port
/// drew them in bright black, which is the background color in Solarized Dark.
#[test]
fn skit_theme_hints_are_dim_default_text() {
    let run = shown(&TRUECOLOR, "run-form");
    let hint = cells_of(&run, "none yet");
    assert!(
        hint.iter()
            .all(|cell| cell.fgcolor() == vt100::Color::Default && cell.dim()),
        "run form hint: {}",
        describe(&hint)
    );
}

/// Version 0.4 paints scrollbars `#4A413C` (`theme.py:100`).
#[test]
fn skit_theme_scrollbars_use_the_version_0_4_color() {
    let settings = shown(&TRUECOLOR, "entry-settings");
    let screen = settings.screen();
    let (rows, columns) = screen.size();
    let bar = (0..rows)
        .map(|row| screen.cell(row, columns - 1).unwrap().clone())
        .filter(|cell| ["▲", "▼", "█", "║"].contains(&cell.contents()))
        .collect::<Vec<_>>();
    assert!(!bar.is_empty(), "the settings screen shows no scrollbar");
    assert!(
        bar.iter()
            .all(|cell| cell.fgcolor() == vt100::Color::Rgb(0x4a, 0x41, 0x3c)),
        "settings scrollbar: {}",
        describe(&bar)
    );
}

/// Version 0.4 widgets draw their labels in the default foreground too. An unselected radio
/// option and an unchecked box sit on the terminal background.
#[test]
fn skit_theme_widget_labels_use_the_default_foreground() {
    let run = shown(&TRUECOLOR, "run-typed");
    for needle in ["slow", "off"] {
        let label = cells_of(&run, needle);
        assert!(
            label
                .iter()
                .all(|cell| cell.fgcolor() == vt100::Color::Default),
            "{needle}: {}",
            describe(&label)
        );
    }
}

/// Every color that `screen` shows in `environment`, by name.
fn colors_on(environment: &Environment, name: &str) -> Vec<String> {
    let parser = shown(environment, name);
    let screen = parser.screen();
    let (rows, columns) = screen.size();
    (0..rows)
        .flat_map(|row| (0..columns).map(move |column| (row, column)))
        .flat_map(|(row, column)| {
            let cell = screen.cell(row, column).unwrap();
            [color_name(cell.fgcolor()), color_name(cell.bgcolor())]
        })
        .collect()
}

const DEPTH_SCREENS: [&str; 5] = [
    "library",
    "run-form",
    "entry-settings",
    "preferences",
    "remove",
];

/// Version 0.4 lets Rich pick the color system: with no `COLORTERM`, a `TERM` that ends in
/// `-256color` gives 256 colors (`console.py:789-811`), and Rich converts every 24-bit color.
#[test]
fn skit_theme_uses_256_colors_without_colorterm() {
    for name in DEPTH_SCREENS {
        let rgb = colors_on(&NO_COLORTERM, name)
            .into_iter()
            .filter(|color| color.starts_with('#'))
            .collect::<std::collections::BTreeSet<_>>();
        assert!(
            rgb.is_empty(),
            "{name}: 24-bit colors on a 256-color terminal: {rgb:?}"
        );
    }
}

/// A `TERM` with no color suffix gives Rich's 16-color system.
#[test]
fn skit_theme_uses_16_colors_on_a_basic_terminal() {
    for name in DEPTH_SCREENS {
        let wide = colors_on(&BASIC_TERM, name)
            .into_iter()
            .filter(|color| {
                color.starts_with('#')
                    || color
                        .strip_prefix("idx:")
                        .is_some_and(|index| index.parse::<u8>().unwrap() >= 16)
            })
            .collect::<std::collections::BTreeSet<_>>();
        assert!(
            wide.is_empty(),
            "{name}: colors beyond 16 on a basic terminal: {wide:?}"
        );
    }
}

/// The 256-color forms are Rich's own conversions (`color.py:512-568`): the accent `#D97757`
/// becomes 173, the selection `#EEEEEE` on `#5A2D1E` becomes 254 on 52.
#[test]
fn skit_theme_256_color_forms_match_rich() {
    let library = shown(&NO_COLORTERM, "library");
    let selected = cells_of(&library, "│Alpha")[1..].to_vec();
    assert!(
        selected.iter().all(|cell| {
            cell.fgcolor() == vt100::Color::Idx(254) && cell.bgcolor() == vt100::Color::Idx(52)
        }),
        "selected row: {}",
        describe(&selected)
    );
    let run = shown(&NO_COLORTERM, "run-form");
    let required = cells_of(&run, "required");
    assert!(
        required
            .iter()
            .all(|cell| cell.fgcolor() == vt100::Color::Idx(173)),
        "required mark: {}",
        describe(&required)
    );
}

fn all(cells: &[vt100::Cell], check: impl Fn(&vt100::Cell) -> bool) -> bool {
    !cells.is_empty() && cells.iter().all(check)
}

/// The terminal theme shows the selected row with reverse video and the default colors, with or
/// without `NO_COLOR`.
#[test]
fn terminal_theme_reverses_the_selected_row() {
    for environment in [&TERMINAL_UNKNOWN, &TERMINAL_NO_COLOR] {
        let library = shown(environment, "library");
        let row = cells_of(&library, "│Alpha")[1..].to_vec();
        assert!(
            all(&row, |cell| cell.inverse()
                && cell.fgcolor() == vt100::Color::Default),
            "{}: {}",
            environment.name,
            describe(&row)
        );
    }
}

/// A footer key is a reversed keycap in the accent: cyan on a dark background, magenta on a
/// light one, and the default foreground when the background is unknown. The label stays plain.
#[test]
fn terminal_theme_draws_footer_keys_as_keycaps() {
    for (environment, accent) in [
        (&TERMINAL_DARK, vt100::Color::Idx(6)),
        (&TERMINAL_LIGHT, vt100::Color::Idx(5)),
        (&TERMINAL_UNKNOWN, vt100::Color::Default),
    ] {
        let library = shown(environment, "library");
        let key = cells_of(&library, " Enter ");
        assert!(
            all(&key, |cell| cell.inverse()
                && cell.bold()
                && cell.fgcolor() == accent),
            "{} key: {}",
            environment.name,
            describe(&key)
        );
        let label = cells_of(&library, "Run    r")[..3].to_vec();
        assert!(
            all(&label, |cell| !cell.inverse()
                && !cell.bold()
                && cell.fgcolor() == vt100::Color::Default),
            "{} label: {}",
            environment.name,
            describe(&label)
        );
    }
}

/// The focused dialog button is reversed; the other button is plain.
#[test]
fn terminal_theme_reverses_the_focused_dialog_button() {
    let remove = shown(&TERMINAL_UNKNOWN, "remove");
    let keep = cells_of(&remove, "Keep");
    let delete = cells_of(&remove, "Remove ")[..6].to_vec();
    assert!(
        all(&keep, vt100::Cell::inverse),
        "Keep: {}",
        describe(&keep)
    );
    assert!(
        all(&delete, |cell| !cell.inverse()),
        "Remove: {}",
        describe(&delete)
    );
}

/// A status line colors only its glyph.
#[test]
fn terminal_theme_colors_only_the_status_glyph() {
    let health = shown(&TERMINAL_DARK, "health");
    let line = cells_of(&health, "✓ 5 entries");
    assert_eq!(
        line[0].fgcolor(),
        vt100::Color::Idx(2),
        "glyph: {}",
        describe(&line)
    );
    assert!(
        all(&line[2..], |cell| cell.fgcolor() == vt100::Color::Default),
        "text: {}",
        describe(&line)
    );
}

/// The selected option of an unfocused radio group is bold, not reversed.
#[test]
fn terminal_theme_marks_the_selected_radio_option_bold() {
    let run = shown(&TERMINAL_UNKNOWN, "run-typed");
    let fast = cells_of(&run, "fast");
    let slow = cells_of(&run, "slow");
    assert!(
        all(&fast, |cell| cell.bold() && !cell.inverse()),
        "fast: {}",
        describe(&fast)
    );
    assert!(all(&slow, |cell| !cell.bold()), "slow: {}", describe(&slow));
}

/// Idle borders are dim; the terminal theme gives them no color.
#[test]
fn terminal_theme_dims_idle_borders() {
    let library = shown(&TERMINAL_UNKNOWN, "library");
    let border = cells_of(&library, "╰──");
    assert!(
        all(&border, |cell| cell.dim()
            && cell.fgcolor() == vt100::Color::Default),
        "{}",
        describe(&border)
    );
}

/// Switching the theme in Preferences repaints the running session.
///
/// The session starts in the `skit` theme. Preferences picks `terminal`, the save writes it to
/// `config.toml`, and the next library frame already reverses the selected row.
#[test]
fn a_theme_saved_in_preferences_repaints_the_session() {
    const SWITCH: Screen = screen(
        "preferences-theme-switch",
        &[
            OPEN_PREFERENCES,
            // Shift+Tab from the first control wraps to the last one, the palette.
            step(b"\x1b[Z", ""),
            step(b"\x1b[D", ""),
            step(CTRL_S, "saved"),
        ],
    );
    let switch = &SWITCH;
    let fixture = Fixture::new(&TRUECOLOR);
    let raw = fixture.capture(&TRUECOLOR, switch);
    let parser = replay(&raw, switch.rows, switch.columns);
    let row = cells_of(&parser, "│Alpha")[1..].to_vec();
    assert!(
        all(&row, |cell| cell.inverse()
            && cell.fgcolor() == vt100::Color::Default),
        "selected row after the switch: {}",
        describe(&row)
    );
    let config = fs::read_to_string(fixture.path("config").join("config.toml")).unwrap();
    assert!(config.contains("theme = \"terminal\""), "{config}");
}

/// A title keeps only bold. The panel border under it is dim, and a title that also carried the
/// dim would be faint on a terminal that draws both.
///
/// `vt100` keeps only the last of bold and dim, so a title cell that the terminal received with
/// both shows here as dim.
#[test]
fn terminal_theme_titles_are_bold_and_never_dim() {
    for (name, title) in [
        ("library", "Library"),
        ("add-file-picker", "Source path"),
        ("add-review", "Add hello.sh"),
    ] {
        let parser = shown(&TERMINAL_UNKNOWN, name);
        let cells = cells_of(&parser, title);
        assert!(
            all(&cells, |cell| cell.bold() && !cell.dim()),
            "{name}: {}",
            describe(&cells)
        );
    }
}

/// An idle button shows brackets in its padding cells, so it reads as a button without a color.
/// The focused button is reversed instead, and the `skit` theme keeps its pills.
#[test]
fn terminal_theme_marks_idle_buttons_with_brackets() {
    for (name, labels) in [
        (
            "entry-settings-discard",
            &["[Discard]", "[Keep editing]"][..],
        ),
        (
            "run-path-suggestion",
            &["[▾ insert]", "[↺ default]", "[📁 browse]"],
        ),
        ("remove", &["[Remove]"]),
        ("preferences-agent-skill", &["[Cancel]"]),
    ] {
        let parser = shown(&TERMINAL_UNKNOWN, name);
        let text = parser.screen().contents();
        for label in labels {
            assert!(text.contains(label), "{name} lacks {label}:\n{text}");
        }
    }
    let skit = shown(&TRUECOLOR, "entry-settings-discard");
    assert!(!skit.screen().contents().contains("[Discard]"));
}

/// The focused dialog button reverses its label and its padding, and no cell after them.
#[test]
fn terminal_theme_reverses_only_the_focused_button() {
    let parser = shown(&TERMINAL_UNKNOWN, "remove");
    let cells = cells_of(&parser, " Keep   ");
    assert!(
        all(&cells[..6], vt100::Cell::inverse),
        "{}",
        describe(&cells)
    );
    assert!(
        all(&cells[6..], |cell| !cell.inverse()),
        "{}",
        describe(&cells)
    );
}

/// The keys of the agent rows in Preferences are keycaps, as the footer keys are.
#[test]
fn terminal_theme_draws_agent_row_keys_as_keycaps() {
    let parser = shown(&TERMINAL_UNKNOWN, "preferences-agents");
    for key in [" Enter ", " Del "] {
        let cells = cells_of(&parser, key);
        assert!(
            all(&cells, |cell| cell.inverse()),
            "{key}: {}",
            describe(&cells)
        );
    }
}

/// A footer label in a wide script keeps no bold. The keycap sizes its label in display cells,
/// not in characters.
#[test]
fn terminal_theme_footer_labels_stay_plain_in_a_wide_script() {
    const WIDE: Environment = Environment {
        name: "terminal-zh-tw",
        variables: &[("COLORTERM", "truecolor"), ("SKIT_LANG", "zh-TW")],
        theme: "terminal",
        colors: TerminalColors::Dark,
    };
    let label = "重新命名";
    let fixture = Fixture::new(&WIDE);
    let library = &SCREENS[0];
    let mut child = fixture.spawn(&WIDE, library);
    child.wait_cursor_query_after(0);
    wait_for_screen(&mut child, library, label);
    let raw = child.raw_after(0);
    quit(&mut child);
    let parser = replay(&raw, library.rows, library.columns);
    let screen = parser.screen();
    let (rows, columns) = screen.size();
    let row = (0..rows)
        .rev()
        .find(|row| {
            screen
                .contents_between(*row, 0, *row, columns)
                .contains(label)
        })
        .expect("the footer shows the label");
    // A wide character fills two columns; the second one has no contents.
    let shown = (0..columns)
        .filter_map(|column| screen.cell(row, column))
        .filter(|cell| !cell.contents().is_empty())
        .cloned()
        .collect::<Vec<_>>();
    let characters = label.chars().map(String::from).collect::<Vec<_>>();
    let start = shown
        .windows(characters.len())
        .position(|window| {
            window
                .iter()
                .zip(&characters)
                .all(|(cell, character)| cell.contents() == character)
        })
        .expect("the footer row shows the label in one piece");
    let cells = shown[start..start + characters.len()].to_vec();
    let bold = cells.iter().map(vt100::Cell::bold).collect::<Vec<_>>();
    assert_eq!(bold, [false; 4], "{}", describe(&cells));
}

/// An idle select is dim, as an idle input is. An overlay picker holds the focus, so its border
/// takes the accent, as a dialog border does.
#[test]
fn terminal_theme_dims_idle_selects_and_colors_overlay_borders() {
    let review = shown(&TERMINAL_UNKNOWN, "add-review");
    let corner = cells_of(&review, "┌ Storage mode");
    assert!(corner[0].dim(), "{}", describe(&corner));
    // A select that scrolled half out of view is drawn by hand, and it is dim too.
    let clipped = shown(&TERMINAL_UNKNOWN, "preferences-clipped-select");
    let edge = cells_of(&clipped, "└──");
    assert!(all(&edge, vt100::Cell::dim), "{}", describe(&edge));
    let skill = shown(&TERMINAL_DARK, "preferences-agent-skill");
    let edge = cells_of(&skill, "╭ Teach");
    assert!(
        edge[0].fgcolor() == vt100::Color::Idx(6) && !edge[0].dim(),
        "{}",
        describe(&edge)
    );
}

/// An environment filter that matches nothing shows an empty list, as version 0.4 does. The
/// list widget would otherwise draw an English "No items" in gray.
#[test]
fn an_empty_environment_filter_shows_an_empty_list() {
    for environment in [&TRUECOLOR, &TERMINAL_UNKNOWN] {
        let parser = shown(environment, "run-environment-empty");
        let text = parser.screen().contents();
        assert!(!text.contains("No items"), "{}:\n{text}", environment.name);
    }
}

/// Switching back to the `skit` theme repaints the session with the fixed palette.
///
/// The session starts in the terminal theme. Preferences picks `skit`, the save writes it, and
/// the next library frame draws the selected row on the fixed selection color, not reversed.
#[test]
fn switching_to_the_skit_theme_repaints_the_session() {
    const SWITCH: Screen = screen(
        "preferences-theme-switch-back",
        &[
            OPEN_PREFERENCES,
            // Shift+Tab from the first control wraps to the last one, the palette.
            step(b"\x1b[Z", ""),
            step(b"\x1b[C", ""),
            step(CTRL_S, "saved"),
        ],
    );
    let switch = &SWITCH;
    let fixture = Fixture::new(&TERMINAL_DARK);
    let raw = fixture.capture(&TERMINAL_DARK, switch);
    let parser = replay(&raw, switch.rows, switch.columns);
    let row = cells_of(&parser, "│Alpha")[1..].to_vec();
    assert!(
        all(&row, |cell| !cell.inverse()
            && matches!(cell.bgcolor(), vt100::Color::Rgb(..))),
        "selected row after the switch: {}",
        describe(&row)
    );
    let config = fs::read_to_string(fixture.path("config").join("config.toml")).unwrap();
    assert!(config.contains("theme = \"skit\""), "{config}");
}

/// A `theme` that a person typed into `config.toml` by hand, and that skit does not know, gets
/// the default: the interface and the Preferences choice both show the terminal theme.
#[test]
fn an_unknown_theme_in_the_file_falls_back_to_the_terminal_theme() {
    const UNKNOWN: Environment = Environment {
        name: "unknown-theme",
        variables: &[("COLORTERM", "truecolor")],
        theme: "",
        colors: TerminalColors::Dark,
    };
    let fixture = Fixture::new(&UNKNOWN);
    let file = fixture.path("config").join("config.toml");
    let written = fs::read_to_string(&file).unwrap();
    fs::write(&file, format!("theme = \"neon\"\n{written}")).unwrap();

    // Shift+Tab from the first control wraps to the last one, the palette.
    const PALETTE: Screen = screen(
        "preferences-palette",
        &[OPEN_PREFERENCES, step(b"\x1b[Z", "Colors")],
    );
    let raw = fixture.capture(&UNKNOWN, &PALETTE);
    let parser = replay(&raw, PALETTE.rows, PALETTE.columns);
    let shown = parser.screen().contents();
    assert!(
        shown.contains("◉  Terminal colors"),
        "the palette choice:\n{shown}"
    );
    let library = &SCREENS[0];
    let raw = fixture.capture(&UNKNOWN, library);
    let parser = replay(&raw, library.rows, library.columns);
    let row = cells_of(&parser, "│Alpha")[1..].to_vec();
    assert!(
        all(&row, |cell| cell.inverse()
            && cell.fgcolor() == vt100::Color::Default),
        "selected row with an unknown theme: {}",
        describe(&row)
    );
    assert_eq!(
        fs::read_to_string(&file).unwrap(),
        format!("theme = \"neon\"\n{written}"),
        "opening the interface changed config.toml"
    );
}

/// Keys typed before skit asks for the colors stay for the interface, and skit does not ask.
///
/// The census writes the keys at once after the start, before the child has read its
/// configuration. A question would read them as a broken answer, and the library would keep its
/// first row selected.
#[test]
fn keys_typed_ahead_skip_the_color_question() {
    let fixture = Fixture::new(&TERMINAL_DARK);
    let library = &SCREENS[0];
    let mut child = fixture.spawn(&TERMINAL_DARK, library);
    child.write_raw(TO_BETA);
    child.wait_cursor_query_after(0);
    wait_for_screen(&mut child, library, "failed");
    let raw = child.raw_after(0);
    quit(&mut child);
    assert!(
        !raw.windows(b"\x1b]11;?".len())
            .any(|window| window == b"\x1b]11;?"),
        "skit asked for the colors with keys waiting"
    );
    let parser = replay(&raw, library.rows, library.columns);
    let row = cells_of(&parser, "│Beta")[1..].to_vec();
    assert!(
        all(&row, |cell| cell.inverse()),
        "the typed-ahead key did not select Beta: {}",
        describe(&row)
    );
}

/// A `config.toml` with no `theme` gets the terminal theme.
#[test]
fn the_terminal_theme_is_the_default() {
    const DEFAULT: Environment = Environment {
        name: "default-theme",
        variables: &[("COLORTERM", "truecolor")],
        theme: "",
        colors: TerminalColors::Unknown,
    };
    let fixture = Fixture::new(&DEFAULT);
    let library = &SCREENS[0];
    let raw = fixture.capture(&DEFAULT, library);
    let parser = replay(&raw, library.rows, library.columns);
    let row = cells_of(&parser, "│Alpha")[1..].to_vec();
    assert!(
        all(&row, |cell| cell.inverse()
            && cell.fgcolor() == vt100::Color::Default),
        "selected row with the default theme: {}",
        describe(&row)
    );
}

/// A terminal that answers the color questions after skit stopped waiting must not type keys.
///
/// crossterm reads a late `ESC ] 11 ; rgb:…` as Alt+`]` and then one key per character, and the
/// library binds `r` (Rerun) and `/` (Search). The library must look the same after the late
/// answers arrive, and no run may start.
#[test]
fn a_late_color_answer_never_becomes_keys() {
    const LATE: Environment = Environment {
        name: "terminal-late",
        variables: &[("COLORTERM", "truecolor")],
        theme: "terminal",
        colors: TerminalColors::LateDark,
    };
    let fixture = Fixture::new(&LATE);
    let state = fixture.path("state").join("values").join("alpha.toml");
    let before = fs::read(&state).unwrap();
    let library = &SCREENS[0];
    let mut child = fixture.spawn(&LATE, library);
    child.wait_cursor_query_after(0);
    wait_for_screen(&mut child, library, LIBRARY_READY);
    let deadline = Instant::now() + Duration::from_secs(10);
    while !child.late_color_answers_sent() {
        assert!(Instant::now() < deadline, "the late answers never went out");
        // A checkpoint drains the output, which is where the harness answers.
        let _ = child.checkpoint();
        std::thread::sleep(Duration::from_millis(20));
    }
    child.settle_quiet(Duration::from_millis(500));
    let raw = child.raw_after(0);
    let mut parser = vt100::Parser::new(library.rows, library.columns, 0);
    parser.process(&raw);
    let shown = parser.screen().contents();
    assert!(
        parser.screen().alternate_screen(),
        "the session left the interface, as a run does:\n{shown}"
    );
    assert!(!shown.contains("Back to list"), "search opened:\n{shown}");
    assert!(
        shown.contains("2/5 entries") || shown.contains("entries"),
        "{shown}"
    );
    assert_eq!(
        fs::read(&state).unwrap(),
        before,
        "a run changed the state of Alpha"
    );
    quit(&mut child);
}

/// Over a remote login skit does not ask for the terminal colors: answers from a slow link can
/// arrive after any timeout. The terminal theme then has no accent hue.
#[test]
fn a_remote_login_skips_the_color_question() {
    const REMOTE: Environment = Environment {
        name: "terminal-remote",
        variables: &[
            ("COLORTERM", "truecolor"),
            ("SSH_CONNECTION", "192.0.2.1 50000 192.0.2.2 22"),
        ],
        theme: "terminal",
        colors: TerminalColors::Dark,
    };
    let fixture = Fixture::new(&REMOTE);
    let library = &SCREENS[0];
    let raw = fixture.capture(&REMOTE, library);
    assert!(
        !raw.windows(b"\x1b]11;?".len())
            .any(|window| window == b"\x1b]11;?"),
        "skit asked for the background over a remote login"
    );
    let parser = replay(&raw, library.rows, library.columns);
    let key = cells_of(&parser, " Enter ");
    assert!(
        all(&key, |cell| cell.inverse()
            && cell.fgcolor() == vt100::Color::Default),
        "the keycap took a hue: {}",
        describe(&key)
    );
}
