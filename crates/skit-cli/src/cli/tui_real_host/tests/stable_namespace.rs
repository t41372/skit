//! Stable namespace, marker, initialization, and lease contracts.

use super::*;

#[test]
fn sandbox_errors_keep_typed_causes_and_stable_diagnostics() {
    let path = PathBuf::from("/tmp/skit-ui-walker-v1/leases/error.lock");
    let unsupported = SandboxError::UnsupportedPlatform { platform: "macos" };
    assert_eq!(
        unsupported.to_string(),
        "stable walker sandboxes are not supported on macos"
    );
    assert!(std::error::Error::source(&unsupported).is_none());
    let unsupported = SandboxError::from(
        crate::cli::tui_real_sandbox_fs::SandboxFsError::Unsupported {
            operation: "rename",
            path: path.clone(),
            source: Box::new(io::Error::new(io::ErrorKind::Unsupported, "not supported")),
        },
    );
    assert!(unsupported.to_string().contains("not supported"));
    assert!(std::error::Error::source(&unsupported).is_some());
    let native = SandboxError::from(
        crate::cli::tui_real_sandbox_fs::SandboxFsError::NativeStatus {
            operation: "rename",
            path: path.clone(),
            status: -1,
        },
    );
    assert!(native.to_string().contains("native status 0xffffffff"));
    assert!(std::error::Error::source(&native).is_none());
    let platform =
        SandboxError::from(crate::cli::tui_real_sandbox_fs::SandboxFsError::UnsupportedPlatform);
    assert!(platform.to_string().contains(std::env::consts::OS));
    let dual = SandboxError::from(crate::cli::tui_real_sandbox_fs::SandboxFsError::Dual {
        primary: Box::new(crate::cli::tui_real_sandbox_fs::SandboxFsError::Invalid {
            path: path.clone(),
            reason: "primary evidence".to_owned(),
        }),
        release: Box::new(crate::cli::tui_real_sandbox_fs::SandboxFsError::Io {
            operation: "unlock",
            path: path.clone(),
            source: io::Error::new(io::ErrorKind::PermissionDenied, "release denied"),
        }),
    });
    assert!(dual.to_string().contains("sandbox release also failed"));
    assert!(std::error::Error::source(&dual).is_some());

    let busy = SandboxError::Busy {
        profile: safe_profile("error"),
        path: path.clone(),
    };
    assert!(busy.to_string().contains("profile error is busy"));
    let invalid = SandboxError::invalid(&path, "invalid evidence");
    assert!(invalid.to_string().contains("invalid evidence"));
    let contract_error = map_contract_result::<(), _>(&path, Err("contract error")).unwrap_err();
    assert!(matches!(
        contract_error,
        SandboxError::InvalidEvidence { reason, .. }
            if reason == "contract error"
    ));
    let io = SandboxError::io(
        "lock",
        &path,
        io::Error::new(io::ErrorKind::PermissionDenied, "lock denied"),
    );
    assert!(
        io.to_string()
            .contains("could not lock walker sandbox path")
    );
    assert!(std::error::Error::source(&io).is_some());
    let injected = SandboxError::InjectedFault {
        point: SandboxFaultPoint::RemoveMarker,
    };
    assert!(injected.to_string().contains("RemoveMarker"));

    let seed = StableHostPrimaryError::Seed("seed denied".to_owned());
    assert!(seed.to_string().contains("seed denied"));
    let sandbox = StableHostError::Sandbox(busy);
    assert!(sandbox.to_string().contains("is busy"));
    let primary = StableHostError::Primary {
        primary: StableHostPrimaryError::Seed("seed only".to_owned()),
        cleanup: None,
    };
    assert_eq!(
        primary.to_string(),
        "could not seed the stable walker host: seed only"
    );
    let combined = StableHostError::Primary {
        primary: StableHostPrimaryError::Seed("primary".to_owned()),
        cleanup: Some(injected),
    };
    assert!(combined.to_string().contains("sandbox cleanup also failed"));
}

