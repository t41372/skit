//! Deterministic file allocator and private mode contracts.

use super::*;

fn recording_allocator(system_temp: &Path) -> RecordingAdapters {
    fs::create_dir_all(system_temp).unwrap();
    RecordingAdapters::new(Rc::new(RefCell::new(Vec::new())), system_temp.to_path_buf())
}

#[test]
fn deterministic_file_allocator_resets_and_isolates_each_purpose_counter() {
    let first_root = tempfile::TempDir::new().unwrap();
    let first = recording_allocator(&first_root.path().join("system-temp"));

    let draft_zero = first
        .temporary_file(
            TemporaryFilePurpose::AuthoredDraft,
            TempLocation::Directory(first_root.path()),
            ".prompt.md",
        )
        .unwrap();
    let injected_zero = first
        .temporary_file(
            TemporaryFilePurpose::InjectedSource,
            TempLocation::Directory(first_root.path()),
            ".ts",
        )
        .unwrap();
    let quarantine_zero = first
        .private_directory(PrivateDirectoryPurpose::DraftQuarantine, first_root.path())
        .unwrap();
    let draft_one = first
        .temporary_file(
            TemporaryFilePurpose::AuthoredDraft,
            TempLocation::Directory(first_root.path()),
            ".py",
        )
        .unwrap();

    assert_eq!(
        draft_zero.path().file_name().unwrap(),
        "skit-new-000000.prompt.md"
    );
    assert_eq!(
        injected_zero.path().file_name().unwrap(),
        ".injected-000000.ts"
    );
    assert_eq!(
        quarantine_zero.file_name().unwrap(),
        ".skit-quarantine-000000"
    );
    assert_eq!(draft_one.path().file_name().unwrap(), "skit-new-000001.py");

    let second_root = tempfile::TempDir::new().unwrap();
    let second = recording_allocator(&second_root.path().join("system-temp"));
    let reset = second
        .temporary_file(
            TemporaryFilePurpose::AuthoredDraft,
            TempLocation::Directory(second_root.path()),
            ".py",
        )
        .unwrap();
    assert_eq!(reset.path().file_name().unwrap(), "skit-new-000000.py");
}

#[test]
fn concurrent_hosts_isolate_the_walker_system_temp_location() {
    let first = RealWalkerHost::spawn(profile()).unwrap();
    let second = RealWalkerHost::spawn(profile()).unwrap();

    let first_file = first
        .adapters
        .temporary_file(
            TemporaryFilePurpose::InjectedSource,
            TempLocation::System,
            ".sh",
        )
        .unwrap();
    let second_file = second
        .adapters
        .temporary_file(
            TemporaryFilePurpose::InjectedSource,
            TempLocation::System,
            ".sh",
        )
        .unwrap();

    assert_eq!(
        first_file.path().file_name().unwrap(),
        ".injected-000000.sh"
    );
    assert_eq!(
        second_file.path().file_name().unwrap(),
        ".injected-000000.sh"
    );
    assert!(
        first_file
            .path()
            .starts_with(first._sandbox.path().join("system-temp"))
    );
    assert!(
        second_file
            .path()
            .starts_with(second._sandbox.path().join("system-temp"))
    );
    assert_ne!(first_file.path().parent(), second_file.path().parent());
}

