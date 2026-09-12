//! Profile sandbox candidate, cleanup, quarantine, recovery, and acquisition contracts.

use super::*;

#[cfg(target_os = "linux")]
#[test]
fn candidate_open_refuses_fixed_child_kinds_and_identity_replacements() {
    for use_link in [false, true] {
        let parent = tempfile::TempDir::new().unwrap();
        let paths = stable_paths(
            parent.path().join(STABLE_SANDBOX_NAMESPACE),
            safe_profile(if use_link {
                "candidate-child-link"
            } else {
                "candidate-child-file"
            }),
        )
        .unwrap();
        let (handles, namespace_marker) =
            initialize_namespace_retained(&paths, &SandboxFaults::default()).unwrap();
        create_valid_sandbox_evidence(&paths.sandbox_root, &paths);
        fs::remove_dir(paths.sandbox_root.join("data")).unwrap();
        let outside = parent.path().join("outside-data");
        if use_link {
            fs::write(&outside, b"outside bytes").unwrap();
            std::os::unix::fs::symlink(&outside, paths.sandbox_root.join("data")).unwrap();
        } else {
            write_private_test_file(&paths.sandbox_root.join("data"), b"owned file");
        }

        assert!(matches!(
            validate_original_candidate(handles.sandboxes(), &paths.sandbox_name(), &paths,),
            Err(SandboxError::InvalidEvidence { .. })
        ));
        if use_link {
            assert_eq!(fs::read(&outside).unwrap(), b"outside bytes");
            assert!(
                fs::symlink_metadata(paths.sandbox_root.join("data"))
                    .unwrap()
                    .file_type()
                    .is_symlink()
            );
        } else {
            assert_eq!(
                fs::read(paths.sandbox_root.join("data")).unwrap(),
                b"owned file"
            );
        }
        drop(namespace_marker);
    }

    let child_parent = tempfile::TempDir::new().unwrap();
    let child_paths = stable_paths(
        child_parent.path().join(STABLE_SANDBOX_NAMESPACE),
        safe_profile("candidate-child-swap"),
    )
    .unwrap();
    let (handles, namespace_marker) =
        initialize_namespace_retained(&child_paths, &SandboxFaults::default()).unwrap();
    create_valid_sandbox_evidence(&child_paths.sandbox_root, &child_paths);
    let parked_child = child_paths.sandbox_root.join("parked-data");
    let original_identity = std::cell::Cell::new(None);
    let replacement_identity = std::cell::Cell::new(None);
    let child_result = open_candidate_parts_with_hooks(
        handles.sandboxes(),
        &child_paths.sandbox_name(),
        &child_paths,
        |root, tickets| {
            let data_name = crate::cli::tui_real_sandbox_fs::ChildName::literal("data");
            let ticket = tickets
                .iter()
                .find(|ticket| ticket.name() == &data_name)
                .unwrap();
            original_identity.set(Some(ticket.identity()));
            fs::rename(root.path().join("data"), &parked_child).unwrap();
            let replacement = root.create_directory(&data_name).unwrap();
            replacement_identity.set(Some(replacement.identity()));
        },
        no_candidate_root_hook,
    );
    assert!(matches!(
        child_result,
        Err(SandboxError::InvalidEvidence { .. })
    ));
    assert_eq!(
        handles
            .sandboxes()
            .open_directory(&child_paths.sandbox_name())
            .unwrap()
            .open_directory(&crate::cli::tui_real_sandbox_fs::ChildName::literal(
                "parked-data"
            ))
            .unwrap()
            .identity(),
        original_identity.get().unwrap()
    );
    assert_ne!(
        original_identity.get().unwrap(),
        replacement_identity.get().unwrap()
    );
    drop(namespace_marker);

    let root_parent = tempfile::TempDir::new().unwrap();
    let root_paths = stable_paths(
        root_parent.path().join(STABLE_SANDBOX_NAMESPACE),
        safe_profile("candidate-root-swap"),
    )
    .unwrap();
    let (handles, namespace_marker) =
        initialize_namespace_retained(&root_paths, &SandboxFaults::default()).unwrap();
    create_valid_sandbox_evidence(&root_paths.sandbox_root, &root_paths);
    let parked_name = crate::cli::tui_real_sandbox_fs::ChildName::literal("parked-candidate-root");
    let parked_root = root_paths.sandboxes_root.join(parked_name.as_os_str());
    let original_identity = std::cell::Cell::new(None);
    let replacement_identity = std::cell::Cell::new(None);
    let root_result = open_candidate_parts_with_hooks(
        handles.sandboxes(),
        &root_paths.sandbox_name(),
        &root_paths,
        no_candidate_tickets_hook,
        |parent, root, name| {
            original_identity.set(Some(root.identity()));
            fs::rename(root.path(), &parked_root).unwrap();
            let replacement = parent.create_directory(name).unwrap();
            replacement_identity.set(Some(replacement.identity()));
        },
    );
    assert!(matches!(
        root_result,
        Err(SandboxError::InvalidEvidence { .. })
    ));
    let parked = handles.sandboxes().open_directory(&parked_name).unwrap();
    let replacement = handles
        .sandboxes()
        .open_directory(&root_paths.sandbox_name())
        .unwrap();
    assert_eq!(parked.identity(), original_identity.get().unwrap());
    assert_eq!(replacement.identity(), replacement_identity.get().unwrap());
    assert_ne!(parked.identity(), replacement.identity());
    assert_eq!(
        fs::read(parked_root.join(SANDBOX_MARKER_FILE)).unwrap(),
        root_paths.sandbox_marker_bytes()
    );
    assert!(replacement.tickets().unwrap().is_empty());
    drop(namespace_marker);
}

#[cfg(target_os = "linux")]
#[test]
fn candidate_classification_propagates_real_open_io() {
    let parent = tempfile::TempDir::new().unwrap();
    let paths = stable_paths(
        parent.path().join(STABLE_SANDBOX_NAMESPACE),
        safe_profile("candidate-open-io"),
    )
    .unwrap();
    let (handles, namespace_marker) =
        initialize_namespace_retained(&paths, &SandboxFaults::default()).unwrap();
    create_valid_sandbox_evidence(&paths.sandbox_root, &paths);
    let parked_root = paths.sandboxes_root.join("parked-open-io");

    let result = classify_retained_sandbox(
        handles.sandboxes(),
        &paths.sandbox_name(),
        |parent, name| {
            fs::rename(&paths.sandbox_root, &parked_root).unwrap();
            validate_original_candidate(parent, name, &paths)
        },
    );

    assert!(matches!(
        result,
        Err(SandboxError::Io { source, .. })
            if source.kind() == io::ErrorKind::NotFound
    ));
    assert_eq!(
        fs::read(parked_root.join(SANDBOX_MARKER_FILE)).unwrap(),
        paths.sandbox_marker_bytes()
    );
    assert!(!paths.sandbox_root.exists());
    drop(namespace_marker);
}

