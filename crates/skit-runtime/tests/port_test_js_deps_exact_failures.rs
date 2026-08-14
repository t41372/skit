//! Exact dependency failure ports from Python v0.4 `tests/test_js_deps.py`.

use std::{
    collections::BTreeMap,
    fs,
    io,
    path::{Path, PathBuf},
};

use skit_runtime::{
    DependencyCommand, DependencyCommandRunner, DependencyError, ProgramProbe,
    clear_javascript_dependencies, ensure_javascript_dependencies,
};
use tempfile::TempDir;

#[derive(Debug, Default)]
struct Probe {
    npm: bool,
}

impl ProgramProbe for Probe {
    fn find_program(&self, name: &str) -> Option<PathBuf> {
        (self.npm && name == "npm").then(|| PathBuf::from("/bin/npm"))
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

#[derive(Debug)]
enum Outcome {
    Fail,
    Spawn,
}

#[derive(Debug)]
struct Runner(Outcome);

impl DependencyCommandRunner for Runner {
    fn run(&self, _command: &DependencyCommand) -> io::Result<bool> {
        match self.0 {
            Outcome::Fail => Ok(false),
            Outcome::Spawn => Err(io::Error::from_raw_os_error(8)),
        }
    }
}

#[test]
fn test_dependency_failure_messages_verbatim() {
    let root = TempDir::new().unwrap();
    let dependencies = ["chalk".to_owned()];

    let missing = ensure_javascript_dependencies(
        root.path(),
        "node",
        &dependencies,
        &Probe::default(),
        &Runner(Outcome::Fail),
    )
    .unwrap_err();
    assert_eq!(
        missing.to_string(),
        "npm is needed to install this script's dependencies, but it isn't on your PATH."
    );

    let spawn = ensure_javascript_dependencies(
        root.path(),
        "node",
        &dependencies,
        &Probe { npm: true },
        &Runner(Outcome::Spawn),
    )
    .unwrap_err();
    assert_eq!(spawn.to_string(), "Couldn't run npm: [Errno 8] Exec format error");

    let failed = ensure_javascript_dependencies(
        root.path(),
        "node",
        &dependencies,
        &Probe { npm: true },
        &Runner(Outcome::Fail),
    )
    .unwrap_err();
    assert_eq!(
        failed.to_string(),
        "Installing dependencies failed (npm): npm error it failed"
    );
}

#[test]
fn test_clean_failure_is_loud_not_silent() {
    let root = TempDir::new().unwrap();
    fs::write(root.path().join(".skit-deps"), "v1\nnode\ndeadbeef\n").unwrap();
    fs::create_dir(root.path().join("package.json")).unwrap();

    let error = clear_javascript_dependencies(root.path())
        .expect_err("a malformed owned artifact must refuse cleanup loudly");
    let rendered = error.to_string();
    assert!(rendered.contains("package.json"), "{rendered}");
    assert!(rendered.contains("directory"), "{rendered}");
}

#[test]
fn test_clean_rmtree_failure_is_loud() {
    let root = TempDir::new().unwrap();
    fs::write(root.path().join(".skit-deps"), "v1\nnode\ndeadbeef\n").unwrap();
    fs::write(root.path().join("package.json"), "{}\n").unwrap();
    fs::write(root.path().join(".skit-deps.backup"), "occupied").unwrap();

    let error = clear_javascript_dependencies(root.path())
        .expect_err("cleanup transaction failure must not be silently ignored");
    assert!(error.to_string().contains(".skit-deps.backup"), "{error}");
}

#[test]
fn test_clean_failure_message_verbatim() {
    let error = DependencyError::Io {
        operation: "clear",
        path: "package.json".to_owned(),
        reason: "held by another process".to_owned(),
    };
    assert_eq!(
        error.to_string(),
        "Couldn't clear the old dependency environment: package.json: held by another process"
    );
}