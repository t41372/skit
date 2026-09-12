//! Observation mode provenance and copy fact contracts.

use super::*;

/// Every row is umask-independent except a symlink's own mode on macOS, where the OS applies
/// the umask to the link. The macOS branch pins both link modes and compares every other
/// field.
#[cfg(unix)]
#[test]
fn tree_observation_is_umask_independent_and_keeps_raw_mode_evidence() {
    use std::os::unix::fs::PermissionsExt as _;

    const CHILD: &str = "SKIT_TREE_OBSERVATION_UMASK_CHILD";
    const RESULT: &str = "SKIT_TREE_OBSERVATION_UMASK_RESULT";
    let test = harness_test_name(
        module_path!(),
        "tree_observation_is_umask_independent_and_keeps_raw_mode_evidence",
    );
    if std::env::var_os(CHILD).is_some() {
        let mut spec = profile();
        spec.external.extend([
            WalkerExternalSeed::File {
                path: PathBuf::from("readonly.txt"),
                bytes: b"readonly fixture\n".to_vec(),
                readonly: true,
                unix_mode: 0o440,
            },
            WalkerExternalSeed::Symlink {
                path: PathBuf::from("outside-link.sh"),
                target: PathBuf::from("outside.sh"),
            },
        ]);
        let mut host = RealWalkerHost::spawn(spec).unwrap();
        let config_path = host.roots().config.join("config.toml");
        let binary = host.service.show("Binary bytes").unwrap();
        let binary_path = host.service.repository().payload_path(&binary).unwrap();
        let external_path = host.external_root.join("outside.sh");
        let readonly_path = host.external_root.join("readonly.txt");
        let symlink_path = host.external_root.join("outside-link.sh");
        let raw_mode =
            |path: &Path| fs::symlink_metadata(path).unwrap().permissions().mode() & 0o7777;
        let before = json!({
            "data": raw_mode(&host.roots().data),
            "config": raw_mode(&config_path),
            "binary": raw_mode(&binary_path),
            "external": raw_mode(&external_path),
            "readonly_mode": raw_mode(&readonly_path),
            "readonly": fs::symlink_metadata(&readonly_path).unwrap().permissions().readonly(),
            "symlink": raw_mode(&symlink_path),
            "symlink_target": fs::read_link(&symlink_path).unwrap(),
        });
        let observation = host.observe(&host.initial_state().unwrap()).unwrap();
        let after = json!({
            "data": raw_mode(&host.roots().data),
            "config": raw_mode(&config_path),
            "binary": raw_mode(&binary_path),
            "external": raw_mode(&external_path),
            "readonly_mode": raw_mode(&readonly_path),
            "readonly": fs::symlink_metadata(&readonly_path).unwrap().permissions().readonly(),
            "symlink": raw_mode(&symlink_path),
            "symlink_target": fs::read_link(&symlink_path).unwrap(),
        });
        let tree_row = |path: &str| {
            observation
                .tree
                .iter()
                .find(|row| row.path == path)
                .unwrap()
        };
        assert_eq!(
            tree_row("data/scripts/binary-bytes/script.py").mode,
            Some(raw_mode(&binary_path))
        );
        assert_eq!(raw_mode(&binary_path), 0o4751);
        assert_eq!(
            tree_row("external/outside.sh").mode,
            Some(raw_mode(&external_path))
        );
        assert_eq!(raw_mode(&external_path), 0o640);
        let readonly = tree_row("external/readonly.txt");
        assert!(readonly.readonly);
        assert_eq!(readonly.mode, Some(raw_mode(&readonly_path)));
        assert_eq!(raw_mode(&readonly_path), 0o440);
        let symlink = tree_row("external/outside-link.sh");
        assert_eq!(symlink.kind, "symlink");
        assert_eq!(symlink.mode, Some(raw_mode(&symlink_path)));
        assert_eq!(
            symlink.target.as_deref(),
            Some("<profile:fixture>/external/outside.sh")
        );
        fs::write(
            PathBuf::from(std::env::var_os(RESULT).unwrap()),
            serde_json::to_vec(&json!({
                "observation": observation,
                "raw_before": before,
                "raw_after": after,
            }))
            .unwrap(),
        )
        .unwrap();
        return;
    }

    let results = tempfile::TempDir::new().unwrap();
    let run = |mask: &str, output: &Path| {
        let coverage_profile = restrictive_umask_child_profile();
        let mut command = std::process::Command::new("sh");
        command
            .arg("-c")
            .arg("umask \"$1\"; exec \"$2\" --exact \"$3\" --nocapture")
            .arg("sh")
            .arg(mask)
            .arg(std::env::current_exe().unwrap())
            .arg(&test)
            .env(CHILD, "1")
            .env(RESULT, output);
        if let Some(profile) = coverage_profile.as_ref() {
            command.env("LLVM_PROFILE_FILE", profile);
        }
        assert!(command.status().unwrap().success());
        serde_json::from_slice::<Value>(&fs::read(output).unwrap()).unwrap()
    };
    let public = run("0022", &results.path().join("0022.json"));
    let private = run("0077", &results.path().join("0077.json"));

    assert_ne!(public["raw_before"]["data"], private["raw_before"]["data"]);
    assert_ne!(
        public["raw_before"]["config"],
        private["raw_before"]["config"]
    );
    assert_eq!(public["raw_before"], public["raw_after"]);
    assert_eq!(private["raw_before"], private["raw_after"]);
    // macOS applies the umask to a new symbolic link. Linux always makes the link mode 0o777.
    // The observation keeps the raw symlink mode. Thus on macOS the link mode is the only
    // field that the umask changes. Prove its exact value, then compare all other fields.
    #[cfg(target_os = "macos")]
    {
        const LINK: &str = "external/outside-link.sh";
        let link_mode = |observation: &Value| {
            observation["tree"]
                .as_array()
                .unwrap()
                .iter()
                .find(|row| row["path"] == LINK)
                .unwrap()["mode"]
                .clone()
        };
        let without_link_mode = |observation: &Value| {
            let mut observation = observation.clone();
            for row in observation["tree"].as_array_mut().unwrap() {
                if row["path"] == LINK {
                    row["mode"] = Value::Null;
                }
            }
            observation
        };
        assert_eq!(link_mode(&public["observation"]), json!(0o755));
        assert_eq!(link_mode(&private["observation"]), json!(0o700));
        assert_eq!(
            without_link_mode(&public["observation"]),
            without_link_mode(&private["observation"])
        );
    }
    #[cfg(not(target_os = "macos"))]
    assert_eq!(public["observation"], private["observation"]);

    let tree = public["observation"]["tree"].as_array().unwrap();
    for path in ["data", "config/config.toml"] {
        let row = tree.iter().find(|row| row["path"] == path).unwrap();
        assert_eq!(row["mode"], Value::Null);
    }
}