#[cfg(target_os = "linux")]
#[test]
fn fresh_candidate_refuses_marker_collision_and_post_publish_child_replacement() {
    let marker_parent = tempfile::TempDir::new().unwrap();
    let marker_paths = stable_paths(
        marker_parent.path().join(STABLE_SANDBOX_NAMESPACE),
        safe_profile("fresh-marker-collision"),
    )
    .unwrap();
    let (handles, namespace_marker) =
        initialize_namespace_retained(&marker_paths, &SandboxFaults::default()).unwrap();
    let inserted_identity = std::cell::Cell::new(None);
    let marker_result = create_fresh_sandbox_retained_with_hooks(
        &marker_paths,
        &handles,
        &SandboxFaults::default(),
        |root, marker_name| {
            let inserted = root.create_directory(marker_name).unwrap();
            inserted_identity.set(Some(inserted.identity()));
        },
        no_fresh_sandbox_after_marker_hook,
    );
    assert!(matches!(
        marker_result,
        Err(SandboxError::Io { source, .. })
            if source.kind() == io::ErrorKind::AlreadyExists
    ));
    let root = handles
        .sandboxes()
        .open_directory(&marker_paths.sandbox_name())
        .unwrap();
    assert_eq!(
        root.open_directory(&marker_paths.sandbox_marker_name())
            .unwrap()
            .identity(),
        inserted_identity.get().unwrap()
    );
    for name in marker_paths.sandbox_child_names() {
        assert!(root.open_directory(&name).is_ok());
    }
    assert!(
        prepare_fresh_sandbox_retained(&marker_paths, &handles, &SandboxFaults::default(),)
            .is_err()
    );
    assert_eq!(
        root.open_directory(&marker_paths.sandbox_marker_name())
            .unwrap()
            .identity(),
        inserted_identity.get().unwrap()
    );
    drop(namespace_marker);

    let child_parent = tempfile::TempDir::new().unwrap();
    let child_paths = stable_paths(
        child_parent.path().join(STABLE_SANDBOX_NAMESPACE),
        safe_profile("fresh-child-replacement"),
    )
    .unwrap();
    let (handles, namespace_marker) =
        initialize_namespace_retained(&child_paths, &SandboxFaults::default()).unwrap();
    let parked_child = child_paths.sandbox_root.join("parked-data");
    let original_identity = std::cell::Cell::new(None);
    let replacement_identity = std::cell::Cell::new(None);
    let child_result = create_fresh_sandbox_retained_with_hooks(
        &child_paths,
        &handles,
        &SandboxFaults::default(),
        no_fresh_sandbox_before_marker_hook,
        |root| {
            let data_name = crate::cli::tui_real_sandbox_fs::ChildName::literal("data");
            let original = root.open_directory(&data_name).unwrap();
            original_identity.set(Some(original.identity()));
            drop(original);
            fs::rename(root.path().join("data"), &parked_child).unwrap();
            let replacement = root.create_directory(&data_name).unwrap();
            replacement_identity.set(Some(replacement.identity()));
        },
    );
    assert!(matches!(
        child_result,
        Err(SandboxError::InvalidEvidence { .. })
    ));
    assert_eq!(
        fs::read(child_paths.sandbox_root.join(SANDBOX_MARKER_FILE)).unwrap(),
        child_paths.sandbox_marker_bytes()
    );
    assert_ne!(
        original_identity.get().unwrap(),
        replacement_identity.get().unwrap()
    );
    assert!(parked_child.is_dir());
    assert!(child_paths.sandbox_root.join("data").is_dir());
    drop(namespace_marker);
}

#[cfg(target_os = "linux")]
#[test]
fn live_original_and_cleanup_validation_refuse_child_replacements() {
    for cleanup in [false, true] {
        let parent = tempfile::TempDir::new().unwrap();
        let paths = stable_paths(
            parent.path().join(STABLE_SANDBOX_NAMESPACE),
            safe_profile(if cleanup {
                "live-cleanup-child"
            } else {
                "live-original-child"
            }),
        )
        .unwrap();
        let (handles, namespace_marker) =
            initialize_namespace_retained(&paths, &SandboxFaults::default()).unwrap();
        let (name, root_path) = if cleanup {
            (paths.cleanup_name(), paths.cleanup_root.clone())
        } else {
            (paths.sandbox_name(), paths.sandbox_root.clone())
        };
        create_valid_sandbox_evidence(&root_path, &paths);
        let parked_child = root_path.join("parked-data");
        let original_identity = std::cell::Cell::new(None);
        let replacement_identity = std::cell::Cell::new(None);

        let result = if cleanup {
            let sandbox = validate_cleanup_candidate(handles.sandboxes(), &name, &paths).unwrap();
            validate_same_cleanup_with_hook(
                &sandbox,
                handles.sandboxes(),
                &name,
                &paths,
                |root, child_name, child| {
                    if child_name == &crate::cli::tui_real_sandbox_fs::ChildName::literal("data") {
                        original_identity.set(Some(child.identity()));
                        fs::rename(child.path(), &parked_child).unwrap();
                        let replacement = root.create_directory(child_name).unwrap();
                        replacement_identity.set(Some(replacement.identity()));
                    }
                },
            )
        } else {
            let sandbox = validate_original_candidate(handles.sandboxes(), &name, &paths).unwrap();
            validate_same_original_with_hook(
                &sandbox,
                handles.sandboxes(),
                &name,
                &paths,
                |root, child_name, child| {
                    if child_name == &crate::cli::tui_real_sandbox_fs::ChildName::literal("data") {
                        original_identity.set(Some(child.identity()));
                        fs::rename(child.path(), &parked_child).unwrap();
                        let replacement = root.create_directory(child_name).unwrap();
                        replacement_identity.set(Some(replacement.identity()));
                    }
                },
            )
        };

        assert!(matches!(result, Err(SandboxError::InvalidEvidence { .. })));
        assert_ne!(
            original_identity.get().unwrap(),
            replacement_identity.get().unwrap()
        );
        assert!(parked_child.is_dir());
        assert!(root_path.join("data").is_dir());
        drop(namespace_marker);
    }
}

#[cfg(target_os = "linux")]
fn retained_cleanup_fixture(
    profile: &str,
) -> (
    tempfile::TempDir,
    StableSandboxPaths,
    crate::cli::tui_real_sandbox_fs::NamespaceHandles,
    crate::cli::tui_real_sandbox_fs::PinnedFile,
    crate::cli::tui_real_sandbox_fs::ValidatedCleanupSandbox,
) {
    let parent = tempfile::TempDir::new().unwrap();
    let paths = stable_paths(
        parent.path().join(STABLE_SANDBOX_NAMESPACE),
        safe_profile(profile),
    )
    .unwrap();
    let (handles, namespace_marker) =
        initialize_namespace_retained(&paths, &SandboxFaults::default()).unwrap();
    create_valid_sandbox_evidence(&paths.cleanup_root, &paths);
    let sandbox =
        validate_cleanup_candidate(handles.sandboxes(), &paths.cleanup_name(), &paths).unwrap();
    (parent, paths, handles, namespace_marker, sandbox)
}