#[test]
fn injected_wrapper_preserves_both_location_fallback_orders() {
    let adjacent = RealWalkerHost::spawn(profile()).unwrap();
    let blocked_entry = adjacent.roots().state.join("blocked-entry");
    fs::write(&blocked_entry, b"not a directory").unwrap();

    let adjacent_file =
        new_injected_file_with_allocator(&blocked_entry, ".js", true, &adjacent.adapters).unwrap();

    assert!(
        adjacent_file
            .path()
            .starts_with(&adjacent.adapters.system_temp)
    );
    assert!(matches!(
        adjacent.adapters.events.borrow().as_slice(),
        [
            PortEvent::Allocation {
                location: AllocationLocation::Directory(first_path),
                outcome: AllocationOutcome::Rejected(_),
                ..
            },
            PortEvent::Allocation {
                location: AllocationLocation::System(second_path),
                outcome: AllocationOutcome::Accepted,
                ..
            },
        ] if first_path == &blocked_entry
            && second_path == &adjacent.adapters.system_temp
    ));
    drop(adjacent_file);
    assert_system_temp_is_empty(&adjacent);

    let private = RealWalkerHost::spawn(profile()).unwrap();
    fs::remove_dir(&private.adapters.system_temp).unwrap();
    fs::write(&private.adapters.system_temp, b"not a directory").unwrap();
    let entry_dir = private.roots().state.join("entry");
    fs::create_dir(&entry_dir).unwrap();

    let private_file =
        new_injected_file_with_allocator(&entry_dir, ".sh", false, &private.adapters).unwrap();

    assert!(private_file.path().starts_with(&entry_dir));
    assert!(matches!(
        private.adapters.events.borrow().as_slice(),
        [
            PortEvent::Allocation {
                location: AllocationLocation::System(first_path),
                outcome: AllocationOutcome::Rejected(_),
                ..
            },
            PortEvent::Allocation {
                location: AllocationLocation::Directory(second_path),
                outcome: AllocationOutcome::Accepted,
                ..
            },
        ] if first_path == &private.adapters.system_temp && second_path == &entry_dir
    ));
    drop(private_file);
    fs::remove_file(&private.adapters.system_temp).unwrap();
    fs::create_dir(&private.adapters.system_temp).unwrap();
    assert_system_temp_is_empty(&private);
}

#[test]
fn walker_system_temp_residual_is_normalized_and_refused() {
    let mut host = RealWalkerHost::spawn(profile()).unwrap();
    let temporary = host
        .adapters
        .temporary_file(
            TemporaryFilePurpose::InjectedSource,
            TempLocation::System,
            ".sh",
        )
        .unwrap();
    drop(temporary);
    fs::write(
        host.adapters.system_temp.join(".injected-residual.sh"),
        b"residual bytes",
    )
    .unwrap();

    let error = host.observe(&host.initial_state().unwrap()).unwrap_err();

    assert!(error.contains(
            "walker system temp retained an artifact: <profile:fixture>/system-temp/.injected-residual.sh"
        ));
    assert!(!error.contains(&host._sandbox.path().display().to_string()));
    fs::remove_file(host.adapters.system_temp.join(".injected-residual.sh")).unwrap();

    let observation = host.observe(&host.initial_state().unwrap()).unwrap();

    assert_eq!(
        events_with_keys(&observation.transcript, &["allocation"]),
        vec![json!({ "allocation": {
            "purpose": "injected_source",
            "attempt": 0,
            "location": { "kind": "system" },
            "path": "<profile:fixture>/system-temp/<injected-source:0>.sh",
            "outcome": { "accepted": true },
        }})]
    );
}

#[test]
fn walker_system_temp_inspection_errors_are_stable_and_do_not_drain_events() {
    assert_eq!(
        first_system_temp_residual([Err(io::Error::other(
            "injected directory iteration failure",
        ))])
        .unwrap_err(),
        "could not inspect walker system temp: injected directory iteration failure"
    );

    let mut host = RealWalkerHost::spawn(profile()).unwrap();
    let temporary = host
        .adapters
        .temporary_file(
            TemporaryFilePurpose::InjectedSource,
            TempLocation::System,
            ".sh",
        )
        .unwrap();
    drop(temporary);
    fs::remove_dir(&host.adapters.system_temp).unwrap();
    let state = host.initial_state().unwrap();

    let error = host.observe(&state).unwrap_err();

    assert!(error.starts_with("could not inspect walker system temp:"));
    fs::create_dir(&host.adapters.system_temp).unwrap();
    let observation = host.observe(&state).unwrap();
    assert_eq!(
        events_with_keys(&observation.transcript, &["allocation"]).len(),
        1
    );
}

