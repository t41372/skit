//! Mechanical port of the Python oracle module `tests/test_launcher_fix.py`
//! (`origin/main@206f9ef`): "Launcher regressions around template substitution, exit codes,
//! uv ordering, and quoting." Each `#[test]` keeps its Python `def test_*` name so it traces
//! back to its origin, and each Python "WHY" comment is preserved above it.
//!
//! CRATE HINT: the hint said `skit-runtime`, and it is correct — the launcher surface lives in
//! `crates/skit-runtime/src/launch.rs`. The whole file is POSIX-only (`#![cfg(not(windows))]`),
//! mirroring the oracle's per-test `skipif sys.platform == "win32"` guards and matching the
//! sibling `port_test_template_context_quoting.rs`.
//!
//! Concept mapping used throughout:
//! - Python `launcher.build_command(entry, values=…)` for a COMMAND entry returns the rendered
//!   shell string (`payload.command`) -> `render_command_template(template, &values)` (public).
//!   The `store.add_command` + `Entry` machinery is only a vehicle to reach the renderer and is
//!   elided (sibling precedent), except where a test needs the full plan (extra args, execution).
//! - Python `launcher.run_entry(entry, values=…, invoke_cwd=…)` (build plan, spawn `sh -c`, then
//!   normalize the exit code) -> `build_launch_plan(entry, paths, assembly, …, probe)` +
//!   `execute_launch(&plan)`. The command entry drives `sh -c <rendered>` through the same path.
//! - Python private `launcher._normalize_exit_code(returncode)` (a negative `-N` signal death ->
//!   `128 + N`, non-negative unchanged) is UNREACHABLE from an integration test (its Rust twin
//!   `status_code(ExitStatus)` in launch.rs is private). Its behavior is fully observable through
//!   the public `execute_launch`, so the pure-function test is PROMOTED to a real end-to-end
//!   assertion — the same promotion the sibling made for `_posix_quote_value`. Raw `-11`/`-15`
//!   map directly to `kill -11 $$` (SIGSEGV, 139) and `kill -15 $$` (SIGTERM, 143).
//! - Python `store.add_python(p)` -> a `python`-kind `Entry`; `store.add_command(t)` -> a
//!   `command`-kind `Entry` with `EntrySettings.template = t`.
//! - Python monkeypatch of `skit.langs.launch.ensure_uv` (assert it is NOT called before the
//!   script-exists check / IS called once after) -> a `ProgramProbe` whose `find_program("uv")`
//!   panics (ordering guard) or counts (called-once guard). In Rust `python_plan` resolves uv via
//!   `require_program("uv", probe)` AFTER `require_file(&paths.script, probe)`, so the probe seam
//!   is the exact observation point.
//! - Python `launch.quote_for_shell` POSIX branch (`shlex.quote`) -> the private `quote_posix_arg`
//!   (single-quote wrapping); the two agree byte-for-byte on every value asserted here.
//!
//! Divergence NOTE (not a gap): the oracle's missing-script error message is
//! the script-missing message and asserts `match="script"`; the Rust
//! `LaunchError::TargetMissing` message is "launch target does not exist: …". Because
//! `match="script"` is a discriminator (not a full-string contract), the port asserts the
//! `TargetMissing` VARIANT with a path-identity check (a stronger discriminator), not the word
//! "script".
//!
//! Buckets:
//! - REAL (11): the substitution / quoting string tests, the two uv-ordering probe tests, the
//!   exit-code and end-to-end execution tests.
//! - cross-crate `#[ignore]` (1): `test_quote_for_shell_uses_list2cmdline_on_windows`. Rust selects
//!   the Windows quoting branch (`render_windows_command_template`) at COMPILE time via
//!   `#[cfg(windows)]`; it is not built on this POSIX target, and `quote_windows_arg` is private.
//!   The oracle reaches it by monkeypatching `sys.platform`, which a compile-time cfg cannot
//!   emulate. Python assertions kept as comments.