#[cfg(target_os = "linux")]
#[test]
fn cleanup_plan_refuses_unexpected_and_replaced_entries_before_removal() {
    #[derive(Clone, Copy, Debug)]
    enum Race {
        Unexpected,
        FileReplacement,
        DirectoryReplacement,
    }

    for race in [
        Race::Unexpected,
        Race::FileReplacement,
        Race::DirectoryReplacement,
    ] {
        let (parent, paths, handles, namespace_marker, sandbox) =
            retained_cleanup_fixture(match race {
                Race::Unexpected => "cleanup-unexpected",
                Race::FileReplacement => "cleanup-file-replacement",
                Race::DirectoryReplacement => "cleanup-directory-replacement",
            });
        let config_name = crate::cli::tui_real_sandbox_fs::ChildName::literal("config");
        let original_identity = sandbox.children().get(&config_name).unwrap().identity();
        let parked_child = paths.cleanup_root.join("parked-config");
        let replacement_identity = std::cell::Cell::new(None);
        let unexpected_identity = std::cell::Cell::new(None);

        let result = remove_cleanup_sandbox_with_hooks(
            &paths,
            &handles,
            sandbox,
            &SandboxFaults::default(),
            CleanupHooks {
                after_plan: |root: &crate::cli::tui_real_sandbox_fs::PinnedDirectory| match race {
                    Race::Unexpected => {
                        let unexpected = root
                            .create_directory(&crate::cli::tui_real_sandbox_fs::ChildName::literal(
                                "0-unexpected",
                            ))
                            .unwrap();
                        unexpected_identity.set(Some(unexpected.identity()));
                    }
                    Race::FileReplacement => {
                        fs::rename(root.path().join("config"), &parked_child).unwrap();
                        let replacement = root.create_file(&config_name).unwrap();
                        replacement
                            .publish_bytes(root, &config_name, b"replacement file")
                            .unwrap();
                        replacement_identity.set(Some(replacement.identity()));
                    }
                    Race::DirectoryReplacement => {
                        fs::rename(root.path().join("config"), &parked_child).unwrap();
                        let replacement = root.create_directory(&config_name).unwrap();
                        replacement_identity.set(Some(replacement.identity()));
                    }
                },
                after_children: no_cleanup_marker_hook,
                after_marker_read: no_cleanup_marker_hook,
                before_root_ticket: no_cleanup_root_hook,
            },
        );

        assert!(matches!(result, Err(SandboxError::InvalidEvidence { .. })));
        let root = handles
            .sandboxes()
            .open_directory(&paths.cleanup_name())
            .unwrap();
        assert_eq!(
            fs::read(root.path().join(SANDBOX_MARKER_FILE)).unwrap(),
            paths.sandbox_marker_bytes()
        );
        match race {
            Race::Unexpected => {
                assert_eq!(
                    root.open_directory(&crate::cli::tui_real_sandbox_fs::ChildName::literal(
                        "0-unexpected"
                    ))
                    .unwrap()
                    .identity(),
                    unexpected_identity.get().unwrap()
                );
                for name in paths.sandbox_child_names() {
                    assert!(root.open_directory(&name).is_ok());
                }
            }
            Race::FileReplacement => {
                assert_eq!(
                    root.open_directory(&crate::cli::tui_real_sandbox_fs::ChildName::literal(
                        "parked-config"
                    ))
                    .unwrap()
                    .identity(),
                    original_identity
                );
                let replacement = root.open_file(&config_name, false).unwrap();
                assert_eq!(replacement.identity(), replacement_identity.get().unwrap());
                assert_eq!(fs::read(replacement.path()).unwrap(), b"replacement file");
            }
            Race::DirectoryReplacement => {
                assert_eq!(
                    root.open_directory(&crate::cli::tui_real_sandbox_fs::ChildName::literal(
                        "parked-config"
                    ))
                    .unwrap()
                    .identity(),
                    original_identity
                );
                let replacement = root.open_directory(&config_name).unwrap();
                assert_eq!(replacement.identity(), replacement_identity.get().unwrap());
            }
        }
        assert!(paths.cleanup_root.exists());
        drop(namespace_marker);
        drop(parent);
    }
}

#[cfg(target_os = "linux")]
#[test]
fn cleanup_marker_rechecks_preserve_each_marker_last_failure_boundary() {
    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    enum Race {
        Corrupt,
        Remove,
        Replace,
    }

    for race in [Race::Corrupt, Race::Remove, Race::Replace] {
        let (parent, paths, handles, namespace_marker, sandbox) =
            retained_cleanup_fixture(match race {
                Race::Corrupt => "cleanup-corrupt-marker",
                Race::Remove => "cleanup-removed-marker",
                Race::Replace => "cleanup-replaced-marker",
            });
        let original_identity = sandbox.marker().identity();
        let parked_marker = paths.cleanup_root.join("parked-marker");
        let replacement_identity = std::cell::Cell::new(None);
        let corrupt_bytes = b"corrupt cleanup marker";

        let result = remove_cleanup_sandbox_with_hooks(
            &paths,
            &handles,
            sandbox,
            &SandboxFaults::default(),
            CleanupHooks {
                after_plan: no_cleanup_plan_hook,
                after_children:
                    |root: &crate::cli::tui_real_sandbox_fs::PinnedDirectory,
                     marker: &crate::cli::tui_real_sandbox_fs::PinnedFile| {
                        if race == Race::Corrupt {
                            use std::io::{Seek as _, Write as _};

                            let writable =
                                root.open_file(&paths.sandbox_marker_name(), true).unwrap();
                            assert_eq!(writable.identity(), marker.identity());
                            let mut file = writable.file().try_clone().unwrap();
                            file.set_len(0).unwrap();
                            file.seek(io::SeekFrom::Start(0)).unwrap();
                            file.write_all(corrupt_bytes).unwrap();
                            file.sync_all().unwrap();
                        }
                    },
                after_marker_read:
                    |root: &crate::cli::tui_real_sandbox_fs::PinnedDirectory,
                     marker: &crate::cli::tui_real_sandbox_fs::PinnedFile| {
                        if race == Race::Remove {
                            fs::remove_file(marker.path()).unwrap();
                        } else {
                            assert_eq!(race, Race::Replace);
                            fs::rename(marker.path(), &parked_marker).unwrap();
                            let replacement =
                                root.create_file(&paths.sandbox_marker_name()).unwrap();
                            replacement
                                .publish_bytes(
                                    root,
                                    &paths.sandbox_marker_name(),
                                    paths.sandbox_marker_bytes(),
                                )
                                .unwrap();
                            replacement_identity.set(Some(replacement.identity()));
                        }
                    },
                before_root_ticket: no_cleanup_root_hook,
            },
        );

        assert!(matches!(result, Err(SandboxError::InvalidEvidence { .. })));
        let root = handles
            .sandboxes()
            .open_directory(&paths.cleanup_name())
            .unwrap();
        for name in paths.sandbox_child_names() {
            assert!(root.open_directory(&name).is_err());
        }
        match race {
            Race::Corrupt => {
                let marker = root.open_file(&paths.sandbox_marker_name(), false).unwrap();
                assert_eq!(marker.identity(), original_identity);
                assert_eq!(fs::read(marker.path()).unwrap(), corrupt_bytes);
            }
            Race::Remove => {
                assert!(root.open_file(&paths.sandbox_marker_name(), false).is_err());
                assert!(root.tickets().unwrap().is_empty());
            }
            Race::Replace => {
                let parked = root
                    .open_file(
                        &crate::cli::tui_real_sandbox_fs::ChildName::literal("parked-marker"),
                        false,
                    )
                    .unwrap();
                let replacement = root.open_file(&paths.sandbox_marker_name(), false).unwrap();
                assert_eq!(parked.identity(), original_identity);
                assert_eq!(replacement.identity(), replacement_identity.get().unwrap());
                assert_ne!(parked.identity(), replacement.identity());
                assert_eq!(
                    fs::read(parked.path()).unwrap(),
                    paths.sandbox_marker_bytes()
                );
                assert_eq!(
                    fs::read(replacement.path()).unwrap(),
                    paths.sandbox_marker_bytes()
                );
            }
        }
        assert!(paths.cleanup_root.is_dir());
        drop(namespace_marker);
        drop(parent);
    }
}