#[test]
fn retained_evidence_and_release_results_keep_each_typed_state() {
    let absent = Evidence::<u8>::Absent;
    assert_eq!(absent.state(), SandboxEvidenceState::Absent);
    assert_eq!(absent.reason(), None);
    assert_eq!(absent.into_valid(), None);

    let valid = Evidence::Valid(7_u8);
    assert_eq!(valid.state(), SandboxEvidenceState::Valid);
    assert_eq!(valid.reason(), None);
    assert_eq!(valid.into_valid(), Some(7));

    let invalid = Evidence::<u8>::Invalid {
        reason: "invalid evidence".to_owned(),
    };
    assert_eq!(invalid.state(), SandboxEvidenceState::Invalid);
    assert_eq!(invalid.reason(), Some("invalid evidence"));
    assert_eq!(invalid.into_valid(), None);

    let primary = || SandboxError::invalid(Path::new("primary"), "primary");
    let release = || SandboxError::invalid(Path::new("release"), "release");
    assert!(combine_sandbox_release(Ok(()), Ok(())).is_ok());
    assert!(matches!(
        combine_sandbox_release(Err(primary()), Ok(())),
        Err(SandboxError::InvalidEvidence { path, .. })
            if path == Path::new("primary")
    ));
    assert!(matches!(
        combine_sandbox_release(Ok(()), Err(release())),
        Err(SandboxError::InvalidEvidence { path, .. })
            if path == Path::new("release")
    ));
    assert!(matches!(
        combine_sandbox_release(Err(primary()), Err(release())),
        Err(SandboxError::Dual { .. })
    ));
}

#[test]
fn namespace_shape_validation_fails_closed() {
    assert!(StableSandboxNamespace::explicit(PathBuf::from(STABLE_SANDBOX_NAMESPACE)).is_err());
    assert!(StableSandboxNamespace::explicit(PathBuf::from("/tmp/wrong-namespace")).is_err());
    assert!(StableSandboxNamespace::explicit(PathBuf::from("/tmp/../skit-ui-walker-v1",)).is_err());
}

#[cfg(target_os = "linux")]
#[test]
fn invalid_namespace_literals_return_typed_errors_without_panics() {
    use std::os::unix::ffi::OsStringExt as _;

    let parent = tempfile::TempDir::new().unwrap();
    let parent_literal = parent.path().to_str().unwrap();
    let mut paths = vec![
        PathBuf::from(format!(
            "{parent_literal}/newline\n/{STABLE_SANDBOX_NAMESPACE}"
        )),
        PathBuf::from(format!(
            "{parent_literal}/carriage\r/{STABLE_SANDBOX_NAMESPACE}"
        )),
        PathBuf::from(format!("{parent_literal}/nul\0/{STABLE_SANDBOX_NAMESPACE}")),
        PathBuf::from(format!("{parent_literal}//{STABLE_SANDBOX_NAMESPACE}")),
        PathBuf::from(format!("{parent_literal}/{STABLE_SANDBOX_NAMESPACE}/")),
        PathBuf::from(format!("{parent_literal}/./{STABLE_SANDBOX_NAMESPACE}")),
        PathBuf::from(format!(
            "{parent_literal}/parent/../{STABLE_SANDBOX_NAMESPACE}"
        )),
    ];
    paths.push(
        parent
            .path()
            .join(std::ffi::OsString::from_vec(b"non-utf8-\xff".to_vec()))
            .join(STABLE_SANDBOX_NAMESPACE),
    );

    for path in paths {
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            StableSandboxNamespace::explicit(path.clone())
        }));
        assert!(outcome.is_ok(), "panicked for namespace path {path:?}");
        assert!(
            outcome.unwrap().is_err(),
            "accepted namespace path {path:?}"
        );
    }
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
#[test]
fn stable_paths_cache_exact_typed_marker_bytes() {
    let parent = tempfile::TempDir::new().unwrap();
    let profile = safe_profile("marker-derivation");
    let paths = stable_paths(
        parent.path().join(STABLE_SANDBOX_NAMESPACE),
        profile.clone(),
    )
    .unwrap();
    let namespace = paths.namespace_root.to_str().unwrap();
    assert_eq!(
        paths.parent_lock_bytes(),
        ParentInitLockMarker::new(paths.platform, namespace)
            .unwrap()
            .canonical_bytes()
            .unwrap()
    );
    assert_eq!(
        paths.namespace_marker_bytes(),
        NamespaceMarker::new(paths.platform, namespace)
            .unwrap()
            .canonical_bytes()
            .unwrap()
    );
    assert_eq!(
        paths.lease_marker_bytes(),
        ProfileLeaseMarker::new(paths.platform, profile.clone(), namespace)
            .unwrap()
            .canonical_bytes()
            .unwrap()
    );
    let sandbox_marker = SandboxMarker::new(paths.platform, profile, namespace).unwrap();
    assert_eq!(&paths.roots, sandbox_marker.roots());
    assert_eq!(
        paths.sandbox_marker_bytes(),
        sandbox_marker.canonical_bytes().unwrap()
    );

    let rejected_root = parent.path().join(STABLE_SANDBOX_NAMESPACE);
    let rejected = StableSandboxPaths::new_with_contract_builder(
        StableSandboxNamespace::explicit(rejected_root.clone()).unwrap(),
        safe_profile("marker-builder-error"),
        |_, _, _| Err("injected marker contract failure"),
    )
    .unwrap_err();
    assert!(matches!(
        rejected,
        SandboxError::InvalidEvidence { path, reason }
            if path == rejected_root && reason == "injected marker contract failure"
    ));
}

