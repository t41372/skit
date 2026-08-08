use std::{
    cell::RefCell,
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

use skit_runtime::{
    DependencyCommand, DependencyCommandRunner, DependencyError, ProgramProbe,
    ensure_javascript_dependencies, javascript_dependency_manifest,
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

#[derive(Debug, Default)]
struct Runner {
    commands: RefCell<Vec<DependencyCommand>>,
    succeeds: bool,
}

impl DependencyCommandRunner for Runner {
    fn run(&self, command: &DependencyCommand) -> std::io::Result<bool> {
        self.commands.borrow_mut().push(command.clone());
        if self.succeeds {
            fs::create_dir_all(command.cwd.join("node_modules"))?;
        }
        Ok(self.succeeds)
    }
}

#[test]
fn manifest_is_deterministic_private_and_supports_scoped_version_specs() {
    let manifest = javascript_dependency_manifest(&[
        "zod@^4".to_owned(),
        "@scope/tool@2.1.0".to_owned(),
        "chalk".to_owned(),
    ])
    .unwrap();
    assert_eq!(
        manifest,
        "{\n  \"name\": \"skit-private-entry\",\n  \"private\": true,\n  \"dependencies\": {\n    \"@scope/tool\": \"2.1.0\",\n    \"chalk\": \"*\",\n    \"zod\": \"^4\"\n  }\n}\n"
    );
    assert!(matches!(
        javascript_dependency_manifest(&["../local".to_owned()]),
        Err(DependencyError::InvalidPackage { .. })
    ));
}

#[test]
fn each_runtime_uses_its_own_installer_and_disables_lifecycle_scripts() {
    let cases = [
        (
            "node",
            "npm",
            vec!["install", "--ignore-scripts", "--no-audit", "--no-fund"],
        ),
        (
            "bun",
            "bun",
            vec!["install", "--ignore-scripts", "--production"],
        ),
        (
            "deno",
            "deno",
            vec!["install", "--node-modules-dir=auto", "--prod"],
        ),
    ];

    for (runtime, installer, expected) in cases {
        let root = TempDir::new().unwrap();
        let runner = Runner {
            succeeds: true,
            ..Runner::default()
        };
        let probe = Probe {
            programs: BTreeMap::from([(
                installer.to_owned(),
                PathBuf::from(format!("/bin/{installer}")),
            )]),
        };
        ensure_javascript_dependencies(
            root.path(),
            runtime,
            &["chalk@5".to_owned()],
            &probe,
            &runner,
        )
        .unwrap();

        let commands = runner.commands.borrow();
        assert_eq!(commands.len(), 1);
        assert_eq!(
            commands[0].program,
            PathBuf::from(format!("/bin/{installer}"))
        );
        assert_eq!(commands[0].args, expected, "runtime={runtime}");
        assert_eq!(commands[0].cwd, root.path());
        assert!(root.path().join("package.json").is_file());
        assert!(root.path().join(".skit-deps").is_file());
    }
}

#[test]
fn a_matching_stamp_and_node_modules_skip_install_and_clearing_removes_owned_artifacts() {
    let root = TempDir::new().unwrap();
    let runner = Runner {
        succeeds: true,
        ..Runner::default()
    };
    let probe = Probe {
        programs: BTreeMap::from([("npm".to_owned(), PathBuf::from("/bin/npm"))]),
    };
    let dependencies = ["chalk@5".to_owned()];
    ensure_javascript_dependencies(root.path(), "node", &dependencies, &probe, &runner).unwrap();
    ensure_javascript_dependencies(root.path(), "node", &dependencies, &probe, &runner).unwrap();
    assert_eq!(runner.commands.borrow().len(), 1);

    fs::write(root.path().join("keep.txt"), "keep").unwrap();
    ensure_javascript_dependencies(root.path(), "node", &[], &probe, &runner).unwrap();
    assert!(!root.path().join("package.json").exists());
    assert!(!root.path().join("node_modules").exists());
    assert!(!root.path().join(".skit-deps").exists());
    assert!(root.path().join("keep.txt").is_file());
}

#[test]
fn installer_lookup_and_failure_are_typed_refusals_without_a_success_stamp() {
    let root = TempDir::new().unwrap();
    let runner = Runner::default();
    let missing = ensure_javascript_dependencies(
        root.path(),
        "node",
        &["chalk".to_owned()],
        &Probe::default(),
        &runner,
    )
    .unwrap_err();
    assert!(matches!(missing, DependencyError::InstallerNotFound { .. }));

    let probe = Probe {
        programs: BTreeMap::from([("npm".to_owned(), PathBuf::from("/bin/npm"))]),
    };
    let failed =
        ensure_javascript_dependencies(root.path(), "node", &["chalk".to_owned()], &probe, &runner)
            .unwrap_err();
    assert!(matches!(failed, DependencyError::InstallFailed { .. }));
    assert!(!root.path().join(".skit-deps").exists());
}
