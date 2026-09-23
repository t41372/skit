//! Stable host lifecycle contracts on the real filesystem.

use super::*;

#[cfg(windows)]
fn windows_file_identity(path: &Path) -> (u64, u128) {
    let file = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .unwrap();
    let identity = fs_id::FileID::new(&file).unwrap();
    (identity.storage_id(), identity.internal_file_id())
}

#[cfg(target_os = "linux")]
#[test]
fn stable_namespace_default_is_the_literal_linux_root() {
    assert_eq!(
        default_stable_namespace_root().unwrap(),
        PathBuf::from("/tmp").join(STABLE_SANDBOX_NAMESPACE)
    );
}

#[test]
fn stable_namespace_platform_policy_is_pure_and_complete() {
    let windows_temp = Path::new(r"C:\walker-temp");
    assert_eq!(
        stable_namespace_root_for(SandboxPlatform::Linux, windows_temp).unwrap(),
        PathBuf::from("/tmp").join(STABLE_SANDBOX_NAMESPACE)
    );
    assert_eq!(
        stable_namespace_root_for(SandboxPlatform::Windows, windows_temp).unwrap(),
        windows_temp.join(STABLE_SANDBOX_NAMESPACE)
    );
    assert!(matches!(
        stable_namespace_root_for(SandboxPlatform::Macos, windows_temp),
        Err(SandboxError::UnsupportedPlatform { platform: "macos" })
    ));
    #[cfg(target_os = "linux")]
    assert_eq!(
        StableSandboxNamespace::system().unwrap(),
        StableSandboxNamespace::explicit(PathBuf::from("/tmp").join(STABLE_SANDBOX_NAMESPACE),)
            .unwrap()
    );
}

#[cfg(target_os = "windows")]
#[test]
fn stable_namespace_default_uses_the_windows_temp_root() {
    assert_eq!(
        default_stable_namespace_root().unwrap(),
        std::env::temp_dir().join(STABLE_SANDBOX_NAMESPACE)
    );
}

#[cfg(target_os = "macos")]
#[test]
fn macos_reports_random_metadata_but_refuses_stable_acquisition() {
    let review_profile = safe_profile("macos-random");
    let host = RealWalkerHost::spawn(profile()).unwrap();
    assert_eq!(
        host.sandbox_metadata(&review_profile).unwrap().platform(),
        SandboxPlatform::Macos
    );
    host.close().unwrap();
    assert!(matches!(
        StableSandboxNamespace::system(),
        Err(SandboxError::UnsupportedPlatform { platform: "macos" })
    ));
}

#[test]
fn random_owner_reports_truthful_single_profile_metadata() {
    let review_profile = safe_profile("engine-smoke-60x24");
    let host = RealWalkerHost::spawn(profile()).unwrap();
    let root = host.sandbox_root().to_path_buf();
    assert!(host._sandbox.stable().is_none());

    let metadata = host.sandbox_metadata(&review_profile).unwrap();

    assert_eq!(metadata.mode(), SandboxMode::Random);
    assert_eq!(metadata.platform(), current_sandbox_platform().unwrap());
    assert_eq!(metadata.root(), root.to_str().unwrap());
    assert_eq!(
        metadata.profiles().get(&review_profile).map(String::as_str),
        Some(root.to_str().unwrap())
    );
    host.close().unwrap();
    assert!(!root.exists());

    let creation = SandboxOwner::random_with(|| {
        Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "random sandbox denied",
        ))
    })
    .unwrap_err();
    assert!(matches!(
        creation,
        SandboxError::Io {
            operation: "create random sandbox",
            ..
        }
    ));
}