#[cfg(target_os = "linux")]
#[test]
fn cleanup_root_recheck_preserves_unmarked_original_and_replacement() {
    let (parent, paths, handles, namespace_marker, sandbox) =
        retained_cleanup_fixture("cleanup-root-replacement");
    let original_identity = sandbox.root().identity();
    let parked_name = crate::cli::tui_real_sandbox_fs::ChildName::literal("parked-cleanup-root");
    let parked_root = paths.sandboxes_root.join(parked_name.as_os_str());
    let replacement_identity = std::cell::Cell::new(None);

    let result = remove_cleanup_sandbox_with_hooks(
        &paths,
        &handles,
        sandbox,
        &SandboxFaults::default(),
        CleanupHooks {
            after_plan: no_cleanup_plan_hook,
            after_children: no_cleanup_marker_hook,
            after_marker_read: no_cleanup_marker_hook,
            before_root_ticket:
                |parent: &crate::cli::tui_real_sandbox_fs::PinnedDirectory,
                 root: &crate::cli::tui_real_sandbox_fs::PinnedDirectory,
                 name: &crate::cli::tui_real_sandbox_fs::ChildName| {
                    fs::rename(root.path(), &parked_root).unwrap();
                    let replacement = parent.create_directory(name).unwrap();
                    replacement_identity.set(Some(replacement.identity()));
                },
        },
    );

    assert!(matches!(result, Err(SandboxError::InvalidEvidence { .. })));
    let parked = handles.sandboxes().open_directory(&parked_name).unwrap();
    let replacement = handles
        .sandboxes()
        .open_directory(&paths.cleanup_name())
        .unwrap();
    assert_eq!(parked.identity(), original_identity);
    assert_eq!(replacement.identity(), replacement_identity.get().unwrap());
    assert_ne!(parked.identity(), replacement.identity());
    assert!(parked.tickets().unwrap().is_empty());
    assert!(replacement.tickets().unwrap().is_empty());
    assert!(!parked_root.join(SANDBOX_MARKER_FILE).exists());
    assert!(!paths.cleanup_root.join(SANDBOX_MARKER_FILE).exists());
    drop(namespace_marker);
    drop(parent);
}

#[cfg(target_os = "linux")]
#[test]
fn quarantine_recheck_refuses_a_valid_replacement_with_the_wrong_identity() {
    let parent = tempfile::TempDir::new().unwrap();
    let paths = stable_paths(
        parent.path().join(STABLE_SANDBOX_NAMESPACE),
        safe_profile("quarantine-destination-swap"),
    )
    .unwrap();
    let (handles, namespace_marker) =
        initialize_namespace_retained(&paths, &SandboxFaults::default()).unwrap();
    create_valid_sandbox_evidence(&paths.sandbox_root, &paths);
    let sandbox =
        validate_original_candidate(handles.sandboxes(), &paths.sandbox_name(), &paths).unwrap();
    let original_identity = sandbox.root().identity();
    let parked_name = crate::cli::tui_real_sandbox_fs::ChildName::literal("parked-moved-original");
    let parked_root = paths.sandboxes_root.join(parked_name.as_os_str());
    let replacement_identity = std::cell::Cell::new(None);

    let result = quarantine_original_sandbox_with_hook(
        &paths,
        &handles,
        paths.sandbox_name(),
        sandbox,
        &SandboxFaults::default(),
        |parent, cleanup_name| {
            fs::rename(&paths.cleanup_root, &parked_root).unwrap();
            let replacement = parent.create_directory(cleanup_name).unwrap();
            let marker = replacement
                .create_file(&paths.sandbox_marker_name())
                .unwrap();
            marker
                .publish_bytes(
                    &replacement,
                    &paths.sandbox_marker_name(),
                    paths.sandbox_marker_bytes(),
                )
                .unwrap();
            replacement_identity.set(Some(replacement.identity()));
        },
    );

    assert!(matches!(result, Err(SandboxError::InvalidEvidence { .. })));
    assert!(!paths.sandbox_root.exists());
    let parked = handles.sandboxes().open_directory(&parked_name).unwrap();
    let replacement = handles
        .sandboxes()
        .open_directory(&paths.cleanup_name())
        .unwrap();
    assert_eq!(parked.identity(), original_identity);
    assert_eq!(replacement.identity(), replacement_identity.get().unwrap());
    assert_ne!(parked.identity(), replacement.identity());
    for name in paths.sandbox_child_names() {
        assert!(parked.open_directory(&name).is_ok());
        assert!(replacement.open_directory(&name).is_err());
    }
    assert_eq!(
        fs::read(parked_root.join(SANDBOX_MARKER_FILE)).unwrap(),
        paths.sandbox_marker_bytes()
    );
    assert_eq!(
        fs::read(paths.cleanup_root.join(SANDBOX_MARKER_FILE)).unwrap(),
        paths.sandbox_marker_bytes()
    );
    drop(namespace_marker);
}

#[cfg(target_os = "linux")]
#[test]
fn recovery_revalidates_retained_cleanup_children_and_marker() {
    #[derive(Clone, Copy, Debug)]
    enum Race {
        Child,
        Marker,
    }

    for race in [Race::Child, Race::Marker] {
        let (parent, paths, handles, namespace_marker, sandbox) =
            retained_cleanup_fixture(match race {
                Race::Child => "recover-child-swap",
                Race::Marker => "recover-marker-corrupt",
            });
        let parked_child = paths.cleanup_root.join("parked-data");
        let original_identity = std::cell::Cell::new(None);
        let replacement_identity = std::cell::Cell::new(None);
        let corrupt_bytes = b"corrupt recovery marker";

        let result = recover_cleanup_sandbox_with_hook(
            &paths,
            &handles,
            sandbox,
            &SandboxFaults::default(),
            |sandbox| match race {
                Race::Child => {
                    let data_name = crate::cli::tui_real_sandbox_fs::ChildName::literal("data");
                    let child = sandbox.children().get(&data_name).unwrap();
                    original_identity.set(Some(child.identity()));
                    fs::rename(child.path(), &parked_child).unwrap();
                    let replacement = sandbox.root().create_directory(&data_name).unwrap();
                    replacement_identity.set(Some(replacement.identity()));
                }
                Race::Marker => {
                    use std::io::{Seek as _, Write as _};

                    original_identity.set(Some(sandbox.marker().identity()));
                    let writable = sandbox
                        .root()
                        .open_file(&paths.sandbox_marker_name(), true)
                        .unwrap();
                    assert_eq!(writable.identity(), sandbox.marker().identity());
                    let mut file = writable.file().try_clone().unwrap();
                    file.set_len(0).unwrap();
                    file.seek(io::SeekFrom::Start(0)).unwrap();
                    file.write_all(corrupt_bytes).unwrap();
                    file.sync_all().unwrap();
                }
            },
        );

        assert!(matches!(result, Err(SandboxError::InvalidEvidence { .. })));
        let root = handles
            .sandboxes()
            .open_directory(&paths.cleanup_name())
            .unwrap();
        match race {
            Race::Child => {
                let parked = root
                    .open_directory(&crate::cli::tui_real_sandbox_fs::ChildName::literal(
                        "parked-data",
                    ))
                    .unwrap();
                let replacement = root
                    .open_directory(&crate::cli::tui_real_sandbox_fs::ChildName::literal("data"))
                    .unwrap();
                assert_eq!(parked.identity(), original_identity.get().unwrap());
                assert_eq!(replacement.identity(), replacement_identity.get().unwrap());
                assert_ne!(parked.identity(), replacement.identity());
                assert_eq!(
                    fs::read(root.path().join(SANDBOX_MARKER_FILE)).unwrap(),
                    paths.sandbox_marker_bytes()
                );
            }
            Race::Marker => {
                let marker = root.open_file(&paths.sandbox_marker_name(), false).unwrap();
                assert_eq!(marker.identity(), original_identity.get().unwrap());
                assert_eq!(fs::read(marker.path()).unwrap(), corrupt_bytes);
                for name in paths.sandbox_child_names() {
                    assert!(root.open_directory(&name).is_ok());
                }
            }
        }
        drop(namespace_marker);
        drop(parent);
    }
}

