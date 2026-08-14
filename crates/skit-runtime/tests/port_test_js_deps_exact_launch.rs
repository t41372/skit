//! Exact RunnerLaunch dependency ports from Python v0.4 `tests/test_js_deps.py`.
//!
//! Python kept runtime resolution, dependency preparation, and argv planning in one `RunnerLaunch`.
//! Rust splits those responsibilities into public runtime operations. These tests execute the same
//! transaction through those public boundaries rather than inventing a private compatibility seam.

use std::{
    cell::RefCell,
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

use skit_application::delivery::Assembly;
use skit_domain::{Entry, EntryKind, EntryMeta, EntrySettings, Slug, StorageMode};
use skit_runtime::{
    DependencyCommand, DependencyCommandRunner, DependencyError, LaunchPaths, ProgramProbe,
    build_launch_plan, ensure_javascript_dependencies_with_environment, resolve_javascript_runtime,
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

fn js_entry(mode: StorageMode, dependencies: &[&str]) -> Entry {
    let mut entry = Entry {
        slug: Slug::parse("demo").unwrap(),
        meta: EntryMeta::minimal("Demo", EntryKind::parse("js").unwrap()),
    };
    entry.meta.mode = mode;
    let settings = EntrySettings {
        dependencies: dependencies.iter().map(|item| (*item).to_owned()).collect(),
        ..EntrySettings::default()
    };
    settings.write_to_meta(&mut entry.meta);
    entry
}

fn launch_paths(root: &TempDir) -> LaunchPaths {
    let script = root.path().join("script.js");
    fs::write(&script, "console.log(1);\n").unwrap();
    LaunchPaths {
        script,
        entry_dir: root.path().to_path_buf(),
        invoke_cwd: root.path().to_path_buf(),
    }
}

#[test]
fn test_build_installs_declared_deps_with_the_resolved_runner() {
    let root = TempDir::new().unwrap();
    let entry = js_entry(StorageMode::Copy, &["chalk"]);
    let probe = Probe::with(&["node", "npm"]);
    let runner_name = resolve_javascript_runtime(&EntrySettings::from_meta(&entry.meta), &probe).unwrap();
    assert_eq!(runner_name, "node");

    let runner = Runner::default();
    let environment = BTreeMap::from([
        ("PATH".to_owned(), "/bin".to_owned()),
        (
            "NPM_CONFIG_REGISTRY".to_owned(),
            "https://registry.npmmirror.com".to_owned(),
        ),
    ]);
    ensure_javascript_dependencies_with_environment(
        root.path(),
        &runner_name,
        &["chalk".to_owned()],
        &environment,
        &probe,
        &runner,
    )
    .unwrap();

    let commands = runner.commands.borrow();
    assert_eq!(commands.len(), 1);
    assert_eq!(commands[0].cwd, root.path());
    assert_eq!(commands[0].environment["NPM_CONFIG_REGISTRY"], "https://registry.npmmirror.com");
    assert_eq!(commands[0].program, PathBuf::from("/bin/npm"));
}

#[test]
fn test_build_skips_the_engine_without_copy_mode_deps() {
    for (mode, dependencies) in [
        (StorageMode::Copy, Vec::<&str>::new()),
        (StorageMode::Reference, vec!["chalk"]),
    ] {
        let root = TempDir::new().unwrap();
        let entry = js_entry(mode, &dependencies);
        let probe = Probe::with(&["node"]);
        let plan = build_launch_plan(
            &entry,
            &launch_paths(&root),
            &Assembly::default(),
            None,
            None,
            &probe,
        )
        .unwrap_or_else(|error| panic!("mode={mode:?}, dependencies={dependencies:?}: {error}"));
        assert_eq!(plan.program, PathBuf::from("/bin/node"));
        assert!(plan.args.iter().any(|arg| arg.ends_with("script.js")), "{:?}", plan.args);
    }
}

#[test]
fn test_preflight_requires_the_installer_when_deps_are_declared() {
    let root = TempDir::new().unwrap();
    let entry = js_entry(StorageMode::Copy, &["chalk"]);
    let probe = Probe::with(&["node"]);
    let runner_name = resolve_javascript_runtime(&EntrySettings::from_meta(&entry.meta), &probe).unwrap();
    let error = ensure_javascript_dependencies_with_environment(
        root.path(),
        &runner_name,
        &["chalk".to_owned()],
        &BTreeMap::new(),
        &probe,
        &Runner::default(),
    )
    .unwrap_err();
    assert!(matches!(error, DependencyError::InstallerNotFound { ref name } if name == "npm"));
    assert!(error.to_string().contains("npm"), "{error}");
}

#[test]
fn test_preflight_without_deps_does_not_ask_for_an_installer() {
    let root = TempDir::new().unwrap();
    let entry = js_entry(StorageMode::Copy, &[]);
    let probe = Probe::with(&["node"]);
    let plan = build_launch_plan(
        &entry,
        &launch_paths(&root),
        &Assembly::default(),
        None,
        None,
        &probe,
    )
    .expect("a dependency-free JS entry must not require npm during preflight");
    assert_eq!(plan.program, PathBuf::from("/bin/node"));
}
