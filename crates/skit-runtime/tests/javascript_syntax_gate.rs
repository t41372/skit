//! Runtime-side owners for the JavaScript injection syntax gate.
//!
//! The parser-backed offline gate stays in `skit-language`. This module owns only resolved runtime
//! identity and the optional `node --check` process boundary. The caller still owns launch order.

use std::{
    cell::RefCell,
    collections::{BTreeMap, VecDeque},
    fs,
    path::{Path, PathBuf},
    process::Command,
};

use skit_domain::EntrySettings;
use skit_runtime::{
    JavaScriptRuntimeKind, JavaScriptSyntaxGateOutput, JavaScriptSyntaxGateRunner,
    JavaScriptSyntaxGateUnavailable, ProgramProbe, ResolvedJavaScriptRuntime,
    SystemJavaScriptSyntaxGateRunner, check_javascript_syntax, resolve_javascript_runtime_program,
    retain_javascript_source_if_valid,
};
use tempfile::{Builder as TempBuilder, TempDir};

#[derive(Debug, Default)]
struct Probe {
    programs: BTreeMap<String, PathBuf>,
}

impl ProgramProbe for Probe {
    fn find_program(&self, name: &str) -> Option<PathBuf> {
        self.programs.get(name).cloned()
    }

    fn is_file(&self, _path: &Path) -> bool {
        false
    }

    fn is_dir(&self, _path: &Path) -> bool {
        false
    }

    fn is_executable(&self, _path: &Path) -> bool {
        false
    }
}

#[derive(Debug)]
struct FakeGate {
    outcomes:
        RefCell<VecDeque<Result<JavaScriptSyntaxGateOutput, JavaScriptSyntaxGateUnavailable>>>,
    calls: RefCell<Vec<(PathBuf, PathBuf)>>,
}

impl FakeGate {
    fn one(outcome: Result<JavaScriptSyntaxGateOutput, JavaScriptSyntaxGateUnavailable>) -> Self {
        Self {
            outcomes: RefCell::new(VecDeque::from([outcome])),
            calls: RefCell::new(Vec::new()),
        }
    }

    fn success() -> Self {
        Self::one(Ok(JavaScriptSyntaxGateOutput {
            success: true,
            stderr: Vec::new(),
        }))
    }
}

impl JavaScriptSyntaxGateRunner for FakeGate {
    fn check(
        &self,
        program: &Path,
        source: &Path,
        _timeout: std::time::Duration,
    ) -> Result<JavaScriptSyntaxGateOutput, JavaScriptSyntaxGateUnavailable> {
        self.calls
            .borrow_mut()
            .push((program.to_path_buf(), source.to_path_buf()));
        self.outcomes
            .borrow_mut()
            .pop_front()
            .expect("the test supplied one gate outcome")
    }
}

fn runtime(kind: JavaScriptRuntimeKind) -> ResolvedJavaScriptRuntime {
    ResolvedJavaScriptRuntime {
        kind,
        program: PathBuf::from("/runtime/program"),
    }
}

fn rejected(stderr: &[u8]) -> JavaScriptSyntaxGateOutput {
    JavaScriptSyntaxGateOutput {
        success: false,
        stderr: stderr.to_vec(),
    }
}

#[test]
fn test_resolve_runner_respects_pinned_interpreter_and_normalizes() {
    let settings = EntrySettings {
        interpreter: "dir/node.exe".to_owned(),
        ..EntrySettings::default()
    };
    let probe = Probe {
        programs: BTreeMap::from([(
            "dir/node.exe".to_owned(),
            PathBuf::from("/abs/dir/node.exe"),
        )]),
    };

    assert_eq!(
        resolve_javascript_runtime_program(&settings, &probe).unwrap(),
        ResolvedJavaScriptRuntime {
            kind: JavaScriptRuntimeKind::Node,
            program: PathBuf::from("/abs/dir/node.exe"),
        }
    );
}

#[test]
fn test_gate_node_skips_ts_suffix() {
    let gate = FakeGate::success();
    check_javascript_syntax(
        Some(&runtime(JavaScriptRuntimeKind::Node)),
        Path::new("x.ts"),
        &gate,
    )
    .unwrap();
    assert!(gate.calls.borrow().is_empty());
}

#[test]
fn rust_additive_gate_skips_without_a_resolved_runtime() {
    let gate = FakeGate::success();
    check_javascript_syntax(None, Path::new("x.js"), &gate).unwrap();
    assert!(gate.calls.borrow().is_empty());
}

#[test]
fn rust_additive_gate_node_skips_every_typescript_suffix() {
    let gate = FakeGate::success();
    for suffix in ["ts", "mts", "cts", "tsx"] {
        check_javascript_syntax(
            Some(&runtime(JavaScriptRuntimeKind::Node)),
            Path::new(&format!("x.{suffix}")),
            &gate,
        )
        .unwrap();
    }
    assert!(gate.calls.borrow().is_empty());
}

