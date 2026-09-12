//! Directory seed and file picker tree contracts.

use super::*;

#[test]
fn g2_profile_directory_seed_duplicates_and_overlaps_are_idempotent_private_and_discoverable() {
    let mut host = RealWalkerHost::spawn(WalkerSeedSpec {
        profile: "g2-directories".to_owned(),
        directories: vec![
            directory_seed(WalkerDirectoryRoot::Home, ".codex"),
            directory_seed(WalkerDirectoryRoot::Home, ".codex/skills"),
            directory_seed(WalkerDirectoryRoot::Home, ".codex/skills"),
            directory_seed(WalkerDirectoryRoot::Cwd, ".claude/skills"),
        ],
        ..WalkerSeedSpec::default()
    })
    .unwrap();

    let home = host.roots().home.as_ref().unwrap().join(".codex");
    let cwd = host.roots().cwd.join(".claude");
    assert!(home.join("skills").is_dir());
    assert!(cwd.join("skills").is_dir());
    let action = host
        .dispatch(Effect::Preferences(
            PreferencesEffect::DiscoverAgentSkillTargets,
        ))
        .unwrap();
    assert!(matches!(
        &action,
        Action::Preferences(PreferencesAction::PresentAgentSkillTargets(_))
    ));
    let action = serde_json::to_value(action).unwrap();
    let targets = action["preferences"]["present_agent_skill_targets"]
        .as_array()
        .unwrap();
    assert!(targets.iter().any(|target| {
        target["name"] == "codex" && target["scope"] == "user" && target["base"] == json!(home)
    }));
    assert!(targets.iter().any(|target| {
        target["name"] == "claude" && target["scope"] == "project" && target["base"] == json!(cwd)
    }));

    let observation = host.observe(&host.initial_state().unwrap()).unwrap();
    for relative in [
        "home/.codex",
        "home/.codex/skills",
        "cwd/.claude",
        "cwd/.claude/skills",
    ] {
        let row = observation
            .tree
            .iter()
            .find(|row| row.path == relative)
            .unwrap();
        assert_eq!(row.kind, "directory");
        #[cfg(unix)]
        assert_eq!(row.mode, Some(0o700));
        #[cfg(not(unix))]
        assert_eq!(row.mode, None);
    }
}

fn g2_directory_seed_fault_cases() -> Vec<(G2DirectorySeedFault, &'static str)> {
    let mut cases = vec![
        (G2DirectorySeedFault::FileConflict, "is not a directory"),
        (G2DirectorySeedFault::Create, "could not create"),
        (G2DirectorySeedFault::Inspect, "could not inspect"),
        (
            G2DirectorySeedFault::PrivateMode,
            "could not set private mode",
        ),
    ];
    #[cfg(unix)]
    cases.insert(
        1,
        (G2DirectorySeedFault::SymlinkConflict, "is not a directory"),
    );
    cases
}

