//! Mechanical port of the Python oracle module `tests/test_entrypoint.py`
//! (`origin/main@206f9ef`): the console-script dispatcher.
//!
//! The oracle module exists for a Python-only reason: `skit.cli` is a Typer app whose
//! decorators run at import time, so `src/skit/__main__.py` answers `--version` ahead of
//! that import to avoid loading ~230 modules for a machine-facing flag. A compiled Rust
//! binary has no import cost, so the Rust rewrite drops the two-tier dispatcher: `--version`
//! and `-V` are a plain Clap bool flag handled inside `execute()`
//! (`crates/skit-cli/src/cli.rs:721-724`: `if cli.version { println!("skit {}", ...); return
//! Ok(0) }`).
//!
//! Concept mapping used throughout:
//! - Python `skit.__main__:main` + `skit.cli:app` -> the single `skit` binary built from
//!   `crates/skit-cli/src/main.rs`, which calls `skit_cli::entry()`.
//! - Python `_run(argv)` (fresh interpreter, report of loaded modules) -> a subprocess run of
//!   the real `skit` binary via `assert_cmd`; the module-import graph it reports has no
//!   observable equivalent in a compiled binary.
//! - Python `skit.__version__` -> `env!("CARGO_PKG_VERSION")` (the test crate is the same
//!   package `skit-cli-rs`, so this equals the binary's version).
//!
//! Buckets:
//! - Real asserting tests (8 def -> 9 `#[test]`): the observable half of every version-flag
//!   contract -- exact stdout line, plain text, exit code, the `--version`-vs-callback byte
//!   equality, and "a real command reaches the CLI". The Python "did not import typer/rich/
//!   textual/tree_sitter" assertions have no equivalent in a monolithic binary; the version
//!   line and exit code are what a shell or agent observes, and those are asserted.
//! - Divergence gap (1 `#[test]`, `#[ignore]`): `skit --install-completion --version` prints
//!   the version and drops the install on the Rust side, opposite to the oracle's
//!   eager-option-first ordering. Full failing body kept per protocol.
//! - UNMAPPED (1 `#[test]`, `#[ignore]`): `python -m skit` has no module-execution form for a
//!   compiled binary; the single-binary `--version` contract is covered elsewhere. Not a gap
//!   -- no analog exists by design.

use std::fs;

use tempfile::TempDir;

/// The three skit directories plus `SKIT_LANG=en`, so every `skit` invocation writes only
/// inside the temp sandbox and speaks the English source locale.
struct Sandbox {
    data: TempDir,
    state: TempDir,
    config: TempDir,
}

impl Sandbox {
    fn new() -> Self {
        Self {
            data: TempDir::new().unwrap(),
            state: TempDir::new().unwrap(),
            config: TempDir::new().unwrap(),
        }
    }

    fn command(&self) -> assert_cmd::Command {
        let mut command = assert_cmd::cargo::cargo_bin_cmd!("skit");
        command
            .env("SKIT_DATA_DIR", self.data.path())
            .env("SKIT_STATE_DIR", self.state.path())
            .env("SKIT_CONFIG_DIR", self.config.path())
            .env("SKIT_LANG", "en");
        command
    }

    /// Run `skit <args>` and return `(exit code, stdout, stderr)`.
    fn run(&self, args: &[&str]) -> (Option<i32>, String, String) {
        let output = self.command().args(args).output().unwrap();
        (
            output.status.code(),
            String::from_utf8_lossy(&output.stdout).into_owned(),
            String::from_utf8_lossy(&output.stderr).into_owned(),
        )
    }
}

/// The one version line both paths print: `skit <version>\n`.
fn version_line() -> String {
    format!("skit {}\n", env!("CARGO_PKG_VERSION"))
}

#[test]
fn test_version_flag_is_answered_without_building_the_cli() {
    // Both spellings answer with the plain version line and exit 0. The oracle also asserts
    // that neither spelling imports typer, rich, textual or tree_sitter -- a lazy-import fact
    // with no observable equivalent in a monolithic Rust binary. The observable contract (the
    // exact line and exit 0) is what an agent parses, and it is asserted here for both flags.
    for flag in ["--version", "-V"] {
        let sandbox = Sandbox::new();
        let (code, stdout, _stderr) = sandbox.run(&[flag]);
        assert_eq!(code, Some(0), "flag {flag}");
        assert_eq!(stdout, version_line(), "flag {flag}");
    }
}