#[test]
fn observation_mode_provenance_is_absolute_duplicate_and_kind_safe() {
    let root = tempfile::TempDir::new().unwrap();
    let file = root.path().join("exact-file");
    let directory = root.path().join("exact-directory");
    fs::write(&file, b"exact\n").unwrap();
    fs::create_dir(&directory).unwrap();
    let mut modes = ObservationModeProvenance::default();

    modes.register(&file, ObservationNodeKind::File).unwrap();
    modes.register(&file, ObservationNodeKind::File).unwrap();
    assert_eq!(
        modes
            .register(&file, ObservationNodeKind::Directory)
            .unwrap_err(),
        format!(
            "observation mode provenance conflicts at {}: file and directory",
            file.display()
        )
    );
    assert_eq!(
        modes
            .register(Path::new("relative"), ObservationNodeKind::File)
            .unwrap_err(),
        "an observation mode provenance path is not absolute: relative"
    );
    #[cfg(unix)]
    {
        assert_eq!(
            modes
                .register(Path::new("/"), ObservationNodeKind::File)
                .unwrap_err(),
            "an observation mode provenance path has no final component: /"
        );
        assert_eq!(
            modes
                .register_if_parent_present(Path::new("/"), ObservationNodeKind::File,)
                .unwrap_err(),
            "an observation mode provenance path has no final component: /"
        );
    }
    let missing_parent = root.path().join("missing-parent/leaf");
    let error = modes
        .register(&missing_parent, ObservationNodeKind::File)
        .unwrap_err();
    assert!(error.starts_with("could not resolve observation mode provenance parent "));
    assert!(error.contains(&missing_parent.parent().unwrap().display().to_string()));

    fs::remove_file(&file).unwrap();
    fs::create_dir(&file).unwrap();
    assert_eq!(
        modes.mode(&file, &fs::symlink_metadata(&file).unwrap()),
        Err(format!(
            "observation mode provenance expected file at {}, but found directory",
            file.display()
        ))
    );

    modes
        .register(&directory, ObservationNodeKind::Directory)
        .unwrap();
    assert_eq!(
        modes.mode(&directory, &fs::symlink_metadata(&directory).unwrap()),
        Ok(portable_mode(&fs::symlink_metadata(&directory).unwrap()))
    );

    #[cfg(unix)]
    {
        let blocker = root.path().join("not-a-directory");
        let child = blocker.join("child");
        fs::write(&blocker, b"blocker\n").unwrap();
        let error = modes.register_existing(&child).unwrap_err();
        assert!(error.starts_with("could not inspect observation mode provenance at "));
        assert!(error.contains(&child.display().to_string()));
    }
}

