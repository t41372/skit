use std::{
    cell::RefCell,
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    time::{Duration, SystemTime},
};

use skit_runtime::{
    DependencyCommand, DependencyCommandOutput, DependencyCommandRunner, DependencyError,
    JavaScriptModuleType, ProgramProbe, SystemDependencyCommandRunner,
    clear_javascript_dependencies, ensure_javascript_dependencies,
    ensure_javascript_dependencies_for_module, ensure_javascript_dependencies_with_environment,
    javascript_dependency_manifest, javascript_module_type, sweep_stale_injected_sources,
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

#[test]
fn public_sweep_removes_only_injected_sources_older_than_one_hour() {
    let root = TempDir::new().unwrap();
    let stale = root.path().join(".injected-stale.js");
    let fresh = root.path().join(".injected-fresh.js");
    let unrelated = root.path().join("other.js");
    for path in [&stale, &fresh, &unrelated] {
        fs::write(path, b"value\n").unwrap();
    }
    let old = SystemTime::now()
        .checked_sub(Duration::from_secs(2 * 60 * 60))
        .unwrap();
    for path in [&stale, &unrelated] {
        fs::File::options()
            .write(true)
            .open(path)
            .unwrap()
            .set_times(fs::FileTimes::new().set_modified(old))
            .unwrap();
    }

    sweep_stale_injected_sources(root.path());

    assert!(!stale.exists());
    assert!(fresh.exists());
    assert!(unrelated.exists());
}

#[derive(Debug, Default)]
struct Runner {
    commands: RefCell<Vec<DependencyCommand>>,
    succeeds: bool,
}

#[derive(Debug)]
struct PartialFailureRunner {
    expected_cwd: PathBuf,
}

impl DependencyCommandRunner for PartialFailureRunner {
    fn run(&self, command: &DependencyCommand) -> std::io::Result<DependencyCommandOutput> {
        assert_eq!(command.cwd, self.expected_cwd);
        fs::write(command.cwd.join("package-lock.json"), b"partial lock\n")?;
        fs::create_dir_all(command.cwd.join("node_modules"))?;
        fs::write(
            command.cwd.join("node_modules/partial"),
            b"partial module\n",
        )?;
        Ok(DependencyCommandOutput {
            success: false,
            exit_code: Some(1),
            stderr: b"partial install failed".to_vec(),
        })
    }
}

impl DependencyCommandRunner for Runner {
    fn run(&self, command: &DependencyCommand) -> std::io::Result<DependencyCommandOutput> {
        self.commands.borrow_mut().push(command.clone());
        if self.succeeds {
            fs::create_dir_all(command.cwd.join("node_modules"))?;
        }
        Ok(DependencyCommandOutput {
            success: self.succeeds,
            exit_code: Some(i32::from(!self.succeeds)),
            stderr: Vec::new(),
        })
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
        "{\n  \"private\": true,\n  \"dependencies\": {\n    \"zod\": \"^4\",\n    \"@scope/tool\": \"2.1.0\",\n    \"chalk\": \"*\"\n  }\n}\n"
    );
    assert!(
        javascript_dependency_manifest(&["../local".to_owned()])
            .unwrap()
            .contains("\"../local\": \"*\"")
    );
    assert_eq!(
        javascript_dependency_manifest(&[
            "zod@3".to_owned(),
            "chalk@5".to_owned(),
            "zod@4".to_owned(),
        ])
        .unwrap(),
        "{\n  \"private\": true,\n  \"dependencies\": {\n    \"zod\": \"4\",\n    \"chalk\": \"5\"\n  }\n}\n"
    );
    assert!(
        javascript_dependency_manifest(&["@scope/@".to_owned()])
            .unwrap()
            .contains("\"@scope/@\": \"*\"")
    );
}

#[test]
fn module_flavor_is_materialized_and_part_of_the_freshness_stamp() {
    assert_eq!(
        javascript_module_type("/old/source.mjs"),
        Some(JavaScriptModuleType::Module)
    );
    assert_eq!(
        javascript_module_type("/old/source.cts"),
        Some(JavaScriptModuleType::CommonJs)
    );
    assert_eq!(javascript_module_type("/old/source.js"), None);

    let root = TempDir::new().unwrap();
    let runner = Runner {
        succeeds: true,
        ..Runner::default()
    };
    let probe = Probe {
        programs: BTreeMap::from([("npm".to_owned(), PathBuf::from("/bin/npm"))]),
    };
    let dependencies = ["chalk@5".to_owned()];
    ensure_javascript_dependencies_for_module(
        root.path(),
        "node",
        &dependencies,
        Some(JavaScriptModuleType::Module),
        &BTreeMap::new(),
        &probe,
        &runner,
    )
    .unwrap();
    let module_stamp = fs::read(root.path().join("node_modules/.skit-deps-ok")).unwrap();
    assert!(
        fs::read_to_string(root.path().join("package.json"))
            .unwrap()
            .contains("\"type\": \"module\"")
    );

    ensure_javascript_dependencies_for_module(
        root.path(),
        "node",
        &dependencies,
        Some(JavaScriptModuleType::CommonJs),
        &BTreeMap::new(),
        &probe,
        &runner,
    )
    .unwrap();
    assert_eq!(runner.commands.borrow().len(), 2);
    assert_ne!(
        fs::read(root.path().join("node_modules/.skit-deps-ok")).unwrap(),
        module_stamp
    );
    assert!(
        fs::read_to_string(root.path().join("package.json"))
            .unwrap()
            .contains("\"type\": \"commonjs\"")
    );
}

#[test]
fn a_dependency_free_module_keeps_only_an_explicit_module_manifest() {
    let root = TempDir::new().unwrap();
    ensure_javascript_dependencies_for_module(
        root.path(),
        "node",
        &[],
        Some(JavaScriptModuleType::Module),
        &BTreeMap::new(),
        &Probe::default(),
        &Runner::default(),
    )
    .unwrap();

    assert_eq!(
        fs::read_to_string(root.path().join("package.json")).unwrap(),
        "{\n  \"private\": true,\n  \"type\": \"module\"\n}\n"
    );
    assert!(!root.path().join(".skit-deps").exists());
    assert!(!root.path().join("node_modules").exists());
}

#[test]
fn a_module_manifest_read_error_is_typed_and_does_not_replace_the_path() {
    let root = TempDir::new().unwrap();
    fs::create_dir(root.path().join("package.json")).unwrap();

    let error = ensure_javascript_dependencies_for_module(
        root.path(),
        "node",
        &[],
        Some(JavaScriptModuleType::Module),
        &BTreeMap::new(),
        &Probe::default(),
        &Runner::default(),
    )
    .unwrap_err();

    assert!(matches!(
        error,
        DependencyError::Io {
            operation: "read",
            ..
        }
    ));
    assert!(root.path().join("package.json").is_dir());
}

#[test]
fn each_runtime_uses_its_own_installer_and_disables_lifecycle_scripts() {
    let cases = [
        (
            "node",
            "npm",
            vec!["install", "--no-audit", "--no-fund", "--ignore-scripts"],
        ),
        ("bun", "bun", vec!["install", "--ignore-scripts"]),
        ("deno", "deno", vec!["install"]),
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
        assert!(root.path().join("node_modules/.skit-deps-ok").is_file());
    }
}

#[test]
fn dependency_commands_receive_only_the_explicit_environment_overlay() {
    let root = TempDir::new().unwrap();
    let runner = Runner {
        succeeds: true,
        ..Runner::default()
    };
    let probe = Probe {
        programs: BTreeMap::from([("npm".to_owned(), PathBuf::from("/bin/npm"))]),
    };
    let environment = BTreeMap::from([(
        "NPM_CONFIG_REGISTRY".to_owned(),
        "https://registry.example.test".to_owned(),
    )]);

    ensure_javascript_dependencies_with_environment(
        root.path(),
        "node",
        &["chalk@5".to_owned()],
        &environment,
        &probe,
        &runner,
    )
    .unwrap();

    assert_eq!(runner.commands.borrow()[0].environment, environment);
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
    assert!(!root.path().join("node_modules/.skit-deps-ok").exists());
    assert!(root.path().join("keep.txt").is_file());
}

#[test]
fn installer_lookup_and_failure_are_typed_refusals_without_a_success_stamp() {
    let root = TempDir::new().unwrap();
    fs::write(root.path().join("meta.toml"), "name = \"Demo\"\n").unwrap();
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
    assert!(!root.path().join("package.json").exists());
    assert!(!root.path().join("node_modules/.skit-deps-ok").exists());

    let probe = Probe {
        programs: BTreeMap::from([("npm".to_owned(), PathBuf::from("/bin/npm"))]),
    };
    let failed =
        ensure_javascript_dependencies(root.path(), "node", &["chalk".to_owned()], &probe, &runner)
            .unwrap_err();
    assert!(matches!(
        failed,
        DependencyError::InstallFailed {
            exit_code: Some(1),
            ref detail,
            ..
        } if detail == "?"
    ));
    assert!(!root.path().join("package.json").exists());
    assert!(!root.path().join(".skit-deps").exists());
}

#[test]
fn a_failed_reinstall_restores_the_last_complete_environment() {
    let root = TempDir::new().unwrap();
    fs::write(root.path().join("meta.toml"), "name = \"Demo\"\n").unwrap();
    fs::write(root.path().join("package.json"), b"old manifest\n").unwrap();
    fs::write(root.path().join("package-lock.json"), b"old lock\n").unwrap();
    fs::create_dir(root.path().join("node_modules")).unwrap();
    fs::write(root.path().join("node_modules/old"), b"old module\n").unwrap();
    fs::write(
        root.path().join("node_modules/.skit-deps-ok"),
        b"old stamp\n",
    )
    .unwrap();
    let probe = Probe {
        programs: BTreeMap::from([("npm".to_owned(), PathBuf::from("/bin/npm"))]),
    };

    let error = ensure_javascript_dependencies(
        root.path(),
        "node",
        &["chalk@5".to_owned()],
        &probe,
        &Runner::default(),
    )
    .unwrap_err();

    assert!(matches!(error, DependencyError::InstallFailed { .. }));
    assert_eq!(
        fs::read(root.path().join("package.json")).unwrap(),
        b"old manifest\n"
    );
    assert_eq!(
        fs::read(root.path().join("package-lock.json")).unwrap(),
        b"old lock\n"
    );
    assert_eq!(
        fs::read(root.path().join("node_modules/.skit-deps-ok")).unwrap(),
        b"old stamp\n"
    );
    assert_eq!(
        fs::read(root.path().join("node_modules/old")).unwrap(),
        b"old module\n"
    );
}

#[test]
fn an_in_place_installer_failure_removes_partial_output_and_restores_the_old_tree() {
    let root = TempDir::new().unwrap();
    fs::write(root.path().join("package.json"), b"old manifest\n").unwrap();
    fs::write(root.path().join("package-lock.json"), b"old lock\n").unwrap();
    fs::create_dir(root.path().join("node_modules")).unwrap();
    fs::write(root.path().join("node_modules/old"), b"old module\n").unwrap();
    fs::write(root.path().join("node_modules/.skit-deps-ok"), b"old stamp").unwrap();
    let probe = Probe {
        programs: BTreeMap::from([("npm".to_owned(), PathBuf::from("/bin/npm"))]),
    };

    let error = ensure_javascript_dependencies(
        root.path(),
        "node",
        &["chalk@5".to_owned()],
        &probe,
        &PartialFailureRunner {
            expected_cwd: root.path().to_owned(),
        },
    )
    .unwrap_err();

    assert!(matches!(error, DependencyError::InstallFailed { .. }));
    assert_eq!(
        fs::read(root.path().join("package.json")).unwrap(),
        b"old manifest\n"
    );
    assert_eq!(
        fs::read(root.path().join("package-lock.json")).unwrap(),
        b"old lock\n"
    );
    assert_eq!(
        fs::read(root.path().join("node_modules/old")).unwrap(),
        b"old module\n"
    );
    assert!(!root.path().join("node_modules/partial").exists());
    assert_eq!(
        fs::read(root.path().join("node_modules/.skit-deps-ok")).unwrap(),
        b"old stamp"
    );
    assert!(!root.path().join(".skit-deps.backup").exists());
}

#[test]
fn a_crash_backup_is_restored_and_staging_leftovers_are_removed() {
    let root = TempDir::new().unwrap();
    fs::write(root.path().join("meta.toml"), "name = \"Demo\"\n").unwrap();
    fs::write(root.path().join("package.json"), b"partial manifest\n").unwrap();
    fs::write(root.path().join("package-lock.json"), b"partial lock\n").unwrap();
    let backup = root.path().join(".skit-deps.backup");
    fs::create_dir(&backup).unwrap();
    fs::write(backup.join(".items"), b"package.json\nnode_modules\n").unwrap();
    fs::write(backup.join("package.json"), b"stable manifest\n").unwrap();
    fs::create_dir(backup.join("node_modules")).unwrap();
    fs::write(backup.join("node_modules/stable"), b"stable module\n").unwrap();
    fs::write(backup.join("node_modules/.skit-deps-ok"), b"stable stamp").unwrap();
    fs::create_dir(root.path().join(".skit-deps.tmp-abandoned")).unwrap();

    let error = ensure_javascript_dependencies(
        root.path(),
        "node",
        &["chalk@5".to_owned()],
        &Probe::default(),
        &Runner::default(),
    )
    .unwrap_err();

    assert!(matches!(error, DependencyError::InstallerNotFound { .. }));
    assert_eq!(
        fs::read(root.path().join("package.json")).unwrap(),
        b"stable manifest\n"
    );
    assert_eq!(
        fs::read(root.path().join("node_modules/stable")).unwrap(),
        b"stable module\n"
    );
    assert_eq!(
        fs::read(root.path().join("node_modules/.skit-deps-ok")).unwrap(),
        b"stable stamp"
    );
    assert!(!root.path().join("package-lock.json").exists());
    assert!(!backup.exists());
    assert!(!root.path().join(".skit-deps.tmp-abandoned").exists());
}

#[test]
fn an_install_does_not_recreate_a_removed_entry() {
    let root = TempDir::new().unwrap();
    let entry = root.path().join("scripts/demo");
    fs::create_dir_all(entry.parent().unwrap()).unwrap();
    let probe = Probe {
        programs: BTreeMap::from([("npm".to_owned(), PathBuf::from("/bin/npm"))]),
    };
    let runner = Runner {
        succeeds: true,
        ..Runner::default()
    };

    let error =
        ensure_javascript_dependencies(&entry, "node", &["chalk@5".to_owned()], &probe, &runner)
            .unwrap_err();

    assert!(matches!(
        error,
        DependencyError::Io {
            operation: "inspect",
            ..
        }
    ));
    assert!(!entry.exists());
    assert!(runner.commands.borrow().is_empty());
}

#[test]
fn a_failed_update_keeps_the_last_complete_dependency_environment() {
    let root = TempDir::new().unwrap();
    let probe = Probe {
        programs: BTreeMap::from([("npm".to_owned(), PathBuf::from("/bin/npm"))]),
    };
    let success = Runner {
        succeeds: true,
        ..Runner::default()
    };
    ensure_javascript_dependencies(
        root.path(),
        "node",
        &["chalk@5".to_owned()],
        &probe,
        &success,
    )
    .unwrap();
    let old_manifest = fs::read(root.path().join("package.json")).unwrap();
    let old_stamp = fs::read(root.path().join("node_modules/.skit-deps-ok")).unwrap();

    let failure = Runner::default();
    assert!(matches!(
        ensure_javascript_dependencies(
            root.path(),
            "node",
            &["zod@4".to_owned()],
            &probe,
            &failure,
        ),
        Err(DependencyError::InstallFailed { .. })
    ));

    assert_eq!(
        fs::read(root.path().join("package.json")).unwrap(),
        old_manifest
    );
    assert_eq!(
        fs::read(root.path().join("node_modules/.skit-deps-ok")).unwrap(),
        old_stamp
    );
    assert!(root.path().join("node_modules").is_dir());
}

#[derive(Debug)]
struct ErrorRunner;

impl DependencyCommandRunner for ErrorRunner {
    fn run(&self, _command: &DependencyCommand) -> std::io::Result<DependencyCommandOutput> {
        Err(std::io::Error::other("cannot spawn"))
    }
}

#[test]
fn package_and_filesystem_refusals_do_not_escape_the_private_entry() {
    assert!(
        javascript_dependency_manifest(&[String::new()])
            .unwrap()
            .contains("\"dependencies\": {}")
    );
    for package in [".hidden", "a..b", "a/b", "@scope", "@/name", "name@"] {
        assert!(
            javascript_dependency_manifest(&[package.to_owned()]).is_ok(),
            "package={package:?}",
        );
    }

    let root = TempDir::new().unwrap();
    assert!(matches!(
        ensure_javascript_dependencies(
            root.path(),
            "future",
            &["chalk".to_owned()],
            &Probe::default(),
            &Runner::default(),
        ),
        Err(DependencyError::InstallerNotFound { name }) if name == "npm"
    ));
    let probe = Probe {
        programs: BTreeMap::from([("npm".to_owned(), PathBuf::from("/bin/npm"))]),
    };
    assert!(matches!(
        ensure_javascript_dependencies(
            root.path(),
            "node",
            &["chalk".to_owned()],
            &probe,
            &ErrorRunner,
        ),
        Err(DependencyError::InstallerStartFailed { installer, reason })
            if installer == "npm" && reason == "cannot spawn"
    ));
    assert!(matches!(
        clear_javascript_dependencies(Path::new("/")),
        Err(DependencyError::Io { .. })
    ));

    let blocker = root.path().join("blocker");
    fs::write(&blocker, "file").unwrap();
    assert!(matches!(
        clear_javascript_dependencies(&blocker.join("entry")),
        Err(DependencyError::Io { .. })
    ));

    let corrupt_stamp = root.path().join("corrupt-stamp");
    fs::create_dir_all(corrupt_stamp.join("node_modules")).unwrap();
    fs::write(corrupt_stamp.join("node_modules/.skit-deps-ok"), [0xff]).unwrap();
    assert!(matches!(
        ensure_javascript_dependencies(
            &corrupt_stamp,
            "node",
            &["chalk".to_owned()],
            &probe,
            &Runner::default(),
        ),
        Err(DependencyError::InstallFailed { .. })
    ));
}

#[test]
fn public_clear_removes_dependency_artifacts_and_preserves_entry_files() {
    let root = TempDir::new().unwrap();
    fs::write(root.path().join("keep.txt"), "keep").unwrap();
    fs::write(root.path().join("package.json"), "{\"name\":\"user\"}\n").unwrap();
    clear_javascript_dependencies(root.path()).unwrap();
    assert!(!root.path().join("package.json").exists());
    assert!(root.path().join("keep.txt").is_file());

    fs::write(
        root.path().join("package.json"),
        "{\"name\":\"skit-private-entry\"}\n",
    )
    .unwrap();
    fs::write(root.path().join("package-lock.json"), "lock").unwrap();
    fs::create_dir(root.path().join("node_modules")).unwrap();
    clear_javascript_dependencies(root.path()).unwrap();
    assert!(!root.path().join("package.json").exists());
    assert!(!root.path().join("package-lock.json").exists());

    fs::write(root.path().join(".skit-deps"), "owned").unwrap();
    fs::create_dir(root.path().join("node_modules")).unwrap();
    fs::write(root.path().join("node_modules/keep"), "old").unwrap();
    fs::write(root.path().join("package-lock.json"), "old lock").unwrap();
    fs::create_dir(root.path().join("package.json")).unwrap();
    assert!(matches!(
        clear_javascript_dependencies(root.path()),
        Err(DependencyError::Io { .. })
    ));
    assert_eq!(
        fs::read(root.path().join("node_modules/keep")).unwrap(),
        b"old"
    );
    assert_eq!(
        fs::read(root.path().join("package-lock.json")).unwrap(),
        b"old lock"
    );
    fs::remove_dir(root.path().join("package.json")).unwrap();
    fs::remove_dir_all(root.path().join("node_modules")).unwrap();
    fs::remove_file(root.path().join("package-lock.json")).unwrap();
    fs::remove_file(root.path().join(".skit-deps")).unwrap();

    #[cfg(unix)]
    {
        use std::os::unix::fs::symlink;
        let outside = root.path().join("outside");
        fs::create_dir(&outside).unwrap();
        fs::write(root.path().join(".skit-deps"), "owned").unwrap();
        symlink(&outside, root.path().join("node_modules")).unwrap();
        clear_javascript_dependencies(root.path()).unwrap();
        assert!(outside.is_dir());
        assert!(!root.path().join("node_modules").exists());
    }
}

#[test]
fn crash_left_backup_index_temp_does_not_block_recovery() {
    let root = TempDir::new().unwrap();
    let backup = root.path().join(".skit-deps.backup");
    fs::create_dir(&backup).unwrap();
    fs::write(
        backup.join(format!(".items.tmp-{}", std::process::id())),
        "partial",
    )
    .unwrap();

    clear_javascript_dependencies(root.path()).unwrap();

    assert!(!backup.exists());
}

#[test]
fn system_dependency_runner_reports_real_child_status() {
    // The contract is real child status and stderr passthrough, not one shell: each host runs
    // its own command interpreter and expects the exact bytes that interpreter writes. The cmd
    // forms are the flat Windows-validated idioms (`>&2 echo`, no space before `&`), and cmd's
    // echo always ends a line with CRLF, so the expected bytes name that per host.
    let (program, flag) = if cfg!(windows) {
        (PathBuf::from("cmd.exe"), "/C")
    } else {
        (PathBuf::from("/bin/sh"), "-c")
    };
    let (success_command, success_stderr): (&str, &[u8]) = if cfg!(windows) {
        (
            "echo ignored& >&2 echo diagnostic& exit 0",
            b"diagnostic\r\n",
        )
    } else {
        (
            "printf ignored; printf diagnostic >&2; exit 0",
            b"diagnostic",
        )
    };
    let (failure_command, failure_stderr): (&str, &[u8]) = if cfg!(windows) {
        (">&2 echo actionable& exit 23", b"actionable\r\n")
    } else {
        ("printf actionable >&2; exit 23", b"actionable")
    };
    let root = TempDir::new().unwrap();
    let runner = SystemDependencyCommandRunner;
    let success = runner
        .run(&DependencyCommand {
            program: program.clone(),
            args: vec![flag.to_owned(), success_command.to_owned()],
            cwd: root.path().to_owned(),
            environment: BTreeMap::new(),
        })
        .unwrap();
    assert!(success.success);
    assert_eq!(success.exit_code, Some(0));
    assert_eq!(success.stderr, success_stderr);
    let failure = runner
        .run(&DependencyCommand {
            program,
            args: vec![flag.to_owned(), failure_command.to_owned()],
            cwd: root.path().to_owned(),
            environment: BTreeMap::new(),
        })
        .unwrap();
    assert!(!failure.success);
    assert_eq!(failure.exit_code, Some(23));
    assert_eq!(failure.stderr, failure_stderr);
}

/// Build one crash backup directory with the given items.
fn crash_backup(root: &Path) -> PathBuf {
    let backup = root.join(".skit-deps.backup");
    fs::create_dir(&backup).unwrap();
    backup
}

#[test]
fn a_backup_that_holds_a_foreign_item_is_refused_without_touching_the_entry() {
    let root = TempDir::new().unwrap();
    fs::write(root.path().join("package.json"), b"user manifest\n").unwrap();
    fs::write(crash_backup(root.path()).join("mystery"), b"foreign\n").unwrap();

    let error = clear_javascript_dependencies(root.path()).unwrap_err();

    assert!(error.to_string().contains("unknown item"));
    assert_eq!(
        fs::read(root.path().join("package.json")).unwrap(),
        b"user manifest\n"
    );
}

#[test]
fn a_backup_index_that_names_a_foreign_item_is_refused() {
    let root = TempDir::new().unwrap();
    fs::write(crash_backup(root.path()).join(".items"), b"mystery\n").unwrap();

    let error = clear_javascript_dependencies(root.path()).unwrap_err();

    assert!(error.to_string().contains("index contains an unknown item"));
}

#[test]
fn a_backup_index_that_is_not_utf8_is_refused() {
    let root = TempDir::new().unwrap();
    fs::write(crash_backup(root.path()).join(".items"), [0xff, 0xfe]).unwrap();

    let error = clear_javascript_dependencies(root.path()).unwrap_err();

    assert!(matches!(error, DependencyError::Io { .. }));
}

#[test]
fn a_backup_index_that_names_a_missing_item_is_refused() {
    let root = TempDir::new().unwrap();
    fs::write(crash_backup(root.path()).join(".items"), b"package.json\n").unwrap();

    let error = clear_javascript_dependencies(root.path()).unwrap_err();

    assert!(error.to_string().contains("a backup item is missing"));
}

#[test]
fn a_backup_without_an_index_recovers_before_clear() {
    let root = TempDir::new().unwrap();
    let backup = crash_backup(root.path());
    fs::write(backup.join("package.json"), b"saved manifest\n").unwrap();
    fs::write(root.path().join("package.json"), b"partial manifest\n").unwrap();

    clear_javascript_dependencies(root.path()).unwrap();

    assert!(!root.path().join("package.json").exists());
    assert!(!root.path().join(".skit-deps.backup").exists());
}

// APFS rejects non-UTF-8 path components before the adapter can inspect them.
#[cfg(target_os = "linux")]
#[test]
fn a_backup_item_name_that_is_not_utf8_is_refused() {
    use std::os::unix::ffi::OsStrExt as _;

    let root = TempDir::new().unwrap();
    let backup = crash_backup(root.path());
    let name = std::ffi::OsStr::from_bytes(&[0xff, 0xfe]);
    fs::write(backup.join(name), b"bytes\n").unwrap();

    let error = clear_javascript_dependencies(root.path()).unwrap_err();

    assert!(error.to_string().contains("not valid UTF-8"));
}

#[cfg(unix)]
#[test]
fn an_unreadable_backup_item_is_a_typed_refusal() {
    use std::os::unix::fs::PermissionsExt as _;

    let root = TempDir::new().unwrap();
    let backup = crash_backup(root.path());
    fs::write(backup.join(".items.tmp-1"), b"partial\n").unwrap();
    // The listing still works, but no item inside can be inspected.
    fs::set_permissions(&backup, fs::Permissions::from_mode(0o444)).unwrap();

    let error = clear_javascript_dependencies(root.path()).unwrap_err();
    fs::set_permissions(&backup, fs::Permissions::from_mode(0o755)).unwrap();

    assert!(matches!(
        error,
        DependencyError::Io {
            operation: "inspect",
            ..
        }
    ));
}

#[cfg(unix)]
#[test]
fn an_entry_path_that_is_a_symlink_is_refused_before_any_write() {
    use std::os::unix::fs::symlink;

    let root = TempDir::new().unwrap();
    let real = root.path().join("real");
    fs::create_dir(&real).unwrap();
    let link = root.path().join("link");
    symlink(&real, &link).unwrap();

    let error = ensure_javascript_dependencies(
        &link,
        "node",
        &["chalk".to_owned()],
        &Probe {
            programs: BTreeMap::from([("npm".to_owned(), PathBuf::from("/bin/npm"))]),
        },
        &Runner::default(),
    )
    .unwrap_err();

    assert!(error.to_string().contains("not a directory"));
    assert!(!real.join("package.json").exists());
}

#[cfg(unix)]
#[test]
fn an_installer_cannot_make_the_success_marker_escape_through_a_node_modules_symlink() {
    use std::os::unix::fs::symlink;

    #[derive(Debug)]
    struct SymlinkRunner {
        target: PathBuf,
    }

    impl DependencyCommandRunner for SymlinkRunner {
        fn run(&self, command: &DependencyCommand) -> std::io::Result<DependencyCommandOutput> {
            symlink(&self.target, command.cwd.join("node_modules"))?;
            Ok(DependencyCommandOutput {
                success: true,
                exit_code: Some(0),
                stderr: Vec::new(),
            })
        }
    }

    let root = TempDir::new().unwrap();
    let outside = root.path().join("outside");
    fs::create_dir(&outside).unwrap();
    let entry = root.path().join("entry");
    fs::create_dir(&entry).unwrap();
    let error = ensure_javascript_dependencies(
        &entry,
        "node",
        &["chalk".to_owned()],
        &Probe {
            programs: BTreeMap::from([("npm".to_owned(), PathBuf::from("/bin/npm"))]),
        },
        &SymlinkRunner {
            target: outside.clone(),
        },
    )
    .unwrap_err();

    assert!(matches!(error, DependencyError::Io { .. }));
    assert!(!outside.join(".skit-deps-ok").exists());
    assert!(!entry.join("package.json").exists());
    assert!(!entry.join("node_modules").exists());
    assert!(!entry.join(".skit-deps.backup").exists());
}

#[cfg(unix)]
#[test]
fn a_symlinked_entry_directory_never_receives_a_write_or_a_removal() {
    use std::os::unix::fs::symlink;

    let root = TempDir::new().unwrap();
    let outside = root.path().join("outside");
    fs::create_dir(&outside).unwrap();
    fs::write(outside.join("package.json"), b"user manifest\n").unwrap();
    fs::write(outside.join(".skit-deps"), b"user stamp\n").unwrap();
    fs::create_dir(outside.join("node_modules")).unwrap();
    fs::write(outside.join("node_modules/keepme"), b"user module\n").unwrap();
    // A crash backup that recovery would otherwise restore over the user's files.
    let backup = outside.join(".skit-deps.backup");
    fs::create_dir(&backup).unwrap();
    fs::write(backup.join("package.json"), b"backup manifest\n").unwrap();
    fs::write(backup.join(".items"), b"package.json\n").unwrap();

    let link = root.path().join("link");
    symlink(&outside, &link).unwrap();

    let install = ensure_javascript_dependencies(
        &link,
        "node",
        &["chalk".to_owned()],
        &Probe {
            programs: BTreeMap::from([("npm".to_owned(), PathBuf::from("/bin/npm"))]),
        },
        &Runner::default(),
    )
    .unwrap_err();
    assert!(install.to_string().contains("not a directory"), "{install}");

    let clear = clear_javascript_dependencies(&link).unwrap_err();
    assert!(clear.to_string().contains("not a directory"), "{clear}");

    // Nothing behind the link changed.
    assert_eq!(
        fs::read(outside.join("package.json")).unwrap(),
        b"user manifest\n"
    );
    assert_eq!(
        fs::read(outside.join(".skit-deps")).unwrap(),
        b"user stamp\n"
    );
    assert_eq!(
        fs::read(outside.join("node_modules/keepme")).unwrap(),
        b"user module\n"
    );
    assert_eq!(
        fs::read(backup.join("package.json")).unwrap(),
        b"backup manifest\n"
    );
}