#[cfg(target_os = "linux")]
#[test]
fn marker_and_ticket_reverse_checks_refuse_real_namespace_replacements() {
    let temporary = tempfile::TempDir::new().unwrap();
    let directory =
        crate::cli::tui_real_sandbox_fs::PinnedDirectory::open_ambient_parent(temporary.path())
            .unwrap();
    let marker_name = crate::cli::tui_real_sandbox_fs::ChildName::literal("marker");
    let marker = publish_relative_marker(&directory, &marker_name, b"marker bytes").unwrap();
    let original_identity = marker.identity();
    drop(marker);
    let parked = temporary.path().join("parked-marker");
    let replacement_identity = std::cell::Cell::new(None);

    let replaced = validate_relative_marker_with_hook(
        &directory,
        &marker_name,
        b"marker bytes",
        |directory, marker, name| {
            fs::rename(marker.path(), &parked).unwrap();
            let replacement = directory.create_file(name).unwrap();
            replacement
                .publish_bytes(directory, name, b"marker bytes")
                .unwrap();
            replacement_identity.set(Some(replacement.identity()));
        },
    );

    assert!(matches!(
        replaced,
        Err(SandboxError::InvalidEvidence { .. })
    ));
    assert_eq!(
        directory
            .open_file(
                &crate::cli::tui_real_sandbox_fs::ChildName::literal("parked-marker"),
                false
            )
            .unwrap()
            .identity(),
        original_identity
    );
    assert_ne!(replacement_identity.get().unwrap(), original_identity);
    assert_eq!(fs::read(&parked).unwrap(), b"marker bytes");
    assert_eq!(
        fs::read(temporary.path().join("marker")).unwrap(),
        b"marker bytes"
    );

    let removed_name = crate::cli::tui_real_sandbox_fs::ChildName::literal("removed-marker");
    drop(publish_relative_marker(&directory, &removed_name, b"removed bytes").unwrap());
    let removed = validate_relative_marker_with_hook(
        &directory,
        &removed_name,
        b"removed bytes",
        |_, marker, _| {
            fs::remove_file(marker.path()).unwrap();
        },
    );
    assert!(matches!(removed, Err(SandboxError::InvalidEvidence { .. })));
    assert!(!temporary.path().join("removed-marker").exists());

    let directory_marker = crate::cli::tui_real_sandbox_fs::ChildName::literal("directory-marker");
    drop(directory.create_directory(&directory_marker).unwrap());
    assert!(matches!(
        validate_relative_marker(&directory, &directory_marker, b"marker bytes"),
        Err(SandboxError::InvalidEvidence { .. })
    ));
    let outside = temporary.path().join("outside-marker");
    fs::write(&outside, b"outside bytes").unwrap();
    let linked_marker = crate::cli::tui_real_sandbox_fs::ChildName::literal("linked-marker");
    std::os::unix::fs::symlink(&outside, temporary.path().join("linked-marker")).unwrap();
    assert!(matches!(
        validate_relative_marker(&directory, &linked_marker, b"marker bytes"),
        Err(SandboxError::InvalidEvidence { .. })
    ));
    assert_eq!(fs::read(outside).unwrap(), b"outside bytes");

    let first_name = crate::cli::tui_real_sandbox_fs::ChildName::literal("first-directory");
    let first = directory.create_directory(&first_name).unwrap();
    let second_name = crate::cli::tui_real_sandbox_fs::ChildName::literal("second-directory");
    let second = directory.create_directory(&second_name).unwrap();
    assert!(
        require_ticket(
            &directory,
            &crate::cli::tui_real_sandbox_fs::ChildName::literal("missing-directory"),
            crate::cli::tui_real_sandbox_fs::NodeKind::Directory,
            first.identity(),
        )
        .is_err()
    );
    assert!(
        require_ticket(
            &directory,
            &first_name,
            crate::cli::tui_real_sandbox_fs::NodeKind::RegularFile,
            first.identity(),
        )
        .is_err()
    );
    assert!(
        require_ticket(
            &directory,
            &first_name,
            crate::cli::tui_real_sandbox_fs::NodeKind::Directory,
            second.identity(),
        )
        .is_err()
    );
}

