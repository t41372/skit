//! Mechanical port of the Python oracle module `tests/test_launcher.py`
//! (`origin/main@206f9ef`): "Launcher command assembly and workdir policy".
//! Each `#[test]` keeps its Python `def test_*` name and its WHY comment, and drives
//! the REAL public skit-runtime launch API.
//!
//! Concept mapping (Python `skit.launcher` / `skit.langs.launch` -> Rust `skit_runtime`):
//! - `launcher.build_command(entry, extra)` -> `build_launch_plan(entry, paths, assembly, …)`
//!   returning a `LaunchPlan { program, args, cwd, display, … }`. Python returns a bare argv
//!   list (python/exe) or a shell string (command); Rust always returns the same `LaunchPlan`,
//!   with the program split out from `args`, and a command template lowered to `sh -c <text>`.
//! - `launcher.describe_command(entry, extra)` -> `build_launch_preview(…).display` (no PATH
//!   lookup; the literal program name stands in).
//! - `launcher._resolve_workdir(entry, invoke_cwd)` -> `resolve_launch_workdir(entry, paths, probe)`.
//! - `launcher.run_entry(entry)` execution -> `execute_launch(&plan)` (returns the child code).
//! - Python monkeypatches (`shutil.which`, `find_uv`, filesystem state) -> the `ProgramProbe`
//!   seam. Tests inject a `FakeProbe`; the two real-execution integration tests use `SystemProbe`.
//!
//! Buckets:
//! - ASSERTING (17): everything `build_launch_plan` / `build_launch_preview` /
//!   `resolve_launch_workdir` / `execute_launch` reach directly.
//! - CROSS-CRATE `#[ignore]` (21): the Python `launcher` module folds in four
//!   responsibilities that the Rust rewrite moved OUT of `skit-runtime`, so they are
//!   unreachable from this crate's integration tests:
//!   1. uv discovery + auto-download (`find_uv` PATH->private bin, `ensure_uv`): the Rust
//!      launch path only looks up `uv` through the probe; the PATH->private-bin fallback and
//!      the download live in `skit-cli/src/run/command.rs:349-421,731` (`managed_uv_path` +
//!      `ensure_managed_uv`). `build_launch_plan` never downloads.
//!   2. mirror-env overlay: `execute_launch` runs the plan with `plan.env` only. The
//!      `UV_DEFAULT_INDEX` / `UV_PYTHON_INSTALL_MIRROR` overlay is applied by
//!      `skit-cli/src/run/command.rs:269,436,499` (`mirror_environment`).
//!   3. `target_missing` / `missing_marker`: the "⚠ missing:" listing marker is built by
//!      `skit-cli/src/cli.rs:5453` (`summary_target`) + `list_description` and
//!      `skit-tui/src/screens/library.rs:423`. `skit-runtime` has no per-kind `target()`.
//!   4. `preflight`: the side-effect-free, uv-free validator the TUI runs before it suspends
//!      the terminal (and doctor's `HealthIssueKind::LaunchBlocked`) has NO equivalent in
//!      `skit-runtime`. `build_launch_plan` is uv-coupled (a python plan requires the `uv`
//!      program) and resolves the workdir BEFORE the script check, and `build_launch_preview`
//!      never enforces `needs` (its `PreviewProbe::find_program` always returns `Some`), so
//!      neither is a faithful preflight. The preflight tier lives in the `skit-cli` health
//!      adapter; mapping these onto `build`/`preview` would test the wrong function and could
//!      mask a real preflight divergence, so all nine are stubbed.
//! - ABSENT gaps: NONE. DIVERGENCE (failing-contract) tests: NONE.
//!
//! Error-string note: the oracle matches substrings of localized English messages (e.g.
//! `match="exe"` inside "The executable doesn't exist"). The Rust ports assert the TYPED
//! variant and its payload path instead (`TargetMissing { path }`, `TargetNotExecutable
//! { path }`, `WorkdirMissing { path }`, `UnknownKind`), matching the established port
//! convention in `crates/skit-runtime/tests/launch_plan.rs`. The English wording differs
//! ("launch target does not exist" vs "The script/executable doesn't exist") but the
//! 127/126/125 exit contract and the offending path are preserved.

use std::{
    fs,
    path::{Path, PathBuf},
};

use skit_application::delivery::Assembly;
use skit_domain::{Entry, EntryKind, EntryMeta, EntrySettings, Slug, StorageMode};
use skit_runtime::{
    InterpreterPlatform, LaunchError, LaunchPaths, ProgramProbe, SystemProbe, build_launch_plan,
    build_launch_preview, execute_launch, project_launch_workdir, resolve_interpreter,
    resolve_launch_workdir,
};
use tempfile::TempDir;

