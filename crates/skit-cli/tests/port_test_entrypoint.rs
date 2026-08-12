//! Executable public-surface ports of Python `tests/test_entrypoint.py` at `main@206f9ef`.
//!
//! Python has a runtime import-dispatch seam that Rust cannot reproduce: its console entry point can
//! inspect `sys.modules`, monkeypatch `entry.main`, and be invoked with `python -m skit`. Those exact
//! contracts are kept architecture-closed in the companion manifest rather than impersonated here.
//! The tests below preserve every observable contract that does have a Rust equivalent: plain version
//! output, real command fallthrough, bare-command TUI dispatch, identical fast/slow version text,
//! installed binary wiring, and malformed trailing argv refusing instead of being swallowed.

use std::{
    fs,
    io::{Read as _, Write as _},
    path::{Path, PathBuf},
    thread,
    time::Duration,
};

use assert_cmd::Command;
use portable_pty::{CommandBuilder, PtySize, native_pty_system};
use syn::{Expr, Item, Stmt};
use tempfile::TempDir;

struct Sandbox {
    _root: TempDir,
    data: PathBuf,
    state: PathBuf,
    config: PathBuf,
    home: PathBuf,
}

impl Sandbox {
    fn new() -> Self {
        let root = TempDir::new().unwrap();
        let data = root.path().join("data");
        let state = root.path().join("state");
        let config = root.path().join("config");
        let home = root.path().join("home");
        for path in [&data, &state, &config, &home] {
            fs::create_dir_all(path).unwrap();
        }
        // Bare `skit` is allowed to offer first-run mirror setup. Mark that axis configured so this
        // entrypoint test observes TUI dispatch rather than depending on a network probe.
        fs::write(
            config.join("config.toml"),
            "[mirror]\nenabled = false\n",
        )
        .unwrap();
        Self {
            _root: root,
            data,
            state,
            config,
            home,
        }
    }

    fn command(&self) -> Command {
        let mut command = assert_cmd::cargo::cargo_bin_cmd!("skit");
        self.apply_env(&mut command);
        command
    }

    fn apply_env(&self, command: &mut Command) {
        command
            .env("SKIT_DATA_DIR", &self.data)
            .env("SKIT_STATE_DIR", &self.state)
            .env("SKIT_CONFIG_DIR", &self.config)
            .env("SKIT_LANG", "en")
            .env("HOME", &self.home)
            .env("USERPROFILE", &self.home)
            .env("XDG_CONFIG_HOME", self.home.join("xdg-config"))
            .env("XDG_DATA_HOME", self.home.join("xdg-data"))
            .env("XDG_STATE_HOME", self.home.join("xdg-state"))
            .current_dir(&self.home);
    }

    fn apply_pty_env(&self, command: &mut CommandBuilder) {
        command.cwd(&self.home);
        command.env("TERM", "xterm-256color");
        command.env("SKIT_DATA_DIR", &self.data);
        command.env("SKIT_STATE_DIR", &self.state);
        command.env("SKIT_CONFIG_DIR", &self.config);
        command.env("SKIT_LANG", "en");
        command.env("HOME", &self.home);
        command.env("USERPROFILE", &self.home);
        command.env("XDG_CONFIG_HOME", self.home.join("xdg-config"));
        command.env("XDG_DATA_HOME", self.home.join("xdg-data"));
        command.env("XDG_STATE_HOME", self.home.join("xdg-state"));
    }
}

fn version_line() -> String {
    format!("skit {}\n", env!("CARGO_PKG_VERSION"))
}

fn combined(output: &std::process::Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

#[test]
fn test_version_is_plain_text_not_rich_markup() {
    let sandbox = Sandbox::new();
    for flag in ["--version", "-V"] {
        let output = sandbox.command().arg(flag).output().unwrap();
        assert!(output.status.success(), "{}", combined(&output));
        assert_eq!(output.stdout, version_line().as_bytes());
        assert!(output.stderr.is_empty(), "{}", combined(&output));
        assert!(
            !output.stdout.contains(&0x1b),
            "version output contained ANSI escapes"
        );
    }
}

#[test]
fn test_a_real_command_still_reaches_the_cli() {
    let sandbox = Sandbox::new();
    let output = sandbox
        .command()
        .args(["list", "--json"])
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", combined(&output));
    assert_ne!(output.stdout.as_slice(), version_line().as_bytes());
    let _: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "list --json did not execute the real CLI: {error}; {}",
            combined(&output)
        )
    });
}