#[cfg(target_os = "linux")]
#[test]
fn initialization_reverse_read_and_empty_release_fail_closed() {
    let parent = tempfile::TempDir::new().unwrap();
    let paths = stable_paths(
        parent.path().join(STABLE_SANDBOX_NAMESPACE),
        safe_profile("init-reverse"),
    )
    .unwrap();
    let guard = acquire_initialization_guard(&paths, &SandboxFaults::default()).unwrap();
    let original_identity = guard.file().identity();
    guard.release(|_| Ok(())).unwrap();
    let parked = parent.path().join("parked-init.lock");
    let replacement_identity = std::cell::Cell::new(None);

    let replaced = acquire_initialization_guard_with_hooks(
        &paths,
        &SandboxFaults::default(),
        |guard| {
            fs::rename(&paths.init_lock, &parked).unwrap();
            let replacement = guard.parent().create_file(guard.name()).unwrap();
            replacement
                .publish_bytes(guard.parent(), guard.name(), paths.parent_lock_bytes())
                .unwrap();
            replacement_identity.set(Some(replacement.identity()));
        },
        release_initialization_guard,
    );

    assert!(replaced.is_err());
    assert_eq!(fs::read(&parked).unwrap(), paths.parent_lock_bytes());
    assert_eq!(
        fs::read(&paths.init_lock).unwrap(),
        paths.parent_lock_bytes()
    );
    assert_ne!(replacement_identity.get().unwrap(), original_identity);

    let empty_parent = tempfile::TempDir::new().unwrap();
    let empty_paths = stable_paths(
        empty_parent.path().join(STABLE_SANDBOX_NAMESPACE),
        safe_profile("empty-release"),
    )
    .unwrap();
    let directory =
        crate::cli::tui_real_sandbox_fs::PinnedDirectory::open_ambient_parent(empty_parent.path())
            .unwrap();
    drop(
        directory
            .create_file(&empty_paths.init_lock_name())
            .unwrap(),
    );

    let release = acquire_initialization_guard_with_hooks(
        &empty_paths,
        &SandboxFaults::default(),
        |_| {},
        |guard| {
            guard.release_with_ops(
                |_| Ok(()),
                |_| {
                    Err(io::Error::new(
                        io::ErrorKind::PermissionDenied,
                        "injected init unlock failure",
                    ))
                },
                |_| Ok(()),
            )
        },
    );

    assert!(matches!(
        release,
        Err(SandboxError::Io {
            operation: "unlock initialization file",
            ..
        })
    ));
    assert_eq!(fs::read(&empty_paths.init_lock).unwrap(), b"");
}

