//! Exact freshness/preflight ports from Python v0.4 `tests/test_js_deps.py`.

use std::{
    cell::RefCell,
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

use skit_runtime::{
    DependencyCommand, DependencyCommandRunner, DependencyError, ProgramProbe,
    ensure_javascript_dependencies,
};
use tempfile::TempDir;

#[derive(Debug, Default)]
struct Probe {
    programs: BTreeMap<String, PathBuf>,
}

impl Probe {
    fn with(names: &[&str]) -> Self {
        Self {
            programs: names
                .iter()
                .map(|name| ((*name).to_owned(), PathBuf::from(format!("/bin/{name}"))))
                .collect(),
        }
    }
}

impl ProgramProbe for Probe {
    fn find_program(&self, name: &str) -> Option<PathBuf> {
        self.programs.get(name).cloned()
    }
    fn is_file(&self, path: &Path) -> bool {
        path.is_file()
    }
    fn is_dir(&self, path: &Path) -> bool {
        path.is_dir()
    }
    fn is_executable(&self, _path: &Path) -> bool {
        true
    }
}

#[derive(Debug, Default)]
struct Runner {
    commands: RefCell<Vec<DependencyCommand>>,
}

impl DependencyCommandRunner for Runner {
    fn run(&self, command: &DependencyCommand) -> std::io::Result<bool> {
        self.commands.borrow_mut().push(command.clone());
        fs::create_dir_all(command.cwd.join("node_modules"))?;
        Ok(true)
    }
}

#[derive(Debug)]
struct MustNotRun;

impl DependencyCommandRunner for MustNotRun {
    fn run(&self, command: &DependencyCommand) -> std::io::Result<bool> {
        panic!("fresh dependency state unexpectedly launched: {command:?}");
    }
}

#[test]
fn test_corrupted_marker_triggers_reinstall_not_a_persistent_crash() {
    let root = TempDir::new().unwrap();
    fs::create_dir(root.path().join("node_modules")).unwrap();
    fs::write(
        root.path().join("node_modules/.skit-deps-ok"),
        b"\xff\xfe garbage",
    )
    .unwrap();

    let runner = Runner::default();
    ensure_javascript_dependencies(
        root.path(),
        "node",
        &["chalk".to_owned()],
        &Probe::with(&["npm"]),
        &runner,
    )
    .unwrap();
    assert_eq!(runner.commands.borrow().len(), 1);
    let marker = fs::read_to_string(root.path().join("node_modules/.skit-deps-ok")).unwrap();
    assert_eq!(marker.len(), 64, "a fresh 64-character hex stamp must replace the corrupt marker");
    assert!(marker.chars().all(|ch| ch.is_ascii_hexdigit()), "{marker:?}");
}

#[test]
fn test_needs_install_true_without_a_marker() {
    let root = TempDir::new().unwrap();
    let error = ensure_javascript_dependencies(
        root.path(),
        "node",
        &["chalk".to_owned()],
        &Probe::default(),
        &MustNotRun,
    )
    .unwrap_err();
    assert!(matches!(error, DependencyError::InstallerNotFound { ref name } if name == "npm"));
}

#[test]
fn test_needs_install_false_when_the_marker_matches() {
    let root = TempDir::new().unwrap();
    ensure_javascript_dependencies(
        root.path(),
        "node",
        &["chalk".to_owned()],
        &Probe::with(&["npm"]),
        &Runner::default(),
    )
    .unwrap();
    ensure_javascript_dependencies(
        root.path(),
        "node",
        &["chalk".to_owned()],
        &Probe::default(),
        &MustNotRun,
    )
    .expect("a matching freshness marker must make installer lookup unnecessary");
}

#[test]
fn test_needs_install_true_when_the_declared_deps_changed() {
    let root = TempDir::new().unwrap();
    ensure_javascript_dependencies(
        root.path(),
        "node",
        &["chalk".to_owned()],
        &Probe::with(&["npm"]),
        &Runner::default(),
    )
    .unwrap();
    let error = ensure_javascript_dependencies(
        root.path(),
        "node",
        &["chalk".to_owned(), "zod".to_owned()],
        &Probe::default(),
        &MustNotRun,
    )
    .unwrap_err();
    assert!(matches!(error, DependencyError::InstallerNotFound { ref name } if name == "npm"));
}

#[test]
fn test_preflight_skips_the_installer_when_the_marker_is_already_fresh() {
    let root = TempDir::new().unwrap();
    ensure_javascript_dependencies(
        root.path(),
        "node",
        &["chalk".to_owned()],
        &Probe::with(&["npm"]),
        &Runner::default(),
    )
    .unwrap();
    ensure_javascript_dependencies(
        root.path(),
        "node",
        &["chalk".to_owned()],
        &Probe::default(),
        &MustNotRun,
    )
    .expect("fresh dependency state must not re-require npm in preflight");
}

#[test]
fn test_ensure_installed_unknown_runner_falls_back_to_npm_argv() {
    let root = TempDir::new().unwrap();
    let runner = Runner::default();
    ensure_javascript_dependencies(
        root.path(),
        "some-future-runner",
        &["chalk".to_owned()],
        &Probe::with(&["npm"]),
        &runner,
    )
    .unwrap();
    let commands = runner.commands.borrow();
    assert_eq!(commands.len(), 1);
    assert_eq!(commands[0].program, PathBuf::from("/bin/npm"));
    assert_eq!(
        commands[0].args,
        ["install", "--no-audit", "--no-fund", "--ignore-scripts"]
    );
}