#[test]
fn test_version_is_plain_text_not_rich_markup() {
    // `--version` is a machine-facing answer, so it is printed plainly through `println!`, not a
    // rich Console that would split a PEP 440 version into colored fragments. No escape
    // sequences, whatever the terminal.
    let sandbox = Sandbox::new();
    let (_code, stdout, _stderr) = sandbox.run(&["--version"]);
    assert!(!stdout.contains('\x1b'), "{stdout:?}");
}

#[test]
fn test_version_flag_answers_in_process_too() {
    // The oracle's in-process twin proves the fast path PRINTS and RETURNS rather than falling
    // through to the CLI callback (which opens the TUI). Rust cannot rewrite process argv in
    // process, so the subprocess is the faithful realization: a clean exit 0 with the version on
    // stdout and an EMPTY stderr proves the version branch returned before the TUI. Falling
    // through to the TUI on a non-terminal would instead exit non-zero with a terminal error on
    // stderr (see test_no_arguments_reaches_the_cli).
    for flag in ["--version", "-V"] {
        let sandbox = Sandbox::new();
        let (code, stdout, stderr) = sandbox.run(&[flag]);
        assert_eq!(code, Some(0), "flag {flag}");
        assert_eq!(stdout, version_line(), "flag {flag}");
        assert_eq!(stderr, "", "flag {flag}");
    }
}

#[test]
fn test_a_real_command_still_reaches_the_cli() {
    // Everything that is not the leading version flag falls through to the CLI. `skit list`
    // reaches the list command (empty-library message) and is NOT shortcut into the version
    // line.
    let sandbox = Sandbox::new();
    let (code, stdout, _stderr) = sandbox.run(&["list"]);
    assert_eq!(code, Some(0));
    assert!(stdout.contains("No entries yet"), "{stdout:?}");
    assert_ne!(stdout, version_line());
}

#[test]
fn test_no_arguments_reaches_the_cli() {
    // A bare `skit` opens the TUI, which is the CLI callback's job -- the version flag must not
    // shortcut it. On a non-terminal the TUI cannot initialize, so it fails with skit's own
    // "terminal I/O failed" message; nothing goes to stdout and no version line is printed. That
    // failure IS the proof it reached the TUI path rather than a silent version-exit-0.
    let sandbox = Sandbox::new();
    let (code, stdout, stderr) = sandbox.run(&[]);
    assert_ne!(code, Some(0));
    assert_eq!(stdout, "");
    assert!(stderr.contains("terminal I/O failed"), "{stderr:?}");
}

#[test]
fn test_the_flag_is_claimed_only_as_the_whole_command_line() {
    // `skit --version` (nothing else) is the one invocation that answers as the bare version
    // line. A dispatcher that claimed a LEADING flag and ignored the rest of argv would turn
    // these into a silent exit 0. The oracle pins the invariant, not Typer's exact wording:
    //   - `list --version`, `--version foo`, `-V bar baz`  -> NOT a silent version-exit-0.
    //   - `--version list`                                 -> prints the version, exit 0
    //     (oracle: "prints the version through the callback"; here the version branch answers).
    // The eager `--install-completion --version` case is split out below because Rust diverges.
    let sandbox = Sandbox::new();
    let cases: [(&[&str], bool); 4] = [
        (&["list", "--version"], false),
        (&["--version", "foo"], false),
        (&["-V", "bar", "baz"], false),
        (&["--version", "list"], true),
    ];
    for (argv, should_be_version) in cases {
        let (code, stdout, _stderr) = sandbox.run(argv);
        if should_be_version {
            assert_eq!(code, Some(0), "{argv:?}");
            assert_eq!(stdout, version_line(), "{argv:?}");
        } else {
            let silent_version = code == Some(0) && stdout == version_line();
            assert!(!silent_version, "{argv:?} was silently claimed as version");
        }
    }
}

#[test]
#[ignore = "FAILING CONTRACT (divergence): oracle __main__.py:13-15 and test_entrypoint.py:118-141 -- `skit --install-completion --version` exits on the eager --install-completion option FIRST (installs completion), it does not print the version. Rust cli.rs:721-729 checks cli.version before install_completion, so it prints the version and silently drops the install. Fix ordering, then delete this #[ignore]."]
fn test_the_flag_is_claimed_only_as_the_whole_command_line_install_completion() {
    // Split from the parametrized oracle case above. HOME/XDG are sandboxed too: once the impl
    // is fixed and this #[ignore] is deleted, the invocation actually installs completion, and
    // `completion_path()` resolves under HOME/XDG_DATA_HOME, outside the SKIT_* dirs.
    let sandbox = Sandbox::new();
    let home = TempDir::new().unwrap();
    let xdg_data = TempDir::new().unwrap();
    let output = sandbox
        .command()
        .env("SHELL", "/bin/bash")
        .env_remove("PSModulePath")
        .env("HOME", home.path())
        .env("XDG_DATA_HOME", xdg_data.path())
        .args(["--install-completion", "--version"])
        .output()
        .unwrap();
    let code = output.status.code();
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let silent_version = code == Some(0) && stdout == version_line();
    assert!(
        !silent_version,
        "the eager --install-completion must run first, not print the version"
    );
}