#[test]
fn test_no_arguments_reaches_the_cli() {
    let sandbox = Sandbox::new();
    let pair = native_pty_system()
        .openpty(PtySize {
            rows: 28,
            cols: 110,
            pixel_width: 0,
            pixel_height: 0,
        })
        .unwrap();
    let mut command = CommandBuilder::new(PathBuf::from(env!("CARGO_BIN_EXE_skit")));
    sandbox.apply_pty_env(&mut command);
    let mut child = pair.slave.spawn_command(command).unwrap();
    drop(pair.slave);
    let mut reader = pair.master.try_clone_reader().unwrap();
    let drain = thread::spawn(move || {
        let mut bytes = Vec::new();
        reader.read_to_end(&mut bytes).unwrap();
        bytes
    });
    let mut writer = pair.master.take_writer().unwrap();
    thread::sleep(Duration::from_millis(60));
    writer.write_all(b"\x1b[1;1R").unwrap();
    writer.flush().unwrap();
    thread::sleep(Duration::from_millis(220));
    writer.write_all(b"q").unwrap();
    writer.flush().unwrap();
    let status = child.wait().unwrap();
    drop(writer);
    let transcript = drain.join().unwrap();

    assert_eq!(
        status.exit_code(),
        0,
        "{}",
        String::from_utf8_lossy(&transcript)
    );
    assert!(
        transcript
            .windows(b"\x1b[?1049h".len())
            .any(|window| window == b"\x1b[?1049h"),
        "bare skit never entered the TUI alternate screen: {}",
        String::from_utf8_lossy(&transcript)
    );
}

#[test]
fn test_both_version_paths_print_the_identical_line() {
    let sandbox = Sandbox::new();
    let fast = sandbox.command().arg("--version").output().unwrap();
    let callback = sandbox
        .command()
        .args(["--version", "list"])
        .output()
        .unwrap();

    assert!(fast.status.success(), "{}", combined(&fast));
    assert!(callback.status.success(), "{}", combined(&callback));
    assert_eq!(fast.stdout, version_line().as_bytes());
    assert_eq!(callback.stdout, fast.stdout);
    assert_eq!(callback.stderr, fast.stderr);
}

fn path_is(path: &syn::Path, segments: &[&str]) -> bool {
    path.segments
        .iter()
        .map(|segment| segment.ident.to_string())
        .eq(segments.iter().copied())
}

fn called_path(expr: &Expr) -> Option<&syn::Path> {
    let Expr::Path(path) = expr else {
        return None;
    };
    Some(&path.path)
}

#[test]
fn test_the_console_script_points_at_the_dispatcher() {
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let manifest: toml::Value = fs::read_to_string(manifest_dir.join("Cargo.toml"))
        .unwrap()
        .parse()
        .unwrap();
    let bins = manifest
        .get("bin")
        .and_then(toml::Value::as_array)
        .expect("the CLI package declares [[bin]]");
    assert_eq!(
        bins.len(),
        1,
        "the installed skit command must have one canonical binary entry"
    );
    assert_eq!(
        bins[0].get("name").and_then(toml::Value::as_str),
        Some("skit")
    );
    assert_eq!(
        bins[0].get("path").and_then(toml::Value::as_str),
        Some("src/main.rs")
    );

    let source = fs::read_to_string(manifest_dir.join("src/main.rs")).unwrap();
    let syntax = syn::parse_file(&source).unwrap();
    let [Item::Fn(main)] = syntax.items.as_slice() else {
        panic!("src/main.rs must contain only the binary main function");
    };
    assert_eq!(main.sig.ident, "main");
    let [Stmt::Expr(Expr::Call(exit_call), Some(_))] = main.block.stmts.as_slice() else {
        panic!("main must directly exit with the composition-root entry result");
    };
    assert!(
        called_path(&exit_call.func)
            .is_some_and(|path| path_is(path, &["std", "process", "exit"])),
        "main's only call must be std::process::exit"
    );
    assert_eq!(exit_call.args.len(), 1);
    let Expr::Call(entry_call) = exit_call.args.first().unwrap() else {
        panic!("std::process::exit must receive exactly the skit entry call");
    };
    assert!(entry_call.args.is_empty());
    assert!(
        called_path(&entry_call.func)
            .is_some_and(|path| path_is(path, &["skit_cli", "entry"])),
        "the installed binary must dispatch through skit_cli::entry"
    );
}

#[test]
fn test_a_bad_invocation_still_fails_through_the_dispatcher() {
    let sandbox = Sandbox::new();
    for argv in [
        ["--version", "foo"].as_slice(),
        ["-V", "bar", "baz"].as_slice(),
    ] {
        let output = sandbox.command().args(argv).output().unwrap();
        assert!(
            !output.status.success(),
            "bad argv unexpectedly succeeded: {argv:?}; {}",
            combined(&output)
        );
        assert_ne!(
            output.stdout.as_slice(),
            version_line().as_bytes(),
            "bad argv was swallowed by the version path"
        );
        assert!(!String::from_utf8_lossy(&output.stdout).starts_with("skit "));
    }
}