// --- Fixtures: the `ProgramProbe` seam replaces the oracle's monkeypatching of
// `shutil.which`, `skit.langs.launch.find_uv`, and the real filesystem. ---

#[derive(Debug, Default)]
struct FakeProbe {
    programs: std::collections::BTreeMap<String, PathBuf>,
    files: Vec<PathBuf>,
    dirs: Vec<PathBuf>,
    executable: Vec<PathBuf>,
}

impl ProgramProbe for FakeProbe {
    fn find_program(&self, name: &str) -> Option<PathBuf> {
        self.programs.get(name).cloned()
    }

    fn is_file(&self, path: &Path) -> bool {
        self.files.iter().any(|item| item == path)
    }

    fn is_dir(&self, path: &Path) -> bool {
        self.dirs.iter().any(|item| item == path)
    }

    fn is_executable(&self, path: &Path) -> bool {
        self.executable.iter().any(|item| item == path)
    }
}

/// One entry of a given kind, in the oracle's default copy mode.
fn entry(kind: &str) -> Entry {
    Entry {
        slug: Slug::parse("demo").unwrap(),
        meta: EntryMeta::minimal("Demo", EntryKind::parse(kind).unwrap()),
    }
}

/// The three launch paths (script / entry dir / invoke cwd), like the oracle's `py_entry`
/// living under a data store while the user invokes skit from elsewhere.
fn paths(script: &str) -> LaunchPaths {
    LaunchPaths {
        script: PathBuf::from(script),
        entry_dir: PathBuf::from("/data/scripts/demo"),
        invoke_cwd: PathBuf::from("/invoke"),
    }
}

/// A probe that finds the script on disk and knows both the store dir and the invoke cwd.
fn probe_for(script: &str) -> FakeProbe {
    FakeProbe {
        files: vec![PathBuf::from(script)],
        dirs: vec![
            PathBuf::from("/invoke"),
            PathBuf::from("/data/scripts/demo"),
        ],
        executable: vec![PathBuf::from(script)],
        ..FakeProbe::default()
    }
}

// ================================================================================================
// Windows interpreter resolution (a typed, platform-neutral policy exercised on every host)
// ================================================================================================

#[test]
fn test_resolve_bash_on_win32_uses_config_path_when_it_exists() {
    let configured = PathBuf::from("C:/Program Files/Git/bin/bash.exe");
    let mut probe = FakeProbe {
        files: vec![configured.clone()],
        ..FakeProbe::default()
    };

    assert_eq!(
        resolve_interpreter(
            "bash",
            InterpreterPlatform::Windows,
            Some(&configured),
            &probe,
        )
        .unwrap(),
        configured
    );

    probe.programs.insert(
        "bash".to_owned(),
        PathBuf::from("C:/Program Files/Git/usr/bin/bash.exe"),
    );
    assert_eq!(
        resolve_interpreter(
            "bash",
            InterpreterPlatform::Windows,
            Some(Path::new("C:/configured/bash.exe")),
            &probe,
        )
        .unwrap(),
        PathBuf::from("C:/Program Files/Git/usr/bin/bash.exe")
    );
}

#[test]
fn test_resolve_bash_on_win32_configured_but_missing_falls_through() {
    let probe = FakeProbe::default();
    let error = resolve_interpreter(
        "bash",
        InterpreterPlatform::Windows,
        Some(Path::new("C:/gone/bash.exe")),
        &probe,
    )
    .unwrap_err();

    assert!(matches!(error, LaunchError::WindowsShellMissing { ref name } if name == "bash"));
    assert_windows_shell_error_messages(&error, "bash");
}

#[test]
fn test_resolve_bash_on_win32_unset_names_both_escape_hatches() {
    let error = resolve_interpreter(
        "zsh",
        InterpreterPlatform::Windows,
        None,
        &FakeProbe::default(),
    )
    .unwrap_err();

    assert!(matches!(error, LaunchError::WindowsShellMissing { ref name } if name == "zsh"));
    assert_windows_shell_error_messages(&error, "zsh");
}

#[test]
fn test_resolve_nonbash_on_win32_gets_generic_message() {
    let error = resolve_interpreter(
        "ruby",
        InterpreterPlatform::Windows,
        Some(Path::new("C:/configured/bash.exe")),
        &FakeProbe::default(),
    )
    .unwrap_err();

    assert!(matches!(error, LaunchError::ProgramNotFound { ref name } if name == "ruby"));
    assert!(error.to_string().contains("ruby"));
    assert!(!error.to_string().contains("Git for Windows"));
}

