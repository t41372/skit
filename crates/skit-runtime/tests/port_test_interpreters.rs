//! Mechanical port of the Python oracle module `tests/test_interpreters.py`
//! (`origin/main@206f9ef`): "Tier-0 multi-language launch: interpreter resolution, the
//! InterpreterLaunch / RunnerLaunch strategies, shebang sniffing + kind inference, the
//! `needs` preflight contract, and the CLI surfaces the feature adds". Each `#[test]`
//! keeps its Python `def test_*` name and its WHY comment.
//!
//! Concept mapping used throughout (the Rust rewrite folds the per-family strategy
//! classes into one `build_launch_plan` match, so a "build a payload and assert its
//! argv" test drives that entry point instead of a strategy object):
//! - Python `launch._which(name)` (the PATH seam every strategy funnels through) ->
//!   the `ProgramProbe` trait; tests inject a `FakeProbe` whose `programs` map IS the
//!   `_which_map`. `SystemProbe` is the real `shutil.which` seam.
//! - Python `launch.resolve_interpreter(name)` -> `require_program`, reached by building
//!   an interpreted plan; a missing interpreter is `LaunchError::ProgramNotFound` (exit
//!   126, the `NotExecutableError` twin).
//! - Python `launch.InterpreterLaunch(default, prefix).build(...)` -> `build_launch_plan`
//!   for an interpreted kind; `.describe(...)` -> `build_launch_preview` (no PATH lookup);
//!   `.preflight(...)` -> `build_launch_plan` (a superset of the existence/resolve checks).
//! - Python `launch.RunnerLaunch().build(...)` -> `build_launch_plan` for "js"/"ts";
//!   `resolve_javascript_runtime` is the deno>bun>node order + pin override.
//! - Python `launcher.preflight` / `run_entry` needs gate -> `build_launch_plan`'s
//!   `MissingNeed` (both are pre-spawn).
//!
//! Bucket disposition:
//! - REAL (18 asserting, pass): the resolve/InterpreterLaunch/RunnerLaunch/needs cases that
//!   the skit-runtime launch surface owns.
//! - ARCHITECTURE CLOSURE (1 `#[ignore]`): the path-reading unreadable-source wrapper is split
//!   between caller-owned I/O and skit-language's text parser.
//! - REHOMED (55): detection, config composition, store projections, and CLI E2E contracts run at
//!   their executable owners. This file keeps no cross-crate stub or known divergence. The port
//!   ledger records every stronger owner and exact rehome.

use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

use skit_application::delivery::Assembly;
use skit_domain::{Entry, EntryKind, EntryMeta, EntrySettings, Slug};
use skit_runtime::{
    LaunchError, LaunchPaths, ProgramProbe, SystemProbe, build_launch_plan, build_launch_preview,
};

/// A fake `_which` + filesystem probe. `programs` IS the oracle's `_which_map`: a present
/// name resolves to its path, an absent one is missing. `panic_on_find_program` proves a
/// describe/ordering path never touches PATH — the oracle's `boom` seam.
#[derive(Debug, Default)]
struct FakeProbe {
    programs: BTreeMap<String, PathBuf>,
    files: Vec<PathBuf>,
    dirs: Vec<PathBuf>,
    executable: Vec<PathBuf>,
    panic_on_find_program: bool,
}