#[cfg(unix)]
#[test]
fn g2c_host_spawn_refuses_conflicting_external_leaf_kinds_before_writing() {
    let error = RealWalkerHost::spawn(WalkerSeedSpec {
        profile: "mode-conflict".to_owned(),
        external: vec![
            WalkerExternalSeed::Symlink {
                path: PathBuf::from("alias"),
                target: PathBuf::from("target"),
            },
            WalkerExternalSeed::File {
                path: PathBuf::from("alias"),
                bytes: b"target\n".to_vec(),
                readonly: false,
                unix_mode: 0o640,
            },
        ],
        ..WalkerSeedSpec::default()
    })
    .unwrap_err();

    assert_eq!(
        error,
        "walker external fixture leaf paths overlap: alias and alias"
    );
}

#[test]
fn created_copy_fact_failures_are_deferred_and_noncopy_results_are_ignored() {
    let host = RealWalkerHost::spawn(profile()).unwrap();
    let commit = |slug: &str| {
        serde_json::from_value::<Action>(json!({
            "add": {"commit_finished": {
                "request": 0,
                "result": {"Ok": slug},
            }},
        }))
        .unwrap()
    };
    let mut failed = ObservationModeProvenance::default();
    failed.record_created_copy(&host.service, &commit("missing-one"));
    let first = failed.deferred_error.clone().unwrap();
    failed.record_created_copy(&host.service, &commit("missing-two"));

    assert_eq!(failed.deferred_error.as_deref(), Some(first.as_str()));
    assert_eq!(failed.refresh_live_sources(&host.service), Err(first));

    let mut noncopy = ObservationModeProvenance::default();
    noncopy
        .try_record_created_copy(&host.service, &commit("reference"))
        .unwrap();
    noncopy
        .try_record_created_copy(&host.service, &commit("command"))
        .unwrap();
    assert!(noncopy.exact.is_empty());
    assert!(noncopy.deferred_error.is_none());
}

#[cfg(unix)]
#[test]
fn observation_modes_distinguish_equal_raw_bits_and_keep_special_bits() {
    use std::os::unix::fs::PermissionsExt as _;

    let root = tempfile::TempDir::new().unwrap();
    let exact = root.path().join("exact");
    let ordinary = root.path().join("ordinary");
    let special_file = root.path().join("special-file");
    let special_directory = root.path().join("special-directory");
    for path in [&exact, &ordinary, &special_file] {
        fs::write(path, b"mode\n").unwrap();
    }
    fs::create_dir(&special_directory).unwrap();
    fs::set_permissions(&exact, fs::Permissions::from_mode(0o600)).unwrap();
    fs::set_permissions(&ordinary, fs::Permissions::from_mode(0o600)).unwrap();
    fs::set_permissions(&special_file, fs::Permissions::from_mode(0o4751)).unwrap();
    fs::set_permissions(&special_directory, fs::Permissions::from_mode(0o1770)).unwrap();
    let mut modes = ObservationModeProvenance::default();
    modes.register(&exact, ObservationNodeKind::File).unwrap();

    assert_eq!(
        modes.mode(&exact, &fs::symlink_metadata(&exact).unwrap()),
        Ok(Some(0o600))
    );
    assert_eq!(
        modes.mode(&ordinary, &fs::symlink_metadata(&ordinary).unwrap()),
        Ok(None)
    );
    assert_eq!(
        modes.mode(&special_file, &fs::symlink_metadata(&special_file).unwrap()),
        Ok(Some(0o4751))
    );
    assert_eq!(
        modes.mode(
            &special_directory,
            &fs::symlink_metadata(&special_directory).unwrap()
        ),
        Ok(Some(0o1770))
    );
    assert_eq!(
        portable_mode(&fs::symlink_metadata(&ordinary).unwrap()),
        Some(0o600)
    );
}

#[cfg(unix)]
#[test]
fn observation_mode_provenance_refuses_a_file_replaced_by_a_symlink() {
    let root = tempfile::TempDir::new().unwrap();
    let source = root.path().join("source");
    let replacement = root.path().join("replacement");
    fs::write(&source, b"source\n").unwrap();
    fs::write(&replacement, b"replacement\n").unwrap();
    let mut modes = ObservationModeProvenance::default();
    modes.register(&source, ObservationNodeKind::File).unwrap();
    fs::remove_file(&source).unwrap();
    std::os::unix::fs::symlink(&replacement, &source).unwrap();

    assert_eq!(
        modes.mode(&source, &fs::symlink_metadata(&source).unwrap()),
        Err(format!(
            "observation mode provenance expected file at {}, but found symlink",
            source.display()
        ))
    );
}