fn assert_windows_shell_error_messages(error: &LaunchError, name: &str) {
    use skit_i18n::{Locale, Localize as _};

    for (locale, expected) in [
        (
            Locale::En,
            format!(
                "{name} isn't available on this system. Install Git for Windows (its bash works) or WSL, or point skit at one with: skit config shell.bash_path <path>"
            ),
        ),
        (
            Locale::ZhCn,
            format!(
                "此系统上没有 {name}。请安装 Git for Windows（自带的 bash 即可）或 WSL，或用 skit config shell.bash_path <path> 指定一个。"
            ),
        ),
        (
            Locale::ZhTw,
            format!(
                "此系統上沒有 {name}。請安裝 Git for Windows（內附的 bash 即可）或 WSL，或用 skit config shell.bash_path <path> 指定一個。"
            ),
        ),
    ] {
        assert_eq!(error.message().localize(locale), expected);
    }
}

// ==================================================================================
// ASSERTING TESTS
// ==================================================================================

#[test]
fn test_python_command_uses_uv_run_script() {
    // C2: --no-project unconditionally (uv would otherwise attach a block-less script to any
    // enclosing project), and --script passed explicitly.
    let mut py = entry("python");
    py.meta.workdir = "invoke".to_owned();
    let script = "/data/scripts/demo/script.py";
    let mut probe = probe_for(script);
    probe
        .programs
        .insert("uv".to_owned(), PathBuf::from("/fake/uv"));
    let assembly = Assembly {
        args: vec!["--x".to_owned(), "1".to_owned()],
        masked_args: vec!["--x".to_owned(), "1".to_owned()],
        ..Assembly::default()
    };

    let plan = build_launch_plan(&py, &paths(script), &assembly, None, None, &probe).unwrap();

    assert_eq!(plan.program, PathBuf::from("/fake/uv"));
    assert_eq!(plan.args[0..3], ["run", "--no-project", "--script"]);
    assert!(plan.args[3].ends_with("script.py"));
    assert_eq!(plan.args[plan.args.len() - 2..], ["--x", "1"]);
}

#[test]
fn test_command_template_appends_extra_args() {
    // A command entry builds a shell command with the extra args appended. Python returns the
    // bare shell string; Rust lowers it to `sh -c "<rendered> <extra>"`, so the rendered text
    // is `args[1]`.
    let mut command = entry("command");
    command.meta.workdir = "invoke".to_owned();
    EntrySettings {
        template: "echo hello".to_owned(),
        ..EntrySettings::default()
    }
    .write_to_meta(&mut command.meta);
    let mut probe = probe_for("/unused");
    probe
        .programs
        .insert("sh".to_owned(), PathBuf::from("/bin/sh"));
    let assembly = Assembly {
        args: vec!["world".to_owned()],
        masked_args: vec!["world".to_owned()],
        ..Assembly::default()
    };

    let plan =
        build_launch_plan(&command, &paths("/unused"), &assembly, None, None, &probe).unwrap();

    assert_eq!(plan.program, PathBuf::from("/bin/sh"));
    assert_eq!(plan.args[0], "-c");
    assert!(plan.args[1].starts_with("echo hello"));
    assert!(plan.args[1].contains("world"));
}

#[test]
fn test_workdir_origin_is_source_parent() {
    // `_resolve_workdir`'s mapping for policy="origin" when the origin dir is present: the
    // parent of the recorded source.
    let mut py = entry("python");
    py.meta.mode = StorageMode::Copy;
    py.meta.workdir = "origin".to_owned();
    py.meta.source = "/work/s.py".to_owned();
    let probe = FakeProbe {
        dirs: vec![PathBuf::from("/work")],
        ..FakeProbe::default()
    };
    let paths = LaunchPaths {
        script: PathBuf::from("/work/s.py"),
        entry_dir: PathBuf::from("/data/scripts/demo"),
        invoke_cwd: PathBuf::from("/work/elsewhere"),
    };

    assert_eq!(
        resolve_launch_workdir(&py, &paths, &probe).unwrap(),
        PathBuf::from("/work")
    );
}

#[test]
fn test_workdir_store_and_invoke() {
    // policy="store" -> the entry dir; policy="invoke" -> the invoke cwd.
    let mut py = entry("python");
    let paths = paths("/data/scripts/demo/script.py");
    let probe = probe_for("/data/scripts/demo/script.py");

    py.meta.workdir = "store".to_owned();
    assert_eq!(
        resolve_launch_workdir(&py, &paths, &probe).unwrap(),
        PathBuf::from("/data/scripts/demo")
    );

    py.meta.workdir = "invoke".to_owned();
    assert_eq!(
        resolve_launch_workdir(&py, &paths, &probe).unwrap(),
        PathBuf::from("/invoke")
    );
}

