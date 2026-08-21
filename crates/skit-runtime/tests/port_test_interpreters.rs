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
//! - REAL (asserting, pass): the resolve/InterpreterLaunch/RunnerLaunch/needs cases that
//!   the skit-runtime launch surface actually owns.
//! - DIVERGENCE (`#[ignore]`, full body kept): two oracle contracts the Rust code breaks —
//!   `RunnerLaunch` gives bun a `run` subcommand (Rust omits it), and the "no JS runtime"
//!   refusal names the `skit config js.runner` escape hatch (Rust drops it).
//! - CROSS-CRATE (`#[ignore]` stub): behavior owned by another tier this integration test
//!   cannot reach without a forbidden dependency edit — the path-reading shebang wrapper,
//!   the `js.runner` / Windows `shell.bash_path` config resolution, and every `CliRunner`
//!   surface (`skit-cli-rs` / `skit-store`). Store and Library projection stubs move to their
//!   executable stronger owners, as recorded in the port ledger.
//!   The pure shebang and kind-inference contracts run at their public `skit-language` seams.

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
#[ignore = "CROSS-CRATE (skit-cli-rs / skit-store): the Windows bash escape hatch resolves \
bash from the `shell.bash_path` config; the skit-runtime launch surface takes an already-resolved \
interpreter name and has no config fallback. Config lives in skit-store::config and is read at the \
skit-cli tier (cli.rs `shell.bash_path`). Platform is a compile-time cfg here, not runtime-patchable."]
fn test_resolve_bash_on_win32_uses_config_path_when_it_exists() {}

#[test]
#[ignore = "CROSS-CRATE (skit-cli-rs / skit-store): Windows bash resolution + its \"configured but \
missing\" fall-through live at the config/CLI tier, not the skit-runtime launch surface."]
fn test_resolve_bash_on_win32_configured_but_missing_falls_through() {}

#[test]
#[ignore = "CROSS-CRATE (skit-cli-rs / skit-store): the win32 refusal that names both escape hatches \
(\"Git for Windows\" + \"skit config shell.bash_path\") belongs to the config/CLI tier. Verify that \
exact wording there; the skit-runtime ProgramNotFound message carries neither."]
fn test_resolve_bash_on_win32_unset_names_both_escape_hatches() {}

#[test]
#[ignore = "CROSS-CRATE (skit-cli-rs / skit-store): the win32-only distinction (only bash/sh/zsh get \
the escape hatch, ruby gets the generic message) does not exist in the runtime tier, which has no \
per-platform escape hatch for any interpreter."]
fn test_resolve_nonbash_on_win32_gets_generic_message() {}

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
#[ignore = "CROSS-CRATE (skit-store / skit-cli-rs): the `js.runner` config override (used when the \
entry has no pin) is read from skit-store::config and resolved at the skit-cli run tier \
(run/command.rs `js.runner`); skit-runtime `resolve_javascript_runtime` consults only the entry pin, \
so the config layer cannot be injected through this surface."]
fn test_runner_config_override() {}

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
// CLI: add --kind  (owned by skit-cli-rs)
// ==========================================================================

#[test]
#[ignore = "CROSS-CRATE (skit-cli-rs): `skit add` shebang/interpreter capture is a CliRunner \
surface owned by skit-cli-rs."]
fn test_cli_add_shell_script_records_interpreter() {}

#[test]
#[ignore = "CROSS-CRATE (skit-cli-rs): `skit add --kind shell` forcing an extensionless file is a \
CliRunner surface owned by skit-cli-rs."]
fn test_cli_add_kind_forces_extensionless_file() {}

#[test]
#[ignore = "CROSS-CRATE (skit-cli-rs): `skit add --kind exe` is a CliRunner surface owned by \
skit-cli-rs."]
fn test_cli_add_kind_exe() {}

#[test]
#[ignore = "CROSS-CRATE (skit-cli-rs): `skit add --kind cobol` usage error (exit 2, lists valid \
kinds) is a Clap surface owned by skit-cli-rs."]
fn test_cli_add_kind_unknown_is_usage_error() {}

#[test]
#[ignore = "CROSS-CRATE (skit-cli-rs): the `--kind` + `--exe` conflict (exit 2) is a Clap surface \
owned by skit-cli-rs."]
fn test_cli_add_kind_and_exe_conflict() {}

#[test]
#[ignore = "CROSS-CRATE (skit-cli-rs): `--kind command` rejection is a Clap surface owned by \
skit-cli-rs."]
fn test_cli_add_command_kind_rejected() {}

// ==========================================================================
// CLI: deps --need / --clear-needs / read view  (owned by skit-cli-rs)
// ==========================================================================