#[cfg(target_os = "linux")]
#[test]
fn namespace_marker_publish_collision_retains_the_unmarked_skeleton() {
    let parent = tempfile::TempDir::new().unwrap();
    let paths = stable_paths(
        parent.path().join(STABLE_SANDBOX_NAMESPACE),
        safe_profile("marker-publish-collision"),
    )
    .unwrap();
    let inserted_identity = std::cell::Cell::new(None);

    let result = initialize_namespace_retained_with_hooks(
        &paths,
        &SandboxFaults::default(),
        |namespace, marker_name| {
            let inserted = namespace.create_directory(marker_name).unwrap();
            inserted_identity.set(Some(inserted.identity()));
        },
        no_before_namespace_release_hook,
        no_after_namespace_release_hook,
    );

    assert!(matches!(
        result,
        Err(SandboxError::Io { source, .. })
            if source.kind() == io::ErrorKind::AlreadyExists
    ));
    assert!(paths.leases_root.is_dir());
    assert!(paths.sandboxes_root.is_dir());
    let parent_handle =
        crate::cli::tui_real_sandbox_fs::PinnedDirectory::open_ambient_parent(parent.path())
            .unwrap();
    let namespace = parent_handle
        .open_directory(&paths.namespace_name())
        .unwrap();
    let inserted = namespace
        .open_directory(&paths.namespace_marker_name())
        .unwrap();
    assert_eq!(inserted.identity(), inserted_identity.get().unwrap());
    assert!(!paths.namespace_root.join(NAMESPACE_MARKER_FILE).is_file());

    assert!(initialize_namespace_retained(&paths, &SandboxFaults::default()).is_err());
    assert_eq!(
        namespace
            .open_directory(&paths.namespace_marker_name())
            .unwrap()
            .identity(),
        inserted_identity.get().unwrap()
    );
}