#[test]
fn tree_failure_does_not_drain_pending_allocation_or_launch_events() {
    let mut spec = profile();
    spec.entries.push(injected_script(
        "shell",
        "Tree retry shell",
        "TOKEN=before\nprintf '%s' \"$TOKEN\"\n",
        "script.sh",
    ));
    let mut host = RealWalkerHost::spawn(spec).unwrap();
    host.clear_transcript();
    assert!(matches!(
        host.dispatch(Effect::Submit {
            purpose: FormPurpose::Run,
            selector: Some("Tree retry shell".to_owned()),
            values: BTreeMap::from([("value:TOKEN".to_owned(), FieldValue::text("after"))]),
        })
        .unwrap(),
        Action::Complete { .. }
    ));
    let state = host.initial_state().unwrap();
    let parked = host._sandbox.path().join("external-parked");
    fs::rename(&host.external_root, &parked).unwrap();

    assert!(host.observe(&state).is_err());
    fs::rename(&parked, &host.external_root).unwrap();
    let observation = host.observe(&state).unwrap();
    let recorded = events_with_keys(&observation.transcript, &["allocation", "launch"]);

    assert_eq!(recorded.len(), 2);
    assert!(recorded[0].get("allocation").is_some());
    assert!(recorded[1].get("launch").is_some());
}

#[test]
fn deterministic_file_allocator_retries_collisions_without_clobbering() {
    let root = tempfile::TempDir::new().unwrap();
    let occupied = root.path().join("skit-new-000000.py");
    fs::write(&occupied, b"keep me").unwrap();
    let allocator = recording_allocator(&root.path().join("system-temp"));

    let allocated = allocator
        .temporary_file(
            TemporaryFilePurpose::AuthoredDraft,
            TempLocation::Directory(root.path()),
            ".py",
        )
        .unwrap();

    assert_eq!(fs::read(occupied).unwrap(), b"keep me");
    assert_eq!(allocated.path().file_name().unwrap(), "skit-new-000001.py");
    assert!(matches!(
        allocator.events.borrow().as_slice(),
        [
            PortEvent::Allocation { attempt: 0, outcome, .. },
            PortEvent::Allocation {
                attempt: 1,
                outcome: AllocationOutcome::Accepted,
                ..
            },
        ] if matches!(
            outcome,
            AllocationOutcome::Rejected(error)
                if error.kind == io::ErrorKind::AlreadyExists
        )
    ));
}

#[test]
fn deterministic_allocator_transcript_orders_collision_success_and_overflow() {
    let maximums = AllocationMaximums {
        authored_draft: 1,
        ..AllocationMaximums::default()
    };
    let mut host = RealWalkerHost::spawn_with_allocation_maximums(profile(), maximums).unwrap();
    let drafts = crate::cli::create_owned_drafts_dir(&host.roots().data).unwrap();
    let occupied = drafts.join("skit-new-000000.py");
    fs::write(&occupied, b"occupied").unwrap();

    let allocated = host
        .adapters
        .temporary_file(
            TemporaryFilePurpose::AuthoredDraft,
            TempLocation::Directory(&drafts),
            ".py",
        )
        .unwrap();
    let overflow = host.adapters.temporary_file(
        TemporaryFilePurpose::AuthoredDraft,
        TempLocation::Directory(&drafts),
        ".py",
    );
    let transcript = host.adapters.transcript(&mut host.path_map);

    assert_eq!(fs::read(occupied).unwrap(), b"occupied");
    assert_eq!(allocated.path().file_name().unwrap(), "skit-new-000001.py");
    assert_eq!(
        overflow.unwrap_err().to_string(),
        "authored_draft allocation counter overflow"
    );
    assert_eq!(
        events_with_keys(&transcript, &["allocation"]),
        vec![
            json!({ "allocation": {
                "purpose": "authored_draft",
                "attempt": 0,
                "location": {
                    "kind": "directory",
                    "path": "<profile:fixture>/data/drafts",
                },
                "path": "<profile:fixture>/data/drafts/skit-new-000000.py",
                "outcome": { "rejected": {
                    "kind": "AlreadyExists",
                    "reason": "the allocation candidate already exists",
                }},
            }}),
            json!({ "allocation": {
                "purpose": "authored_draft",
                "attempt": 1,
                "location": {
                    "kind": "directory",
                    "path": "<profile:fixture>/data/drafts",
                },
                "path": "<profile:fixture>/data/.drafts/<draft:0>.py",
                "outcome": { "accepted": true },
            }}),
            json!({ "allocation": {
                "purpose": "authored_draft",
                "attempt": 2,
                "location": {
                    "kind": "directory",
                    "path": "<profile:fixture>/data/drafts",
                },
                "path": null,
                "outcome": { "rejected": {
                    "kind": "Other",
                    "reason": "authored_draft allocation counter overflow",
                }},
            }}),
        ]
    );
}