// Nine tests here state the unix contract (sh lowering, POSIX quoting, signal exit codes),
// so on Windows their helpers and imports go unused; the allowance keeps that compile clean
// without loosening the unix build.
#![cfg_attr(not(unix), allow(dead_code, unused_imports))]
#![cfg(not(windows))]

use std::cell::Cell;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use skit_application::delivery::Assembly;
use skit_domain::{Entry, EntryKind, EntryMeta, EntrySettings, Slug};
use skit_runtime::{
    LaunchError, LaunchPaths, LaunchPlan, ProgramProbe, build_launch_plan, execute_launch,
    render_command_template,
};
use tempfile::TempDir;

// --- Shared helpers (self-contained; the harness is duplicated per port file by design) ---

/// Build a value map from `&[(name, value)]` pairs.
fn map(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

#[cfg(unix)]
/// The oracle's real-execution proof: run one shell string through absolute `/bin/sh -c` and
/// return (success, stdout).
fn run_sh(command: &str) -> (bool, String) {
    let output = Command::new("/bin/sh")
        .arg("-c")
        .arg(command)
        .output()
        .expect("spawn /bin/sh");
    (
        output.status.success(),
        String::from_utf8(output.stdout).expect("utf-8 stdout"),
    )
}

/// A `python`-kind entry (Python `store.add_python`).
fn python_entry() -> Entry {
    Entry {
        slug: Slug::parse("demo").unwrap(),
        meta: EntryMeta::minimal("Demo", EntryKind::parse("python").unwrap()),
    }
}

/// A `command`-kind entry with `template` (Python `store.add_command`).
fn command_entry(template: &str) -> Entry {
    let mut entry = Entry {
        slug: Slug::parse("demo").unwrap(),
        meta: EntryMeta::minimal("Demo", EntryKind::parse("command").unwrap()),
    };
    EntrySettings {
        template: template.to_owned(),
        ..EntrySettings::default()
    }
    .write_to_meta(&mut entry.meta);
    entry
}

/// The `LaunchPaths` a python entry is planned against.
fn python_paths() -> LaunchPaths {
    LaunchPaths {
        script: PathBuf::from("/data/scripts/demo/script.py"),
        entry_dir: PathBuf::from("/data/scripts/demo"),
        invoke_cwd: PathBuf::from("/invoke"),
    }
}

/// Probe used by the missing-script ordering test. `find_program` PANICS: it stands in for the
/// oracle's `ensure_uv` monkeypatch that raises "must not call ensure_uv before the script-exists
/// check". The script is reported ABSENT, so `require_file` must fail before uv is ever resolved.
#[derive(Debug)]
struct ScriptCheckOrderProbe {
    invoke_cwd: PathBuf,
}

impl ProgramProbe for ScriptCheckOrderProbe {
    fn find_program(&self, name: &str) -> Option<PathBuf> {
        panic!("must not resolve {name:?} before the script-exists check");
    }

    fn is_file(&self, _path: &Path) -> bool {
        false
    }

    fn is_dir(&self, path: &Path) -> bool {
        path == self.invoke_cwd
    }

    fn is_executable(&self, _path: &Path) -> bool {
        false
    }
}

/// Probe used by the healthy-script test. The script exists and `find_program("uv")` returns a
/// fake path while counting its calls, so the port can assert uv is resolved EXACTLY once
/// (Python's `calls == [1]`).
#[derive(Debug)]
struct CountingUvProbe {
    script: PathBuf,
    invoke_cwd: PathBuf,
    uv_lookups: Cell<usize>,
}

impl ProgramProbe for CountingUvProbe {
    fn find_program(&self, name: &str) -> Option<PathBuf> {
        if name == "uv" {
            self.uv_lookups.set(self.uv_lookups.get() + 1);
            Some(PathBuf::from("/fake/uv"))
        } else {
            None
        }
    }

    fn is_file(&self, path: &Path) -> bool {
        path == self.script
    }

    fn is_dir(&self, path: &Path) -> bool {
        path == self.invoke_cwd
    }

    fn is_executable(&self, _path: &Path) -> bool {
        false
    }
}

/// Probe for the end-to-end command runs: `sh` resolves to the real `/bin/sh`, and `cwd` is a
/// real directory so the launch workdir check passes.
#[derive(Debug)]
struct ShellProbe {
    cwd: PathBuf,
}

impl ProgramProbe for ShellProbe {
    fn find_program(&self, name: &str) -> Option<PathBuf> {
        if name == "sh" {
            Some(PathBuf::from("/bin/sh"))
        } else {
            None
        }
    }

    fn is_file(&self, _path: &Path) -> bool {
        false
    }

    fn is_dir(&self, path: &Path) -> bool {
        path == self.cwd
    }

    fn is_executable(&self, _path: &Path) -> bool {
        false
    }
}

#[cfg(unix)]
/// A directly-built `sh -c <script>` plan (Python `run_entry` reduces to this after render).
fn sh_plan(script: &str, cwd: &Path) -> LaunchPlan {
    LaunchPlan {
        program: PathBuf::from("/bin/sh"),
        args: vec!["-c".to_owned(), script.to_owned()],
        env: BTreeMap::new(),
        cwd: cwd.to_path_buf(),
        display: String::new(),
        warnings: Vec::new(),
    }
}

#[cfg(unix)]
/// Run a command entry end-to-end: render + spawn `sh -c` + normalize the exit code, exactly the
/// Python `run_entry(entry, values=…, invoke_cwd=cwd)` path.
fn run_command_entry(template: &str, values: &[(&str, &str)], cwd: &Path) -> i32 {
    let mut entry = command_entry(template);
    entry.meta.workdir = "invoke".to_owned();
    let assembly = Assembly {
        command_values: map(values),
        masked_command_values: map(values),
        ..Assembly::default()
    };
    let paths = LaunchPaths {
        script: PathBuf::from("/unused"),
        entry_dir: cwd.to_path_buf(),
        invoke_cwd: cwd.to_path_buf(),
    };
    let probe = ShellProbe {
        cwd: cwd.to_path_buf(),
    };
    let plan = build_launch_plan(&entry, &paths, &assembly, None, None, &probe).unwrap();
    execute_launch(&plan).unwrap()
}

// ---------- Double-brace unescape must not corrupt substituted values ----------

// The active contract here is the POSIX arm of a compile-time host branch (command
// templates lower through `sh -c` only under cfg(not(windows)); Windows lowers through
// render_windows_command_template). Asserting the POSIX rendering or running /bin/sh on a
// Windows host tests nothing real, so the unix gate states the contract's platform.
#[cfg(unix)]
#[test]
fn test_placeholder_value_with_double_braces_round_trips() {
    // A value containing a literal "{{"/"}}" (e.g. a Jinja/Go-template fragment) must survive
    // substitution unchanged — the old two-pass implementation (substitute, then str.replace the
    // whole string) collapsed these to single braces because it could not tell a template-level
    // escape from characters that came from the injected value.
    let cmd =
        render_command_template("run --q {q}", &map(&[("q", "{{ .name }}")])).expect("renders");
    // The old two-pass implementation collapsed this to "run --q { .name }" (single braces).
    // The value lands unquoted, so it is single-quoted like `shlex.quote("{{ .name }}")`.
    assert_eq!(cmd, "run --q '{{ .name }}'");
}

#[cfg(unix)]
#[test]
fn test_placeholder_value_with_double_braces_inside_quoted_template_slot() {
    // An `echo {msg}` template must preserve escape-like braces embedded in msg itself.
    let cmd = render_command_template("echo {msg}", &map(&[("msg", "prefix{{inner}}suffix")]))
        .expect("renders");
    assert!(cmd.contains("prefix{{inner}}suffix"), "{cmd:?}");
}

#[cfg(unix)]
#[test]
fn test_template_escape_still_unescaped_alongside_a_corrupting_value() {
    // The template's OWN {{name}} escape must still be unescaped to a literal brace, while a
    // substituted value's incidental "{{"/"}}" must NOT be — proving the two are now
    // distinguished (single regex pass over the original template) rather than conflated.
    let cmd = render_command_template(
        "echo {{literal}} {msg}",
        &map(&[("msg", "{{escaped-looking}}")]),
    )
    .expect("renders");
    assert!(cmd.contains("{literal}"), "{cmd:?}"); // template escape: unescaped to a single brace
    assert!(cmd.contains("{{escaped-looking}}"), "{cmd:?}"); // value content: left exactly as given
}

#[cfg(unix)]
#[test]
fn test_run_entry_executes_correctly_with_double_brace_value() {
    // End-to-end: run the assembled command for real and check the child actually received the
    // value byte-for-byte, including its "{{"/"}}" — not just that build_command's string looks
    // right.
    let dir = TempDir::new().unwrap();
    let outfile = dir.path().join("out.txt");
    // The `{}` below is a Rust format substitution (done before the renderer ever sees the
    // template), matching the oracle's f-string; `{{msg}}` renders as the literal `{msg}`
    // placeholder — the one real skit slot.
    let template = format!(r#"printf "%s" {{msg}} > "{}""#, outfile.display());
    let code = run_command_entry(&template, &[("msg", "prefix{{inner}}suffix")], dir.path());
    assert_eq!(code, 0);
    assert_eq!(
        fs::read_to_string(&outfile).unwrap(),
        "prefix{{inner}}suffix"
    );
}

// ---------- Signal-death exit codes must be normalized to 128+N ----------

#[cfg(unix)]
#[test]
fn test_normalize_exit_code_maps_negative_returncode_to_128_plus_n() {
    // Python asserts the private `_normalize_exit_code(-11) == 139` (SIGSEGV), `-15 == 143`
    // (SIGTERM), `0 == 0`, `2 == 2`. That helper is not on the Rust public surface; the SAME
    // mapping is observed through `execute_launch` running a child that dies by that signal or
    // exits with that code. Raw `-11`/`-15` map directly to `kill -11 $$`/`kill -15 $$`.
    let dir = TempDir::new().unwrap();
    assert_eq!(
        execute_launch(&sh_plan("kill -11 $$", dir.path())).unwrap(),
        139
    ); // SIGSEGV
    assert_eq!(
        execute_launch(&sh_plan("kill -15 $$", dir.path())).unwrap(),
        143
    ); // SIGTERM
    assert_eq!(execute_launch(&sh_plan("exit 0", dir.path())).unwrap(), 0);
    assert_eq!(execute_launch(&sh_plan("exit 2", dir.path())).unwrap(), 2);
}

#[cfg(unix)]
#[test]
fn test_run_entry_normalizes_signal_killed_child_to_shell_convention() {
    // End-to-end: a command entry that kills its own shell with SIGTERM must come back as 143
    // (128+15), not the raw -15 subprocess reports.
    let dir = TempDir::new().unwrap();
    let code = run_command_entry("kill -TERM $$", &[], dir.path());
    assert_eq!(code, 143);
}

// ---------- _build_python must check the script before touching uv ----------

#[test]
fn test_build_python_missing_script_raises_before_calling_ensure_uv() {
    // On the CLI run path (no preflight call), a missing script must be reported without ever
    // resolving/downloading uv — mirrors preflight's existing ordering. The probe PANICS on any
    // `find_program` call (the analog of the oracle's `ensure_uv` that raises), so reaching a
    // TargetMissing without a panic proves the script check ran first.
    let mut entry = python_entry();
    entry.meta.workdir = "invoke".to_owned();
    let probe = ScriptCheckOrderProbe {
        invoke_cwd: PathBuf::from("/invoke"),
    };
    let error = build_launch_plan(
        &entry,
        &python_paths(),
        &Assembly::default(),
        None,
        None,
        &probe,
    )
    .unwrap_err();
    assert!(
        matches!(
            &error,
            LaunchError::TargetMissing { path }
                if path == &PathBuf::from("/data/scripts/demo/script.py")
        ),
        "{error:?}"
    );
}

#[test]
fn test_build_python_healthy_script_still_calls_ensure_uv() {
    // Sanity check for the reordering: when the script DOES exist, ensure_uv must still run (the
    // reorder must not accidentally skip it). Python asserts `calls == [1]` and `cmd[0] == uv`.
    let mut entry = python_entry();
    entry.meta.workdir = "invoke".to_owned();
    let probe = CountingUvProbe {
        script: PathBuf::from("/data/scripts/demo/script.py"),
        invoke_cwd: PathBuf::from("/invoke"),
        uv_lookups: Cell::new(0),
    };
    let plan = build_launch_plan(
        &entry,
        &python_paths(),
        &Assembly::default(),
        None,
        None,
        &probe,
    )
    .unwrap();
    assert_eq!(probe.uv_lookups.get(), 1);
    assert_eq!(plan.program, PathBuf::from("/fake/uv")); // uv is argv[0]
}

// ---------- Command-template placeholder values must be shell-quoted ----------

#[cfg(unix)]
#[test]
fn test_placeholder_value_with_space_is_quoted_as_one_word() {
    let cmd = render_command_template(
        "ffmpeg -i {input} out.mp4",
        &map(&[("input", "My Movie.mp4")]),
    )
    .expect("renders");
    // `shlex.quote("My Movie.mp4") == "'My Movie.mp4'"`; the value stays ONE word.
    assert_eq!(cmd, "ffmpeg -i 'My Movie.mp4' out.mp4");
    // Oracle also asserts `shlex.split(cmd) == ["ffmpeg", "-i", "My Movie.mp4", "out.mp4"]`; the
    // exact single-quoted string above pins that byte-for-byte.
}

#[cfg(unix)]
#[test]
fn test_placeholder_value_with_shell_metacharacters_cannot_inject() {
    let hostile = "a; rm -rf x";
    let cmd = render_command_template("echo {msg}", &map(&[("msg", hostile)])).expect("renders");
    // Quoted as a single shell word, not parsed as a second command.
    assert_eq!(cmd, "echo 'a; rm -rf x'");
    // Oracle: `shlex.split(cmd) == ["echo", hostile]`. Real execution proves it never injects.
    let (ok, out) = run_sh(&cmd);
    assert!(ok);
    assert_eq!(out, format!("{hostile}\n"));
}

#[cfg(unix)]
#[test]
fn test_run_entry_placeholder_value_with_space_reaches_child_intact() {
    // End-to-end: a value with an embedded space must arrive at the child as ONE argument,
    // not be split into two by the shell.
    let dir = TempDir::new().unwrap();
    let outfile = dir.path().join("out.txt");
    let template = format!(r#"printf "%s|%s|" {{a}} {{b}} > "{}""#, outfile.display());
    let code = run_command_entry(
        &template,
        &[("a", "My Movie.mp4"), ("b", "second")],
        dir.path(),
    );
    assert_eq!(code, 0);
    assert_eq!(
        fs::read_to_string(&outfile).unwrap(),
        "My Movie.mp4|second|"
    );
}

#[test]
#[ignore = "cross-crate: the Windows quoting branch (render_windows_command_template, #[cfg(windows)] in crates/skit-runtime/src/launch.rs) is not compiled on this POSIX target, and quote_windows_arg is private. Python reaches it by monkeypatching sys.platform, which a compile-time cfg cannot emulate."]
fn test_quote_for_shell_uses_list2cmdline_on_windows() {
    // The win32 branch of _quote_for_shell must use subprocess.list2cmdline (Windows quoting),
    // not shlex.quote (POSIX). list2cmdline is a pure algorithm that runs on any host, so the
    // oracle drives the branch by faking the platform and asserts the two quoters genuinely differ
    // for a spaced value:
    //   quoted == subprocess.list2cmdline(["My Movie.mp4"]) == '"My Movie.mp4"'  (DOUBLE quotes)
    //   quoted != shlex.quote("My Movie.mp4")                                    (POSIX single)
}
