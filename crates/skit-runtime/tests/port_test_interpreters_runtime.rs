//! Runtime-resolution ports from Python `tests/test_interpreters.py` at `main@206f9ef`.
//!
//! Exact Python test names keep the frozen contract count. `rust_additive_*` cases split pytest
//! parameter rows so one early failure cannot hide later language/runtime rows.

use std::{collections::BTreeMap, path::PathBuf};

use skit_application::delivery::Assembly;
use skit_domain::{Entry, EntryKind, EntryMeta, EntrySettings, Slug};
use skit_runtime::{
    LaunchError, LaunchPaths, ProgramProbe, build_launch_plan, resolve_javascript_runtime,
};

#[derive(Debug, Default)]
struct Probe {
    programs: BTreeMap<String, PathBuf>,
    files: Vec<PathBuf>,
    dirs: Vec<PathBuf>,
}

impl Probe {
    fn with_programs(names: &[&str]) -> Self {
        Self {
            programs: names
                .iter()
                .map(|name| ((*name).to_owned(), PathBuf::from(format!("/bin/{name}"))))
                .collect(),
            files: vec![PathBuf::from("/data/scripts/demo/payload")],
            dirs: vec![
                PathBuf::from("/invoke"),
                PathBuf::from("/data/scripts/demo"),
            ],
        }
    }
}

impl ProgramProbe for Probe {
    fn find_program(&self, name: &str) -> Option<PathBuf> {
        self.programs.get(name).cloned()
    }

    fn is_file(&self, path: &std::path::Path) -> bool {
        self.files.iter().any(|item| item == path)
    }

    fn is_dir(&self, path: &std::path::Path) -> bool {
        self.dirs.iter().any(|item| item == path)
    }

    fn is_executable(&self, path: &std::path::Path) -> bool {
        self.is_file(path)
    }
}

fn entry(kind: &str, interpreter: &str) -> Entry {
    let mut entry = Entry {
        slug: Slug::parse("demo").unwrap(),
        meta: EntryMeta::minimal("Demo", EntryKind::parse(kind).unwrap()),
    };
    entry.meta.workdir = "invoke".to_owned();
    EntrySettings {
        interpreter: interpreter.to_owned(),
        ..EntrySettings::default()
    }
    .write_to_meta(&mut entry.meta);
    entry
}

fn paths() -> LaunchPaths {
    LaunchPaths {
        script: PathBuf::from("/data/scripts/demo/payload"),
        entry_dir: PathBuf::from("/data/scripts/demo"),
        invoke_cwd: PathBuf::from("/invoke"),
    }
}

fn plan(kind: &str, interpreter: &str, programs: &[&str]) -> Result<PathBuf, LaunchError> {
    build_launch_plan(
        &entry(kind, interpreter),
        &paths(),
        &Assembly::default(),
        None,
        None,
        &Probe::with_programs(programs),
    )
    .map(|plan| plan.program)
}

fn assert_default(kind: &str, expected: &str) {
    let program = plan(kind, "", &[expected]).unwrap();
    assert_eq!(program, PathBuf::from(format!("/bin/{expected}")));
}

fn assert_js_fallback(available: &[&str], expected: &str) {
    let settings = EntrySettings::default();
    let runtime = resolve_javascript_runtime(&settings, &Probe::with_programs(available)).unwrap();
    assert_eq!(runtime, expected);
}

#[test]
fn test_interpreter_override_is_used() {
    let program = plan("shell", "custom-shell", &["custom-shell"]).unwrap();
    assert_eq!(program, PathBuf::from("/bin/custom-shell"));
}

#[test]
fn test_missing_override_errors() {
    let error = plan("shell", "custom-shell", &[]).unwrap_err();
    assert!(
        matches!(
            &error,
            LaunchError::ProgramNotFound { name } if name == "custom-shell"
        ),
        "override failure resolved as the wrong launch error: {error:?}"
    );
}

#[test]
fn test_default_interpreter_resolution() {
    for (kind, expected) in [
        ("shell", "bash"),
        ("fish", "fish"),
        ("powershell", "pwsh"),
        ("ruby", "ruby"),
        ("perl", "perl"),
        ("lua", "lua"),
        ("r", "Rscript"),
    ] {
        assert_default(kind, expected);
    }
}

#[test]
fn rust_additive_default_interpreter_shell() {
    assert_default("shell", "bash");
}

#[test]
fn rust_additive_default_interpreter_fish() {
    assert_default("fish", "fish");
}

#[test]
fn rust_additive_default_interpreter_powershell() {
    assert_default("powershell", "pwsh");
}

#[test]
fn rust_additive_default_interpreter_ruby() {
    assert_default("ruby", "ruby");
}

#[test]
fn rust_additive_default_interpreter_perl() {
    assert_default("perl", "perl");
}

#[test]
fn rust_additive_default_interpreter_lua() {
    assert_default("lua", "lua");
}

#[test]
fn rust_additive_default_interpreter_r() {
    assert_default("r", "Rscript");
}

#[test]
fn test_python_always_resolves_uv() {
    let program = plan("python", "", &["uv"]).unwrap();
    assert_eq!(program, PathBuf::from("/bin/uv"));
}

#[test]
fn test_js_runner_pinned() {
    let settings = EntrySettings {
        interpreter: "bun".to_owned(),
        ..EntrySettings::default()
    };
    let runtime =
        resolve_javascript_runtime(&settings, &Probe::with_programs(&["bun", "node"])).unwrap();
    assert_eq!(runtime, "bun");
}

#[test]
fn test_js_runner_fallback_order() {
    for (available, expected) in [
        (&["deno", "bun", "node"][..], "deno"),
        (&["bun", "node"][..], "bun"),
        (&["node"][..], "node"),
    ] {
        assert_js_fallback(available, expected);
    }
}

#[test]
fn rust_additive_js_runner_fallback_deno_first() {
    assert_js_fallback(&["deno", "bun", "node"], "deno");
}

#[test]
fn rust_additive_js_runner_fallback_bun_before_node() {
    assert_js_fallback(&["bun", "node"], "bun");
}

#[test]
fn rust_additive_js_runner_fallback_node_only() {
    assert_js_fallback(&["node"], "node");
}

#[test]
fn test_js_runner_missing_errors() {
    let error = resolve_javascript_runtime(&EntrySettings::default(), &Probe::default()).unwrap_err();
    assert!(matches!(&error, LaunchError::ProgramNotFound { .. }));
    // v0.4 gives this user-facing diagnosis. Keep it even if the current Rust wording is red.
    assert!(
        error.to_string().contains("No JavaScript runtime"),
        "missing-runtime wording drifted from v0.4: {error}"
    );
}

#[test]
fn test_js_runner_entry_pin_missing_does_not_fall_back() {
    let settings = EntrySettings {
        interpreter: "deno".to_owned(),
        ..EntrySettings::default()
    };
    let error =
        resolve_javascript_runtime(&settings, &Probe::with_programs(&["node"])).unwrap_err();
    assert!(
        matches!(&error, LaunchError::ProgramNotFound { name } if name == "deno"),
        "missing entry pin silently fell back: {error:?}"
    );
}