impl ProgramProbe for FakeProbe {
    fn find_program(&self, name: &str) -> Option<PathBuf> {
        assert!(
            !self.panic_on_find_program,
            "find_program must not touch PATH here: {name}"
        );
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

/// The oracle `_entry(tmp_path, kind, ...)`: a minimal entry of `kind`. The default meta is
/// copy mode with an "origin" workdir and an empty source, so the launch cwd resolves to the
/// invoke directory (kept a directory by `probe_for`).
fn entry(kind: &str) -> Entry {
    Entry {
        slug: Slug::parse("demo").unwrap(),
        meta: EntryMeta::minimal("Demo", EntryKind::parse(kind).unwrap()),
    }
}

fn paths(script: &str) -> LaunchPaths {
    LaunchPaths {
        script: PathBuf::from(script),
        entry_dir: PathBuf::from("/data/scripts/demo"),
        invoke_cwd: PathBuf::from("/invoke"),
    }
}

/// A probe where the script exists and both candidate workdirs are directories, so only the
/// interpreter/runtime/needs lookups a test adds decide the outcome.
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

/// Reconstruct the full oracle argv (`_argv` = program then args) from a built plan.
fn full_argv(program: &Path, args: &[String]) -> Vec<String> {
    let mut argv = vec![program.display().to_string()];
    argv.extend(args.iter().cloned());
    argv
}

// ==========================================================================
// resolve_interpreter
// ==========================================================================

#[test]
fn test_resolve_interpreter_found_on_path() {
    // A present interpreter resolves to its PATH location.
    let script = "/data/scripts/demo/script.sh";
    let mut probe = probe_for(script);
    probe
        .programs
        .insert("bash".to_owned(), PathBuf::from("/usr/bin/bash"));
    let plan = build_launch_plan(
        &entry("shell"),
        &paths(script),
        &Assembly::default(),
        None,
        None,
        &probe,
    )
    .unwrap();
    assert_eq!(plan.program, PathBuf::from("/usr/bin/bash"));
}

#[test]
fn test_resolve_interpreter_missing_posix_names_the_interpreter() {
    // A missing interpreter is a clean 126 refusal that names the interpreter.
    let script = "/data/scripts/demo/script.sh";
    let mut shell = entry("shell");
    EntrySettings {
        interpreter: "zsh".to_owned(),
        ..EntrySettings::default()
    }
    .write_to_meta(&mut shell.meta);
    let probe = probe_for(script); // no interpreters resolve
    let error = build_launch_plan(
        &shell,
        &paths(script),
        &Assembly::default(),
        None,
        None,
        &probe,
    )
    .unwrap_err();
    assert!(matches!(&error, LaunchError::ProgramNotFound { name } if name == "zsh"));
    assert!(error.to_string().contains("zsh"));
}

#[test]
fn test_which_seam_is_the_real_shutil_which() {
    // The real PATH-lookup seam: a name that cannot exist resolves to None (proves it
    // delegates to the OS, not a stub).
    let probe = SystemProbe;
    assert!(
        probe
            .find_program("skit-definitely-not-a-real-binary-zzz")
            .is_none()
    );
}

// ==========================================================================
// InterpreterLaunch
// ==========================================================================

#[test]
fn test_interpreter_launch_builds_argv() {
    // `<interpreter> <script> <args>`.
    let script = "/data/scripts/demo/script.sh";
    let mut probe = probe_for(script);
    probe
        .programs
        .insert("bash".to_owned(), PathBuf::from("/bin/bash"));
    let assembly = Assembly {
        args: vec!["--fast".to_owned()],
        masked_args: vec!["--fast".to_owned()],
        ..Assembly::default()
    };
    let plan = build_launch_plan(
        &entry("shell"),
        &paths(script),
        &assembly,
        None,
        None,
        &probe,
    )
    .unwrap();
    assert_eq!(plan.program, PathBuf::from("/bin/bash"));
    assert_eq!(plan.args, [script, "--fast"]);
}

#[test]
fn test_interpreter_launch_meta_interpreter_beats_default() {
    // A pinned interpreter (the script kept its #!/bin/zsh dialect) outranks the default.
    let script = "/data/scripts/demo/script.sh";
    let mut shell = entry("shell");
    EntrySettings {
        interpreter: "zsh".to_owned(),
        ..EntrySettings::default()
    }
    .write_to_meta(&mut shell.meta);
    let mut probe = probe_for(script);
    probe
        .programs
        .insert("zsh".to_owned(), PathBuf::from("/bin/zsh"));
    let plan = build_launch_plan(
        &shell,
        &paths(script),
        &Assembly::default(),
        None,
        None,
        &probe,
    )
    .unwrap();
    assert_eq!(plan.program, PathBuf::from("/bin/zsh"));
}

#[test]
fn test_interpreter_launch_prefix_placement() {
    // The prefix (`-File`) sits between the interpreter and the script (PowerShell file
    // semantics).
    let script = "/data/scripts/demo/script.ps1";
    let mut probe = probe_for(script);
    probe
        .programs
        .insert("pwsh".to_owned(), PathBuf::from("/usr/bin/pwsh"));
    let plan = build_launch_plan(
        &entry("powershell"),
        &paths(script),
        &Assembly::default(),
        None,
        None,
        &probe,
    )
    .unwrap();
    assert_eq!(plan.program, PathBuf::from("/usr/bin/pwsh"));
    assert_eq!(plan.args, ["-File", script]);
}

#[test]
fn test_interpreter_launch_describe_is_side_effect_free() {
    // Describe uses the bare interpreter name with no PATH lookup (the preview probe never
    // consults the real `find_program`, so a panicking seam proves the point).
    let script = "/data/scripts/demo/script.sh";
    let mut probe = probe_for(script);
    probe.panic_on_find_program = true;
    let assembly = Assembly {
        args: vec!["-x".to_owned()],
        masked_args: vec!["-x".to_owned()],
        ..Assembly::default()
    };
    let plan = build_launch_preview(
        &entry("shell"),
        &paths(script),
        &assembly,
        None,
        None,
        None,
        &probe,
    )
    .unwrap();
    assert!(plan.display.starts_with("bash ")); // the bare name stands in
    assert!(plan.display.contains("-x"));
}

#[test]
fn test_interpreter_launch_preflight_missing_interpreter() {
    // Preflight refuses when the interpreter cannot be resolved.
    let script = "/data/scripts/demo/script.sh";
    let probe = probe_for(script); // script present, no bash
    let error = build_launch_plan(
        &entry("shell"),
        &paths(script),
        &Assembly::default(),
        None,
        None,
        &probe,
    )
    .unwrap_err();
    assert!(matches!(error, LaunchError::ProgramNotFound { .. }));
}

#[test]
fn test_interpreter_launch_preflight_ok() {
    // Script exists + interpreter resolves -> no refusal.
    let script = "/data/scripts/demo/script.sh";
    let mut probe = probe_for(script);
    probe
        .programs
        .insert("bash".to_owned(), PathBuf::from("/bin/bash"));
    assert!(
        build_launch_plan(
            &entry("shell"),
            &paths(script),
            &Assembly::default(),
            None,
            None,
            &probe
        )
        .is_ok()
    );
}

#[test]
fn test_interpreter_launch_missing_script_raises_before_resolution() {
    // A missing script is TargetMissing (exit 127) BEFORE the interpreter is resolved (the
    // panicking find_program proves no resolution happens).
    let script = "/data/scripts/demo/script.sh";
    let mut probe = probe_for(script);
    probe.files.clear(); // script is gone
    probe.panic_on_find_program = true;
    let error = build_launch_plan(
        &entry("shell"),
        &paths(script),
        &Assembly::default(),
        None,
        None,
        &probe,
    )
    .unwrap_err();
    assert!(matches!(error, LaunchError::TargetMissing { .. }));
}

// ==========================================================================
// RunnerLaunch
// ==========================================================================

#[test]
fn test_runner_detection_order_prefers_deno() {
    // deno > bun > node: with all present, deno wins and gets `run --allow-all`.
    let script = "/data/scripts/demo/script.js";
    let mut probe = probe_for(script);
    probe
        .programs
        .insert("deno".to_owned(), PathBuf::from("/d"));
    probe.programs.insert("bun".to_owned(), PathBuf::from("/b"));
    probe
        .programs
        .insert("node".to_owned(), PathBuf::from("/n"));
    let plan = build_launch_plan(
        &entry("js"),
        &paths(script),
        &Assembly::default(),
        None,
        None,
        &probe,
    )
    .unwrap();
    assert_eq!(plan.program, PathBuf::from("/d"));
    assert_eq!(plan.args, ["run", "--allow-all", script]);
}

#[test]
fn test_runner_falls_to_bun_then_node() {
    let script = "/data/scripts/demo/script.js";
    let mut bun_probe = probe_for(script);
    bun_probe
        .programs
        .insert("bun".to_owned(), PathBuf::from("/b"));
    bun_probe
        .programs
        .insert("node".to_owned(), PathBuf::from("/n"));
    let plan = build_launch_plan(
        &entry("js"),
        &paths(script),
        &Assembly::default(),
        None,
        None,
        &bun_probe,
    )
    .unwrap();
    assert_eq!(full_argv(&plan.program, &plan.args), ["/b", "run", script]);

    let mut node_probe = probe_for(script);
    node_probe
        .programs
        .insert("node".to_owned(), PathBuf::from("/n"));
    let plan = build_launch_plan(
        &entry("js"),
        &paths(script),
        &Assembly::default(),
        None,
        None,
        &node_probe,
    )
    .unwrap();
    // node takes no "run" subcommand.
    assert_eq!(full_argv(&plan.program, &plan.args), ["/n", script]);
}

#[test]
fn test_runner_meta_interpreter_override() {
    // A pinned runtime overrides the detection order outright.
    let script = "/data/scripts/demo/script.js";
    let mut js = entry("js");
    EntrySettings {
        interpreter: "bun".to_owned(),
        ..EntrySettings::default()
    }
    .write_to_meta(&mut js.meta);
    let mut probe = probe_for(script);
    probe
        .programs
        .insert("deno".to_owned(), PathBuf::from("/d"));
    probe.programs.insert("bun".to_owned(), PathBuf::from("/b"));
    probe
        .programs
        .insert("node".to_owned(), PathBuf::from("/n"));
    let plan = build_launch_plan(
        &js,
        &paths(script),
        &Assembly::default(),
        None,
        None,
        &probe,
    )
    .unwrap();
    assert_eq!(plan.program, PathBuf::from("/b"));
}

#[test]
fn test_runner_none_installed_names_candidates_and_config_key() {
    let script = "/data/scripts/demo/script.js";
    let probe = probe_for(script); // no runtimes resolve
    let error = build_launch_plan(
        &entry("js"),
        &paths(script),
        &Assembly::default(),
        None,
        None,
        &probe,
    )
    .unwrap_err();
    let message = error.to_string();
    for candidate in ["deno", "bun", "node"] {
        assert!(message.contains(candidate), "{message}");
    }
    assert!(message.contains("skit config js.runner"), "{message}");
}

#[test]
fn test_runner_describe_uses_preferred_name_without_path_lookup() {
    // Describe shows ORDER[0] (deno run ...) with no PATH lookup.
    let script = "/data/scripts/demo/script.js";
    let mut probe = probe_for(script);
    probe.panic_on_find_program = true;
    let plan = build_launch_preview(
        &entry("js"),
        &paths(script),
        &Assembly::default(),
        None,
        None,
        None,
        &probe,
    )
    .unwrap();
    assert!(plan.display.starts_with("deno run "));
}

#[test]
fn test_runner_preflight_checks_script_and_runner() {
    // Preflight refuses when no JS runtime is installed.
    let script = "/data/scripts/demo/script.js";
    let probe = probe_for(script);
    let error = build_launch_plan(
        &entry("js"),
        &paths(script),
        &Assembly::default(),
        None,
        None,
        &probe,
    )
    .unwrap_err();
    // The oracle raises the NotExecutableError family (exit 126); the typed Rust
    // refusal for a JS entry is the runtime-missing error with the config hint.
    assert!(matches!(error, LaunchError::JsRuntimeMissing { .. }));
    assert_eq!(error.exit_code(), 126);
}

// ==========================================================================
// shebang_program + infer_kind  (owned by skit-language)
// ==========================================================================
//
// skit-runtime does not depend on skit-language, so every shebang/kind-inference case is a
#[test]
#[ignore = "ARCHITECTURE CLOSURE (skit-language): Python's shebang_program reads a Path and maps \
OSError to None. Rust's public shebang_program deliberately parses one already-read line; source \
I/O belongs to the caller. There is no public combined path-reading seam to drive faithfully."]
fn test_shebang_none_when_unreadable() {}

// ==========================================================================
// needs — preflight / run / missing_needs
// ==========================================================================

#[test]
fn test_preflight_needs_lists_only_missing() {
    // A satisfied requirement is never named; the refusal points at the missing tool.
    let script = "/data/scripts/demo/script.sh";
    let mut shell = entry("shell");
    EntrySettings {
        needs: vec!["jq".to_owned(), "ffmpeg".to_owned()],
        ..EntrySettings::default()
    }
    .write_to_meta(&mut shell.meta);
    let mut probe = probe_for(script);
    probe
        .programs
        .insert("bash".to_owned(), PathBuf::from("/usr/bin/bash"));
    probe
        .programs
        .insert("jq".to_owned(), PathBuf::from("/usr/bin/jq")); // ffmpeg absent
    let error = build_launch_plan(
        &shell,
        &paths(script),
        &Assembly::default(),
        None,
        None,
        &probe,
    )
    .unwrap_err();
    let message = error.to_string();
    assert!(message.contains("ffmpeg"));
    assert!(!message.contains("jq")); // a satisfied requirement is never named
}

#[test]
fn test_run_entry_needs_raises_before_spawn() {
    // The needs gate is a pre-spawn refusal (build_launch_plan happens before any spawn).
    let script = "/data/scripts/demo/script.sh";
    let mut shell = entry("shell");
    EntrySettings {
        needs: vec!["ffmpeg".to_owned()],
        ..EntrySettings::default()
    }
    .write_to_meta(&mut shell.meta);
    let mut probe = probe_for(script);
    probe
        .programs
        .insert("jq".to_owned(), PathBuf::from("/usr/bin/jq")); // ffmpeg absent
    let error = build_launch_plan(
        &shell,
        &paths(script),
        &Assembly::default(),
        None,
        None,
        &probe,
    )
    .unwrap_err();
    assert!(matches!(&error, LaunchError::MissingNeed { name } if name == "ffmpeg"));
    assert!(error.to_string().contains("ffmpeg"));
}

// ==========================================================================
// E2E (POSIX): the overlay reaches a real child  (owned by skit-cli-rs)
// ==========================================================================