#[cfg(target_os = "linux")]
#[test]
fn initialization_final_validation_refuses_namespace_and_marker_replacements() {
    let namespace_parent = tempfile::TempDir::new().unwrap();
    let namespace_paths = stable_paths(
        namespace_parent.path().join(STABLE_SANDBOX_NAMESPACE),
        safe_profile("post-release-namespace"),
    )
    .unwrap();
    let parked_namespace = namespace_parent.path().join("parked-namespace");
    let original_namespace_identity = std::cell::Cell::new(None);
    let replacement_namespace_identity = std::cell::Cell::new(None);

    let namespace_result = initialize_namespace_retained_with_hooks(
        &namespace_paths,
        &SandboxFaults::default(),
        |_, _| {},
        |_, _, _, _, _| {},
        |parent, namespace, _, _, _| {
            original_namespace_identity.set(Some(namespace.identity()));
            fs::rename(namespace.path(), &parked_namespace).unwrap();
            let replacement = parent
                .create_directory(&namespace_paths.namespace_name())
                .unwrap();
            replacement_namespace_identity.set(Some(replacement.identity()));
        },
    );

    assert!(matches!(
        namespace_result,
        Err(SandboxError::InvalidEvidence { .. })
    ));
    let parent_handle = crate::cli::tui_real_sandbox_fs::PinnedDirectory::open_ambient_parent(
        namespace_parent.path(),
    )
    .unwrap();
    let parked = parent_handle
        .open_directory(&crate::cli::tui_real_sandbox_fs::ChildName::literal(
            "parked-namespace",
        ))
        .unwrap();
    let replacement = parent_handle
        .open_directory(&namespace_paths.namespace_name())
        .unwrap();
    assert_eq!(
        parked.identity(),
        original_namespace_identity.get().unwrap()
    );
    assert_eq!(
        replacement.identity(),
        replacement_namespace_identity.get().unwrap()
    );
    assert_ne!(parked.identity(), replacement.identity());
    assert_eq!(
        fs::read(parked_namespace.join(NAMESPACE_MARKER_FILE)).unwrap(),
        namespace_paths.namespace_marker_bytes()
    );
    assert!(replacement.tickets().unwrap().is_empty());

    let marker_parent = tempfile::TempDir::new().unwrap();
    let marker_paths = stable_paths(
        marker_parent.path().join(STABLE_SANDBOX_NAMESPACE),
        safe_profile("post-release-marker"),
    )
    .unwrap();
    let parked_marker = marker_paths.namespace_root.join("parked-marker");
    let original_marker_identity = std::cell::Cell::new(None);
    let replacement_marker_identity = std::cell::Cell::new(None);

    let marker_result = initialize_namespace_retained_with_hooks(
        &marker_paths,
        &SandboxFaults::default(),
        |_, _| {},
        |_, _, _, _, _| {},
        |_, namespace, _, _, marker| {
            original_marker_identity.set(Some(marker.identity()));
            fs::rename(marker.path(), &parked_marker).unwrap();
            let replacement = namespace
                .create_file(&marker_paths.namespace_marker_name())
                .unwrap();
            replacement
                .publish_bytes(
                    namespace,
                    &marker_paths.namespace_marker_name(),
                    marker_paths.namespace_marker_bytes(),
                )
                .unwrap();
            replacement_marker_identity.set(Some(replacement.identity()));
        },
    );

    assert!(matches!(
        marker_result,
        Err(SandboxError::InvalidEvidence { .. })
    ));
    let parent_handle =
        crate::cli::tui_real_sandbox_fs::PinnedDirectory::open_ambient_parent(marker_parent.path())
            .unwrap();
    let namespace = parent_handle
        .open_directory(&marker_paths.namespace_name())
        .unwrap();
    let parked = namespace
        .open_file(
            &crate::cli::tui_real_sandbox_fs::ChildName::literal("parked-marker"),
            false,
        )
        .unwrap();
    let replacement = namespace
        .open_file(&marker_paths.namespace_marker_name(), false)
        .unwrap();
    assert_eq!(parked.identity(), original_marker_identity.get().unwrap());
    assert_eq!(
        replacement.identity(),
        replacement_marker_identity.get().unwrap()
    );
    assert_ne!(parked.identity(), replacement.identity());
    assert_eq!(
        fs::read(&parked_marker).unwrap(),
        marker_paths.namespace_marker_bytes()
    );
    assert_eq!(
        fs::read(marker_paths.namespace_root.join(NAMESPACE_MARKER_FILE)).unwrap(),
        marker_paths.namespace_marker_bytes()
    );
    use std::os::unix::fs::PermissionsExt as _;
    assert_eq!(
        fs::metadata(&parked_marker).unwrap().permissions().mode() & 0o777,
        0o600
    );
    assert_eq!(
        fs::metadata(marker_paths.namespace_root.join(NAMESPACE_MARKER_FILE))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );

    let corrupt_parent = tempfile::TempDir::new().unwrap();
    let corrupt_paths = stable_paths(
        corrupt_parent.path().join(STABLE_SANDBOX_NAMESPACE),
        safe_profile("post-release-corrupt-marker"),
    )
    .unwrap();
    let expected_identity = std::cell::Cell::new(None);
    let corrupt_bytes = b"corrupt marker bytes";
    let corrupt_result = initialize_namespace_retained_with_hooks(
        &corrupt_paths,
        &SandboxFaults::default(),
        no_namespace_marker_publish_hook,
        no_before_namespace_release_hook,
        |_, _, _, _, marker| {
            use std::io::{Seek as _, Write as _};

            expected_identity.set(Some(marker.identity()));
            let mut file = marker.file().try_clone().unwrap();
            file.set_len(0).unwrap();
            file.seek(io::SeekFrom::Start(0)).unwrap();
            file.write_all(corrupt_bytes).unwrap();
            file.sync_all().unwrap();
        },
    );

    assert!(matches!(
        corrupt_result,
        Err(SandboxError::InvalidEvidence { .. })
    ));
    let parent_handle = crate::cli::tui_real_sandbox_fs::PinnedDirectory::open_ambient_parent(
        corrupt_parent.path(),
    )
    .unwrap();
    let namespace = parent_handle
        .open_directory(&corrupt_paths.namespace_name())
        .unwrap();
    let marker = namespace
        .open_file(&corrupt_paths.namespace_marker_name(), false)
        .unwrap();
    assert_eq!(marker.identity(), expected_identity.get().unwrap());
    assert_eq!(fs::read(marker.path()).unwrap(), corrupt_bytes);
    assert!(initialize_namespace_retained(&corrupt_paths, &SandboxFaults::default(),).is_err());
    assert_eq!(
        namespace
            .open_file(&corrupt_paths.namespace_marker_name(), false)
            .unwrap()
            .identity(),
        expected_identity.get().unwrap()
    );
    assert_eq!(fs::read(marker.path()).unwrap(), corrupt_bytes);
}