#[derive(Debug)]
struct RealNodeGate {
    old_module_mode: bool,
}

impl JavaScriptSyntaxGateRunner for RealNodeGate {
    fn check(
        &self,
        program: &Path,
        source: &Path,
        _timeout: std::time::Duration,
    ) -> Result<JavaScriptSyntaxGateOutput, JavaScriptSyntaxGateUnavailable> {
        let mut command = Command::new(program);
        if self.old_module_mode {
            command.env("NODE_OPTIONS", "--no-experimental-detect-module");
        }
        let output = command
            .args(["--check", source.to_string_lossy().as_ref()])
            .output()
            .map_err(|error| JavaScriptSyntaxGateUnavailable::Spawn {
                reason: error.to_string(),
            })?;
        Ok(JavaScriptSyntaxGateOutput {
            success: output.status.success(),
            stderr: output.stderr,
        })
    }
}

#[test]
fn rust_additive_mjs_gate_accepts_esm_before_any_package_json() {
    let Some(program) = skit_runtime::SystemProbe.find_program("node") else {
        return;
    };
    let supports_old_module_mode = Command::new(&program)
        .args(["--no-experimental-detect-module", "-e", ""])
        .status()
        .is_ok_and(|status| status.success());
    if !supports_old_module_mode {
        return;
    }

    let root = TempDir::new().unwrap();
    let source = root.path().join(".injected-test.mjs");
    fs::write(
        &source,
        "import assert from 'node:assert';\nconst N = 7;\nassert.ok(N);\n",
    )
    .unwrap();
    assert!(!root.path().join("package.json").exists());

    let runtime = ResolvedJavaScriptRuntime {
        kind: JavaScriptRuntimeKind::Node,
        program,
    };
    check_javascript_syntax(
        Some(&runtime),
        &source,
        &RealNodeGate {
            old_module_mode: true,
        },
    )
    .unwrap();
    assert!(!root.path().join("package.json").exists());
}

#[test]
fn rust_additive_mjs_is_always_node_check_eligible() {
    let gate = FakeGate::success();
    check_javascript_syntax(
        Some(&runtime(JavaScriptRuntimeKind::Node)),
        Path::new("x.mjs"),
        &gate,
    )
    .unwrap();
    assert_eq!(gate.calls.borrow().len(), 1);
}

#[test]
fn test_gate_node_skips_when_runner_is_not_node() {
    for kind in [JavaScriptRuntimeKind::Deno, JavaScriptRuntimeKind::Bun] {
        let gate = FakeGate::success();
        check_javascript_syntax(Some(&runtime(kind)), Path::new("x.js"), &gate).unwrap();
        assert!(gate.calls.borrow().is_empty());
    }
}

#[test]
fn test_gate_node_passes_on_returncode_zero() {
    let gate = FakeGate::success();
    check_javascript_syntax(
        Some(&runtime(JavaScriptRuntimeKind::Node)),
        Path::new("x.cjs"),
        &gate,
    )
    .unwrap();
    assert_eq!(gate.calls.borrow().len(), 1);
}

#[test]
fn test_gate_node_raises_on_nonzero() {
    let gate = FakeGate::one(Ok(rejected(b"  SyntaxError: boom\nsecond line\n")));
    let error = check_javascript_syntax(
        Some(&runtime(JavaScriptRuntimeKind::Node)),
        Path::new("x.js"),
        &gate,
    )
    .unwrap_err();
    assert_eq!(error.detail(), "SyntaxError: boom");
    assert_eq!(
        error.to_string(),
        "node rejected the injected copy: SyntaxError: boom"
    );
}

#[test]
fn test_gate_node_raises_on_nonzero_with_empty_stderr() {
    let gate = FakeGate::one(Ok(rejected(b"\n\t")));
    let error = check_javascript_syntax(
        Some(&runtime(JavaScriptRuntimeKind::Node)),
        Path::new("x.js"),
        &gate,
    )
    .unwrap_err();
    assert_eq!(error.detail(), "");
    assert_eq!(error.to_string(), "node rejected the injected copy: ");
}

#[test]
fn test_gate_node_survives_a_spawn_failure() {
    for unavailable in [
        JavaScriptSyntaxGateUnavailable::Spawn {
            reason: "no fork".to_owned(),
        },
        JavaScriptSyntaxGateUnavailable::Timeout,
    ] {
        let gate = FakeGate::one(Err(unavailable));
        check_javascript_syntax(
            Some(&runtime(JavaScriptRuntimeKind::Node)),
            Path::new("x.js"),
            &gate,
        )
        .unwrap();
        assert_eq!(gate.calls.borrow().len(), 1);
    }
}