#[cfg(target_os = "linux")]
#[test]
fn prepare_propagates_original_and_cleanup_classification_io() {
    use std::os::unix::fs::PermissionsExt as _;

    for original in [true, false] {
        let parent = tempfile::TempDir::new().unwrap();
        let paths = stable_paths(
            parent.path().join(STABLE_SANDBOX_NAMESPACE),
            safe_profile(if original {
                "prepare-original-io"
            } else {
                "prepare-cleanup-io"
            }),
        )
        .unwrap();
        let (handles, namespace_marker) =
            initialize_namespace_retained(&paths, &SandboxFaults::default()).unwrap();
        let evidence_root = if original {
            &paths.sandbox_root
        } else {
            &paths.cleanup_root
        };
        create_valid_sandbox_evidence(evidence_root, &paths);
        fs::set_permissions(evidence_root, fs::Permissions::from_mode(0o000)).unwrap();

        let result = prepare_fresh_sandbox_retained(&paths, &handles, &SandboxFaults::default());

        fs::set_permissions(evidence_root, fs::Permissions::from_mode(0o700)).unwrap();
        assert!(matches!(
            result,
            Err(SandboxError::Io { source, .. })
                if source.kind() == io::ErrorKind::PermissionDenied
        ));
        assert_eq!(
            fs::read(evidence_root.join(SANDBOX_MARKER_FILE)).unwrap(),
            paths.sandbox_marker_bytes()
        );
        for name in paths.sandbox_child_names() {
            assert!(evidence_root.join(name.as_os_str()).is_dir());
        }
        drop(namespace_marker);
    }
}

#[cfg(target_os = "linux")]
#[test]
fn prepare_faults_keep_cleanup_original_and_recovery_states_recoverable() {
    for original in [true, false] {
        let parent = tempfile::TempDir::new().unwrap();
        let paths = stable_paths(
            parent.path().join(STABLE_SANDBOX_NAMESPACE),
            safe_profile(if original {
                "cleanup-original-propagation"
            } else {
                "recover-cleanup-propagation"
            }),
        )
        .unwrap();
        let (handles, namespace_marker) =
            initialize_namespace_retained(&paths, &SandboxFaults::default()).unwrap();
        let evidence_root = if original {
            &paths.sandbox_root
        } else {
            &paths.cleanup_root
        };
        create_valid_sandbox_evidence(evidence_root, &paths);

        let result = prepare_fresh_sandbox_retained(
            &paths,
            &handles,
            &SandboxFaults::at(SandboxFaultPoint::RemoveChild),
        );

        assert!(matches!(
            result,
            Err(SandboxError::InjectedFault {
                point: SandboxFaultPoint::RemoveChild,
            })
        ));
        assert!(!paths.sandbox_root.exists());
        assert_eq!(
            fs::read(paths.cleanup_root.join(SANDBOX_MARKER_FILE)).unwrap(),
            paths.sandbox_marker_bytes()
        );
        for name in paths.sandbox_child_names() {
            assert!(paths.cleanup_root.join(name.as_os_str()).is_dir());
        }

        let fresh =
            prepare_fresh_sandbox_retained(&paths, &handles, &SandboxFaults::default()).unwrap();
        assert_eq!(fresh.root().path(), paths.sandbox_root);
        assert!(!paths.cleanup_root.exists());
        assert_eq!(
            fs::read(paths.sandbox_root.join(SANDBOX_MARKER_FILE)).unwrap(),
            paths.sandbox_marker_bytes()
        );
        drop(fresh);
        drop(namespace_marker);
    }
}

#[cfg(target_os = "linux")]
#[test]
fn live_close_propagates_cleanup_io_and_refuses_valid_cleanup() {
    use std::os::unix::fs::PermissionsExt as _;

    for permission_error in [true, false] {
        let parent = tempfile::TempDir::new().unwrap();
        let profile = safe_profile(if permission_error {
            "close-cleanup-io"
        } else {
            "close-valid-cleanup"
        });
        let paths = stable_paths(
            parent.path().join(STABLE_SANDBOX_NAMESPACE),
            profile.clone(),
        )
        .unwrap();
        let sandbox = StableSandbox::acquire(
            StableSandboxNamespace::explicit(paths.namespace_root.clone()).unwrap(),
            profile,
            SandboxFaults::default(),
        )
        .unwrap();
        if permission_error {
            create_valid_sandbox_evidence(&paths.cleanup_root, &paths);
            fs::set_permissions(&paths.cleanup_root, fs::Permissions::from_mode(0o000)).unwrap();
        } else {
            create_private_test_directory(&paths.cleanup_root);
            write_private_test_file(
                &paths.cleanup_root.join(SANDBOX_MARKER_FILE),
                paths.sandbox_marker_bytes(),
            );
        }

        let error = sandbox.close().unwrap_err();

        if permission_error {
            fs::set_permissions(&paths.cleanup_root, fs::Permissions::from_mode(0o700)).unwrap();
            assert!(matches!(
                error,
                SandboxError::Io { source, .. }
                    if source.kind() == io::ErrorKind::PermissionDenied
            ));
            for name in paths.sandbox_child_names() {
                assert!(paths.cleanup_root.join(name.as_os_str()).is_dir());
            }
        } else {
            assert!(matches!(error, SandboxError::InvalidEvidence { .. }));
            assert_eq!(
                fs::read(paths.cleanup_root.join(SANDBOX_MARKER_FILE)).unwrap(),
                paths.sandbox_marker_bytes()
            );
            assert_eq!(fs::read_dir(&paths.cleanup_root).unwrap().count(), 1);
        }
        assert_eq!(
            fs::read(paths.sandbox_root.join(SANDBOX_MARKER_FILE)).unwrap(),
            paths.sandbox_marker_bytes()
        );
        for name in paths.sandbox_child_names() {
            assert!(paths.sandbox_root.join(name.as_os_str()).is_dir());
        }
    }
}