#[cfg(unix)]
#[test]
fn observation_mode_keys_resolve_a_registered_parent_alias_to_the_tree_path() {
    let root = tempfile::TempDir::new().unwrap();
    let physical = root.path().join("physical");
    let alias = root.path().join("alias");
    fs::create_dir(&physical).unwrap();
    std::os::unix::fs::symlink(&physical, &alias).unwrap();
    let tree_path = physical.join("source.sh");
    let alias_path = alias.join("source.sh");
    fs::write(&tree_path, b"source\n").unwrap();
    set_unix_mode(&tree_path, 0o640).unwrap();
    let mut modes = ObservationModeProvenance::default();
    modes
        .register(&alias_path, ObservationNodeKind::File)
        .unwrap();

    assert_eq!(
        modes.mode(&tree_path, &fs::symlink_metadata(&tree_path).unwrap()),
        Ok(Some(0o640))
    );
}

#[cfg(unix)]
#[test]
fn observation_mode_keys_resolve_a_tree_path_to_the_lookup_parent_alias() {
    let root = tempfile::TempDir::new().unwrap();
    let physical = root.path().join("physical");
    let alias = root.path().join("alias");
    fs::create_dir(&physical).unwrap();
    std::os::unix::fs::symlink(&physical, &alias).unwrap();
    let tree_path = physical.join("source.sh");
    let alias_path = alias.join("source.sh");
    fs::write(&tree_path, b"source\n").unwrap();
    set_unix_mode(&tree_path, 0o640).unwrap();
    let mut modes = ObservationModeProvenance::default();
    modes
        .register(&tree_path, ObservationNodeKind::File)
        .unwrap();

    assert_eq!(
        modes.mode(&alias_path, &fs::symlink_metadata(&alias_path).unwrap()),
        Ok(Some(0o640))
    );
}

#[cfg(unix)]
#[test]
fn observation_mode_keys_never_follow_the_final_symlink_leaf() {
    let root = tempfile::TempDir::new().unwrap();
    let physical = root.path().join("physical");
    let alias = root.path().join("alias");
    fs::create_dir(&physical).unwrap();
    std::os::unix::fs::symlink(&physical, &alias).unwrap();
    let tree_path = physical.join("source.sh");
    let alias_path = alias.join("source.sh");
    let target = physical.join("target.sh");
    fs::write(&tree_path, b"source\n").unwrap();
    fs::write(&target, b"target\n").unwrap();
    let mut modes = ObservationModeProvenance::default();
    modes
        .register(&alias_path, ObservationNodeKind::File)
        .unwrap();
    fs::remove_file(&tree_path).unwrap();
    std::os::unix::fs::symlink(&target, &tree_path).unwrap();

    assert_eq!(
        modes.mode(&tree_path, &fs::symlink_metadata(&tree_path).unwrap()),
        Err(format!(
            "observation mode provenance expected file at {}, but found symlink",
            tree_path.display()
        ))
    );
    let mut distinct = ObservationModeProvenance::default();
    distinct
        .register(&alias_path, ObservationNodeKind::Symlink)
        .unwrap();
    distinct
        .register(&target, ObservationNodeKind::File)
        .unwrap();
}

#[test]
fn stale_observation_mode_provenance_has_no_tree_row() {
    let mut host = RealWalkerHost::spawn(profile()).unwrap();
    let stale = host.roots().state.join("removed-source");
    host.observation_modes
        .get_mut()
        .register(&stale, ObservationNodeKind::File)
        .unwrap();

    let observation = host.observe(&host.initial_state().unwrap()).unwrap();

    assert!(
        !observation
            .tree
            .iter()
            .any(|row| row.path == "state/removed-source")
    );
}

#[cfg(not(unix))]
#[test]
fn non_unix_observation_modes_are_none_and_readonly_stays_physical() {
    let root = tempfile::TempDir::new().unwrap();
    let path = root.path().join("readonly");
    fs::write(&path, b"readonly\n").unwrap();
    let mut permissions = fs::metadata(&path).unwrap().permissions();
    permissions.set_readonly(true);
    fs::set_permissions(&path, permissions).unwrap();
    let metadata = fs::symlink_metadata(&path).unwrap();
    let mut modes = ObservationModeProvenance::default();
    modes.register(&path, ObservationNodeKind::File).unwrap();

    assert_eq!(modes.mode(&path, &metadata), Ok(None));
    assert!(metadata.permissions().readonly());
}
