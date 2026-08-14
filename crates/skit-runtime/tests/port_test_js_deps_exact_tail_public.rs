//! Remaining public runtime contracts from Python v0.4 `tests/test_js_deps.py`.

use std::{
    collections::BTreeMap,
    fs::{self, FileTimes},
    path::{Path, PathBuf},
    time::SystemTime,
};

use skit_runtime::{
    DependencyCommand, DependencyCommandRunner, JavaScriptModuleType, ProgramProbe,
    clear_javascript_dependencies, ensure_javascript_dependencies_for_module,
};
use tempfile::TempDir;

#[derive(Debug, Default)]
struct NoPrograms;
impl ProgramProbe for NoPrograms {
    fn find_program(&self, _name: &str) -> Option<PathBuf> { None }
    fn is_file(&self, path: &Path) -> bool { path.is_file() }
    fn is_dir(&self, path: &Path) -> bool { path.is_dir() }
    fn is_executable(&self, _path: &Path) -> bool { true }
}

#[derive(Debug, Default)]
struct MustNotRun;
impl DependencyCommandRunner for MustNotRun {
    fn run(&self, command: &DependencyCommand) -> std::io::Result<bool> {
        panic!("dep-free module manifest unexpectedly launched an installer: {command:?}")
    }
}

fn ensure_type(root: &Path, module_type: JavaScriptModuleType) {
    ensure_javascript_dependencies_for_module(
        root,
        "node",
        &[],
        Some(module_type),
        &BTreeMap::new(),
        &NoPrograms,
        &MustNotRun,
    )
    .unwrap();
}

#[test]
fn test_ensure_module_manifest_rewrites_only_on_change() {
    let root = TempDir::new().unwrap();
    ensure_type(root.path(), JavaScriptModuleType::Module);
    let package = root.path().join("package.json");
    let epoch = SystemTime::UNIX_EPOCH;
    let file = fs::File::options().write(true).open(&package).unwrap();
    file.set_times(FileTimes::new().set_modified(epoch)).unwrap();
    assert_eq!(fs::metadata(&package).unwrap().modified().unwrap(), epoch);

    ensure_type(root.path(), JavaScriptModuleType::Module);
    assert_eq!(
        fs::metadata(&package).unwrap().modified().unwrap(),
        epoch,
        "an already-correct module manifest was needlessly rewritten"
    );

    ensure_type(root.path(), JavaScriptModuleType::CommonJs);
    assert_ne!(
        fs::metadata(&package).unwrap().modified().unwrap(),
        epoch,
        "a changed module flavor did not rewrite package.json"
    );
    let manifest = fs::read_to_string(package).unwrap();
    assert!(manifest.contains("\"type\": \"commonjs\""), "{manifest}");
}

#[cfg(unix)]
#[test]
fn test_clean_unlinks_a_symlinked_node_modules_but_keeps_the_target() {
    use std::os::unix::fs::symlink;

    let root = TempDir::new().unwrap();
    let target = root.path().join("shared");
    fs::create_dir_all(target.join("chalk")).unwrap();
    let link = root.path().join("node_modules");
    symlink(&target, &link).unwrap();
    fs::write(root.path().join(".skit-deps"), "v1\nnode\ndeadbeef\n").unwrap();

    clear_javascript_dependencies(root.path()).unwrap();

    assert!(fs::symlink_metadata(&link).is_err(), "node_modules symlink survived cleanup");
    assert!(target.join("chalk").is_dir(), "cleanup followed the symlink and deleted shared dependencies");
}