#[cfg(target_os = "linux")]
#[test]
fn live_namespace_validation_refuses_marker_content_and_identity_changes() {
    let replacement_parent = tempfile::TempDir::new().unwrap();
    let replacement_profile = safe_profile("live-namespace-marker-swap");
    let replacement_paths = stable_paths(
        replacement_parent.path().join(STABLE_SANDBOX_NAMESPACE),
        replacement_profile.clone(),
    )
    .unwrap();
    let replacement_sandbox = StableSandbox::acquire(
        StableSandboxNamespace::explicit(replacement_paths.namespace_root.clone()).unwrap(),
        replacement_profile,
        SandboxFaults::default(),
    )
    .unwrap();
    let parked_marker = replacement_paths.namespace_root.join("parked-marker");
    let original_identity = replacement_sandbox
        .namespace_marker
        .as_ref()
        .unwrap()
        .identity();
    let replacement_identity = std::cell::Cell::new(None);

    let replacement_result =
        replacement_sandbox.validate_namespace_with_hook(|namespace, marker, marker_name| {
            fs::rename(marker.path(), &parked_marker).unwrap();
            let replacement = namespace.create_file(marker_name).unwrap();
            replacement
                .publish_bytes(
                    namespace,
                    marker_name,
                    replacement_paths.namespace_marker_bytes(),
                )
                .unwrap();
            replacement_identity.set(Some(replacement.identity()));
        });

    assert!(matches!(
        replacement_result,
        Err(SandboxError::InvalidEvidence { .. })
    ));
    let namespace = replacement_sandbox.handles().namespace();
    let parked = namespace
        .open_file(
            &crate::cli::tui_real_sandbox_fs::ChildName::literal("parked-marker"),
            false,
        )
        .unwrap();
    let replacement = namespace
        .open_file(&replacement_paths.namespace_marker_name(), false)
        .unwrap();
    assert_eq!(parked.identity(), original_identity);
    assert_eq!(replacement.identity(), replacement_identity.get().unwrap());
    assert_ne!(parked.identity(), replacement.identity());
    assert_eq!(
        fs::read(parked.path()).unwrap(),
        replacement_paths.namespace_marker_bytes()
    );
    assert_eq!(
        fs::read(replacement.path()).unwrap(),
        replacement_paths.namespace_marker_bytes()
    );
    drop(replacement_sandbox);

    let corrupt_parent = tempfile::TempDir::new().unwrap();
    let corrupt_profile = safe_profile("live-namespace-marker-corrupt");
    let corrupt_paths = stable_paths(
        corrupt_parent.path().join(STABLE_SANDBOX_NAMESPACE),
        corrupt_profile.clone(),
    )
    .unwrap();
    let corrupt_sandbox = StableSandbox::acquire(
        StableSandboxNamespace::explicit(corrupt_paths.namespace_root.clone()).unwrap(),
        corrupt_profile,
        SandboxFaults::default(),
    )
    .unwrap();
    let marker = corrupt_sandbox.namespace_marker.as_ref().unwrap();
    let original_identity = marker.identity();
    let writable = corrupt_sandbox
        .handles()
        .namespace()
        .open_file(&corrupt_paths.namespace_marker_name(), true)
        .unwrap();
    assert_eq!(writable.identity(), original_identity);
    let corrupt_bytes = b"corrupt live namespace marker";
    {
        use std::io::{Seek as _, Write as _};

        let mut file = writable.file().try_clone().unwrap();
        file.set_len(0).unwrap();
        file.seek(io::SeekFrom::Start(0)).unwrap();
        file.write_all(corrupt_bytes).unwrap();
        file.sync_all().unwrap();
    }

    assert!(matches!(
        corrupt_sandbox.validate_namespace(),
        Err(SandboxError::InvalidEvidence { .. })
    ));
    let retained = corrupt_sandbox
        .handles()
        .namespace()
        .open_file(&corrupt_paths.namespace_marker_name(), false)
        .unwrap();
    assert_eq!(retained.identity(), original_identity);
    assert_eq!(fs::read(retained.path()).unwrap(), corrupt_bytes);
    drop(corrupt_sandbox);
}

#[cfg(target_os = "linux")]
#[test]
fn acquisition_prepare_failure_releases_lease_and_retains_partial_evidence() {
    let parent = tempfile::TempDir::new().unwrap();
    let profile = safe_profile("abort-partial-prepare");
    let namespace_root = parent.path().join(STABLE_SANDBOX_NAMESPACE);
    let paths = stable_paths(namespace_root.clone(), profile.clone()).unwrap();

    let error = StableSandbox::acquire(
        StableSandboxNamespace::explicit(namespace_root).unwrap(),
        profile,
        SandboxFaults::at(SandboxFaultPoint::CreateSandboxChildIo),
    )
    .unwrap_err();

    assert!(matches!(
        error,
        SandboxError::Io {
            operation: "create sandbox child",
            ..
        }
    ));
    assert!(paths.sandbox_root.is_dir());
    assert!(fs::read_dir(&paths.sandbox_root).unwrap().next().is_none());
    assert!(!paths.sandbox_root.join(SANDBOX_MARKER_FILE).exists());
    let (handles, namespace_marker) =
        initialize_namespace_retained(&paths, &SandboxFaults::default()).unwrap();
    let lease =
        acquire_profile_lease_retained(&paths, &handles, &SandboxFaults::default()).unwrap();
    lease.release(handles.leases()).unwrap();
    assert!(paths.sandbox_root.is_dir());
    drop(namespace_marker);
}

#[cfg(target_os = "linux")]
#[test]
fn acquisition_prepare_and_release_failures_keep_both_typed_causes() {
    let parent = tempfile::TempDir::new().unwrap();
    let profile = safe_profile("abort-prepare-release");
    let namespace_root = parent.path().join(STABLE_SANDBOX_NAMESPACE);
    let release_calls = std::cell::Cell::new(0);

    let error = StableSandbox::acquire_with_hooks(
        StableSandboxNamespace::explicit(namespace_root.clone()).unwrap(),
        profile.clone(),
        SandboxFaults::at(SandboxFaultPoint::CreateSandboxRootIo),
        no_acquisition_after_prepare_hook,
        |lease, leases| {
            release_calls.set(release_calls.get() + 1);
            lease.release_with_ops(leases, |_| {
                Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "injected acquisition unlock failure",
                ))
            })
        },
    )
    .unwrap_err();

    assert_eq!(release_calls.get(), 1);
    assert!(matches!(
        error,
        SandboxError::Dual { primary, release }
            if matches!(*primary, SandboxError::Io {
                operation: "create sandbox",
                ..
            }) && matches!(*release, SandboxError::Io {
                operation: "unlock profile lease",
                ..
            })
    ));
    StableSandbox::acquire(
        StableSandboxNamespace::explicit(namespace_root).unwrap(),
        profile,
        SandboxFaults::default(),
    )
    .unwrap()
    .close()
    .unwrap();
}