#[test]
fn test_run_entry_real_execution() {
    // Integration test: a real uv is present, so run for real. invoke_cwd must be a neutral
    // directory (copy-mode entries default to workdir that lands on the invoke cwd, and
    // `uv run --script` without --no-project does project discovery from there).
    let probe = SystemProbe;
    if probe.find_program("uv").is_none() {
        return; // the pytest.skip("no uv in environment") analogue
    }
    let root = TempDir::new().unwrap();
    let script = root.path().join("s.py");
    fs::write(&script, "print('ok')\n").unwrap();
    let mut py = entry("python");
    py.meta.workdir = "invoke".to_owned();
    let paths = LaunchPaths {
        script: script.clone(),
        entry_dir: root.path().to_path_buf(),
        invoke_cwd: root.path().to_path_buf(),
    };

    let plan = build_launch_plan(&py, &paths, &Assembly::default(), None, None, &probe).unwrap();
    assert_eq!(execute_launch(&plan).unwrap(), 0);
}

#[test]
fn test_workdir_origin_no_source_falls_back_to_cwd() {
    // policy="origin" but the entry recorded no source -> the invoke cwd.
    let mut py = entry("python");
    py.meta.workdir = "origin".to_owned();
    py.meta.source = String::new();
    let paths = paths("/data/scripts/demo/script.py");
    let probe = probe_for("/data/scripts/demo/script.py");

    assert_eq!(
        resolve_launch_workdir(&py, &paths, &probe).unwrap(),
        PathBuf::from("/invoke")
    );
}

#[test]
fn test_workdir_absolute_path_used_directly() {
    // An absolute custom workdir is used verbatim. (Python's resolver does no existence check;
    // the Rust resolver folds one in, so the probe must know the directory exists.)
    let mut py = entry("python");
    let custom = "/custom/work";
    py.meta.workdir = custom.to_owned();
    let probe = FakeProbe {
        dirs: vec![PathBuf::from(custom)],
        ..FakeProbe::default()
    };
    let paths = paths("/data/scripts/demo/script.py");

    assert_eq!(
        resolve_launch_workdir(&py, &paths, &probe).unwrap(),
        PathBuf::from(custom)
    );
}

#[test]
fn test_python_with_deps_and_python_version() {
    // Reference-mode deps and python constraint pass via --with/--python.
    let mut py = entry("python");
    py.meta.workdir = "invoke".to_owned();
    EntrySettings {
        requires_python: ">=3.11".to_owned(),
        dependencies: vec!["requests".to_owned(), "rich".to_owned()],
        ..EntrySettings::default()
    }
    .write_to_meta(&mut py.meta);
    let script = "/data/scripts/demo/script.py";
    let mut probe = probe_for(script);
    probe.programs.insert("uv".to_owned(), PathBuf::from("/uv"));

    let plan = build_launch_plan(
        &py,
        &paths(script),
        &Assembly::default(),
        None,
        None,
        &probe,
    )
    .unwrap();

    assert!(plan.args.iter().any(|arg| arg == "--python"));
    assert!(plan.args.iter().any(|arg| arg == ">=3.11"));
    assert_eq!(plan.args.iter().filter(|arg| *arg == "--with").count(), 2);
}

#[test]
fn test_exe_missing_source_raises() {
    // An exe whose source is gone from disk refuses before spawn (exit 127, TargetMissing).
    let mut exe = entry("exe");
    exe.meta.mode = StorageMode::Reference;
    exe.meta.source = "/gone/tool".to_owned();
    exe.meta.workdir = "invoke".to_owned();
    let probe = FakeProbe {
        dirs: vec![PathBuf::from("/invoke")],
        ..FakeProbe::default()
    };

    let error = build_launch_plan(
        &exe,
        &paths("/gone/tool"),
        &Assembly::default(),
        None,
        None,
        &probe,
    )
    .unwrap_err();
    assert!(
        matches!(error, LaunchError::TargetMissing { ref path } if path.as_path() == Path::new("/gone/tool")),
        "{error:?}"
    );
}

