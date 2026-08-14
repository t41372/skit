//! Exact public-boundary ports from Python v0.4 `tests/test_js_deps.py`.
//!
//! The frozen Python assertions are authoritative. Several tests are intentionally expected to
//! fail against the current Rust implementation (package-spec edge cases, package-manager argv,
//! staging cwd, stamp layout, and cleanup semantics). Those are parity findings, not reasons to
//! dilute the oracle.

use std::{
    cell::RefCell,
    collections::BTreeMap,
    fs,
    io,
    path::{Path, PathBuf},
};

use serde_json::Value as JsonValue;
use skit_runtime::{
    DependencyCommand, DependencyCommandRunner, DependencyError, ProgramProbe,
    clear_javascript_dependencies, ensure_javascript_dependencies,
    ensure_javascript_dependencies_with_environment, javascript_dependency_manifest,
};
use tempfile::TempDir;

#[derive(Debug, Default)]
struct Probe {
    programs: BTreeMap<String, PathBuf>,
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RunnerOutcome {
    SuccessWithNodeModules,
    SuccessWithoutNodeModules,
    FailedStatus,
    SpawnError,
}

#[derive(Debug)]
struct RecordingRunner {
    commands: RefCell<Vec<DependencyCommand>>,
    outcome: RunnerOutcome,
}

impl RecordingRunner {
    fn new(outcome: RunnerOutcome) -> Self {
        Self {
            commands: RefCell::new(Vec::new()),
            outcome,
        }
    }
}

impl DependencyCommandRunner for RecordingRunner {
    fn run(&self, command: &DependencyCommand) -> io::Result<bool> {
        self.commands.borrow_mut().push(command.clone());
        match self.outcome {
            RunnerOutcome::SuccessWithNodeModules => {
                fs::create_dir_all(command.cwd.join("node_modules"))?;
                Ok(true)
            }
            RunnerOutcome::SuccessWithoutNodeModules => Ok(true),
            RunnerOutcome::FailedStatus => Ok(false),
            RunnerOutcome::SpawnError => Err(io::Error::other("exec format error")),
        }
    }
}

fn probe(name: &str) -> Probe {
    Probe {
        programs: BTreeMap::from([(name.to_owned(), PathBuf::from(format!("/bin/{name}")))]),
    }
}

fn dependency_rows(text: &str) -> BTreeMap<String, String> {
    let json: JsonValue = serde_json::from_str(text).expect("dependency manifest must be JSON");
    json["dependencies"]
        .as_object()
        .expect("manifest dependencies must be an object")
        .iter()
        .map(|(name, version)| {
            (
                name.clone(),
                version
                    .as_str()
                    .expect("dependency range must be a string")
                    .to_owned(),
            )
        })
        .collect()
}

#[test]
fn test_split_requirement() {
    for (requirement, expected_name, expected_version) in [
        ("chalk", "chalk", "*"),
        ("chalk@^5", "chalk", "^5"),
        ("chalk@5.6.2", "chalk", "5.6.2"),
        ("chalk@", "chalk", "*"),
        ("@scope/pkg", "@scope/pkg", "*"),
        ("@scope/pkg@>=1,<2", "@scope/pkg", ">=1,<2"),
        ("@scope", "@scope", "*"),
    ] {
        let manifest = javascript_dependency_manifest(&[requirement.to_owned()])
            .unwrap_or_else(|error| panic!("frozen requirement {requirement:?} was rejected: {error}"));
        assert_eq!(
            dependency_rows(&manifest),
            BTreeMap::from([(expected_name.to_owned(), expected_version.to_owned())]),
            "requirement={requirement:?}"
        );
    }
}

#[test]
fn test_manifest_text_is_deterministic_and_private() {
    let dependencies = ["chalk@^5".to_owned(), " zod ".to_owned(), String::new()];
    let first = javascript_dependency_manifest(&dependencies).unwrap();
    let second = javascript_dependency_manifest(&dependencies).unwrap();
    assert_eq!(first, second);
    assert!(first.contains("\"private\": true"), "{first}");
    assert!(first.contains("\"chalk\": \"^5\""), "{first}");
    assert!(first.contains("\"zod\": \"*\""), "{first}");
    assert!(first.ends_with('\n'));
}

#[test]
fn test_manifest_text_skips_an_empty_requirement() {
    let manifest = javascript_dependency_manifest(&[String::new(), "  ".to_owned()]).unwrap();
    assert!(manifest.contains("\"dependencies\": {}"), "{manifest}");
}

#[test]
fn test_clean_removes_manifest_lockfiles_and_node_modules() {
    let root = TempDir::new().unwrap();
    for name in ["package.json", "package-lock.json", "bun.lock", "bun.lockb", "deno.lock"] {
        fs::write(root.path().join(name), "{}\n").unwrap();
    }
    fs::create_dir_all(root.path().join("node_modules/chalk")).unwrap();
    fs::write(root.path().join("meta.toml"), "").unwrap();

    clear_javascript_dependencies(root.path()).unwrap();

    let mut names = fs::read_dir(root.path())
        .unwrap()
        .map(|item| item.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|name| name != ".locks")
        .collect::<Vec<_>>();
    names.sort();
    assert_eq!(names, ["meta.toml"]);
}

#[test]
fn test_clean_on_an_already_clean_dir_is_a_no_op() {
    let root = TempDir::new().unwrap();
    clear_javascript_dependencies(root.path()).unwrap();
    let names = fs::read_dir(root.path())
        .unwrap()
        .map(|item| item.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|name| name != ".locks")
        .collect::<Vec<_>>();
    assert!(names.is_empty(), "unexpected dependency artifacts: {names:?}");
}

#[test]
fn test_require_installer_maps_runner_to_its_own_installer() {
    for (runtime, installer) in [("node", "npm"), ("bun", "bun"), ("deno", "deno"), ("weird", "npm")] {
        let root = TempDir::new().unwrap();
        let runner = RecordingRunner::new(RunnerOutcome::SuccessWithNodeModules);
        ensure_javascript_dependencies(
            root.path(),
            runtime,
            &["chalk".to_owned()],
            &probe(installer),
            &runner,
        )
        .unwrap_or_else(|error| panic!("runtime {runtime:?} did not map to {installer:?}: {error}"));
        let commands = runner.commands.borrow();
        assert_eq!(commands.len(), 1, "runtime={runtime}");
        assert_eq!(commands[0].program, PathBuf::from(format!("/bin/{installer}")));
    }
}

#[test]
fn test_require_installer_missing_raises_126_family() {
    let root = TempDir::new().unwrap();
    let error = ensure_javascript_dependencies(
        root.path(),
        "node",
        &["chalk".to_owned()],
        &Probe::default(),
        &RecordingRunner::new(RunnerOutcome::SuccessWithNodeModules),
    )
    .unwrap_err();
    assert!(matches!(error, DependencyError::InstallerNotFound { ref name } if name == "npm"));
    assert!(error.to_string().contains("npm"), "{error}");
}

#[test]
fn test_ensure_installed_writes_manifest_runs_installer_and_stamps() {
    let root = TempDir::new().unwrap();
    let runner = RecordingRunner::new(RunnerOutcome::SuccessWithNodeModules);
    let environment = BTreeMap::from([
        ("PATH".to_owned(), "/bin".to_owned()),
        ("X".to_owned(), "y".to_owned()),
    ]);
    ensure_javascript_dependencies_with_environment(
        root.path(),
        "node",
        &["chalk@^5".to_owned()],
        &environment,
        &probe("npm"),
        &runner,
    )
    .unwrap();

    let commands = runner.commands.borrow();
    assert_eq!(commands.len(), 1);
    assert_eq!(commands[0].program, PathBuf::from("/bin/npm"));
    assert_eq!(commands[0].args, ["install", "--no-audit", "--no-fund", "--ignore-scripts"]);
    assert_eq!(commands[0].cwd, root.path());
    assert_eq!(commands[0].environment, environment);
    assert_eq!(
        fs::read_to_string(root.path().join("package.json")).unwrap(),
        javascript_dependency_manifest(&["chalk@^5".to_owned()]).unwrap()
    );
    assert!(root.path().join("node_modules/.skit-deps-ok").is_file());
}

#[test]
fn test_ensure_installed_uses_the_runners_own_installer() {
    for (runtime, installer, argv_tail) in [
        ("bun", "bun", vec!["install", "--ignore-scripts"]),
        ("deno", "deno", vec!["install"]),
    ] {
        let root = TempDir::new().unwrap();
        let runner = RecordingRunner::new(RunnerOutcome::SuccessWithNodeModules);
        ensure_javascript_dependencies(
            root.path(),
            runtime,
            &["zod".to_owned()],
            &probe(installer),
            &runner,
        )
        .unwrap();
        let command = runner.commands.borrow()[0].clone();
        assert_eq!(command.program, PathBuf::from(format!("/bin/{installer}")));
        assert_eq!(command.args, argv_tail, "runtime={runtime}");
    }
}

#[test]
fn test_ensure_installed_fresh_marker_short_circuits() {
    let root = TempDir::new().unwrap();
    let first = RecordingRunner::new(RunnerOutcome::SuccessWithNodeModules);
    ensure_javascript_dependencies(
        root.path(),
        "node",
        &["chalk".to_owned()],
        &probe("npm"),
        &first,
    )
    .unwrap();
    assert_eq!(first.commands.borrow().len(), 1);

    let second = RecordingRunner::new(RunnerOutcome::FailedStatus);
    ensure_javascript_dependencies(
        root.path(),
        "node",
        &["chalk".to_owned()],
        &Probe::default(),
        &second,
    )
    .expect("fresh dependency marker must skip installer lookup and execution");
    assert!(second.commands.borrow().is_empty());
}

#[test]
fn test_ensure_installed_stale_marker_rebuilds_from_scratch() {
    for (new_dependencies, new_runtime, installer) in [
        (vec!["chalk".to_owned(), "zod".to_owned()], "node", "npm"),
        (vec!["chalk".to_owned()], "bun", "bun"),
    ] {
        let root = TempDir::new().unwrap();
        let first = RecordingRunner::new(RunnerOutcome::SuccessWithNodeModules);
        ensure_javascript_dependencies(
            root.path(),
            "node",
            &["chalk".to_owned()],
            &probe("npm"),
            &first,
        )
        .unwrap();
        fs::create_dir(root.path().join("node_modules/leftover")).unwrap();
        fs::write(root.path().join("deno.lock"), "{}\n").unwrap();

        let second = RecordingRunner::new(RunnerOutcome::SuccessWithNodeModules);
        ensure_javascript_dependencies(
            root.path(),
            new_runtime,
            &new_dependencies,
            &probe(installer),
            &second,
        )
        .unwrap();
        assert_eq!(second.commands.borrow().len(), 1);
        assert!(!root.path().join("node_modules/leftover").exists());
        assert!(!root.path().join("deno.lock").exists());
        assert_eq!(
            fs::read_to_string(root.path().join("package.json")).unwrap(),
            javascript_dependency_manifest(&new_dependencies).unwrap()
        );
    }
}

#[test]
fn test_ensure_installed_failure_without_stderr_still_reports() {
    let root = TempDir::new().unwrap();
    let error = ensure_javascript_dependencies(
        root.path(),
        "node",
        &["x".to_owned()],
        &probe("npm"),
        &RecordingRunner::new(RunnerOutcome::FailedStatus),
    )
    .unwrap_err();
    assert!(error.to_string().contains("npm"), "{error}");
}

#[test]
fn test_ensure_installed_spawn_oserror_is_wrapped() {
    let root = TempDir::new().unwrap();
    let error = ensure_javascript_dependencies(
        root.path(),
        "node",
        &["x".to_owned()],
        &probe("npm"),
        &RecordingRunner::new(RunnerOutcome::SpawnError),
    )
    .unwrap_err();
    assert!(error.to_string().contains("exec format error"), "{error}");
}

#[test]
fn test_ensure_installed_missing_installer_raises_before_touching_the_dir() {
    let root = TempDir::new().unwrap();
    assert!(ensure_javascript_dependencies(
        root.path(),
        "node",
        &["chalk".to_owned()],
        &Probe::default(),
        &RecordingRunner::new(RunnerOutcome::SuccessWithNodeModules),
    )
    .is_err());
    assert!(!root.path().join("package.json").exists());
}

#[test]
fn test_ensure_installed_stamps_even_when_installer_creates_no_node_modules() {
    let root = TempDir::new().unwrap();
    ensure_javascript_dependencies(
        root.path(),
        "node",
        &["@5".to_owned()],
        &probe("npm"),
        &RecordingRunner::new(RunnerOutcome::SuccessWithoutNodeModules),
    )
    .unwrap();
    assert!(root.path().join("node_modules/.skit-deps-ok").is_file());
}