#[test]
fn rust_additive_system_gate_runner_reports_a_missing_program() {
    let error = SystemJavaScriptSyntaxGateRunner
        .check(
            Path::new("/definitely/missing/skit-node"),
            Path::new("x.js"),
            std::time::Duration::from_millis(1),
        )
        .unwrap_err();
    assert!(matches!(
        error,
        JavaScriptSyntaxGateUnavailable::Spawn { .. }
    ));
}

#[test]
fn rust_additive_system_gate_runner_captures_node_rejection() {
    let Some(program) = skit_runtime::SystemProbe.find_program("node") else {
        return;
    };
    let root = TempDir::new().unwrap();
    let source = root.path().join("broken.js");
    fs::write(&source, "const broken = ;\n").unwrap();

    let output = SystemJavaScriptSyntaxGateRunner
        .check(&program, &source, std::time::Duration::from_secs(5))
        .unwrap();

    assert!(!output.success);
    assert!(!output.stderr.is_empty());
}

/// Run one gate check, and retry only the fork window of a sibling spawn.
///
/// A test writes the program it is about to run. Another test in the same binary can fork for its
/// own child while that write handle is still open, and the fork gives the new child a copy of the
/// handle. An exec of the program then fails with ETXTBSY until that child reaches its own exec.
/// The window is short and closes without help, so a few tries are enough. Every other answer,
/// including the timeout these tests want, returns at once.
#[cfg(unix)]
fn check_past_the_fork_window(
    program: &Path,
    source: &Path,
    timeout: std::time::Duration,
) -> Result<JavaScriptSyntaxGateOutput, JavaScriptSyntaxGateUnavailable> {
    for _ in 0..9 {
        let result = SystemJavaScriptSyntaxGateRunner.check(program, source, timeout);
        let busy = matches!(
            &result,
            Err(JavaScriptSyntaxGateUnavailable::Spawn { reason }) if reason.contains("Text file busy")
        );
        if !busy {
            return result;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    SystemJavaScriptSyntaxGateRunner.check(program, source, timeout)
}

#[cfg(unix)]
#[test]
fn rust_additive_system_gate_runner_bounds_a_stuck_process() {
    use std::{io::Write as _, os::unix::fs::PermissionsExt as _};

    let root = TempDir::new().unwrap();
    let program = root.path().join("stuck-node");
    let mut file = fs::File::create(&program).unwrap();
    file.write_all(b"#!/bin/sh\nwhile :; do :; done\n").unwrap();
    file.sync_all().unwrap();
    drop(file);
    fs::set_permissions(&program, fs::Permissions::from_mode(0o755)).unwrap();

    let error = check_past_the_fork_window(
        &program,
        Path::new("x.js"),
        std::time::Duration::from_millis(10),
    )
    .unwrap_err();

    assert_eq!(error, JavaScriptSyntaxGateUnavailable::Timeout);
}

#[cfg(unix)]
#[test]
fn rust_additive_system_gate_drains_large_stderr_before_waiting() {
    use std::{io::Write as _, os::unix::fs::PermissionsExt as _};

    let root = TempDir::new().unwrap();
    let program = root.path().join("loud-node");
    let mut file = fs::File::create(&program).unwrap();
    file.write_all(
        b"#!/bin/sh\nprintf 'SyntaxError: first line\\n' >&2\nhead -c 262144 /dev/zero >&2\nexit 1\n",
    )
    .unwrap();
    file.sync_all().unwrap();
    drop(file);
    fs::set_permissions(&program, fs::Permissions::from_mode(0o755)).unwrap();
    let source = root.path().join("source.js");
    fs::write(&source, "const broken = ;\n").unwrap();

    let output =
        check_past_the_fork_window(&program, &source, std::time::Duration::from_secs(5)).unwrap();
    assert!(!output.success);
    assert!(output.stderr.len() > 65_536);
    let gate = FakeGate::one(Ok(output));
    let error =
        check_javascript_syntax(Some(&runtime(JavaScriptRuntimeKind::Node)), &source, &gate)
            .unwrap_err();
    assert_eq!(error.detail(), "SyntaxError: first line");
}

#[test]
fn test_gate2_failure_removes_the_temp_copy() {
    let staged = TempBuilder::new().suffix(".js").tempfile().unwrap();
    let path = staged.path().to_path_buf();
    fs::write(&path, "const T = 'x';\n").unwrap();
    let gate = FakeGate::one(Ok(rejected(b"boom")));

    let error = retain_javascript_source_if_valid(
        staged,
        Some(&runtime(JavaScriptRuntimeKind::Node)),
        &path,
        &gate,
    )
    .unwrap_err();

    assert_eq!(error.detail(), "boom");
    assert!(!path.exists());
}