#[test]
fn test_exe_directory_source_refused_as_not_executable() {
    // An exe entry pointing at a directory (a macOS .app bundle, a typo'd path) must be refused
    // with a clean NotExecutable refusal (exit 126) that names the offending path, not crash at
    // spawn. Rust surfaces `TargetNotExecutable { path }` carrying the source.
    let mut exe = entry("exe");
    exe.meta.mode = StorageMode::Reference;
    exe.meta.source = "/apps/Bundle.app".to_owned();
    exe.meta.workdir = "invoke".to_owned();
    // The source exists but as a directory, not a regular file.
    let probe = FakeProbe {
        dirs: vec![PathBuf::from("/invoke"), PathBuf::from("/apps/Bundle.app")],
        ..FakeProbe::default()
    };

    let error = build_launch_plan(
        &exe,
        &paths("/apps/Bundle.app"),
        &Assembly::default(),
        None,
        None,
        &probe,
    )
    .unwrap_err();
    assert!(
        matches!(error, LaunchError::TargetNotExecutable { ref path } if path.as_path() == Path::new("/apps/Bundle.app")),
        "{error:?}"
    );
}

#[test]
fn test_build_command_unknown_kind_raises() {
    // A kind this build does not know refuses with UnknownKind. Rust checks needs, then resolves
    // the workdir, THEN dispatches on kind — so the probe supplies a valid workdir to reach the
    // kind arm (unlike the oracle, which reaches it directly).
    let mut unknown = entry("unknown");
    unknown.meta.workdir = "invoke".to_owned();
    let probe = FakeProbe {
        dirs: vec![PathBuf::from("/invoke")],
        ..FakeProbe::default()
    };

    let error = build_launch_plan(
        &unknown,
        &paths("/data/scripts/demo/script"),
        &Assembly::default(),
        None,
        None,
        &probe,
    )
    .unwrap_err();
    assert!(
        matches!(error, LaunchError::UnknownKind { .. }),
        "{error:?}"
    );
}

#[test]
fn test_run_entry_missing_workdir_raises() {
    // A non-existent absolute workdir refuses before spawn, naming the missing path. Rust
    // resolves the workdir before any kind dispatch, so no uv lookup is needed to surface it.
    let mut py = entry("python");
    py.meta.workdir = "/nonexistent/path/that/does/not/exist".to_owned();
    let probe = FakeProbe::default();

    let error = build_launch_plan(
        &py,
        &paths("/data/scripts/demo/script.py"),
        &Assembly::default(),
        None,
        None,
        &probe,
    )
    .unwrap_err();
    assert!(
        matches!(error, LaunchError::WorkdirMissing { ref path }
            if path.as_path() == Path::new("/nonexistent/path/that/does/not/exist")),
        "{error:?}"
    );
}

#[test]
fn test_run_entry_command_entry() {
    // A shell command entry runs through the shell and returns exit 0. Real execution via
    // `sh -c "echo hello"`; the workdir must be a real existing directory.
    let root = TempDir::new().unwrap();
    let mut command = entry("command");
    command.meta.workdir = "invoke".to_owned();
    EntrySettings {
        template: "echo hello".to_owned(),
        ..EntrySettings::default()
    }
    .write_to_meta(&mut command.meta);
    let paths = LaunchPaths {
        script: root.path().join("unused"),
        entry_dir: root.path().to_path_buf(),
        invoke_cwd: root.path().to_path_buf(),
    };

    let plan = build_launch_plan(
        &command,
        &paths,
        &Assembly::default(),
        None,
        None,
        &SystemProbe,
    )
    .unwrap();
    assert_eq!(execute_launch(&plan).unwrap(), 0);
}

#[test]
fn test_resolve_workdir_copy_mode_falls_back_when_origin_gone() {
    // Copy mode exists to decouple the entry from its original location, so a vanished origin
    // must not block a run when the store copy is intact: workdir="origin" falls back to the
    // invoke cwd for copy mode.
    let mut copy = entry("python");
    copy.meta.mode = StorageMode::Copy;
    copy.meta.workdir = "origin".to_owned();
    copy.meta.source = "/gone/s.py".to_owned();
    let probe = FakeProbe {
        dirs: vec![PathBuf::from("/invoke")], // note: /gone is NOT a directory
        ..FakeProbe::default()
    };
    let paths = paths("/data/scripts/demo/script.py");

    assert_eq!(
        resolve_launch_workdir(&copy, &paths, &probe).unwrap(),
        PathBuf::from("/invoke")
    );
}