#[cfg(target_os = "linux")]
#[test]
fn acquisition_final_namespace_failure_contains_sandbox_and_releases_once() {
    let parent = tempfile::TempDir::new().unwrap();
    let profile = safe_profile("abort-final-namespace");
    let namespace_root = parent.path().join(STABLE_SANDBOX_NAMESPACE);
    let paths = stable_paths(namespace_root.clone(), profile.clone()).unwrap();
    let parked_marker = namespace_root.join("parked-namespace-marker");
    let original_identity = std::cell::Cell::new(None);
    let replacement_identity = std::cell::Cell::new(None);
    let release_calls = std::cell::Cell::new(0);

    let error = StableSandbox::acquire_with_hooks(
        StableSandboxNamespace::explicit(namespace_root).unwrap(),
        profile,
        SandboxFaults::default(),
        |sandbox| {
            let namespace = sandbox.handles().namespace();
            let marker = sandbox.namespace_marker.as_ref().unwrap();
            original_identity.set(Some(marker.identity()));
            fs::rename(marker.path(), &parked_marker).unwrap();
            let replacement = namespace
                .create_file(&sandbox.paths.namespace_marker_name())
                .unwrap();
            replacement
                .publish_bytes(
                    namespace,
                    &sandbox.paths.namespace_marker_name(),
                    sandbox.paths.namespace_marker_bytes(),
                )
                .unwrap();
            replacement_identity.set(Some(replacement.identity()));
        },
        |lease, leases| {
            release_calls.set(release_calls.get() + 1);
            lease.release(leases)
        },
    )
    .unwrap_err();

    assert_eq!(release_calls.get(), 1);
    assert!(matches!(error, SandboxError::InvalidEvidence { .. }));
    assert!(!paths.sandbox_root.exists());
    assert!(!paths.cleanup_root.exists());
    let parent_handle =
        crate::cli::tui_real_sandbox_fs::PinnedDirectory::open_ambient_parent(parent.path())
            .unwrap();
    let namespace = parent_handle
        .open_directory(&paths.namespace_name())
        .unwrap();
    let parked = namespace
        .open_file(
            &crate::cli::tui_real_sandbox_fs::ChildName::literal("parked-namespace-marker"),
            false,
        )
        .unwrap();
    let replacement = namespace
        .open_file(&paths.namespace_marker_name(), false)
        .unwrap();
    assert_eq!(parked.identity(), original_identity.get().unwrap());
    assert_eq!(replacement.identity(), replacement_identity.get().unwrap());
    assert_ne!(parked.identity(), replacement.identity());
    assert_eq!(
        fs::read(parked.path()).unwrap(),
        paths.namespace_marker_bytes()
    );
    assert_eq!(
        fs::read(replacement.path()).unwrap(),
        paths.namespace_marker_bytes()
    );
}

#[cfg(target_os = "linux")]
#[test]
fn acquisition_final_lease_and_live_failures_retain_every_replacement() {
    #[derive(Clone, Copy, Debug)]
    enum Race {
        Lease,
        LiveChild,
    }

    for race in [Race::Lease, Race::LiveChild] {
        let parent = tempfile::TempDir::new().unwrap();
        let profile = safe_profile(match race {
            Race::Lease => "abort-final-lease",
            Race::LiveChild => "abort-final-live",
        });
        let namespace_root = parent.path().join(STABLE_SANDBOX_NAMESPACE);
        let paths = stable_paths(namespace_root.clone(), profile.clone()).unwrap();
        let parked = match race {
            Race::Lease => paths.leases_root.join("parked-lease"),
            Race::LiveChild => paths.sandbox_root.join("parked-data"),
        };
        let original_identity = std::cell::Cell::new(None);
        let replacement_identity = std::cell::Cell::new(None);
        let release_calls = std::cell::Cell::new(0);

        let error = StableSandbox::acquire_with_hooks(
            StableSandboxNamespace::explicit(namespace_root).unwrap(),
            profile,
            SandboxFaults::default(),
            |sandbox| match race {
                Race::Lease => {
                    let leases = sandbox.handles().leases();
                    let lease = sandbox.lease.as_ref().unwrap();
                    original_identity.set(Some(lease.file().identity()));
                    fs::rename(lease.file().path(), &parked).unwrap();
                    let replacement = leases.create_file(&sandbox.paths.lease_name()).unwrap();
                    replacement
                        .publish_bytes(
                            leases,
                            &sandbox.paths.lease_name(),
                            sandbox.paths.lease_marker_bytes(),
                        )
                        .unwrap();
                    replacement_identity.set(Some(replacement.identity()));
                }
                Race::LiveChild => {
                    let root = sandbox.sandbox.as_ref().unwrap().root();
                    let data_name = crate::cli::tui_real_sandbox_fs::ChildName::literal("data");
                    let child = root.open_directory(&data_name).unwrap();
                    original_identity.set(Some(child.identity()));
                    drop(child);
                    fs::rename(root.path().join("data"), &parked).unwrap();
                    let replacement = root.create_directory(&data_name).unwrap();
                    replacement_identity.set(Some(replacement.identity()));
                }
            },
            |lease, leases| {
                release_calls.set(release_calls.get() + 1);
                match race {
                    Race::Lease => lease.release(leases),
                    Race::LiveChild => lease.release_with_ops(leases, |_| {
                        Err(io::Error::new(
                            io::ErrorKind::PermissionDenied,
                            "injected final-validation unlock failure",
                        ))
                    }),
                }
            },
        )
        .unwrap_err();

        assert_eq!(release_calls.get(), 1);
        assert_ne!(
            original_identity.get().unwrap(),
            replacement_identity.get().unwrap()
        );
        match race {
            Race::Lease => {
                assert!(matches!(
                    error,
                    SandboxError::Dual { primary, release }
                        if matches!(*primary, SandboxError::InvalidEvidence { .. })
                            && matches!(*release, SandboxError::InvalidEvidence { .. })
                ));
                assert!(!paths.sandbox_root.exists());
                assert!(!paths.cleanup_root.exists());
                assert_eq!(fs::read(&parked).unwrap(), paths.lease_marker_bytes());
                assert_eq!(
                    fs::read(&paths.lease_path).unwrap(),
                    paths.lease_marker_bytes()
                );
            }
            Race::LiveChild => {
                assert!(matches!(
                    error,
                    SandboxError::Dual { primary, release }
                        if matches!(primary.as_ref(), SandboxError::Dual {
                            primary: nested_primary,
                            release: nested_cleanup,
                        } if matches!(nested_primary.as_ref(), SandboxError::InvalidEvidence { .. })
                            && matches!(nested_cleanup.as_ref(), SandboxError::InvalidEvidence { .. }))
                            && matches!(*release, SandboxError::Io {
                                operation: "unlock profile lease",
                                ..
                            })
                ));
                assert!(paths.sandbox_root.is_dir());
                assert!(parked.is_dir());
                assert!(paths.sandbox_root.join("data").is_dir());
                assert_eq!(
                    fs::read(paths.sandbox_root.join(SANDBOX_MARKER_FILE)).unwrap(),
                    paths.sandbox_marker_bytes()
                );
            }
        }
    }
}

