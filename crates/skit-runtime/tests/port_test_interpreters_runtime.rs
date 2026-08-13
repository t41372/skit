//! Runtime-resolution ports from Python `tests/test_interpreters.py` at `main@206f9ef`.
//!
//! Exact Python test names keep the frozen contract count. `rust_additive_*` cases split multi-case
//! assertions so one early failure cannot hide later interpreter/runtime paths.

use std::{collections::BTreeMap, path::PathBuf};

use skit_application::delivery::Assembly;
use skit_domain::{Entry, EntryKind, EntryMeta, EntrySettings, Slug};
use skit_runtime::{
    LaunchError, LaunchPaths, ProgramProbe, build_launch_plan, build_launch_preview,
    resolve_javascript_runtime,
};

const SCRIPT: &str = "/data/scripts/demo/payload";

#[derive(Debug)]
struct Probe {
    programs: BTreeMap<String, PathBuf>,
    script_present: bool,
    panic_on_program_lookup: bool,
}

impl Default for Probe {
    fn default() -> Self {
        Self {
            programs: BTreeMap::new(),
            script_present: true,
            panic_on_program_lookup: false,
        }
    }
}

impl Probe {
    fn with_programs(names: &[&str]) -> Self {
        Self {
            programs: names
                .iter()
                .map(|name| ((*name).to_owned(), PathBuf::from(format!("/usr/bin/{name}"))))
                .collect(),
            ..Self::default()
        }
    }

    fn without_script(mut self) -> Self {
        self.script_present = false;
        self
    }

    fn panic_on_program_lookup(mut self) -> Self {
        self.panic_on_program_lookup = true;
        self
    }
}

impl ProgramProbe for Probe {
    fn find_program(&self, name: &str) -> Option<PathBuf> {
        assert!(
            !self.panic_on_program_lookup,
            "program lookup happened before the contract allowed it: {name}"
        );
        self.programs.get(name).cloned()
    }

    fn is_file(&self, path: &std::path::Path) -> bool {
        self.script_present && path == std::path::Path::new(SCRIPT)
    }

    fn is_dir(&self, path: &std::path::Path) -> bool {
        matches!(path.to_str(), Some("/invoke" | "/data/scripts/demo"))
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
        script: PathBuf::from(SCRIPT),
        entry_dir: PathBuf::from("/data/scripts/demo"),
        invoke_cwd: PathBuf::from("/invoke"),
    }
}

fn assembly(args: &[&str]) -> Assembly {
    Assembly {
        args: args.iter().map(|value| (*value).to_owned()).collect(),
        masked_args: args.iter().map(|value| (*value).to_owned()).collect(),
        ..Assembly::default()
    }
}

fn assert_runner(available: &[&str], expected: &str) {
    let plan = build_launch_plan(
        &entry("js", ""),
        &paths(),
        &Assembly::default(),
        None,
        None,
        &Probe::with_programs(available),
    )
    .unwrap();
    assert_eq!(plan.program, PathBuf::from(format!("/usr/bin/{expected}")));
}

#[test]
fn test_resolve_interpreter_found_on_path() {
    let plan = build_launch_plan(
        &entry("shell", ""),
        &paths(),
        &Assembly::default(),
        None,
        None,
        &Probe::with_programs(&["bash"]),
    )
    .unwrap();
    assert_eq!(plan.program, PathBuf::from("/usr/bin/bash"));
}

#[cfg(not(windows))]
#[test]
fn test_resolve_interpreter_missing_posix_names_the_interpreter() {
    let error = build_launch_plan(
        &entry("shell", ""),
        &paths(),
        &Assembly::default(),
        None,
        None,
        &Probe::default(),
    )
    .unwrap_err();
    assert!(matches!(&error, LaunchError::ProgramNotFound { name } if name == "bash"));
    let message = error.to_string();
    assert!(message.contains("bash"), "{message}");
    assert!(!message.contains("Git for Windows"), "{message}");
}

#[test]
fn test_interpreter_launch_builds_argv() {
    let plan = build_launch_plan(
        &entry("shell", ""),
        &paths(),
        &assembly(&["--fast"]),
        None,
        None,
        &Probe::with_programs(&["bash"]),
    )
    .unwrap();
    assert_eq!(plan.program, PathBuf::from("/usr/bin/bash"));
    assert_eq!(plan.args, vec![SCRIPT.to_owned(), "--fast".to_owned()]);
}

#[test]
fn test_interpreter_launch_meta_interpreter_beats_default() {
    let plan = build_launch_plan(
        &entry("shell", "zsh"),
        &paths(),
        &Assembly::default(),
        None,
        None,
        &Probe::with_programs(&["bash", "zsh"]),
    )
    .unwrap();
    assert_eq!(plan.program, PathBuf::from("/usr/bin/zsh"));
}

#[test]
fn test_interpreter_launch_prefix_placement() {
    let plan = build_launch_plan(
        &entry("powershell", ""),
        &paths(),
        &assembly(&["arg1"]),
        None,
        None,
        &Probe::with_programs(&["pwsh"]),
    )
    .unwrap();
    assert_eq!(plan.program, PathBuf::from("/usr/bin/pwsh"));
    assert_eq!(
        plan.args,
        vec!["-File".to_owned(), SCRIPT.to_owned(), "arg1".to_owned()]
    );
}