#[test]
fn test_run_entry_succeeds_for_copy_mode_entry_with_deleted_origin() {
    // Same copy-mode fallback, but through the real run path (build + workdir check + spawn): a
    // copy-mode script survives deletion of its original because the store copy is untouched.
    let probe = SystemProbe;
    if probe.find_program("uv").is_none() {
        return; // no uv in environment
    }
    let root = TempDir::new().unwrap();
    let script = root.path().join("script.py");
    fs::write(&script, "print('ok')\n").unwrap();
    let mut copy = entry("python");
    copy.meta.mode = StorageMode::Copy;
    copy.meta.workdir = "origin".to_owned();
    // The origin directory does not exist (never created), so its parent is gone.
    copy.meta.source = root
        .path()
        .join("deleted_origin")
        .join("s.py")
        .display()
        .to_string();
    let paths = LaunchPaths {
        script: script.clone(),
        entry_dir: root.path().to_path_buf(),
        invoke_cwd: root.path().to_path_buf(),
    };

    let plan = build_launch_plan(&copy, &paths, &Assembly::default(), None, None, &probe).unwrap();
    assert_eq!(execute_launch(&plan).unwrap(), 0);
}

#[test]
fn test_resolve_workdir_reference_mode_not_masked_when_origin_gone() {
    // Reference mode is NOT decoupled from its origin (there is no store copy), so the
    // copy-mode fallback must not apply — masking a genuinely-gone original would just relocate
    // a real failure. The oracle asserts `_resolve_workdir == src_dir` (its resolver does no
    // existence check). The Rust resolver folds the existence check in, so the same decision
    // ("reference mode selects the origin, never the invoke cwd") surfaces as
    // `WorkdirMissing { path == origin }`; were it masked, it would be `Ok(invoke_cwd)`.
    let mut reference = entry("python");
    reference.meta.mode = StorageMode::Reference;
    reference.meta.workdir = "origin".to_owned();
    reference.meta.source = "/refdir/ref.py".to_owned();
    let probe = FakeProbe {
        dirs: vec![PathBuf::from("/invoke")], // the invoke cwd exists; /refdir does not
        ..FakeProbe::default()
    };
    let paths = paths("/data/scripts/demo/script.py");

    assert_eq!(
        project_launch_workdir(&reference, &paths, &probe).unwrap(),
        PathBuf::from("/refdir"),
        "a form keeps the semantic origin so completion can go silent and its picker can degrade"
    );

    let error = resolve_launch_workdir(&reference, &paths, &probe).unwrap_err();
    assert!(
        matches!(error, LaunchError::WorkdirMissing { ref path } if path.as_path() == Path::new("/refdir")),
        "{error:?}"
    );
}

#[test]
fn test_describe_command_isolates_like_build_command() {
    // The transparency line mirrors the real isolation (--no-project). `describe_command` maps to
    // `build_launch_preview`, which stands the literal program name in and does no PATH lookup.
    let mut py = entry("python");
    py.meta.workdir = "invoke".to_owned();
    let script = "/data/scripts/demo/script.py";
    let probe = probe_for(script);
    let assembly = Assembly {
        args: vec!["--x".to_owned()],
        masked_args: vec!["--x".to_owned()],
        ..Assembly::default()
    };

    let plan =
        build_launch_preview(&py, &paths(script), &assembly, None, None, None, &probe).unwrap();

    assert!(plan.display.contains("--no-project"));
}

// ==================================================================================
// CROSS-CRATE STUBS (#[ignore]): the behavior lives outside skit-runtime and cannot be
// reached from this crate's integration tests without a forbidden dependency edit.
// ==================================================================================

// ---------- uv discovery + auto-download: skit-cli/src/run/command.rs:349-421,731 ----------

#[test]
#[ignore = "CROSS-CRATE (skit-cli run::command uv orchestration): when uv is absent, the run path calls ensure_managed_uv (run/command.rs:421) to auto-download a managed copy; build_launch_plan itself never downloads. Oracle: tests/test_launcher.py:43 test_python_without_uv_auto_downloads."]
fn test_python_without_uv_auto_downloads() {
    // Python: find_uv -> None, uvman.ensure_uv_downloaded -> "/downloaded/uv"; build_command
    // returns a command whose program is the downloaded uv. Rust equivalent is orchestrated in
    // skit-cli, which resolves uv (managed_uv_path + ensure_managed_uv) before build.
}

#[test]
#[ignore = "CROSS-CRATE (skit-cli run::command uv orchestration): a failed download surfaces UvBootstrapError from ensure_managed_uv (run/command.rs:421); skit-runtime never downloads uv. Oracle: tests/test_launcher.py:54 test_python_uv_download_failure_raises."]
fn test_python_uv_download_failure_raises() {
    // Python: ensure_uv_downloaded raises UvDownloadError -> LaunchError matching "uv".
}