#[cfg(target_os = "linux")]
#[test]
fn namespace_validation_refuses_replaced_children_and_unexpected_names() {
    let lease_parent = tempfile::TempDir::new().unwrap();
    let lease_paths = initialized_namespace_paths(&lease_parent, &safe_profile("lease-swap"));
    let (parent, namespace, leases, sandboxes) = opened_namespace_parts(&lease_paths);
    let parked_lease = namespace.path().join("parked-leases");
    let replacement_identity = std::cell::Cell::new(None);
    let lease_result = validate_namespace_opened_with_hook(
        &lease_paths,
        &parent,
        &namespace,
        &leases,
        &sandboxes,
        |_, namespace, leases, _| {
            fs::rename(leases.path(), &parked_lease).unwrap();
            let replacement = namespace
                .create_directory(&lease_paths.leases_name())
                .unwrap();
            replacement_identity.set(Some(replacement.identity()));
        },
    );
    assert!(matches!(
        lease_result,
        Err(SandboxError::InvalidEvidence { .. })
    ));
    assert!(parked_lease.is_dir());
    assert_ne!(replacement_identity.get().unwrap(), leases.identity());

    let sandbox_parent = tempfile::TempDir::new().unwrap();
    let sandbox_paths = initialized_namespace_paths(&sandbox_parent, &safe_profile("sandbox-swap"));
    let (parent, namespace, leases, sandboxes) = opened_namespace_parts(&sandbox_paths);
    let parked_sandboxes = namespace.path().join("parked-sandboxes");
    let replacement_identity = std::cell::Cell::new(None);
    let sandbox_result = validate_namespace_opened_with_hook(
        &sandbox_paths,
        &parent,
        &namespace,
        &leases,
        &sandboxes,
        |_, namespace, _, sandboxes| {
            fs::rename(sandboxes.path(), &parked_sandboxes).unwrap();
            let replacement = namespace
                .create_directory(&sandbox_paths.sandboxes_name())
                .unwrap();
            replacement_identity.set(Some(replacement.identity()));
        },
    );
    assert!(matches!(
        sandbox_result,
        Err(SandboxError::InvalidEvidence { .. })
    ));
    assert!(parked_sandboxes.is_dir());
    assert_ne!(replacement_identity.get().unwrap(), sandboxes.identity());

    let extra_parent = tempfile::TempDir::new().unwrap();
    let extra_paths = initialized_namespace_paths(&extra_parent, &safe_profile("unexpected-child"));
    let (parent, namespace, leases, sandboxes) = opened_namespace_parts(&extra_paths);
    let extra_result = validate_namespace_opened_with_hook(
        &extra_paths,
        &parent,
        &namespace,
        &leases,
        &sandboxes,
        |_, namespace, _, _| {
            drop(
                namespace
                    .create_directory(&crate::cli::tui_real_sandbox_fs::ChildName::literal(
                        "unexpected",
                    ))
                    .unwrap(),
            );
        },
    );
    assert!(matches!(
        extra_result,
        Err(SandboxError::InvalidEvidence { .. })
    ));
    assert!(extra_paths.namespace_root.join("unexpected").is_dir());
}

#[cfg(target_os = "linux")]
#[test]
fn initialization_release_refuses_each_replaced_namespace_identity() {
    #[derive(Clone, Copy, Debug)]
    enum Target {
        Namespace,
        Leases,
        Sandboxes,
        Marker,
    }

    for target in [
        Target::Namespace,
        Target::Leases,
        Target::Sandboxes,
        Target::Marker,
    ] {
        let parent = tempfile::TempDir::new().unwrap();
        let paths = stable_paths(
            parent.path().join(STABLE_SANDBOX_NAMESPACE),
            safe_profile("release-swap"),
        )
        .unwrap();
        let parked = match target {
            Target::Namespace => parent.path().join("parked-namespace"),
            Target::Leases => paths.namespace_root.join("parked-leases"),
            Target::Sandboxes => paths.namespace_root.join("parked-sandboxes"),
            Target::Marker => paths.namespace_root.join("parked-marker"),
        };
        let original_identity = std::cell::Cell::new(None);
        let replacement_identity = std::cell::Cell::new(None);

        let result = initialize_namespace_retained_with_hook(
            &paths,
            &SandboxFaults::default(),
            |guard, namespace, leases, sandboxes, marker| match target {
                Target::Namespace => {
                    original_identity.set(Some(namespace.identity()));
                    fs::rename(namespace.path(), &parked).unwrap();
                    let replacement = guard
                        .parent()
                        .create_directory(&paths.namespace_name())
                        .unwrap();
                    replacement_identity.set(Some(replacement.identity()));
                }
                Target::Leases => {
                    original_identity.set(Some(leases.identity()));
                    fs::rename(leases.path(), &parked).unwrap();
                    let replacement = namespace.create_directory(&paths.leases_name()).unwrap();
                    replacement_identity.set(Some(replacement.identity()));
                }
                Target::Sandboxes => {
                    original_identity.set(Some(sandboxes.identity()));
                    fs::rename(sandboxes.path(), &parked).unwrap();
                    let replacement = namespace.create_directory(&paths.sandboxes_name()).unwrap();
                    replacement_identity.set(Some(replacement.identity()));
                }
                Target::Marker => {
                    original_identity.set(Some(marker.identity()));
                    fs::rename(marker.path(), &parked).unwrap();
                    let replacement = namespace
                        .create_file(&paths.namespace_marker_name())
                        .unwrap();
                    replacement
                        .publish_bytes(
                            namespace,
                            &paths.namespace_marker_name(),
                            paths.namespace_marker_bytes(),
                        )
                        .unwrap();
                    replacement_identity.set(Some(replacement.identity()));
                }
            },
        );

        assert!(result.is_err(), "replacement {target:?} was accepted");
        assert!(parked.exists(), "parked {target:?} was removed");
        assert_ne!(
            original_identity.get().unwrap(),
            replacement_identity.get().unwrap()
        );
    }
}

#[cfg(target_os = "linux")]
#[test]
fn namespace_parent_lock_blocks_a_second_initializer_until_release() {
    use std::sync::mpsc;
    use std::time::Duration;

    let parent = tempfile::TempDir::new().unwrap();
    let paths = stable_paths(
        parent.path().join(STABLE_SANDBOX_NAMESPACE),
        safe_profile("init-barrier"),
    )
    .unwrap();
    let parent_path = paths.namespace_root.parent().unwrap().to_path_buf();
    let init_name = paths.init_lock_name();
    let (started_tx, started_rx) = mpsc::channel();
    let (acquired_tx, acquired_rx) = mpsc::channel();
    let mut worker = None;

    let initialized = initialize_namespace_retained_with_hook(
        &paths,
        &SandboxFaults::default(),
        |_, _, _, _, _| {
            worker = Some(std::thread::spawn(move || {
                started_tx.send(()).unwrap();
                let guard = crate::cli::tui_real_sandbox_fs::InitializationGuard::acquire(
                    &parent_path,
                    init_name,
                )
                .unwrap();
                acquired_tx.send(()).unwrap();
                guard.release(|_| Ok(())).unwrap();
            }));
            started_rx.recv_timeout(Duration::from_secs(1)).unwrap();
            assert!(matches!(
                acquired_rx.recv_timeout(Duration::from_millis(100)),
                Err(mpsc::RecvTimeoutError::Timeout)
            ));
        },
    )
    .unwrap();

    acquired_rx.recv_timeout(Duration::from_secs(1)).unwrap();
    worker.take().unwrap().join().unwrap();
    drop(initialized);
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
fn opened_namespace_parts(
    paths: &StableSandboxPaths,
) -> (
    crate::cli::tui_real_sandbox_fs::PinnedDirectory,
    crate::cli::tui_real_sandbox_fs::PinnedDirectory,
    crate::cli::tui_real_sandbox_fs::PinnedDirectory,
    crate::cli::tui_real_sandbox_fs::PinnedDirectory,
) {
    let parent = crate::cli::tui_real_sandbox_fs::PinnedDirectory::open_ambient_parent(
        paths
            .namespace_root
            .parent()
            .expect("the namespace has a parent"),
    )
    .unwrap();
    let namespace = parent.open_directory(&paths.namespace_name()).unwrap();
    let leases = namespace.open_directory(&paths.leases_name()).unwrap();
    let sandboxes = namespace.open_directory(&paths.sandboxes_name()).unwrap();
    (parent, namespace, leases, sandboxes)
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
fn initialized_namespace_paths(
    parent: &tempfile::TempDir,
    profile: &SafeProfileId,
) -> StableSandboxPaths {
    let paths = stable_paths(
        parent.path().join(STABLE_SANDBOX_NAMESPACE),
        profile.clone(),
    )
    .unwrap();
    drop(initialize_namespace_retained(&paths, &SandboxFaults::default()).unwrap());
    paths
}