#[cfg(target_os = "linux")]
#[test]
fn profile_lease_validation_refuses_replacement_after_lock() {
    let parent = tempfile::TempDir::new().unwrap();
    let paths = stable_paths(
        parent.path().join(STABLE_SANDBOX_NAMESPACE),
        safe_profile("lease-after-lock"),
    )
    .unwrap();
    let (handles, namespace_marker) =
        initialize_namespace_retained(&paths, &SandboxFaults::default()).unwrap();
    let original =
        acquire_profile_lease_retained(&paths, &handles, &SandboxFaults::default()).unwrap();
    let original_identity = original.file().identity();
    original.release(handles.leases()).unwrap();
    let parked_name = crate::cli::tui_real_sandbox_fs::ChildName::literal("parked-lease.lock");
    let parked_path = paths.leases_root.join(parked_name.as_os_str());
    let replacement_identity = std::cell::Cell::new(None);

    let result = acquire_profile_lease_retained_with_hooks(
        &paths,
        &handles,
        &SandboxFaults::default(),
        |lease, leases| {
            fs::rename(lease.file().path(), &parked_path).unwrap();
            let replacement = leases.create_file(&paths.lease_name()).unwrap();
            replacement
                .publish_bytes(leases, &paths.lease_name(), paths.lease_marker_bytes())
                .unwrap();
            replacement_identity.set(Some(replacement.identity()));
        },
        publish_profile_lease_marker,
        no_profile_lease_hook,
        release_profile_lease,
    );

    let error = result.unwrap_err();
    assert!(matches!(
        error,
        SandboxError::Dual { primary, release }
            if matches!(*primary, SandboxError::InvalidEvidence { .. })
                && matches!(*release, SandboxError::InvalidEvidence { .. })
    ));
    let parked = handles.leases().open_file(&parked_name, false).unwrap();
    let replacement = handles
        .leases()
        .open_file(&paths.lease_name(), false)
        .unwrap();
    assert_eq!(parked.identity(), original_identity);
    assert_eq!(replacement.identity(), replacement_identity.get().unwrap());
    assert_ne!(parked.identity(), replacement.identity());
    assert_eq!(fs::read(parked.path()).unwrap(), paths.lease_marker_bytes());
    assert_eq!(
        fs::read(replacement.path()).unwrap(),
        paths.lease_marker_bytes()
    );
    drop(namespace_marker);
}