#[test]
#[ignore = "CROSS-CRATE (skit-cli run::command uv discovery): the PATH->private-bin fallback (find_uv) is composed in skit-cli via managed_uv_path (run/command.rs:349,358,731); the skit-runtime launch path only looks uv up through the probe's PATH lookup. Oracle: tests/test_launcher.py:119 test_find_uv_private_bin_fallback."]
fn test_find_uv_private_bin_fallback() {
    // Python: uv absent from PATH -> find_uv returns the skit-private bin path.
}

#[test]
#[ignore = "CROSS-CRATE (skit-cli run::command uv discovery): with uv in neither PATH nor the private bin, the composed resolver reports none (skit-cli run/command.rs). Oracle: tests/test_launcher.py:129 test_find_uv_returns_none_when_absent."]
fn test_find_uv_returns_none_when_absent() {
    // Python: uv in neither PATH nor private bin -> find_uv returns None.
}

// ---------- mirror-env overlay: skit-cli/src/run/command.rs:269,436,499 ----------

#[test]
#[ignore = "CROSS-CRATE (skit-cli run::command mirror overlay): execute_launch runs the plan with plan.env only; the UV_DEFAULT_INDEX / UV_PYTHON_INSTALL_MIRROR overlay is built by mirror_environment (run/command.rs:269) and merged into the child env (run/command.rs:436,499). Oracle: tests/test_launcher.py:257 test_run_entry_injects_mirror_env."]
fn test_run_entry_injects_mirror_env() {
    // Python: run_entry overlays config.mirror_env onto the child; enabled mirror injects both vars.
}

#[test]
#[ignore = "CROSS-CRATE (skit-cli run::command mirror overlay): the disabled-mirror no-op (adds no vars) is a property of mirror_environment (run/command.rs:269), not execute_launch. Oracle: tests/test_launcher.py:301 test_run_entry_no_mirror_env_when_disabled."]
fn test_run_entry_no_mirror_env_when_disabled() {
    // Python: mirror disabled (default) -> the child env gets no mirror variables.
}

#[test]
#[ignore = "CROSS-CRATE (skit-cli run::command mirror overlay): the user's own UV_DEFAULT_INDEX winning over the mirror is enforced by mirror_environment (run/command.rs:269,499). Oracle: tests/test_launcher.py:310 test_run_entry_keeps_user_index_when_mirror_enabled."]
fn test_run_entry_keeps_user_index_when_mirror_enabled() {
    // Python: a user-set UV_DEFAULT_INDEX is preserved while the untouched install mirror is injected.
}

// ---------- target_missing / missing_marker: skit-cli/src/cli.rs:5453 (summary_target) +
// ---------- list_description; skit-tui/src/screens/library.rs:423 ----------

#[test]
#[ignore = "CROSS-CRATE (skit-cli listing tier): target_missing/missing_marker are computed by summary_target (cli.rs:5453) + list_description; skit-runtime has no per-kind target() query. Oracle: tests/test_launcher.py:326 test_target_missing_false_for_healthy_python_entry."]
fn test_target_missing_false_for_healthy_python_entry() {
    // Python: a healthy python entry reports target_missing False and missing_marker None.
}

#[test]
#[ignore = "CROSS-CRATE (skit-cli listing tier): the stored-copy missing marker is built by summary_target (cli.rs:5453) + list_description. Oracle: tests/test_launcher.py:333 test_target_missing_true_when_python_copy_deleted."]
fn test_target_missing_true_when_python_copy_deleted() {
    // Python: a deleted copy-mode script -> target_missing True, marker "⚠ missing: <script_path>".
}

#[test]
#[ignore = "CROSS-CRATE (skit-cli listing tier): reference-mode source missing marker is built by summary_target (cli.rs:5453). Oracle: tests/test_launcher.py:341 test_target_missing_true_when_python_reference_source_deleted."]
fn test_target_missing_true_when_python_reference_source_deleted() {
    // Python: a deleted reference-mode source -> target_missing True, marker names the source.
}

#[test]
#[ignore = "CROSS-CRATE (skit-cli listing tier): exe target missing marker (target IS the source) is built by summary_target (cli.rs:5453). Oracle: tests/test_launcher.py:352 test_target_missing_true_when_exe_deleted."]
fn test_target_missing_true_when_exe_deleted() {
    // Python: a deleted exe source -> target_missing True, marker names the source path.
}