#[test]
#[ignore = "CROSS-CRATE (skit-cli-rs): `skit deps --need` replacing the whole list is a CliRunner \
surface owned by skit-cli-rs."]
fn test_deps_need_replaces_whole_list() {}

#[test]
#[ignore = "CROSS-CRATE (skit-cli-rs): the `--need` + `--clear-needs` conflict (exit 2, \"not \
both\") is a Clap surface owned by skit-cli-rs."]
fn test_deps_need_and_clear_needs_conflict() {}

#[test]
#[ignore = "CROSS-CRATE (skit-cli-rs): `skit deps --need` on a python entry is a CliRunner surface \
owned by skit-cli-rs."]
fn test_deps_need_works_on_python_too() {}

#[test]
#[ignore = "CROSS-CRATE (skit-cli-rs): `skit deps --dep` refusal on shell (exit 2, \"doesn't take \
package dependencies\") is a CliRunner surface owned by skit-cli-rs."]
fn test_deps_dep_on_shell_is_refused() {}

#[test]
#[ignore = "CROSS-CRATE (skit-cli-rs): the `skit deps` read view showing needs is a CliRunner \
surface owned by skit-cli-rs."]
fn test_deps_read_view_shows_needs_for_shell() {}

#[test]
#[ignore = "CROSS-CRATE (skit-cli-rs): `skit deps --json` needs field is a CliRunner surface owned \
by skit-cli-rs."]
fn test_deps_json_view_includes_needs() {}

#[test]
#[ignore = "CROSS-CRATE (skit-cli-rs): the empty-needs dash in the `skit deps` read view is a \
CliRunner surface owned by skit-cli-rs."]
fn test_deps_read_view_needs_dash_when_empty() {}

// ==========================================================================
// CLI: doctor / show needs surfaces  (owned by skit-cli-rs)
// ==========================================================================

#[test]
#[ignore = "CROSS-CRATE (skit-cli-rs): `skit doctor` flagging missing needs is a CliRunner surface \
owned by skit-cli-rs."]
fn test_doctor_flags_missing_needs() {}

#[test]
#[ignore = "CROSS-CRATE (skit-cli-rs): `skit doctor --json` `needs_missing` map is a CliRunner \
surface owned by skit-cli-rs."]
fn test_doctor_json_needs_missing() {}

#[test]
#[ignore = "CROSS-CRATE (skit-cli-rs): `skit show` printing the Needs line is a CliRunner surface \
owned by skit-cli-rs."]
fn test_show_human_prints_needs_line() {}

#[test]
#[ignore = "CROSS-CRATE (skit-cli-rs): `skit show --json` needs field is a CliRunner surface owned \
by skit-cli-rs."]
fn test_show_json_includes_needs() {}

#[test]
#[ignore = "CROSS-CRATE (skit-cli-rs): the `skit show` interpreted header/Source/run-hint is a \
CliRunner surface owned by skit-cli-rs."]
fn test_show_interpreted_header_and_source() {}

// ==========================================================================
// CLI: edit refusal is kind-neutral  (owned by skit-cli-rs)
// ==========================================================================

#[test]
#[ignore = "CROSS-CRATE (skit-cli-rs): `skit edit` on an exe (exit 1, \"no editable source\") is a \
CliRunner surface owned by skit-cli-rs."]
fn test_edit_program_refusal_is_kind_neutral() {}

#[test]
#[ignore = "CROSS-CRATE (skit-cli-rs): `skit edit` on a command (exit 1, \"no editable source\") is \
a CliRunner surface owned by skit-cli-rs."]
fn test_edit_command_refusal_is_kind_neutral() {}

// ==========================================================================
// E2E (POSIX): the overlay reaches a real child  (owned by skit-cli-rs)
// ==========================================================================

#[test]
#[ignore = "CROSS-CRATE (skit-cli-rs): the `skit run` shell E2E is a CliRunner surface owned by \
skit-cli-rs (skit-runtime's `execute_launch` real-spawn path is covered by its own private tests)."]
fn test_e2e_run_shell_script() {}

#[test]
#[ignore = "CROSS-CRATE (skit-cli-rs): the env-overlay-reaches-child E2E is a CliRunner surface \
owned by skit-cli-rs."]
fn test_e2e_run_shell_env_param_reaches_child() {}

#[test]
#[ignore = "CROSS-CRATE (skit-cli-rs): the `skit run --dry-run` transparency E2E is a CliRunner \
surface owned by skit-cli-rs."]
fn test_e2e_dry_run_shows_interpreter_and_script() {}

#[test]
#[ignore = "CROSS-CRATE (skit-cli-rs): the reference-mode shell run E2E is a CliRunner surface \
owned by skit-cli-rs."]
fn test_e2e_run_reference_mode_shell() {}