#[test]
fn private_mode_failures_roll_back_and_record_stable_noncollision_errors() {
    let mut host = RealWalkerHost::spawn(profile()).unwrap();
    let drafts = crate::cli::create_owned_drafts_dir(&host.roots().data).unwrap();
    host.adapters
        .private_mode_failure
        .replace(Some(PrivateModeFailure {
            target: PrivateModeTarget::File,
            kind: io::ErrorKind::PermissionDenied,
            reason: "walker file chmod denied".to_owned(),
        }));

    let file_error = host
        .adapters
        .temporary_file(
            TemporaryFilePurpose::AuthoredDraft,
            TempLocation::Directory(&drafts),
            ".py",
        )
        .unwrap_err();
    host.adapters
        .private_mode_failure
        .replace(Some(PrivateModeFailure {
            target: PrivateModeTarget::Directory,
            kind: io::ErrorKind::PermissionDenied,
            reason: "walker directory chmod denied".to_owned(),
        }));
    let directory_error = host
        .adapters
        .private_directory(PrivateDirectoryPurpose::DraftQuarantine, &drafts)
        .unwrap_err();
    let transcript = host.adapters.transcript(&mut host.path_map);

    assert_eq!(file_error.to_string(), "walker file chmod denied");
    assert_eq!(directory_error.to_string(), "walker directory chmod denied");
    assert!(!drafts.join("skit-new-000000.py").exists());
    assert!(!drafts.join(".skit-quarantine-000000").exists());
    assert_eq!(
        events_with_keys(&transcript, &["allocation"]),
        vec![
            json!({ "allocation": {
                "purpose": "authored_draft",
                "attempt": 0,
                "location": {
                    "kind": "directory",
                    "path": "<profile:fixture>/data/drafts",
                },
                "path": "<profile:fixture>/data/drafts/skit-new-000000.py",
                "outcome": { "rejected": {
                    "kind": "PermissionDenied",
                    "reason": "walker file chmod denied",
                }},
            }}),
            json!({ "allocation": {
                "purpose": "draft_quarantine",
                "attempt": 0,
                "location": {
                    "kind": "directory",
                    "path": "<profile:fixture>/data/drafts",
                },
                "path": "<profile:fixture>/data/drafts/.skit-quarantine-000000",
                "outcome": { "rejected": {
                    "kind": "PermissionDenied",
                    "reason": "walker directory chmod denied",
                }},
            }}),
        ]
    );
}

#[cfg(unix)]
#[test]
fn deterministic_file_allocator_uses_private_file_and_directory_modes() {
    use std::os::unix::fs::PermissionsExt as _;

    let root = tempfile::TempDir::new().unwrap();
    let allocator = recording_allocator(&root.path().join("system-temp"));
    let file = allocator
        .temporary_file(
            TemporaryFilePurpose::InjectedSource,
            TempLocation::Directory(root.path()),
            ".js",
        )
        .unwrap();
    let directory = allocator
        .private_directory(PrivateDirectoryPurpose::DraftQuarantine, root.path())
        .unwrap();

    assert_eq!(
        fs::metadata(file.path()).unwrap().permissions().mode() & 0o777,
        0o600
    );
    assert_eq!(
        fs::metadata(directory).unwrap().permissions().mode() & 0o777,
        0o700
    );
}