#[test]
fn g2_random_directory_seed_io_failures_roll_back_without_touching_outside() {
    for (fault, message) in g2_directory_seed_fault_cases() {
        let outside = tempfile::TempDir::new().unwrap();
        let sentinel = outside.path().join("sentinel");
        fs::write(&sentinel, b"outside stays").unwrap();
        let io = G2DirectorySeedIo::new(fault, sentinel.clone());
        let error = RealWalkerHost::spawn_with_directory_seed_io(
            WalkerSeedSpec {
                directories: vec![directory_seed(WalkerDirectoryRoot::Home, ".codex")],
                ..WalkerSeedSpec::default()
            },
            &io,
        )
        .unwrap_err();

        assert!(error.contains(message), "{fault:?}: {error}");
        let attempted = io.attempts.borrow();
        let sandbox = attempted[0].parent().unwrap().parent().unwrap();
        assert!(!sandbox.exists(), "{fault:?}: {}", sandbox.display());
        assert_eq!(fs::read(&sentinel).unwrap(), b"outside stays");
    }
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
#[test]
fn g2_stable_directory_seed_io_failures_remove_profile_and_release_lease() {
    for (index, (fault, message)) in g2_directory_seed_fault_cases().into_iter().enumerate() {
        let parent = tempfile::TempDir::new().unwrap();
        let namespace_root = parent.path().join(STABLE_SANDBOX_NAMESPACE);
        let namespace = StableSandboxNamespace::explicit(namespace_root.clone()).unwrap();
        let profile = safe_profile(&format!("g2-seed-fault-{index}"));
        let outside = parent.path().join("outside-sentinel");
        fs::write(&outside, b"outside stays").unwrap();
        let io = G2DirectorySeedIo::new(fault, outside.clone());
        let error = RealWalkerHost::spawn_stable_in_with_directory_seed_io(
            WalkerSeedSpec {
                directories: vec![directory_seed(WalkerDirectoryRoot::Cwd, ".claude")],
                ..WalkerSeedSpec::default()
            },
            profile.clone(),
            namespace,
            &io,
        )
        .unwrap_err();

        assert!(error.to_string().contains(message), "{fault:?}: {error}");
        assert!(matches!(
            error,
            StableHostError::Primary {
                primary: StableHostPrimaryError::Seed(_),
                cleanup: None,
            }
        ));
        assert!(!namespace_root.join(profile_sandbox_path(&profile)).exists());
        assert!(!namespace_root.join(profile_cleanup_path(&profile)).exists());
        assert_eq!(fs::read(&outside).unwrap(), b"outside stays");
        RealWalkerHost::spawn_stable_in(
            WalkerSeedSpec::default(),
            profile,
            StableSandboxNamespace::explicit(namespace_root).unwrap(),
        )
        .unwrap()
        .close()
        .unwrap();
    }
}

#[test]
fn g2_directory_seed_shapes_refuse_before_any_seed_write() {
    let paths = vec![
        PathBuf::new(),
        PathBuf::from("."),
        PathBuf::from("./child"),
        PathBuf::from("nested/./child"),
        PathBuf::from("../escape"),
        PathBuf::from("nested/../escape"),
        PathBuf::from(std::path::MAIN_SEPARATOR.to_string()),
        std::env::temp_dir().join("absolute-seed"),
    ];
    #[cfg(windows)]
    let paths = {
        let mut paths = paths;
        paths.push(PathBuf::from(r"C:prefix"));
        paths
    };

    for (index, path) in paths.into_iter().enumerate() {
        let spec = WalkerSeedSpec {
            directories: vec![WalkerDirectorySeed {
                root: WalkerDirectoryRoot::Home,
                path: path.clone(),
            }],
            external: vec![WalkerExternalSeed::File {
                path: PathBuf::from("would-write"),
                bytes: b"must not exist".to_vec(),
                readonly: false,
                unix_mode: 0o600,
            }],
            ..WalkerSeedSpec::default()
        };
        let error = validate_external_spec(&spec).unwrap_err();
        assert!(error.contains("relative descendants"), "{path:?}: {error}");

        #[cfg(any(target_os = "linux", target_os = "windows"))]
        {
            let parent = tempfile::TempDir::new().unwrap();
            let namespace_root = parent.path().join(STABLE_SANDBOX_NAMESPACE);
            let result = RealWalkerHost::spawn_stable_in(
                spec,
                safe_profile(&format!("invalid-directory-{index}")),
                StableSandboxNamespace::explicit(namespace_root.clone()).unwrap(),
            );
            assert!(matches!(
                result,
                Err(StableHostError::Primary {
                    primary: StableHostPrimaryError::Seed(_),
                    cleanup: None,
                })
            ));
            assert!(!namespace_root.exists(), "{path:?}");
        }
        #[cfg(not(any(target_os = "linux", target_os = "windows")))]
        let _ = (index, spec);
    }
}

#[test]
fn g2_seeded_directory_file_replacement_refuses_before_first_observe() {
    let mut host = RealWalkerHost::spawn(WalkerSeedSpec {
        directories: vec![directory_seed(WalkerDirectoryRoot::Home, ".codex")],
        ..WalkerSeedSpec::default()
    })
    .unwrap();
    let state = host.initial_state().unwrap();
    let path = host.roots().home.as_ref().unwrap().join(".codex");
    fs::remove_dir(&path).unwrap();
    fs::write(&path, b"replacement").unwrap();

    let error = host.observe(&state).unwrap_err();
    assert!(error.contains("expected directory"), "{error}");
    assert!(error.contains("found file"), "{error}");
}

#[cfg(unix)]
#[test]
fn g2_seeded_directory_symlink_replacement_refuses_before_first_observe() {
    let mut host = RealWalkerHost::spawn(WalkerSeedSpec {
        directories: vec![directory_seed(WalkerDirectoryRoot::Cwd, ".claude")],
        ..WalkerSeedSpec::default()
    })
    .unwrap();
    let state = host.initial_state().unwrap();
    let path = host.roots().cwd.join(".claude");
    fs::remove_dir(&path).unwrap();
    std::os::unix::fs::symlink(host.roots().home.as_ref().unwrap(), &path).unwrap();

    let error = host.observe(&state).unwrap_err();
    assert!(error.contains("expected directory"), "{error}");
    assert!(error.contains("found symlink"), "{error}");
}

// The stable sandbox does not support macOS, so this test skips there. Keep it compiled on
// macOS. A cfg gate would make the complete stable owner dead code.
#[cfg_attr(
    target_os = "macos",
    ignore = "the stable sandbox does not support macOS"
)]
#[test]
fn g2_editor_write_queue_drives_real_add_edit_fifo_and_stable_replay() {
    let spec = WalkerSeedSpec {
        profile: "g2-editor-queue".to_owned(),
        editor_writes: vec![b"first\n".to_vec(), b"second\n".to_vec()],
        ..WalkerSeedSpec::default()
    };
    let parent = tempfile::TempDir::new().unwrap();
    let namespace =
        StableSandboxNamespace::explicit(parent.path().join(STABLE_SANDBOX_NAMESPACE)).unwrap();
    let profile = safe_profile("g2-editor-queue");
    let run = |spec, namespace| {
        let mut host = RealWalkerHost::spawn_stable_in(spec, profile.clone(), namespace).unwrap();
        let mut state = host.initial_state().unwrap();
        let opened = host
            .dispatch(Effect::Open {
                request: HostRequest::Add,
                selector: None,
            })
            .unwrap();
        assert_eq!(state.update(opened), Effect::None);
        host.clear_transcript();

        let author = state.update(Action::Add(AddAction::NewDraft(DraftKind::Script)));
        let authored = host.dispatch(author).unwrap();
        let source = expect_authored_draft_source(authored.clone());
        assert_eq!(fs::read(&source.path).unwrap(), b"first\n");
        assert_eq!(source.bytes, b"first\n");
        assert_eq!(state.update(authored), Effect::None);

        let mut bytes = vec![b"first\n".to_vec()];
        for expected in [b"second\n".as_slice(), b"second\n".as_slice()] {
            let edit = state.update(Action::Add(AddAction::EditSource));
            let edited = host.dispatch(edit).unwrap();
            assert!(matches!(
                &edited,
                Action::Add(AddAction::SourceEdited { result: Ok(_), .. })
            ));
            let edited_value = serde_json::to_value(&edited).unwrap();
            let edited_source = &edited_value["add"]["source_edited"]["result"]["Ok"];
            assert_eq!(edited_source["path"], json!(source.path));
            assert_eq!(edited_source["bytes"], json!(expected));
            assert_eq!(fs::read(&source.path).unwrap(), expected);
            bytes.push(expected.to_vec());
            assert_eq!(state.update(edited), Effect::None);
        }

        let raw_editor_events = host
            .adapters
            .pending_events()
            .into_iter()
            .filter_map(|event| match event {
                PortEvent::Editor {
                    argv,
                    path,
                    outcome,
                } => Some((argv, path, outcome)),
                _ => None,
            })
            .collect::<Vec<_>>();
        let observation = host.observe(&state).unwrap();
        let projected_editor_events = events_with_keys(&observation.transcript, &["editor"]);
        host.close().unwrap();
        (
            raw_editor_events,
            projected_editor_events,
            bytes,
            observation,
        )
    };

    let first = run(spec.clone(), namespace.clone());
    let replay = run(spec, namespace);
    assert_eq!(first, replay);
    assert_eq!(first.0.len(), 3);
    assert_eq!(first.1.len(), 3);
    assert_eq!(
        first.2,
        vec![
            b"first\n".to_vec(),
            b"second\n".to_vec(),
            b"second\n".to_vec(),
        ]
    );
    assert_eq!(
        first
            .3
            .state
            .pointer("/workflow/active/add/review/source/bytes"),
        Some(&json!(b"second\n"))
    );
    let draft = first
        .3
        .tree
        .iter()
        .find(|row| row.path == "data/.drafts/<draft:0>.py")
        .unwrap();
    assert_eq!(draft.content, Some(ByteView::Utf8("second\n".to_owned())));
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
#[test]
fn g2_stable_directory_seed_replay_has_equal_paths_modes_and_observations() {
    let spec = WalkerSeedSpec {
        profile: "g2-stable".to_owned(),
        directories: vec![
            directory_seed(WalkerDirectoryRoot::Home, ".codex/skills"),
            directory_seed(WalkerDirectoryRoot::Cwd, ".claude/skills"),
        ],
        ..WalkerSeedSpec::default()
    };
    let parent = tempfile::TempDir::new().unwrap();
    let namespace =
        StableSandboxNamespace::explicit(parent.path().join(STABLE_SANDBOX_NAMESPACE)).unwrap();
    let review_profile = safe_profile("g2-stable");
    let capture = |spec, namespace| {
        let mut host =
            RealWalkerHost::spawn_stable_in(spec, review_profile.clone(), namespace).unwrap();
        let seeded = (
            host.roots().home.as_ref().unwrap().join(".codex/skills"),
            host.roots().cwd.join(".claude/skills"),
        );
        let observation = host.observe(&host.initial_state().unwrap()).unwrap();
        host.close().unwrap();
        (seeded, observation)
    };

    let first = capture(spec.clone(), namespace.clone());
    let second = capture(spec, namespace);
    assert_eq!(first, second);
    for relative in [
        "home/.codex",
        "home/.codex/skills",
        "cwd/.claude",
        "cwd/.claude/skills",
    ] {
        let row = first
            .1
            .tree
            .iter()
            .find(|row| row.path == relative)
            .unwrap();
        assert_eq!(row.kind, "directory");
        #[cfg(unix)]
        assert_eq!(row.mode, Some(0o700));
        #[cfg(not(unix))]
        assert_eq!(row.mode, None);
    }
}

#[test]
fn g2c_file_picker_tree_is_exact_idempotent_and_external_only() {
    validate_external_spec(&profile()).unwrap();
    let spec = WalkerSeedSpec {
        profile: "g2c-picker-tree".to_owned(),
        external: vec![
            WalkerExternalSeed::File {
                path: PathBuf::from("nested/deep/alpha.txt"),
                bytes: b"alpha\n".to_vec(),
                readonly: false,
                unix_mode: 0o640,
            },
            WalkerExternalSeed::File {
                path: PathBuf::from("nested/beta.txt"),
                bytes: b"beta\n".to_vec(),
                readonly: false,
                unix_mode: 0o640,
            },
            WalkerExternalSeed::File {
                path: PathBuf::from("nested/deep/alpha.txt"),
                bytes: b"alpha\n".to_vec(),
                readonly: false,
                unix_mode: 0o640,
            },
        ],
        directories: vec![
            directory_seed(WalkerDirectoryRoot::Home, ".codex/skills"),
            directory_seed(WalkerDirectoryRoot::Cwd, ".claude/skills"),
        ],
        ..WalkerSeedSpec::default()
    };
    let host = RealWalkerHost::spawn(spec).unwrap();
    let root = host.external_root.clone();

    let tree = host.file_picker_tree();

    assert_eq!(tree.root, root);
    assert_eq!(
        tree.directories,
        BTreeSet::from([root.clone(), root.join("nested"), root.join("nested/deep"),])
    );
    assert_eq!(
        tree.files,
        BTreeSet::from([
            root.join("nested/beta.txt"),
            root.join("nested/deep/alpha.txt"),
        ])
    );
    assert!(
        tree.directories
            .iter()
            .chain(&tree.files)
            .all(|path| path.starts_with(&root))
    );
    assert!(
        !tree
            .directories
            .contains(&host.roots().home.as_ref().unwrap().join(".codex/skills"))
    );
    assert!(
        !tree
            .directories
            .contains(&host.roots().cwd.join(".claude/skills"))
    );
}

#[cfg(unix)]
#[test]
fn g2c_file_picker_tree_keeps_symlink_parents_but_omits_the_leaf() {
    let host = RealWalkerHost::spawn(WalkerSeedSpec {
        profile: "g2c-picker-symlink".to_owned(),
        external: vec![
            WalkerExternalSeed::File {
                path: PathBuf::from("targets/real.txt"),
                bytes: b"target\n".to_vec(),
                readonly: false,
                unix_mode: 0o640,
            },
            WalkerExternalSeed::Symlink {
                path: PathBuf::from("links/deep/alias.txt"),
                target: PathBuf::from("targets/real.txt"),
            },
        ],
        ..WalkerSeedSpec::default()
    })
    .unwrap();
    let root = host.external_root.clone();

    let tree = host.file_picker_tree();

    assert_eq!(
        tree.directories,
        BTreeSet::from([
            root.clone(),
            root.join("links"),
            root.join("links/deep"),
            root.join("targets"),
        ])
    );
    assert_eq!(tree.files, BTreeSet::from([root.join("targets/real.txt")]));
    let symlink = root.join("links/deep/alias.txt");
    assert!(
        fs::symlink_metadata(&symlink)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert!(!tree.directories.contains(&symlink));
    assert!(!tree.files.contains(&symlink));
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
#[test]
fn g2c_file_picker_tree_is_equal_across_fresh_stable_hosts() {
    let spec = WalkerSeedSpec {
        profile: "g2c-picker-replay".to_owned(),
        external: vec![WalkerExternalSeed::File {
            path: PathBuf::from("nested/replay.txt"),
            bytes: b"replay\n".to_vec(),
            readonly: false,
            unix_mode: 0o640,
        }],
        ..WalkerSeedSpec::default()
    };
    let parent = tempfile::TempDir::new().unwrap();
    let namespace =
        StableSandboxNamespace::explicit(parent.path().join(STABLE_SANDBOX_NAMESPACE)).unwrap();
    let review_profile = safe_profile("g2c-picker-replay");
    let capture = |spec, namespace| {
        let host =
            RealWalkerHost::spawn_stable_in(spec, review_profile.clone(), namespace).unwrap();
        let tree = host.file_picker_tree();
        host.close().unwrap();
        tree
    };

    let first = capture(spec.clone(), namespace.clone());
    let replay = capture(spec, namespace);

    assert_eq!(first, replay);
}

#[test]
fn g2c_file_picker_tree_drives_the_real_add_picker_with_relative_identity() {
    let host = RealWalkerHost::spawn(WalkerSeedSpec {
        profile: "g2c-picker-session".to_owned(),
        external: vec![WalkerExternalSeed::File {
            path: PathBuf::from("visible.sh"),
            bytes: b"printf visible\n".to_vec(),
            readonly: false,
            unix_mode: 0o640,
        }],
        ..WalkerSeedSpec::default()
    })
    .unwrap();
    let tree = host.file_picker_tree();
    let mut state = host.initial_state().unwrap();
    assert_eq!(
        state.update(Action::Present(skit_ui::Screen::Add(Box::new(
            AddWorkflowState::new(Vec::new()),
        )))),
        Effect::None
    );
    let mut session = TuiSession::with_file_picker_tree(
        tree.root.clone(),
        tree.directories.clone(),
        tree.files.clone(),
    );
    let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
    let mut geometry = ViewGeometry::default();
    terminal
        .draw(|frame| {
            geometry = render_with_session(frame, &state, Locale::En, &mut session);
        })
        .unwrap();
    let binding = session
        .local_action_inventory()
        .actions
        .iter()
        .find(|action| action.target == LocalActionTarget::Add(AddControlId::BrowseSource))
        .unwrap()
        .keys[0];
    assert_eq!(
        session.handle_event(Event::Key(binding.event()), &state, &geometry),
        EventHandling::Consumed
    );

    let snapshot = serde_json::to_value(session.agent_review_snapshot().unwrap()).unwrap();
    let entry = snapshot
        .pointer("/add_overlay/fields/session/fields/explorer/entries")
        .and_then(Value::as_array)
        .unwrap()
        .iter()
        .find(|entry| entry["name"] == "visible.sh")
        .unwrap();
    let absolute = PathBuf::from(entry["path"].as_str().unwrap());
    assert_eq!(absolute, tree.root.join("visible.sh"));
    assert_eq!(
        absolute.strip_prefix(&tree.root).unwrap(),
        Path::new("visible.sh")
    );
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
#[test]
fn g2c_file_symlink_overlaps_refuse_before_acquisition_in_every_order() {
    let file = |path: &str| WalkerExternalSeed::File {
        path: PathBuf::from(path),
        bytes: b"file\n".to_vec(),
        readonly: false,
        unix_mode: 0o640,
    };
    let symlink = |path: &str| WalkerExternalSeed::Symlink {
        path: PathBuf::from(path),
        target: PathBuf::from("target"),
    };
    let cases = [
        (
            vec![symlink("collision"), file("collision")],
            "walker external fixture leaf paths overlap: collision and collision",
        ),
        (
            vec![file("collision"), symlink("collision")],
            "walker external fixture leaf paths overlap: collision and collision",
        ),
        (
            vec![symlink("alias"), file("alias/child.txt")],
            "walker external fixture leaf paths overlap: alias and alias/child.txt",
        ),
        (
            vec![file("alias/child.txt"), symlink("alias")],
            "walker external fixture leaf paths overlap: alias and alias/child.txt",
        ),
        (
            vec![file("leaf"), symlink("leaf/descendant")],
            "walker external fixture leaf paths overlap: leaf and leaf/descendant",
        ),
    ];

    for (index, (external, expected)) in cases.into_iter().enumerate() {
        let parent = tempfile::TempDir::new().unwrap();
        let sentinel = parent.path().join("outside-sentinel");
        fs::write(&sentinel, b"outside stays").unwrap();
        let namespace_root = parent.path().join(STABLE_SANDBOX_NAMESPACE);
        let review_profile = safe_profile(&format!("g2c-picker-overlap-{index}"));
        let spec = WalkerSeedSpec {
            external,
            ..WalkerSeedSpec::default()
        };

        let validation = validate_external_spec(&spec).unwrap_err();
        assert_eq!(validation, expected);
        let error = RealWalkerHost::spawn_stable_in(
            spec,
            review_profile,
            StableSandboxNamespace::explicit(namespace_root.clone()).unwrap(),
        )
        .unwrap_err();

        assert!(matches!(
            error,
            StableHostError::Primary {
                primary: StableHostPrimaryError::Seed(_),
                cleanup: None,
            }
        ));
        assert!(!namespace_root.exists());
        assert_eq!(fs::read(&sentinel).unwrap(), b"outside stays");
    }
}

#[cfg(unix)]
#[test]
fn g2c_identical_external_seed_duplicates_are_written_once() {
    use std::os::unix::fs::PermissionsExt as _;

    let readonly = WalkerExternalSeed::File {
        path: PathBuf::from("nested/target.txt"),
        bytes: b"target\n".to_vec(),
        readonly: true,
        unix_mode: 0o440,
    };
    let symlink = WalkerExternalSeed::Symlink {
        path: PathBuf::from("links/alias.txt"),
        target: PathBuf::from("nested/target.txt"),
    };
    let host = RealWalkerHost::spawn(WalkerSeedSpec {
        external: vec![readonly.clone(), readonly, symlink.clone(), symlink],
        ..WalkerSeedSpec::default()
    })
    .unwrap();
    let root = host.external_root.clone();

    assert_eq!(
        fs::read(root.join("nested/target.txt")).unwrap(),
        b"target\n"
    );
    assert_eq!(
        fs::metadata(root.join("nested/target.txt"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o440
    );
    assert!(
        fs::symlink_metadata(root.join("links/alias.txt"))
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert_eq!(
        host.file_picker_tree().files,
        BTreeSet::from([root.join("nested/target.txt")])
    );
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
#[test]
fn g2c_same_kind_seed_payload_drift_refuses_before_acquisition() {
    let file = |bytes: &[u8], readonly, unix_mode| WalkerExternalSeed::File {
        path: PathBuf::from("collision"),
        bytes: bytes.to_vec(),
        readonly,
        unix_mode,
    };
    let symlink = |target: &str| WalkerExternalSeed::Symlink {
        path: PathBuf::from("collision"),
        target: PathBuf::from(target),
    };
    let cases = [
        vec![
            file(b"first\n", false, 0o640),
            file(b"second\n", false, 0o640),
        ],
        vec![file(b"same\n", false, 0o640), file(b"same\n", true, 0o640)],
        vec![file(b"same\n", false, 0o640), file(b"same\n", false, 0o600)],
        vec![symlink("first-target"), symlink("second-target")],
    ];

    for (index, external) in cases.into_iter().enumerate() {
        let parent = tempfile::TempDir::new().unwrap();
        let sentinel = parent.path().join("outside-sentinel");
        fs::write(&sentinel, b"outside stays").unwrap();
        let namespace_root = parent.path().join(STABLE_SANDBOX_NAMESPACE);
        let spec = WalkerSeedSpec {
            external,
            ..WalkerSeedSpec::default()
        };

        assert_eq!(
            validate_external_spec(&spec).unwrap_err(),
            "walker external fixture seed payloads conflict at collision"
        );
        let error = RealWalkerHost::spawn_stable_in(
            spec,
            safe_profile(&format!("g2c-payload-conflict-{index}")),
            StableSandboxNamespace::explicit(namespace_root.clone()).unwrap(),
        )
        .unwrap_err();

        assert!(matches!(
            error,
            StableHostError::Primary {
                primary: StableHostPrimaryError::Seed(_),
                cleanup: None,
            }
        ));
        assert!(!namespace_root.exists());
        assert_eq!(fs::read(&sentinel).unwrap(), b"outside stays");
    }
}