#[test]
fn explicitly_owned_transient_outside_the_profile_uses_the_transient_root() {
    let mut host = RealWalkerHost::spawn(profile()).unwrap();
    let outside = tempfile::TempDir::new().unwrap();
    let path = outside.path().join(".injected-outside.js");
    let paths = &mut host.path_map;

    paths.register_owned_transient_path(&path, "injected-source", ".injected-");

    assert_eq!(
        paths.normalize_path(&path),
        "<transient>/<injected-source:0>.js"
    );
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
#[test]
fn stable_owner_publishes_exact_layout_and_closes_only_the_profile() {
    let parent = tempfile::TempDir::new().unwrap();
    fs::write(
        parent.path().join("unrelated-parent-file"),
        b"retain parent",
    )
    .unwrap();
    let namespace_root = parent.path().join(STABLE_SANDBOX_NAMESPACE);
    let review_profile = safe_profile("engine-smoke-60x24");
    let host = RealWalkerHost::spawn_stable_in(
        profile(),
        review_profile.clone(),
        StableSandboxNamespace::explicit(namespace_root.clone()).unwrap(),
    )
    .unwrap();
    let profile_root = namespace_root.join(profile_sandbox_path(&review_profile));
    let cleanup_root = namespace_root.join(profile_cleanup_path(&review_profile));
    let lease_path = namespace_root.join(profile_lease_path(&review_profile));
    let init_lock = parent
        .path()
        .join(format!(".{STABLE_SANDBOX_NAMESPACE}.init.lock"));
    let platform = current_sandbox_platform().unwrap();
    let roots = SandboxRoots::new(platform, profile_root.display().to_string()).unwrap();

    assert_eq!(host.sandbox_root(), profile_root);
    assert_eq!(host.adapters.system_temp, profile_root.join("system-temp"));
    let metadata = host.sandbox_metadata(&review_profile).unwrap();
    assert_eq!(metadata.mode(), SandboxMode::Stable);
    assert_eq!(metadata.platform(), platform);
    assert_eq!(metadata.root(), namespace_root.to_str().unwrap());
    assert_eq!(
        metadata.profiles().get(&review_profile).map(String::as_str),
        Some(profile_root.to_str().unwrap())
    );
    assert!(
        host.sandbox_metadata(&safe_profile("another-profile"))
            .is_err()
    );
    assert_eq!(
        fs::read(&init_lock).unwrap(),
        ParentInitLockMarker::new(platform, namespace_root.display().to_string())
            .unwrap()
            .canonical_bytes()
            .unwrap()
    );
    assert_eq!(
        fs::read(namespace_root.join(NAMESPACE_MARKER_FILE)).unwrap(),
        NamespaceMarker::new(platform, namespace_root.display().to_string(),)
            .unwrap()
            .canonical_bytes()
            .unwrap()
    );
    let stable = host._sandbox.stable().unwrap();
    assert_eq!(
        stable
            .lease
            .as_ref()
            .unwrap()
            .file()
            .read_all(stable.handles().leases(), &stable.paths.lease_name())
            .unwrap(),
        ProfileLeaseMarker::new(
            platform,
            review_profile.clone(),
            namespace_root.display().to_string(),
        )
        .unwrap()
        .canonical_bytes()
        .unwrap()
    );
    assert_eq!(
        fs::read(profile_root.join(SANDBOX_MARKER_FILE)).unwrap(),
        SandboxMarker::new(
            platform,
            review_profile.clone(),
            namespace_root.display().to_string(),
        )
        .unwrap()
        .canonical_bytes()
        .unwrap()
    );
    assert_eq!(
        SandboxMarker::new(
            platform,
            review_profile.clone(),
            namespace_root.display().to_string(),
        )
        .unwrap()
        .roots(),
        &roots
    );
    for child in [
        "data",
        "state",
        "config",
        "home",
        "cwd",
        "external",
        "system-temp",
    ] {
        assert!(profile_root.join(child).is_dir());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;

        for path in [
            namespace_root.clone(),
            namespace_root.join("leases"),
            namespace_root.join("sandboxes"),
            profile_root.clone(),
            profile_root.join("data"),
            profile_root.join("state"),
            profile_root.join("config"),
            profile_root.join("home"),
            profile_root.join("cwd"),
            profile_root.join("external"),
            profile_root.join("system-temp"),
        ] {
            assert_eq!(
                fs::metadata(path).unwrap().permissions().mode() & 0o777,
                0o700
            );
        }
        for path in [
            init_lock.clone(),
            namespace_root.join(NAMESPACE_MARKER_FILE),
            lease_path.clone(),
            profile_root.join(SANDBOX_MARKER_FILE),
        ] {
            assert_eq!(
                fs::metadata(path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
    }

    host.close().unwrap();

    assert!(!profile_root.exists());
    assert!(!cleanup_root.exists());
    assert!(init_lock.is_file());
    assert!(lease_path.is_file());
    assert!(namespace_root.join(NAMESPACE_MARKER_FILE).is_file());
    assert_eq!(
        fs::read(parent.path().join("unrelated-parent-file")).unwrap(),
        b"retain parent"
    );
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
fn initialized_sandbox_paths(
    parent: &tempfile::TempDir,
    profile: &SafeProfileId,
) -> StableSandboxPaths {
    let paths = stable_paths(
        parent.path().join(STABLE_SANDBOX_NAMESPACE),
        profile.clone(),
    )
    .unwrap();
    StableSandbox::acquire(
        StableSandboxNamespace::explicit(paths.namespace_root.clone()).unwrap(),
        profile.clone(),
        SandboxFaults::default(),
    )
    .unwrap()
    .close()
    .unwrap();
    paths
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
fn create_invalid_sandbox_evidence(path: &Path) {
    create_private_test_directory(path);
    fs::write(path.join("unowned-evidence"), b"retain invalid bytes").unwrap();
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
fn evidence_snapshot(path: &Path) -> Option<Vec<OutsideRecord>> {
    fs::symlink_metadata(path)
        .ok()
        .map(|_| outside_snapshot_portable(path))
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
#[test]
fn stable_acquisition_executes_all_nine_evidence_states() {
    use SandboxEvidenceState::{Absent, Invalid, Valid};

    for (original_state, cleanup_state, succeeds) in [
        (Valid, Absent, true),
        (Absent, Valid, true),
        (Absent, Absent, true),
        (Valid, Valid, false),
        (Valid, Invalid, false),
        (Invalid, Valid, false),
        (Invalid, Absent, false),
        (Absent, Invalid, false),
        (Invalid, Invalid, false),
    ] {
        let parent = tempfile::TempDir::new().unwrap();
        let profile = safe_profile("nine-state");
        let paths = initialized_sandbox_paths(&parent, &profile);
        for (path, state) in [
            (&paths.sandbox_root, original_state),
            (&paths.cleanup_root, cleanup_state),
        ] {
            match state {
                Absent => {}
                Valid => create_valid_sandbox_evidence(path, &paths),
                Invalid => create_invalid_sandbox_evidence(path),
            }
        }
        let original_before = evidence_snapshot(&paths.sandbox_root);
        let cleanup_before = evidence_snapshot(&paths.cleanup_root);

        let result = StableSandbox::acquire(
            StableSandboxNamespace::explicit(paths.namespace_root.clone()).unwrap(),
            profile,
            SandboxFaults::default(),
        );

        assert_eq!(
            result.is_ok(),
            succeeds,
            "original {original_state:?}, cleanup {cleanup_state:?}"
        );
        if let Ok(sandbox) = result {
            sandbox.close().unwrap();
        } else {
            assert_eq!(evidence_snapshot(&paths.sandbox_root), original_before);
            assert_eq!(evidence_snapshot(&paths.cleanup_root), cleanup_before);
        }
    }
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
#[test]
fn exact_original_and_partial_cleanup_layouts_have_distinct_recovery_rules() {
    let original_parent = tempfile::TempDir::new().unwrap();
    let original_profile = safe_profile("extra-original");
    let original_paths = initialized_sandbox_paths(&original_parent, &original_profile);
    create_valid_sandbox_evidence(&original_paths.sandbox_root, &original_paths);
    fs::write(
        original_paths.sandbox_root.join("unexpected"),
        b"retain unexpected bytes",
    )
    .unwrap();
    let before = evidence_snapshot(&original_paths.sandbox_root);

    assert!(
        StableSandbox::acquire(
            StableSandboxNamespace::explicit(original_paths.namespace_root.clone()).unwrap(),
            original_profile,
            SandboxFaults::default(),
        )
        .is_err()
    );
    assert_eq!(evidence_snapshot(&original_paths.sandbox_root), before);

    let cleanup_parent = tempfile::TempDir::new().unwrap();
    let cleanup_profile = safe_profile("partial-cleanup");
    let cleanup_paths = initialized_sandbox_paths(&cleanup_parent, &cleanup_profile);
    create_valid_sandbox_evidence(&cleanup_paths.cleanup_root, &cleanup_paths);
    fs::remove_dir(cleanup_paths.cleanup_root.join("data")).unwrap();
    fs::remove_dir(cleanup_paths.cleanup_root.join("state")).unwrap();

    StableSandbox::acquire(
        StableSandboxNamespace::explicit(cleanup_paths.namespace_root.clone()).unwrap(),
        cleanup_profile,
        SandboxFaults::default(),
    )
    .unwrap()
    .close()
    .unwrap();

    assert!(!cleanup_paths.cleanup_root.exists());

    let invalid_cleanup_parent = tempfile::TempDir::new().unwrap();
    let invalid_cleanup_profile = safe_profile("extra-cleanup");
    let invalid_cleanup_paths =
        initialized_sandbox_paths(&invalid_cleanup_parent, &invalid_cleanup_profile);
    create_valid_sandbox_evidence(&invalid_cleanup_paths.cleanup_root, &invalid_cleanup_paths);
    fs::write(
        invalid_cleanup_paths.cleanup_root.join("unexpected"),
        b"retain cleanup bytes",
    )
    .unwrap();
    let before = evidence_snapshot(&invalid_cleanup_paths.cleanup_root);

    assert!(
        StableSandbox::acquire(
            StableSandboxNamespace::explicit(invalid_cleanup_paths.namespace_root.clone()).unwrap(),
            invalid_cleanup_profile,
            SandboxFaults::default(),
        )
        .is_err()
    );
    assert_eq!(
        evidence_snapshot(&invalid_cleanup_paths.cleanup_root),
        before
    );
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
#[test]
fn stable_profile_leases_distinguish_contention_and_release() {
    let parent = tempfile::TempDir::new().unwrap();
    let namespace_root = parent.path().join(STABLE_SANDBOX_NAMESPACE);
    let namespace = || StableSandboxNamespace::explicit(namespace_root.clone()).unwrap();
    let review_profile = safe_profile("lease-profile");
    let first =
        RealWalkerHost::spawn_stable_in(profile(), review_profile.clone(), namespace()).unwrap();

    let busy = RealWalkerHost::spawn_stable_in(profile(), review_profile.clone(), namespace())
        .unwrap_err();

    assert!(matches!(
        busy,
        StableHostError::Sandbox(SandboxError::Busy {
            profile,
            path,
        }) if profile == review_profile
            && path == namespace_root.join(profile_lease_path(&review_profile))
    ));
    first.close().unwrap();
    RealWalkerHost::spawn_stable_in(profile(), review_profile, namespace())
        .unwrap()
        .close()
        .unwrap();
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
#[test]
fn init_and_profile_lock_io_failures_are_not_reported_as_contention() {
    for point in [
        SandboxFaultPoint::ParentInitLockIo,
        SandboxFaultPoint::ProfileLeaseLockIo,
    ] {
        let parent = tempfile::TempDir::new().unwrap();
        let namespace_root = parent.path().join(STABLE_SANDBOX_NAMESPACE);

        let error = RealWalkerHost::spawn_stable_in_with_faults(
            profile(),
            safe_profile("lock-io"),
            StableSandboxNamespace::explicit(namespace_root).unwrap(),
            SandboxFaults::at(point),
        )
        .unwrap_err();

        assert!(matches!(
            error,
            StableHostError::Sandbox(SandboxError::Io {
                operation: "lock",
                ..
            })
        ));
    }

    let parent = tempfile::TempDir::new().unwrap();
    let paths = initialized_sandbox_paths(&parent, &safe_profile("mapped-lock-io"));
    let mapped = map_profile_lock_result(
        Err(fs::TryLockError::Error(io::Error::other(
            "injected native lease failure",
        ))),
        &paths,
    )
    .unwrap_err();
    assert!(matches!(
        mapped,
        SandboxError::Io {
            operation: "lock",
            ..
        }
    ));
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
#[test]
fn empty_persistent_lock_evidence_is_retried_then_retained_and_refused() {
    let parent = tempfile::TempDir::new().unwrap();
    let namespace_root = parent.path().join(STABLE_SANDBOX_NAMESPACE);
    let init_lock = parent
        .path()
        .join(format!(".{STABLE_SANDBOX_NAMESPACE}.init.lock"));
    write_private_test_file(&init_lock, b"");

    let init_error = StableSandbox::acquire(
        StableSandboxNamespace::explicit(namespace_root.clone()).unwrap(),
        safe_profile("empty-init"),
        SandboxFaults::default(),
    )
    .unwrap_err();

    assert!(matches!(
        init_error,
        SandboxError::InvalidEvidence { path, .. } if path == init_lock
    ));
    assert_eq!(fs::read(&init_lock).unwrap(), b"");
    fs::write(&init_lock, b"wrong parent marker").unwrap();
    assert!(matches!(
        StableSandbox::acquire(
            StableSandboxNamespace::explicit(namespace_root).unwrap(),
            safe_profile("wrong-init"),
            SandboxFaults::default(),
        ),
        Err(SandboxError::InvalidEvidence { path, .. }) if path == init_lock
    ));
    assert_eq!(fs::read(&init_lock).unwrap(), b"wrong parent marker");

    let lease_parent = tempfile::TempDir::new().unwrap();
    let review_profile = safe_profile("empty-lease");
    let paths = initialized_sandbox_paths(&lease_parent, &review_profile);
    fs::write(&paths.lease_path, b"").unwrap();

    let lease_error = StableSandbox::acquire(
        StableSandboxNamespace::explicit(paths.namespace_root.clone()).unwrap(),
        review_profile,
        SandboxFaults::default(),
    )
    .unwrap_err();

    assert!(matches!(
        lease_error,
        SandboxError::InvalidEvidence { path, .. } if path == paths.lease_path
    ));
    assert_eq!(fs::read(paths.lease_path).unwrap(), b"");
}

// The stable sandbox does not support macOS, so this test skips there. Keep it compiled on
// macOS. A cfg gate would make the complete stable owner dead code.
#[cfg(unix)]
#[cfg_attr(
    target_os = "macos",
    ignore = "the stable sandbox does not support macOS"
)]
#[test]
fn persistent_init_and_profile_lock_inodes_are_reused() {
    use std::os::unix::fs::MetadataExt as _;

    let parent = tempfile::TempDir::new().unwrap();
    let namespace_root = parent.path().join(STABLE_SANDBOX_NAMESPACE);
    let namespace = || StableSandboxNamespace::explicit(namespace_root.clone()).unwrap();
    let review_profile = safe_profile("persistent-inodes");
    let init_lock = parent
        .path()
        .join(format!(".{STABLE_SANDBOX_NAMESPACE}.init.lock"));
    let lease = namespace_root.join(profile_lease_path(&review_profile));
    RealWalkerHost::spawn_stable_in(profile(), review_profile.clone(), namespace())
        .unwrap()
        .close()
        .unwrap();
    let first_init = fs::metadata(&init_lock).unwrap().ino();
    let first_lease = fs::metadata(&lease).unwrap().ino();

    RealWalkerHost::spawn_stable_in(profile(), review_profile, namespace())
        .unwrap()
        .close()
        .unwrap();

    assert_eq!(fs::metadata(init_lock).unwrap().ino(), first_init);
    assert_eq!(fs::metadata(lease).unwrap().ino(), first_lease);
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
#[test]
fn kernel_releases_the_profile_lease_after_process_exit() {
    const CHILD: &str = "SKIT_STABLE_SANDBOX_LEASE_EXIT_CHILD";
    const ROOT: &str = "SKIT_STABLE_SANDBOX_LEASE_EXIT_ROOT";
    let test = harness_test_name(
        module_path!(),
        "kernel_releases_the_profile_lease_after_process_exit",
    );
    let review_profile = safe_profile("kernel-exit");
    if std::env::var_os(CHILD).is_some() {
        let namespace_root = PathBuf::from(std::env::var_os(ROOT).unwrap());
        let host = RealWalkerHost::spawn_stable_in(
            profile(),
            review_profile,
            StableSandboxNamespace::explicit(namespace_root).unwrap(),
        )
        .unwrap();
        assert!(host.sandbox_root().is_dir());
        std::process::exit(73);
    }

    let parent = tempfile::TempDir::new().unwrap();
    let namespace_root = parent.path().join(STABLE_SANDBOX_NAMESPACE);
    let coverage_profile = restrictive_umask_child_profile();
    let mut command = std::process::Command::new(std::env::current_exe().unwrap());
    command
        .arg("--exact")
        .arg(&test)
        .arg("--nocapture")
        .env(CHILD, "1")
        .env(ROOT, &namespace_root);
    if let Some(profile) = coverage_profile.as_ref() {
        command.env("LLVM_PROFILE_FILE", profile);
    }

    let status = command.status().unwrap();

    assert_eq!(status.code(), Some(73));
    #[cfg(windows)]
    let lease_identity = windows_file_identity(
        &namespace_root.join(profile_lease_path(&safe_profile("kernel-exit"))),
    );
    RealWalkerHost::spawn_stable_in(
        profile(),
        safe_profile("kernel-exit"),
        StableSandboxNamespace::explicit(namespace_root.clone()).unwrap(),
    )
    .unwrap()
    .close()
    .unwrap();
    #[cfg(windows)]
    assert_eq!(
        windows_file_identity(
            &namespace_root.join(profile_lease_path(&safe_profile("kernel-exit"))),
        ),
        lease_identity
    );
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
#[test]
fn different_profiles_can_initialize_and_run_in_parallel() {
    use std::sync::{Arc, Barrier};

    let parent = tempfile::TempDir::new().unwrap();
    let namespace_root = parent.path().join(STABLE_SANDBOX_NAMESPACE);
    let barrier = Arc::new(Barrier::new(2));
    let results = std::thread::scope(|scope| {
        ["parallel-a", "parallel-b"]
            .into_iter()
            .map(|profile_id| {
                let namespace_root = namespace_root.clone();
                let barrier = Arc::clone(&barrier);
                scope.spawn(move || {
                    barrier.wait();
                    let review_profile = safe_profile(profile_id);
                    let host = RealWalkerHost::spawn_stable_in(
                        WalkerSeedSpec {
                            profile: "parallel-seed".to_owned(),
                            ..WalkerSeedSpec::default()
                        },
                        review_profile,
                        StableSandboxNamespace::explicit(namespace_root).unwrap(),
                    )
                    .map_err(|error| error.to_string())?;
                    host.close().map_err(|error| error.to_string())
                })
            })
            .collect::<Vec<_>>()
            .into_iter()
            .map(|worker| worker.join().unwrap())
            .collect::<Vec<_>>()
    });

    assert_eq!(results, vec![Ok(()), Ok(())]);
    for profile_id in ["parallel-a", "parallel-b"] {
        assert!(
            namespace_root
                .join(profile_lease_path(&safe_profile(profile_id)))
                .is_file()
        );
    }
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
#[test]
fn marker_publication_faults_leave_only_the_specified_recovery_state() {
    use SandboxFaultPoint::{
        AfterNamespaceMarker, AfterSandboxMarker, BeforeNamespaceMarker, BeforeSandboxMarker,
    };

    for (point, next_acquire_succeeds) in [
        (BeforeNamespaceMarker, false),
        (AfterNamespaceMarker, true),
        (BeforeSandboxMarker, false),
        (AfterSandboxMarker, true),
    ] {
        let parent = tempfile::TempDir::new().unwrap();
        let namespace_root = parent.path().join(STABLE_SANDBOX_NAMESPACE);
        let namespace = || StableSandboxNamespace::explicit(namespace_root.clone()).unwrap();
        let review_profile = safe_profile("marker-fault");

        let error = RealWalkerHost::spawn_stable_in_with_faults(
            profile(),
            review_profile.clone(),
            namespace(),
            SandboxFaults::at(point),
        )
        .unwrap_err();

        assert!(matches!(
            error,
            StableHostError::Sandbox(SandboxError::InjectedFault {
                point: actual,
            }) if actual == point
        ));
        let next = RealWalkerHost::spawn_stable_in(profile(), review_profile.clone(), namespace());
        assert_eq!(next.is_ok(), next_acquire_succeeds, "fault point {point:?}");
        if let Ok(host) = next {
            host.close().unwrap();
        }
    }
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
#[test]
fn primary_directory_creation_faults_preserve_the_exact_recovery_boundary() {
    use SandboxFaultPoint::{
        CreateLeaseDirectoryIo, CreateNamespaceIo, CreateSandboxChildIo, CreateSandboxDirectoryIo,
        CreateSandboxRootIo, InspectNamespaceIo,
    };

    for (point, next_acquire_succeeds) in [
        (CreateNamespaceIo, true),
        (InspectNamespaceIo, true),
        (CreateLeaseDirectoryIo, false),
        (CreateSandboxDirectoryIo, false),
        (CreateSandboxRootIo, true),
        (CreateSandboxChildIo, false),
    ] {
        let parent = tempfile::TempDir::new().unwrap();
        let namespace_root = parent.path().join(STABLE_SANDBOX_NAMESPACE);
        let namespace = || StableSandboxNamespace::explicit(namespace_root.clone()).unwrap();
        let review_profile = safe_profile("create-fault");

        let error = RealWalkerHost::spawn_stable_in_with_faults(
            profile(),
            review_profile.clone(),
            namespace(),
            SandboxFaults::at(point),
        )
        .unwrap_err();

        assert!(matches!(
            error,
            StableHostError::Sandbox(SandboxError::Io { .. })
        ));
        let next = RealWalkerHost::spawn_stable_in(profile(), review_profile.clone(), namespace());
        assert_eq!(
            next.is_ok(),
            next_acquire_succeeds,
            "directory creation fault {point:?}"
        );
        if let Ok(host) = next {
            host.close().unwrap();
        }
    }
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
#[test]
fn cleanup_faults_recover_only_while_the_marker_remains() {
    use SandboxFaultPoint::{
        AfterChildRemoval, AfterRenameIdentity, RemoveChild, RemoveDirectory, RemoveMarker,
        RenameOriginal,
    };

    for (point, next_acquire_succeeds) in [
        (RenameOriginal, true),
        (AfterRenameIdentity, true),
        (RemoveChild, true),
        (AfterChildRemoval, true),
        (RemoveMarker, true),
        (RemoveDirectory, false),
    ] {
        let parent = tempfile::TempDir::new().unwrap();
        let namespace_root = parent.path().join(STABLE_SANDBOX_NAMESPACE);
        let namespace = || StableSandboxNamespace::explicit(namespace_root.clone()).unwrap();
        let review_profile = safe_profile("cleanup-fault");
        let host = RealWalkerHost::spawn_stable_in_with_faults(
            profile(),
            review_profile.clone(),
            namespace(),
            SandboxFaults::at(point),
        )
        .unwrap();

        let error = host.close().unwrap_err();

        assert!(matches!(
            error,
            SandboxError::InjectedFault { point: actual } if actual == point
        ));
        let sandbox_root = namespace_root.join(profile_sandbox_path(&review_profile));
        let cleanup_root = namespace_root.join(profile_cleanup_path(&review_profile));
        assert!(sandbox_root.exists() || cleanup_root.exists());
        if point == RemoveDirectory {
            assert!(cleanup_root.exists());
            assert!(!cleanup_root.join(SANDBOX_MARKER_FILE).exists());
        }
        let next = RealWalkerHost::spawn_stable_in(profile(), review_profile.clone(), namespace());
        assert_eq!(
            next.is_ok(),
            next_acquire_succeeds,
            "cleanup fault point {point:?}"
        );
        if let Ok(host) = next {
            host.close().unwrap();
        }
    }
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
#[test]
fn stable_host_drop_performs_best_effort_containment() {
    let parent = tempfile::TempDir::new().unwrap();
    let namespace_root = parent.path().join(STABLE_SANDBOX_NAMESPACE);
    let review_profile = safe_profile("drop-containment");
    let profile_root = namespace_root.join(profile_sandbox_path(&review_profile));
    let host = RealWalkerHost::spawn_stable_in(
        profile(),
        review_profile,
        StableSandboxNamespace::explicit(namespace_root).unwrap(),
    )
    .unwrap();

    drop(host);

    assert!(!profile_root.exists());
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
#[test]
fn seed_and_cleanup_failures_remain_separate_typed_evidence() {
    let parent = tempfile::TempDir::new().unwrap();
    let namespace_root = parent.path().join(STABLE_SANDBOX_NAMESPACE);
    let review_profile = safe_profile("primary-cleanup");
    let mut spec = profile();
    spec.directories = vec![directory_seed(WalkerDirectoryRoot::Home, ".codex")];
    let io = G2DirectorySeedIo::new(
        G2DirectorySeedFault::Create,
        parent.path().join("unused-symlink-target"),
    );

    let error = RealWalkerHost::spawn_stable_in_with_faults_and_directory_seed_io(
        spec,
        review_profile.clone(),
        StableSandboxNamespace::explicit(namespace_root.clone()).unwrap(),
        SandboxFaults::at(SandboxFaultPoint::RemoveChild),
        &io,
    )
    .unwrap_err();

    assert!(matches!(
        error,
        StableHostError::Primary {
            primary: StableHostPrimaryError::Seed(_),
            cleanup: Some(SandboxError::InjectedFault {
                point: SandboxFaultPoint::RemoveChild,
            }),
        }
    ));
    let cleanup_root = namespace_root.join(profile_cleanup_path(&review_profile));
    assert!(cleanup_root.join(SANDBOX_MARKER_FILE).is_file());

    let clean_parent = tempfile::TempDir::new().unwrap();
    let clean_namespace = clean_parent.path().join(STABLE_SANDBOX_NAMESPACE);
    let mut clean_failure_spec = profile();
    clean_failure_spec.directories = vec![directory_seed(WalkerDirectoryRoot::Cwd, ".claude")];
    let clean_io = G2DirectorySeedIo::new(
        G2DirectorySeedFault::Create,
        clean_parent.path().join("unused-symlink-target"),
    );
    let clean_error = RealWalkerHost::spawn_stable_in_with_directory_seed_io(
        clean_failure_spec.clone(),
        safe_profile("primary-only"),
        StableSandboxNamespace::explicit(clean_namespace.clone()).unwrap(),
        &clean_io,
    )
    .unwrap_err();
    assert!(matches!(
        clean_error,
        StableHostError::Primary {
            primary: StableHostPrimaryError::Seed(_),
            cleanup: None,
        }
    ));
    assert!(
        fs::read_dir(clean_namespace.join("sandboxes"))
            .unwrap()
            .next()
            .is_none()
    );
    let random_parent = tempfile::TempDir::new().unwrap();
    let random_io = G2DirectorySeedIo::new(
        G2DirectorySeedFault::Create,
        random_parent.path().join("unused-symlink-target"),
    );
    assert!(RealWalkerHost::spawn_with_directory_seed_io(clean_failure_spec, &random_io).is_err());

    let invalid_parent = tempfile::TempDir::new().unwrap();
    let invalid_namespace = invalid_parent.path().join(STABLE_SANDBOX_NAMESPACE);
    let invalid = WalkerSeedSpec {
        external: vec![WalkerExternalSeed::File {
            path: PathBuf::from("../escape"),
            bytes: b"never written".to_vec(),
            readonly: false,
            unix_mode: 0o600,
        }],
        ..WalkerSeedSpec::default()
    };
    let invalid_error = RealWalkerHost::spawn_stable_in(
        invalid,
        safe_profile("invalid-seed"),
        StableSandboxNamespace::explicit(invalid_namespace.clone()).unwrap(),
    )
    .unwrap_err();
    assert!(matches!(
        invalid_error,
        StableHostError::Primary {
            primary: StableHostPrimaryError::Seed(_),
            cleanup: None,
        }
    ));
    assert!(!invalid_namespace.exists());
}

#[cfg(unix)]
#[test]
fn random_primary_and_cleanup_failures_are_retained_together() {
    use std::os::unix::fs::PermissionsExt as _;

    let sandbox = tempfile::TempDir::new().unwrap();
    let path = sandbox.path().to_path_buf();
    fs::write(path.join("retained"), b"cleanup evidence").unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o500)).unwrap();

    let error = SeededTuiHost::failed_random_spawn(
        SandboxOwner::Random {
            directory: sandbox,
            root: path.clone(),
        },
        "random primary failure".to_owned(),
    )
    .unwrap_err();

    assert!(error.contains("random primary failure; random sandbox cleanup also failed"));
    fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
    fs::remove_dir_all(path).unwrap();
}

// The stable sandbox does not support macOS, so this test skips there. Keep it compiled on
// macOS. A cfg gate would make the complete stable owner dead code.
#[cfg(unix)]
#[cfg_attr(
    target_os = "macos",
    ignore = "the stable sandbox does not support macOS"
)]
#[test]
fn stable_cleanup_removes_links_and_preserves_the_external_target() {
    let parent = tempfile::TempDir::new().unwrap();
    let namespace_root = parent.path().join(STABLE_SANDBOX_NAMESPACE);
    let review_profile = safe_profile("link-cleanup");
    let outside = parent.path().join("outside-target");
    fs::create_dir(&outside).unwrap();
    fs::write(outside.join("retained.txt"), b"outside bytes").unwrap();
    let host = RealWalkerHost::spawn_stable_in(
        profile(),
        review_profile,
        StableSandboxNamespace::explicit(namespace_root).unwrap(),
    )
    .unwrap();
    std::os::unix::fs::symlink(&outside, host.external_root.join("outside-link")).unwrap();

    host.close().unwrap();

    assert_eq!(
        fs::read(outside.join("retained.txt")).unwrap(),
        b"outside bytes"
    );
}

#[cfg(windows)]
#[test]
fn stable_cleanup_removes_reparse_directories_and_preserves_the_external_target() {
    let parent = tempfile::TempDir::new().unwrap();
    let namespace_root = parent.path().join(STABLE_SANDBOX_NAMESPACE);
    let review_profile = safe_profile("reparse-cleanup");
    let outside = parent.path().join("outside-target");
    fs::create_dir(&outside).unwrap();
    fs::write(outside.join("retained.txt"), b"outside bytes").unwrap();
    let mut host = RealWalkerHost::spawn_stable_in(
        profile(),
        review_profile,
        StableSandboxNamespace::explicit(namespace_root).unwrap(),
    )
    .unwrap();
    std::os::windows::fs::symlink_dir(&outside, host.external_root.join("outside-link")).unwrap();

    host.close().unwrap();

    assert_eq!(
        fs::read(outside.join("retained.txt")).unwrap(),
        b"outside bytes"
    );
}

// The stable sandbox supports Linux and Windows. This test uses Unix links. Thus it runs on
// Linux only.
#[cfg(target_os = "linux")]
#[test]
fn ownership_links_are_refused_without_touching_their_targets() {
    let parent = tempfile::TempDir::new().unwrap();
    let namespace_root = parent.path().join(STABLE_SANDBOX_NAMESPACE);
    let init_lock = parent
        .path()
        .join(format!(".{STABLE_SANDBOX_NAMESPACE}.init.lock"));
    let outside = parent.path().join("outside-lock-target");
    fs::write(&outside, b"outside lock bytes").unwrap();
    std::os::unix::fs::symlink(&outside, &init_lock).unwrap();

    let error = RealWalkerHost::spawn_stable_in(
        profile(),
        safe_profile("linked-owner"),
        StableSandboxNamespace::explicit(namespace_root).unwrap(),
    )
    .unwrap_err();

    assert!(matches!(
        error,
        StableHostError::Sandbox(SandboxError::InvalidEvidence { path, .. })
            if path == init_lock
    ));
    assert_eq!(fs::read(outside).unwrap(), b"outside lock bytes");

    let namespace_parent = tempfile::TempDir::new().unwrap();
    let linked_namespace = namespace_parent.path().join(STABLE_SANDBOX_NAMESPACE);
    let outside_namespace = namespace_parent.path().join("outside-namespace");
    fs::create_dir(&outside_namespace).unwrap();
    fs::write(outside_namespace.join("retained"), b"namespace target").unwrap();
    std::os::unix::fs::symlink(&outside_namespace, &linked_namespace).unwrap();
    assert!(
        RealWalkerHost::spawn_stable_in(
            profile(),
            safe_profile("linked-namespace"),
            StableSandboxNamespace::explicit(linked_namespace).unwrap(),
        )
        .is_err()
    );
    assert_eq!(
        fs::read(outside_namespace.join("retained")).unwrap(),
        b"namespace target"
    );

    let lease_parent = tempfile::TempDir::new().unwrap();
    let lease_profile = safe_profile("linked-lease");
    let lease_paths = initialized_sandbox_paths(&lease_parent, &lease_profile);
    let outside_lease = lease_parent.path().join("outside-lease");
    fs::write(&outside_lease, b"lease target").unwrap();
    fs::remove_file(&lease_paths.lease_path).unwrap();
    std::os::unix::fs::symlink(&outside_lease, &lease_paths.lease_path).unwrap();
    assert!(
        StableSandbox::acquire(
            StableSandboxNamespace::explicit(lease_paths.namespace_root.clone()).unwrap(),
            lease_profile,
            SandboxFaults::default(),
        )
        .is_err()
    );
    assert_eq!(fs::read(outside_lease).unwrap(), b"lease target");

    let sandbox_parent = tempfile::TempDir::new().unwrap();
    let sandbox_profile = safe_profile("linked-sandbox");
    let sandbox_paths = initialized_sandbox_paths(&sandbox_parent, &sandbox_profile);
    let outside_sandbox = sandbox_parent.path().join("outside-sandbox");
    fs::create_dir(&outside_sandbox).unwrap();
    fs::write(outside_sandbox.join("retained"), b"sandbox target").unwrap();
    std::os::unix::fs::symlink(&outside_sandbox, &sandbox_paths.sandbox_root).unwrap();
    assert!(
        StableSandbox::acquire(
            StableSandboxNamespace::explicit(sandbox_paths.namespace_root.clone()).unwrap(),
            sandbox_profile,
            SandboxFaults::default(),
        )
        .is_err()
    );
    assert_eq!(
        fs::read(outside_sandbox.join("retained")).unwrap(),
        b"sandbox target"
    );
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
#[test]
fn corrupt_persistent_markers_are_retained_and_refused() {
    let parent = tempfile::TempDir::new().unwrap();
    let namespace_root = parent.path().join(STABLE_SANDBOX_NAMESPACE);
    let namespace = || StableSandboxNamespace::explicit(namespace_root.clone()).unwrap();
    let review_profile = safe_profile("corrupt-markers");
    RealWalkerHost::spawn_stable_in(profile(), review_profile.clone(), namespace())
        .unwrap()
        .close()
        .unwrap();
    let namespace_marker = namespace_root.join(NAMESPACE_MARKER_FILE);
    fs::write(&namespace_marker, b"{}").unwrap();

    assert!(
        RealWalkerHost::spawn_stable_in(profile(), review_profile.clone(), namespace(),).is_err()
    );
    assert_eq!(fs::read(&namespace_marker).unwrap(), b"{}");

    let second_parent = tempfile::TempDir::new().unwrap();
    let second_root = second_parent.path().join(STABLE_SANDBOX_NAMESPACE);
    let second_namespace = || StableSandboxNamespace::explicit(second_root.clone()).unwrap();
    RealWalkerHost::spawn_stable_in(profile(), review_profile.clone(), second_namespace())
        .unwrap()
        .close()
        .unwrap();
    let lease = second_root.join(profile_lease_path(&review_profile));
    fs::write(&lease, b"{}").unwrap();

    assert!(
        RealWalkerHost::spawn_stable_in(profile(), review_profile, second_namespace()).is_err()
    );
    assert_eq!(fs::read(lease).unwrap(), b"{}");
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
#[test]
fn live_metadata_and_lease_revalidation_refuse_replaced_evidence() {
    let parent = tempfile::TempDir::new().unwrap();
    let namespace_root = parent.path().join(STABLE_SANDBOX_NAMESPACE);
    let namespace = || StableSandboxNamespace::explicit(namespace_root.clone()).unwrap();
    let review_profile = safe_profile("live-revalidation");
    let host =
        RealWalkerHost::spawn_stable_in(profile(), review_profile.clone(), namespace()).unwrap();
    let profile_root = namespace_root.join(profile_sandbox_path(&review_profile));
    let lease = namespace_root.join(profile_lease_path(&review_profile));
    let marker = profile_root.join(SANDBOX_MARKER_FILE);
    let stable = host._sandbox.stable().unwrap();
    let lease_file = stable.lease.as_ref().unwrap().file();
    let read_lease = || {
        lease_file
            .read_all(stable.handles().leases(), &stable.paths.lease_name())
            .unwrap()
    };
    let write_lease = |bytes: &[u8]| {
        lease_file
            .publish_bytes(stable.handles().leases(), &stable.paths.lease_name(), bytes)
            .unwrap()
    };
    let expected_lease = read_lease();
    let expected_marker = fs::read(&marker).unwrap();
    write_lease(b"changed while held");

    assert!(matches!(
        host.sandbox_metadata(&review_profile),
        Err(SandboxError::InvalidEvidence { path, .. }) if path == lease
    ));
    assert_eq!(read_lease(), b"changed while held");
    assert!(profile_root.is_dir());
    write_lease(&expected_lease);
    fs::write(&marker, b"changed sandbox marker").unwrap();

    assert!(matches!(
        host.sandbox_metadata(&review_profile),
        Err(SandboxError::InvalidEvidence { path, .. }) if path == marker
    ));
    assert_eq!(fs::read(&marker).unwrap(), b"changed sandbox marker");
    assert!(profile_root.is_dir());
    fs::write(&marker, expected_marker).unwrap();

    let parked_root = namespace_root.join("sandboxes/.parked-live-revalidation");
    #[cfg(windows)]
    {
        assert!(fs::rename(&profile_root, &parked_root).is_err());
        host.sandbox_metadata(&review_profile).unwrap();
        host.close().unwrap();
    }
    #[cfg(target_os = "linux")]
    {
        fs::rename(&profile_root, &parked_root).unwrap();
        let stable = host
            ._sandbox
            .stable()
            .expect("the host must own a stable sandbox");
        create_valid_sandbox_evidence(&profile_root, &stable.paths);

        assert!(matches!(
            host.sandbox_metadata(&review_profile),
            Err(SandboxError::InvalidEvidence { path, .. }) if path == profile_root
        ));
        assert!(profile_root.is_dir());
        assert!(parked_root.is_dir());

        assert!(matches!(
            host.close(),
            Err(SandboxError::InvalidEvidence { path, .. }) if path == profile_root
        ));
        assert!(profile_root.is_dir());
        assert!(parked_root.is_dir());
    }
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
#[test]
fn stable_seed_keeps_owned_children_or_refuses_missing_ones() {
    let parent = tempfile::TempDir::new().unwrap();
    let namespace_root = parent.path().join(STABLE_SANDBOX_NAMESPACE);
    let review_profile = safe_profile("missing-stable-child");
    let sandbox = StableSandbox::acquire(
        StableSandboxNamespace::explicit(namespace_root).unwrap(),
        review_profile,
        SandboxFaults::default(),
    )
    .unwrap();
    let missing = sandbox.path().join("data");
    #[cfg(windows)]
    {
        assert!(fs::remove_dir(&missing).is_err());
        sandbox.close().unwrap();
        assert!(!missing.exists());
    }
    #[cfg(target_os = "linux")]
    {
        fs::remove_dir(&missing).unwrap();

        let result = SeededTuiHost::spawn_in_sandbox(
            profile(),
            SandboxOwner::Stable(Box::new(sandbox)),
            AllocationMaximums::default(),
            &SystemDirectorySeedIo,
        );

        let (sandbox, _error) =
            result.expect_err("stable seeding must refuse a missing retained child");
        assert!(!missing.exists());
        drop(sandbox);
    }
}

#[cfg(target_os = "linux")]
#[test]
fn stable_directory_creation_refuses_a_restrictive_umask_in_a_child_process() {
    use std::os::unix::fs::PermissionsExt as _;

    const CHILD: &str = "SKIT_STABLE_DIRECTORY_UMASK_CHILD";
    const ROOT: &str = "SKIT_STABLE_DIRECTORY_UMASK_ROOT";
    let test = harness_test_name(
        module_path!(),
        "stable_directory_creation_refuses_a_restrictive_umask_in_a_child_process",
    );
    if std::env::var_os(CHILD).is_some() {
        let namespace_root = PathBuf::from(std::env::var_os(ROOT).unwrap());
        let error = StableSandbox::acquire(
            StableSandboxNamespace::explicit(namespace_root.clone()).unwrap(),
            safe_profile("restrictive-umask"),
            SandboxFaults::default(),
        )
        .unwrap_err();
        assert!(matches!(
            error,
            SandboxError::Io { source, .. }
                if source.kind() == io::ErrorKind::PermissionDenied
        ));
        assert_eq!(
            fs::metadata(namespace_root).unwrap().permissions().mode() & 0o7777,
            0
        );
        return;
    }

    let parent = tempfile::TempDir::new().unwrap();
    let namespace_root = parent.path().join(STABLE_SANDBOX_NAMESPACE);
    let coverage_profile = restrictive_umask_child_profile();
    let mut command = std::process::Command::new("sh");
    command
        .arg("-c")
        .arg("umask 0777; exec \"$1\" --exact \"$2\" --nocapture")
        .arg("sh")
        .arg(std::env::current_exe().unwrap())
        .arg(&test)
        .env(CHILD, "1")
        .env(ROOT, &namespace_root);
    if let Some(profile) = coverage_profile.as_ref() {
        command.env("LLVM_PROFILE_FILE", profile);
    }

    let status = command.status().unwrap();

    assert!(status.success());
    assert_eq!(
        fs::metadata(&namespace_root).unwrap().permissions().mode() & 0o7777,
        0
    );
    fs::set_permissions(&namespace_root, fs::Permissions::from_mode(0o700)).unwrap();
    assert!(fs::read_dir(&namespace_root).unwrap().next().is_none());
    assert!(!namespace_root.join(NAMESPACE_MARKER_FILE).exists());
}