#[test]
fn pending_events_register_every_private_mode_kind_and_the_managed_executable() {
    let host = RealWalkerHost::spawn(profile()).unwrap();
    let mut modes = ObservationModeProvenance::default();
    let authored = host.roots().state.join("authored");
    let injected = host.roots().state.join("injected");
    let quarantine = host.roots().state.join("quarantine");
    let rejected = host.roots().state.join("rejected");
    for (purpose, path) in [
        (
            AllocationPurpose::TemporaryFile(TemporaryFilePurpose::AuthoredDraft),
            authored.clone(),
        ),
        (
            AllocationPurpose::TemporaryFile(TemporaryFilePurpose::InjectedSource),
            injected.clone(),
        ),
        (
            AllocationPurpose::PrivateDirectory(PrivateDirectoryPurpose::DraftQuarantine),
            quarantine.clone(),
        ),
    ] {
        modes
            .register_event(
                &PortEvent::Allocation {
                    purpose,
                    attempt: 0,
                    location: AllocationLocation::Directory(host.roots().state.clone()),
                    path: Some(path),
                    outcome: AllocationOutcome::Accepted,
                },
                host.roots(),
            )
            .unwrap();
    }
    modes
        .register_event(
            &PortEvent::Allocation {
                purpose: AllocationPurpose::TemporaryFile(TemporaryFilePurpose::InjectedSource),
                attempt: 1,
                location: AllocationLocation::Directory(host.roots().state.clone()),
                path: Some(rejected.clone()),
                outcome: AllocationOutcome::Rejected(AllocationFailure {
                    kind: io::ErrorKind::AlreadyExists,
                    reason: "occupied".to_owned(),
                }),
            },
            host.roots(),
        )
        .unwrap();
    let managed = skit_runtime::managed_uv_path(&host.roots().data);
    fs::create_dir_all(managed.parent().unwrap()).unwrap();
    modes
        .register_event(
            &PortEvent::Probe {
                operation: "is_file".to_owned(),
                path: managed.clone(),
                result: ProbeResult::Bool(false),
            },
            host.roots(),
        )
        .unwrap();
    assert_eq!(modes.expected_kind(&managed).unwrap(), None);
    modes
        .register_event(
            &PortEvent::Probe {
                operation: "find_program".to_owned(),
                path: PathBuf::from("uv"),
                result: ProbeResult::Path(Some(managed.clone())),
            },
            host.roots(),
        )
        .unwrap();
    assert_eq!(
        modes.expected_kind(&managed).unwrap(),
        Some(ObservationNodeKind::File)
    );
    modes
        .register_event(
            &PortEvent::Probe {
                operation: "is_file".to_owned(),
                path: managed.clone(),
                result: ProbeResult::Bool(true),
            },
            host.roots(),
        )
        .unwrap();

    for path in [&authored, &injected, &managed] {
        assert_eq!(
            modes.expected_kind(path).unwrap(),
            Some(ObservationNodeKind::File)
        );
    }
    assert_eq!(
        modes.expected_kind(&quarantine).unwrap(),
        Some(ObservationNodeKind::Directory)
    );
    assert_eq!(modes.expected_kind(&rejected).unwrap(), None);
}