#[test]
#[ignore = "CROSS-CRATE (skit-cli listing tier): command entries have no file target; summary_target (cli.rs:5453) returns None for them. Oracle: tests/test_launcher.py:363 test_target_missing_never_true_for_command_entries."]
fn test_target_missing_never_true_for_command_entries() {
    // Python: command entries never report missing (no file target).
}

// ---------- preflight: the side-effect-free, uv-free validator lives in the skit-cli health
// ---------- adapter (HealthIssueKind::LaunchBlocked). skit-runtime has no faithful equivalent
// ---------- (build_launch_plan is uv-coupled and checks workdir before the script; preview
// ---------- never enforces needs). See the module doc for the full reasoning. ----------

#[test]
#[ignore = "CROSS-CRATE (skit-cli preflight tier): preflight is the uv-free pre-suspend/doctor validator; skit-runtime has no such function. The exe-directory refusal itself is asserted through build_launch_plan in test_exe_directory_source_refused_as_not_executable. Oracle: tests/test_launcher.py:203 test_preflight_refuses_exe_directory_source."]
fn test_preflight_refuses_exe_directory_source() {
    // Python: preflight (the TUI's pre-suspend path) refuses an exe pointing at a directory.
}

#[test]
#[ignore = "CROSS-CRATE (skit-cli preflight tier): a healthy python entry must preflight WITHOUT uv; build_launch_plan requires the uv program, so it cannot stand in. Oracle: tests/test_launcher.py:374 test_preflight_passes_for_healthy_entry."]
fn test_preflight_passes_for_healthy_entry() {
    // Python: preflight(py_entry) must not raise (and must not need uv).
}

#[test]
#[ignore = "CROSS-CRATE (skit-cli preflight tier): preflight raises on a missing python script; the equivalent build check has a different ordering (workdir before script). Oracle: tests/test_launcher.py:380 test_preflight_raises_for_missing_python_script."]
fn test_preflight_raises_for_missing_python_script() {
    // Python: a missing python script -> preflight raises (message matches "script").
}

#[test]
#[ignore = "CROSS-CRATE (skit-cli preflight tier): preflight raises on a missing exe. The build-path equivalent is test_exe_missing_source_raises. Oracle: tests/test_launcher.py:388 test_preflight_raises_for_missing_exe."]
fn test_preflight_raises_for_missing_exe() {
    // Python: a missing exe -> preflight raises (message matches "exe").
}

#[test]
#[ignore = "CROSS-CRATE (skit-cli preflight tier): preflight raises on a missing workdir. The build-path equivalent is test_run_entry_missing_workdir_raises. Oracle: tests/test_launcher.py:399 test_preflight_raises_for_missing_workdir."]
fn test_preflight_raises_for_missing_workdir() {
    // Python: a missing workdir -> preflight raises, naming the path.
}

#[test]
#[ignore = "CROSS-CRATE (skit-cli preflight tier): the defining property of preflight is that it does NOT find or download uv; skit-runtime has no preflight to observe it. Oracle: tests/test_launcher.py:408 test_preflight_does_not_invoke_uv."]
fn test_preflight_does_not_invoke_uv() {
    // Python: preflight must not call ensure_uv (that stays inside the suspended run).
}

#[test]
#[ignore = "CROSS-CRATE (skit-cli preflight tier): a command entry preflights clean (no file target, workdir=\"invoke\" always exists). Oracle: tests/test_launcher.py:419 test_preflight_passes_for_command_entry_without_workdir_or_target_issues."]
fn test_preflight_passes_for_command_entry_without_workdir_or_target_issues() {
    // Python: a command entry preflights without raising.
}

#[test]
#[ignore = "CROSS-CRATE (skit-cli preflight tier): a copy-mode entry with a deleted origin preflights clean; the workdir fallback itself is asserted in test_resolve_workdir_copy_mode_falls_back_when_origin_gone. Oracle: tests/test_launcher.py:463 test_preflight_succeeds_for_copy_mode_entry_with_deleted_origin."]
fn test_preflight_succeeds_for_copy_mode_entry_with_deleted_origin() {
    // Python: preflight succeeds for a copy-mode script whose original directory was deleted.
}

#[test]
#[ignore = "CROSS-CRATE (skit-cli preflight tier): reference mode still raises on a missing script when the origin is gone (not masked); the workdir side is asserted in test_resolve_workdir_reference_mode_not_masked_when_origin_gone. Oracle: tests/test_launcher.py:510 test_preflight_reference_mode_still_raises_on_missing_script_when_origin_gone."]
fn test_preflight_reference_mode_still_raises_on_missing_script_when_origin_gone() {
    // Python: reference mode -> preflight raises "script" when the origin (and script) are gone.
}