#[test]
fn test_interpreter_launch_describe_is_side_effect_free() {
    let preview = build_launch_preview(
        &entry("shell", ""),
        &paths(),
        &assembly(&["--flag"]),
        None,
        None,
        None,
        &Probe::default().panic_on_program_lookup(),
    )
    .unwrap();
    assert_eq!(preview.program, PathBuf::from("bash"));
    assert_eq!(preview.args, vec![SCRIPT.to_owned(), "--flag".to_owned()]);
    assert!(preview.display.contains("bash"), "{}", preview.display);
    assert!(preview.display.contains(SCRIPT), "{}", preview.display);
}

#[test]
fn test_interpreter_launch_preflight_missing_interpreter() {
    let error = build_launch_plan(
        &entry("shell", ""),
        &paths(),
        &Assembly::default(),
        None,
        None,
        &Probe::default(),
    )
    .unwrap_err();
    assert!(matches!(&error, LaunchError::ProgramNotFound { name } if name == "bash"));
}

#[test]
fn test_interpreter_launch_preflight_ok() {
    let plan = build_launch_plan(
        &entry("shell", ""),
        &paths(),
        &Assembly::default(),
        None,
        None,
        &Probe::with_programs(&["bash"]),
    )
    .unwrap();
    assert_eq!(plan.program, PathBuf::from("/usr/bin/bash"));
}

#[test]
fn test_interpreter_launch_missing_script_raises_before_resolution() {
    let error = build_launch_plan(
        &entry("shell", ""),
        &paths(),
        &Assembly::default(),
        None,
        None,
        &Probe::default()
            .without_script()
            .panic_on_program_lookup(),
    )
    .unwrap_err();
    assert!(matches!(&error, LaunchError::TargetMissing { .. }));
}

#[test]
fn test_runner_detection_order_prefers_deno() {
    assert_runner(&["node", "bun", "deno"], "deno");
}

#[test]
fn test_runner_falls_to_bun_then_node() {
    assert_runner(&["bun", "node"], "bun");
    assert_runner(&["node"], "node");
}

#[test]
fn rust_additive_runner_falls_to_bun_before_node() {
    assert_runner(&["bun", "node"], "bun");
}

#[test]
fn rust_additive_runner_falls_to_node_when_bun_missing() {
    assert_runner(&["node"], "node");
}

#[test]
fn test_runner_meta_interpreter_override() {
    let plan = build_launch_plan(
        &entry("js", "node"),
        &paths(),
        &Assembly::default(),
        None,
        None,
        &Probe::with_programs(&["deno", "node"]),
    )
    .unwrap();
    assert_eq!(plan.program, PathBuf::from("/usr/bin/node"));
}

#[test]
fn test_runner_none_installed_names_candidates_and_config_key() {
    let error = resolve_javascript_runtime(&EntrySettings::default(), &Probe::default()).unwrap_err();
    let message = error.to_string();
    assert!(message.contains("No JavaScript runtime found"), "{message}");
    assert!(message.contains("deno, bun, node"), "{message}");
    assert!(message.contains("config js.runner"), "{message}");
}

#[test]
fn test_runner_describe_uses_preferred_name_without_path_lookup() {
    let preview = build_launch_preview(
        &entry("js", ""),
        &paths(),
        &Assembly::default(),
        None,
        None,
        None,
        &Probe::default().panic_on_program_lookup(),
    )
    .unwrap();
    assert_eq!(preview.program, PathBuf::from("deno"));
    assert!(preview.display.contains("deno"), "{}", preview.display);
    assert!(preview.display.contains(SCRIPT), "{}", preview.display);
}

#[test]
fn test_runner_preflight_checks_script_and_runner() {
    let missing_script = build_launch_plan(
        &entry("js", ""),
        &paths(),
        &Assembly::default(),
        None,
        None,
        &Probe::default()
            .without_script()
            .panic_on_program_lookup(),
    )
    .unwrap_err();
    assert!(matches!(&missing_script, LaunchError::TargetMissing { .. }));

    let missing_runner = build_launch_plan(
        &entry("js", ""),
        &paths(),
        &Assembly::default(),
        None,
        None,
        &Probe::default(),
    )
    .unwrap_err();
    assert!(matches!(&missing_runner, LaunchError::ProgramNotFound { .. }));
}

#[test]
fn rust_additive_runner_preflight_checks_script_first() {
    let error = build_launch_plan(
        &entry("js", ""),
        &paths(),
        &Assembly::default(),
        None,
        None,
        &Probe::default()
            .without_script()
            .panic_on_program_lookup(),
    )
    .unwrap_err();
    assert!(matches!(&error, LaunchError::TargetMissing { .. }));
}

#[test]
fn rust_additive_runner_preflight_checks_runtime_after_script() {
    let error = build_launch_plan(
        &entry("js", ""),
        &paths(),
        &Assembly::default(),
        None,
        None,
        &Probe::default(),
    )
    .unwrap_err();
    assert!(matches!(&error, LaunchError::ProgramNotFound { .. }));
}