#[test]
fn live_private_allocator_nodes_keep_modes_while_ordinary_siblings_use_none() {
    let mut host = RealWalkerHost::spawn(profile()).unwrap();
    let private_file = host
        .adapters
        .temporary_file(
            TemporaryFilePurpose::InjectedSource,
            TempLocation::Directory(&host.roots().state),
            ".txt",
        )
        .unwrap();
    fs::write(private_file.path(), b"private allocator file\n").unwrap();
    let private_directory = host
        .adapters
        .private_directory(
            PrivateDirectoryPurpose::DraftQuarantine,
            &host.roots().state,
        )
        .unwrap();
    let ordinary_file = host.roots().state.join("ordinary-file");
    let ordinary_directory = host.roots().state.join("ordinary-directory");
    fs::write(&ordinary_file, b"ordinary\n").unwrap();
    fs::create_dir(&ordinary_directory).unwrap();

    let observation = host.observe(&host.initial_state().unwrap()).unwrap();
    let private_directory_path = host.path_map.normalize_path(&private_directory);
    let private_directory_path = private_directory_path
        .strip_prefix(&format!("{}/", host.path_map.profile_label))
        .unwrap()
        .to_owned();
    let private_file = observation
        .tree
        .iter()
        .find(|row| row.content == Some(ByteView::Utf8("private allocator file\n".to_owned())))
        .unwrap();
    let private_directory = observation
        .tree
        .iter()
        .find(|row| row.path == private_directory_path)
        .unwrap();
    let ordinary_file = observation
        .tree
        .iter()
        .find(|row| row.path == "state/ordinary-file")
        .unwrap();
    let ordinary_directory = observation
        .tree
        .iter()
        .find(|row| row.path == "state/ordinary-directory")
        .unwrap();

    #[cfg(unix)]
    {
        assert_eq!(private_file.mode, Some(0o600));
        assert_eq!(private_directory.mode, Some(0o700));
    }
    #[cfg(not(unix))]
    {
        assert_eq!(private_file.mode, None);
        assert_eq!(private_directory.mode, None);
    }
    assert_eq!(ordinary_file.mode, None);
    assert_eq!(ordinary_directory.mode, None);
}

#[cfg(unix)]
#[test]
fn walker_private_modes_ignore_restrictive_umask_in_a_child_process() {
    use std::os::unix::fs::PermissionsExt as _;

    const CHILD: &str = "SKIT_WALKER_ALLOCATOR_UMASK_CHILD";
    let test = harness_test_name(
        module_path!(),
        "walker_private_modes_ignore_restrictive_umask_in_a_child_process",
    );
    if std::env::var_os(CHILD).is_some() {
        let root = tempfile::TempDir::new().unwrap();
        fs::set_permissions(root.path(), fs::Permissions::from_mode(0o700)).unwrap();
        let allocator = recording_allocator(root.path());
        let file = allocator
            .temporary_file(
                TemporaryFilePurpose::InjectedSource,
                TempLocation::Directory(root.path()),
                ".js",
            )
            .unwrap();
        let directory = allocator
            .private_directory(PrivateDirectoryPurpose::DraftQuarantine, root.path())
            .unwrap();
        let system_file = SYSTEM_FILE_ALLOCATOR
            .temporary_file(
                TemporaryFilePurpose::InjectedSource,
                TempLocation::Directory(root.path()),
                ".js",
            )
            .unwrap();
        let system_directory = SYSTEM_FILE_ALLOCATOR
            .private_directory(PrivateDirectoryPurpose::DraftQuarantine, root.path())
            .unwrap();
        assert_eq!(
            fs::metadata(file.path()).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(
            fs::metadata(directory).unwrap().permissions().mode() & 0o777,
            0o700
        );
        assert_eq!(
            fs::metadata(system_file.path())
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
        assert_eq!(
            fs::metadata(system_directory).unwrap().permissions().mode() & 0o777,
            0o700
        );
        return;
    }

    let coverage_profile = restrictive_umask_child_profile();
    let mut command = std::process::Command::new("sh");
    command
        .arg("-c")
        .arg("umask 0777; exec \"$1\" --exact \"$2\" --nocapture")
        .arg("sh")
        .arg(std::env::current_exe().unwrap())
        .arg(&test)
        .env(CHILD, "1");
    if let Some(profile) = coverage_profile.as_ref() {
        command.env("LLVM_PROFILE_FILE", profile);
    }
    let status = command.status().unwrap();

    assert!(status.success());
}