#[cfg(target_os = "linux")]
#[test]
fn profile_lease_publish_and_release_failures_retain_created_evidence() {
    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    enum Failure {
        TruncateAndUnlock,
        Write,
        Sync,
    }

    for failure in [Failure::TruncateAndUnlock, Failure::Write, Failure::Sync] {
        let parent = tempfile::TempDir::new().unwrap();
        let paths = stable_paths(
            parent.path().join(STABLE_SANDBOX_NAMESPACE),
            safe_profile(match failure {
                Failure::TruncateAndUnlock => "lease-truncate-unlock",
                Failure::Write => "lease-write",
                Failure::Sync => "lease-sync",
            }),
        )
        .unwrap();
        let (handles, namespace_marker) =
            initialize_namespace_retained(&paths, &SandboxFaults::default()).unwrap();
        let created_identity = std::cell::Cell::new(None);

        let result = acquire_profile_lease_retained_with_hooks(
            &paths,
            &handles,
            &SandboxFaults::default(),
            no_profile_lease_hook,
            |file, parent, name, bytes| {
                created_identity.set(Some(file.identity()));
                match failure {
                    Failure::TruncateAndUnlock => file.publish_bytes_with_ops(
                        parent,
                        name,
                        bytes,
                        |_| {
                            Err(io::Error::new(
                                io::ErrorKind::PermissionDenied,
                                "injected lease truncate failure",
                            ))
                        },
                        std::io::Write::write_all,
                        fs::File::sync_all,
                    ),
                    Failure::Write => file.publish_bytes_with_ops(
                        parent,
                        name,
                        bytes,
                        |file| file.set_len(0),
                        |_, _| {
                            Err(io::Error::new(
                                io::ErrorKind::PermissionDenied,
                                "injected lease write failure",
                            ))
                        },
                        fs::File::sync_all,
                    ),
                    Failure::Sync => file.publish_bytes_with_ops(
                        parent,
                        name,
                        bytes,
                        |file| file.set_len(0),
                        std::io::Write::write_all,
                        |_| {
                            Err(io::Error::new(
                                io::ErrorKind::PermissionDenied,
                                "injected lease sync failure",
                            ))
                        },
                    ),
                }
            },
            no_profile_lease_hook,
            |lease, leases| {
                if failure == Failure::TruncateAndUnlock {
                    lease.release_with_ops(leases, |_| {
                        Err(io::Error::new(
                            io::ErrorKind::PermissionDenied,
                            "injected lease unlock failure",
                        ))
                    })
                } else {
                    lease.release(leases)
                }
            },
        );

        let error = result.unwrap_err();
        match failure {
            Failure::TruncateAndUnlock => assert!(matches!(
                error,
                SandboxError::Dual { primary, release }
                    if matches!(*primary, SandboxError::Io {
                        operation: "truncate child file",
                        ..
                    }) && matches!(*release, SandboxError::Io {
                        operation: "unlock profile lease",
                        ..
                    })
            )),
            Failure::Write => assert!(matches!(
                error,
                SandboxError::Io {
                    operation: "write child file",
                    ..
                }
            )),
            Failure::Sync => assert!(matches!(
                error,
                SandboxError::Io {
                    operation: "sync child file",
                    ..
                }
            )),
        }
        let retained = handles
            .leases()
            .open_file(&paths.lease_name(), false)
            .unwrap();
        assert_eq!(retained.identity(), created_identity.get().unwrap());
        let expected = if failure == Failure::Sync {
            paths.lease_marker_bytes()
        } else {
            b""
        };
        assert_eq!(fs::read(retained.path()).unwrap(), expected);
        drop(namespace_marker);
    }
}

#[cfg(target_os = "linux")]
#[test]
fn profile_lease_final_read_refuses_post_publish_replacement() {
    let parent = tempfile::TempDir::new().unwrap();
    let paths = stable_paths(
        parent.path().join(STABLE_SANDBOX_NAMESPACE),
        safe_profile("lease-after-publish"),
    )
    .unwrap();
    let (handles, namespace_marker) =
        initialize_namespace_retained(&paths, &SandboxFaults::default()).unwrap();
    let parked_name =
        crate::cli::tui_real_sandbox_fs::ChildName::literal("parked-published-lease.lock");
    let parked_path = paths.leases_root.join(parked_name.as_os_str());
    let original_identity = std::cell::Cell::new(None);
    let replacement_identity = std::cell::Cell::new(None);

    let result = acquire_profile_lease_retained_with_hooks(
        &paths,
        &handles,
        &SandboxFaults::default(),
        no_profile_lease_hook,
        publish_profile_lease_marker,
        |lease, leases| {
            original_identity.set(Some(lease.file().identity()));
            fs::rename(lease.file().path(), &parked_path).unwrap();
            let replacement = leases.create_file(&paths.lease_name()).unwrap();
            replacement
                .publish_bytes(leases, &paths.lease_name(), paths.lease_marker_bytes())
                .unwrap();
            replacement_identity.set(Some(replacement.identity()));
        },
        release_profile_lease,
    );

    let error = result.unwrap_err();
    assert!(matches!(
        error,
        SandboxError::Dual { primary, release }
            if matches!(*primary, SandboxError::InvalidEvidence { .. })
                && matches!(*release, SandboxError::InvalidEvidence { .. })
    ));
    let parked = handles.leases().open_file(&parked_name, false).unwrap();
    let replacement = handles
        .leases()
        .open_file(&paths.lease_name(), false)
        .unwrap();
    assert_eq!(parked.identity(), original_identity.get().unwrap());
    assert_eq!(replacement.identity(), replacement_identity.get().unwrap());
    assert_ne!(parked.identity(), replacement.identity());
    assert_eq!(fs::read(parked.path()).unwrap(), paths.lease_marker_bytes());
    assert_eq!(
        fs::read(replacement.path()).unwrap(),
        paths.lease_marker_bytes()
    );
    drop(namespace_marker);
}