#[test]
fn test_both_version_paths_print_the_identical_line() {
    // The fast path (`skit --version`) and the callback path (`skit --version list`) share one
    // version line. An agent parsing the output must get one answer whatever the argv shape;
    // this asserts byte equality so a second hand-synced format string reappearing in either
    // place fails the day it drifts. (In the Rust rewrite both go through the same `cli.version`
    // branch, so they are identical by construction; the byte-equality assertion still guards a
    // future re-split.)
    let sandbox = Sandbox::new();
    let (fast_code, fast_path, _e1) = sandbox.run(&["--version"]);
    let (slow_code, slow_path, _e2) = sandbox.run(&["--version", "list"]);
    assert_eq!(fast_code, Some(0));
    assert_eq!(slow_code, Some(0));
    assert_eq!(slow_path, fast_path);
    assert_eq!(fast_path, version_line());
}

#[test]
fn test_the_console_script_points_at_the_dispatcher() {
    // Oracle: `pyproject["project"]["scripts"] == {"skit": "skit.__main__:main"}` -- the installed
    // `skit` command must point at the thin dispatcher, not back at the heavy CLI. The Rust
    // analog is the packaging surface that names the installed command: the Cargo `[[bin]]` named
    // `skit` (built from `src/main.rs`, which calls `skit_cli::entry()`) and the maturin bin
    // bindings that ship it. If either regresses, `uv tool install skit-cli` stops producing the
    // `skit` binary that answers `--version`.
    let manifest_dir = env!("CARGO_MANIFEST_DIR");

    let cargo_toml: toml::Table = fs::read_to_string(format!("{manifest_dir}/Cargo.toml"))
        .unwrap()
        .parse()
        .unwrap();
    let bins = cargo_toml["bin"].as_array().expect("[[bin]] targets");
    let skit_bin = bins
        .iter()
        .find(|bin| bin.get("name").and_then(toml::Value::as_str) == Some("skit"))
        .expect("a [[bin]] named skit");
    assert_eq!(skit_bin["path"].as_str(), Some("src/main.rs"));

    let pyproject: toml::Table = fs::read_to_string(format!("{manifest_dir}/../../pyproject.toml"))
        .unwrap()
        .parse()
        .unwrap();
    assert_eq!(
        pyproject["build-system"]["build-backend"].as_str(),
        Some("maturin")
    );
    let maturin = &pyproject["tool"]["maturin"];
    assert_eq!(maturin["bindings"].as_str(), Some("bin"));
    assert_eq!(
        maturin["manifest-path"].as_str(),
        Some("crates/skit-cli/Cargo.toml")
    );
}

#[test]
#[ignore = "UNMAPPED: `python -m skit` (module execution) has no equivalent for a compiled binary; there is exactly one `skit` entry. The single-binary `--version` contract is covered by test_version_flag_is_answered_without_building_the_cli and the packaging surface by test_the_console_script_points_at_the_dispatcher. Not a gap -- no analog exists by design."]
fn test_python_dash_m_skit_is_the_same_entry() {
    // Oracle: `python -m skit --version` and the console script run the same dispatcher and print
    // the same line. A Rust binary has no `-m` module-execution form to compare against, so this
    // stub records the intent without a false-alarm assertion.
}

#[test]
fn test_a_bad_invocation_still_fails_through_the_dispatcher() {
    // The behavioral half of the whole-command-line test: these command lines exited non-zero
    // before the dispatcher existed, and must still. Run end to end so the assertion is about
    // what a user's shell sees -- a non-zero exit and no `skit ` line on stdout -- not which
    // function was called. (Oracle asserts the exact wording "No such command" only in prose;
    // the value it pins is the non-zero exit with no version leaking to stdout.)
    let sandbox = Sandbox::new();
    for argv in [vec!["--version", "foo"], vec!["-V", "bar", "baz"]] {
        let (code, stdout, _stderr) = sandbox.run(&argv);
        // Oracle: `code not in (None, 0)` -- a signal death (None) fails too, not just exit 0.
        assert!(matches!(code, Some(c) if c != 0), "{argv:?}: {code:?}");
        assert!(!stdout.contains("skit "), "{argv:?}: {stdout:?}");
    }
}
