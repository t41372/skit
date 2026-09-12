//! Observation contracts over the production stores.

use super::*;

#[test]
fn profiles_are_isolated_and_one_host_cannot_mutate_another() {
    let first = RealWalkerHost::spawn(profile()).unwrap();
    let mut second = RealWalkerHost::spawn(profile()).unwrap();
    let second_before = second.observe(&second.initial_state().unwrap()).unwrap();

    fs::write(
        first.external_root.join("outside.sh"),
        b"changed only in first\n",
    )
    .unwrap();

    first
        .dispatch(Effect::Remove {
            selector: "Command".to_owned(),
        })
        .unwrap();

    assert!(
        first.service.list().unwrap().entries.len() < second.service.list().unwrap().entries.len()
    );
    assert_eq!(
        second.observe(&second.initial_state().unwrap()).unwrap(),
        second_before
    );
    assert_eq!(
        fs::read(second.external_root.join("outside.sh")).unwrap(),
        b"printf outside\n"
    );
}

#[test]
fn external_seed_paths_refuse_every_lexical_escape_before_writing() {
    let outside = tempfile::TempDir::new().unwrap();
    let sentinel = outside.path().join("sentinel");
    fs::write(&sentinel, b"outside stays exact\n").unwrap();
    let cases = [
        WalkerSeedSpec {
            external: vec![WalkerExternalSeed::File {
                path: sentinel.clone(),
                bytes: b"clobber\n".to_vec(),
                readonly: false,
                unix_mode: 0o640,
            }],
            ..WalkerSeedSpec::default()
        },
        WalkerSeedSpec {
            external: vec![WalkerExternalSeed::File {
                path: PathBuf::from("../escape"),
                bytes: b"escape\n".to_vec(),
                readonly: false,
                unix_mode: 0o640,
            }],
            ..WalkerSeedSpec::default()
        },
        WalkerSeedSpec {
            external: vec![WalkerExternalSeed::Symlink {
                path: PathBuf::from("safe-link"),
                target: PathBuf::from("../escape"),
            }],
            ..WalkerSeedSpec::default()
        },
        WalkerSeedSpec {
            external: vec![WalkerExternalSeed::Symlink {
                path: PathBuf::from("../escape-link"),
                target: PathBuf::from("safe-target"),
            }],
            ..WalkerSeedSpec::default()
        },
        WalkerSeedSpec {
            external_references: vec![WalkerExternalReferenceSeed {
                request: command("Escape reference"),
                source: PathBuf::from("../escape"),
            }],
            ..WalkerSeedSpec::default()
        },
    ];

    for spec in cases {
        assert!(
            RealWalkerHost::spawn(spec)
                .unwrap_err()
                .contains("relative descendants")
        );
        assert_eq!(fs::read(&sentinel).unwrap(), b"outside stays exact\n");
    }
}

#[test]
fn every_known_and_future_entry_kind_reopens_through_the_production_store() {
    let mut spec = WalkerSeedSpec {
        profile: "kinds".to_owned(),
        settings: BTreeMap::from([("lang".to_owned(), "en".to_owned())]),
        ..WalkerSeedSpec::default()
    };
    for (kind, stored_name, bytes) in [
        ("python", "script.py", b"print(1)\n".as_slice()),
        ("shell", "script.sh", b"printf ok\n".as_slice()),
        ("fish", "script.fish", b"echo ok\n".as_slice()),
        ("js", "script.js", b"console.log('ok');\n".as_slice()),
        ("ts", "script.ts", b"console.log('ok');\n".as_slice()),
        ("powershell", "script.ps1", b"Write-Output ok\n".as_slice()),
        ("ruby", "script.rb", b"puts 'ok'\n".as_slice()),
        ("perl", "script.pl", b"print qq(ok);\n".as_slice()),
        ("lua", "script.lua", b"print('ok')\n".as_slice()),
        ("r", "script.r", b"print('ok')\n".as_slice()),
        ("prompt", "prompt.md", b"Review this\n".as_slice()),
        ("future-kind", "payload", b"future bytes\n".as_slice()),
    ] {
        spec.entries.push(CreateEntry {
            name: format!("Kind {kind}"),
            kind: EntryKind::parse(kind).unwrap(),
            mode: StorageMode::Copy,
            source: String::new(),
            workdir: "store".to_owned(),
            description: String::new(),
            payload: Some(EntryPayload {
                bytes: bytes.to_vec(),
                stored_name: Some(stored_name.to_owned()),
                permissions: SourcePermissions::default(),
            }),
            settings: EntrySettings::default(),
        });
    }
    spec.entries.push(command("Kind command"));
    spec.external.push(WalkerExternalSeed::File {
        path: PathBuf::from("tool"),
        bytes: b"executable bytes\n".to_vec(),
        readonly: false,
        unix_mode: 0o751,
    });
    spec.external_references.push(WalkerExternalReferenceSeed {
        request: CreateEntry {
            name: "Kind exe".to_owned(),
            kind: EntryKind::parse("exe").unwrap(),
            mode: StorageMode::Reference,
            source: String::new(),
            workdir: "origin".to_owned(),
            description: String::new(),
            payload: Some(EntryPayload {
                bytes: b"executable bytes\n".to_vec(),
                stored_name: None,
                permissions: SourcePermissions::default(),
            }),
            settings: EntrySettings::default(),
        },
        source: PathBuf::from("tool"),
    });

    let host = RealWalkerHost::spawn(spec).unwrap();
    let kinds = host
        .service
        .list()
        .unwrap()
        .entries
        .into_iter()
        .map(|entry| entry.kind.as_str().to_owned())
        .collect::<std::collections::BTreeSet<_>>();

    assert_eq!(
        kinds,
        [
            "command",
            "exe",
            "fish",
            "future-kind",
            "js",
            "lua",
            "perl",
            "powershell",
            "prompt",
            "python",
            "r",
            "ruby",
            "shell",
            "ts",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect()
    );
}

#[test]
fn normalization_keeps_user_text_that_only_looks_like_an_id_or_path() {
    let mut spec = profile();
    spec.entries.push(CreateEntry {
        name: "Normalizer fields".to_owned(),
        kind: EntryKind::parse("command").unwrap(),
        mode: StorageMode::Copy,
        source: String::new(),
        workdir: "invoke".to_owned(),
        description: String::new(),
        payload: None,
        settings: EntrySettings {
            template: "printf {identity} {id} {path}".to_owned(),
            params: vec!["identity".to_owned(), "id".to_owned(), "path".to_owned()],
            parameters: ["identity", "id", "path"]
                .into_iter()
                .map(ParamDecl::new)
                .collect(),
            ..EntrySettings::default()
        },
    });
    let mut host = RealWalkerHost::spawn(spec).unwrap();
    let command = host.service.show("Command").unwrap();
    let id = command.meta.id.as_ref().unwrap().as_str();
    let root_suffix = format!("{}-user-suffix", host._sandbox.path().display());
    let description = format!("literal user text: {id} and {root_suffix}",);
    host.service.describe(&command, &description).unwrap();
    let normalizer = host.service.show("Normalizer fields").unwrap();
    let declarations = crate::cli::entry_parameters(host.service.repository(), &normalizer);
    FormStateService::new(FileFormStateStore::new(&host.roots().state))
        .save_last(
            &normalizer.slug,
            &declarations,
            Some(&BTreeMap::from([
                ("identity".to_owned(), "literal identity".to_owned()),
                ("id".to_owned(), id.to_owned()),
                ("path".to_owned(), root_suffix.clone()),
            ])),
            None,
            false,
        )
        .unwrap();
    let unknown_meta = host.roots().home.as_ref().unwrap().join("meta.toml");
    fs::write(
        &unknown_meta,
        format!("identity = \"literal identity\"\nid = \"{id}\"\npath = \"{root_suffix}\"\n"),
    )
    .unwrap();
    host.dispatch(Effect::Submit {
        purpose: FormPurpose::Run,
        selector: Some("Command".to_owned()),
        values: BTreeMap::from([("value:value".to_owned(), FieldValue::text(&root_suffix))]),
    })
    .unwrap();

    let observation = host.observe(&host.initial_state().unwrap()).unwrap();
    let encoded = serde_json::to_string_pretty(&observation).unwrap();
    let normalizer_values = &observation.form_state["normalizer-fields"]["values"];

    let encoded_description = serde_json::to_string(&description).unwrap();
    assert!(encoded.contains(&encoded_description[1..encoded_description.len() - 1]));
    assert!(encoded.contains("<entry-id:command>"));
    assert_eq!(
        normalizer_values,
        &json!({
            "identity": "literal identity",
            "id": id,
            "path": root_suffix,
        })
    );
    assert!(observation.tree.iter().any(|row| {
        row.path == "home/meta.toml"
            && matches!(&row.content, Some(ByteView::Utf8(text))
                    if text.contains("identity = \"literal identity\"")
                        && text.contains(id)
                        && text.contains(&root_suffix))
    }));
    let encoded_suffix = serde_json::to_string(&root_suffix).unwrap();
    assert!(
        serde_json::to_string(&observation.transcript)
            .expect("the transcript is serializable")
            .contains(&encoded_suffix[1..encoded_suffix.len() - 1])
    );
    let mut path_map = PathMap::new(
        &host.profile,
        host._sandbox.path(),
        &host.service,
        &host.file_picker_tree.files,
    )
    .unwrap();
    let user_screen = json!({
        "workflow": {
            "active": {
                "form": {
                    "path": host.roots().data.join("user-value"),
                    "identity": "literal identity",
                }
            },
            "history": [],
        }
    });
    assert_eq!(
        path_map
            .normalize_library_json(user_screen.clone())
            .unwrap(),
        user_screen
    );
    path_map.modified_ranks.assign_sorted(&[99]).unwrap();
    let add_source_path = host.roots().data.join("literal-user-input");
    let add_workflow = json!({
        "workflow": {
            "active": {
                "add": {
                    "source": {
                        "path": add_source_path,
                        "drafts": [{
                            "path": host.roots().data.join("typed-draft"),
                            "identity": {
                                "platform": "unix",
                                "device": 7,
                                "inode": 8,
                                "change_time_seconds": 9,
                                "change_time_nanoseconds": 10,
                            },
                            "modified": 99,
                        }],
                    },
                    "pending_source": {
                        "path": host.roots().data.join("typed-artifact"),
                        "identity": {
                            "platform": "unix",
                            "device": 1,
                            "inode": 2,
                            "change_time_seconds": 3,
                            "change_time_nanoseconds": 4,
                        },
                    },
                    "review": { "source": {
                        "path": host.roots().data.join("review-artifact"),
                        "identity": {
                            "platform": "unix",
                            "device": 2,
                            "inode": 3,
                            "change_time_seconds": 4,
                            "change_time_nanoseconds": 5,
                        },
                    }},
                    "pending_delete": ["draft", {
                        "path": host.roots().data.join("delete-artifact"),
                        "identity": {
                            "platform": "unix",
                            "device": 3,
                            "inode": 4,
                            "change_time_seconds": 5,
                            "change_time_nanoseconds": 6,
                        },
                    }],
                    "delete_candidate": {
                        "path": host.roots().data.join("delete-candidate"),
                        "identity": {
                            "platform": "unix",
                            "device": 4,
                            "inode": 5,
                            "change_time_seconds": 6,
                            "change_time_nanoseconds": 7,
                        },
                    },
                }
            },
            "history": [{ "add": {
                "pending_source": {
                    "path": host.roots().data.join("history-artifact"),
                    "identity": {
                        "platform": "unix",
                        "device": 5,
                        "inode": 6,
                        "change_time_seconds": 7,
                        "change_time_nanoseconds": 8,
                    },
                }
            }}],
        }
    });
    let normalized_add = path_map
        .normalize_library_json(add_workflow.clone())
        .unwrap();
    assert_eq!(
        path_map.normalize_library_json(add_workflow).unwrap(),
        normalized_add
    );
    assert_eq!(
        normalized_add["workflow"]["active"]["add"]["source"]["path"],
        json!(add_source_path)
    );
    assert_eq!(
        normalized_add["workflow"]["active"]["add"]["source"]["drafts"][0],
        json!({
            "path": "<profile:fixture>/data/typed-draft",
            "identity": {
                "platform": "unix",
                "device": 0,
                "inode": 0,
                "change_time_seconds": 0,
                "change_time_nanoseconds": 0,
            },
            "modified": 0,
        })
    );
    assert_eq!(
        normalized_add["workflow"]["active"]["add"]["pending_source"],
        json!({
            "path": "<profile:fixture>/data/typed-artifact",
            "identity": {
                "platform": "unix",
                "device": 0,
                "inode": 1,
                "change_time_seconds": 0,
                "change_time_nanoseconds": 0,
            },
        })
    );
    assert_eq!(
        normalized_add["workflow"]["active"]["add"]["pending_delete"][1]["path"],
        "<profile:fixture>/data/delete-artifact"
    );
    assert_eq!(
        normalized_add["workflow"]["history"][0]["add"]["pending_source"]["path"],
        "<profile:fixture>/data/history-artifact"
    );
    let mut missing_identity = json!({
        "path": host.roots().data.join("missing"),
        "identity": null,
    });
    path_map
        .normalize_artifact_object(&mut missing_identity)
        .unwrap();
    assert_eq!(missing_identity["identity"], Value::Null);
    let mut scalar = json!("not an artifact");
    path_map.normalize_artifact_object(&mut scalar).unwrap();
    assert_eq!(scalar, "not an artifact");
    let unregistered_identity = json!({
        "platform": "unix",
        "device": 9,
        "inode": 9,
        "change_time_seconds": 9,
        "change_time_nanoseconds": 9,
    });
    let mut unregistered = json!({
        "path": "/outside/user-value",
        "identity": unregistered_identity.clone(),
    });
    path_map
        .normalize_artifact_object(&mut unregistered)
        .unwrap();
    assert_eq!(unregistered["identity"], unregistered_identity);
    let mut unknown_identity = json!({
        "path": host.roots().data.join("unknown-identity"),
        "identity": { "platform": "plan9" },
    });
    assert_eq!(
        path_map
            .normalize_artifact_object(&mut unknown_identity)
            .unwrap_err(),
        "source identity platform is unknown: plan9"
    );
    let mut unscanned_modified = json!({
        "path": host.roots().data.join("unscanned-draft"),
        "modified": 100,
    });
    assert_eq!(
        path_map
            .normalize_artifact_object(&mut unscanned_modified)
            .unwrap_err(),
        "the sorted scan did not assign a rank to this draft modified value: 100"
    );
    let unknown = || {
        json!({
            "path": host.roots().data.join("unknown-chain"),
            "identity": { "platform": "plan9" },
        })
    };
    assert_eq!(
        path_map
            .normalize_action_json(json!({
                "present": { "add": { "pending_source": unknown() } },
            }))
            .unwrap_err(),
        "a serialized Action is invalid: unknown variant `plan9`, expected `unix` or `windows`"
    );
    assert_eq!(
        path_map
            .normalize_draft_list(json!([unknown()]))
            .unwrap_err(),
        "source identity platform is unknown: plan9"
    );
    assert_eq!(
        path_map
            .normalize_library_json(json!({
                "workflow": {
                    "active": { "add": { "pending_source": unknown() } },
                },
            }))
            .unwrap_err(),
        "source identity platform is unknown: plan9"
    );
    assert_eq!(
        path_map
            .normalize_library_json(json!({
                "workflow": {
                    "history": [{ "add": { "pending_source": unknown() } }],
                },
            }))
            .unwrap_err(),
        "source identity platform is unknown: plan9"
    );
    assert_eq!(
        path_map.normalize_library_json(json!({ "workflow": {} })),
        Ok(json!({ "workflow": {} }))
    );
    assert!(
        path_map
            .normalize_action_json(json!("not an action"))
            .is_err()
    );
    assert_eq!(
        path_map.normalize_draft_list(json!("not a draft list")),
        Ok(json!("not a draft list"))
    );
    assert!(
            path_map
                .normalize_action_json(json!({
                    "present": {
                        "settings": {
                            "sections": [{
                                "items": [{
                                    "item": "note",
                                    "text": "Keep a copy — your original file is never modified. Source: {}",
                                }],
                            }],
                        },
                    },
                }))
                .is_err()
        );
    assert_eq!(
        path_map.normalize_path(host._sandbox.path()),
        "<profile:fixture>"
    );
    path_map.register_owned_transient_path(Path::new("/"), "run-snapshot", ".run-");
    assert!(!path_map.transient_paths.contains_key(Path::new("/")));
    path_map.register_owned_transient_path(Path::new("ordinary.txt"), "run-snapshot", ".run-");
    assert!(
        !path_map
            .transient_paths
            .contains_key(Path::new("ordinary.txt"))
    );
    path_map
        .text_artifacts
        .insert("root".to_owned(), "stable".to_owned());
    assert_eq!(path_map.normalize_host_text("unchanged"), "unchanged");
    assert_eq!(path_map.normalize_host_text("root-suffix"), "root-suffix");
    assert_eq!(
        path_map.normalize_host_text("'root' root"),
        "'stable' stable"
    );
}

#[test]
fn managed_documents_keep_unknown_bytes_and_invalid_cache_proofs_visible() {
    let mut host = RealWalkerHost::spawn(profile()).unwrap();
    let meta_path = host.roots().data.join("scripts/command/meta.toml");
    let mut raw_meta = fs::read_to_string(&meta_path).unwrap();
    raw_meta.push_str(
            "\n# future comment stays byte-visible\n[future]\nwhen = 1979-05-27T07:32:00Z\nspaced    =    \"literal\"\n",
        );
    fs::write(&meta_path, &raw_meta).unwrap();

    let observation = host.observe(&host.initial_state().unwrap()).unwrap();
    let viewed_meta = observation
        .tree
        .iter()
        .find(|row| row.path == "data/scripts/command/meta.toml")
        .and_then(|row| row.content.as_ref())
        .unwrap();
    let viewed_meta = serde_json::to_value(viewed_meta).unwrap();
    assert_eq!(viewed_meta["encoding"], "utf8");
    let viewed_meta = viewed_meta["data"].as_str().unwrap();
    assert!(viewed_meta.contains("# future comment stays byte-visible"));
    assert!(viewed_meta.contains("when = 1979-05-27T07:32:00Z"));
    assert!(viewed_meta.contains("spaced    =    \"literal\""));
    assert_eq!(fs::read_to_string(&meta_path).unwrap(), raw_meta);

    let invalid_registry = r#"[entries.command]
mtime_ns = "not-an-integer"

[entries.command.skit_cache]
schema = 1
platform = "unix"
file_id = ""
file_size = "01"
modified_ns = "bad"
changed_ns = "bad"
metadata_hash = "not-a-sha"
projection_hash = "not-a-sha"
"#;
    let path_map = &host.path_map;
    assert_eq!(
        path_map.byte_view(
            &host.roots().data.join("registry.toml"),
            invalid_registry.as_bytes(),
        ),
        ByteView::Utf8(invalid_registry.to_owned())
    );
    assert_eq!(
        path_map.byte_view(&host.roots().data.join("registry.toml"), b"not = [toml"),
        ByteView::Utf8("not = [toml".to_owned())
    );
    for registry in ["future = true\n", "[entries]\ncommand = \"not a row\"\n"] {
        assert_eq!(
            path_map.byte_view(
                &host.roots().data.join("registry.toml"),
                registry.as_bytes(),
            ),
            ByteView::Utf8(registry.to_owned())
        );
    }
    assert_eq!(
        path_map.byte_view(
            &host.roots().data.join("registry.toml"),
            b"[entries.command]\nmtime_ns = 7\n",
        ),
        ByteView::Utf8("[entries.command]\nmtime_ns = \"<mtime-ns>\"\n".to_owned())
    );
    #[cfg(unix)]
    let (current_platform, foreign_platform) = ("unix", "windows");
    #[cfg(windows)]
    let (current_platform, foreign_platform) = ("windows", "unix");
    #[cfg(not(any(unix, windows)))]
    let (current_platform, foreign_platform) = ("unsupported", "unix");
    for (platform, file_size) in [(foreign_platform, "1"), (current_platform, "-1")] {
        let invalid_proof = format!(
            "[entries.command.skit_cache]\n\
                 schema = 1\n\
                 platform = \"{platform}\"\n\
                 file_id = \"fixture\"\n\
                 file_size = \"{file_size}\"\n\
                 modified_ns = \"1\"\n\
                 changed_ns = \"1\"\n\
                 metadata_hash = \"sha256:{}\"\n\
                 projection_hash = \"sha256:{}\"\n",
            "0".repeat(64),
            "1".repeat(64),
        );
        assert_eq!(
            path_map.byte_view(
                &host.roots().data.join("registry.toml"),
                invalid_proof.as_bytes(),
            ),
            ByteView::Utf8(invalid_proof)
        );
    }

    let mut missing_id = raw_meta.parse::<toml_edit::DocumentMut>().unwrap();
    missing_id.remove("id");
    fs::write(&meta_path, missing_id.to_string()).unwrap();
    let observation = host.observe(&host.initial_state().unwrap()).unwrap();
    assert!(observation.tree.iter().any(|row| {
        row.path == "data/scripts/command/meta.toml"
            && matches!(&row.content, Some(ByteView::Utf8(text))
                    if !text.lines().any(|line| line.starts_with("id =")))
    }));
}

#[cfg(unix)]
#[test]
fn corrupt_siblings_and_symlink_topology_stay_visible_without_following_outside() {
    use std::os::unix::fs::PermissionsExt as _;

    let mut spec = profile();
    spec.external.extend([
        WalkerExternalSeed::File {
            path: PathBuf::from("directory/must-not-be-followed"),
            bytes: b"outside\n".to_vec(),
            readonly: false,
            unix_mode: 0o640,
        },
        WalkerExternalSeed::Symlink {
            path: PathBuf::from("reference-link.sh"),
            target: PathBuf::from("outside.sh"),
        },
    ]);
    spec.external_references[0].source = PathBuf::from("reference-link.sh");
    spec.entries.push(command("Corrupt"));
    let mut host = RealWalkerHost::spawn(spec).unwrap();
    let outside_before = outside_snapshot_portable(&host.external_root);
    std::os::unix::fs::symlink(
        host.external_root.join("directory"),
        host.roots().home.as_ref().unwrap().join("outside-link"),
    )
    .unwrap();
    let corrupt = host.roots().data.join("scripts/corrupt/meta.toml");
    fs::write(&corrupt, b"invalid = [toml\n").unwrap();

    let observation = host.observe(&host.initial_state().unwrap()).unwrap();

    assert!(observation.surface.to_string().contains("corrupt"));
    assert!(observation.surface.to_string().contains("command"));
    assert!(observation.tree.iter().any(|row| {
        row.path == "home/outside-link" && row.kind == "symlink" && row.content.is_none()
    }));
    let reference_link = host.external_root.join("reference-link.sh");
    let reference_row = observation
        .tree
        .iter()
        .find(|row| row.path == "external/reference-link.sh")
        .unwrap();
    assert_eq!(reference_row.kind, "symlink");
    assert_eq!(
        reference_row.mode,
        portable_mode(&fs::symlink_metadata(&reference_link).unwrap())
    );
    assert_eq!(
        host.observation_modes
            .borrow()
            .expected_kind(&reference_link)
            .unwrap(),
        Some(ObservationNodeKind::Symlink)
    );
    assert!(
        !observation
            .tree
            .iter()
            .any(|row| row.path.starts_with("home/outside-link/"))
    );
    assert_eq!(
        outside_snapshot_portable(&host.external_root),
        outside_before
    );
    assert!(
        fs::symlink_metadata(host.external_root.join("reference-link.sh"))
            .unwrap()
            .is_symlink()
    );
    assert_eq!(
        fs::read(host.external_root.join("outside.sh")).unwrap(),
        b"printf outside\n"
    );
    assert_eq!(
        fs::metadata(host.external_root.join("outside.sh"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o640
    );
}

#[cfg(unix)]
#[test]
fn reference_and_executable_external_sources_keep_symlink_provenance() {
    let mut spec = profile();
    spec.external.extend([
        WalkerExternalSeed::File {
            path: PathBuf::from("tool"),
            bytes: b"tool\n".to_vec(),
            readonly: false,
            unix_mode: 0o751,
        },
        WalkerExternalSeed::Symlink {
            path: PathBuf::from("reference-link.sh"),
            target: PathBuf::from("outside.sh"),
        },
        WalkerExternalSeed::Symlink {
            path: PathBuf::from("tool-link"),
            target: PathBuf::from("tool"),
        },
    ]);
    spec.external_references[0].source = PathBuf::from("reference-link.sh");
    spec.external_references.push(WalkerExternalReferenceSeed {
        request: CreateEntry {
            name: "Linked executable".to_owned(),
            kind: EntryKind::parse("exe").unwrap(),
            mode: StorageMode::Reference,
            source: String::new(),
            workdir: "origin".to_owned(),
            description: String::new(),
            payload: None,
            settings: EntrySettings::default(),
        },
        source: PathBuf::from("tool-link"),
    });
    let mut host = RealWalkerHost::spawn(spec).unwrap();

    let observation = host.observe(&host.initial_state().unwrap()).unwrap();

    for path in ["reference-link.sh", "tool-link"] {
        let physical = host.external_root.join(path);
        let row = observation
            .tree
            .iter()
            .find(|row| row.path == format!("external/{path}"))
            .unwrap();
        assert_eq!(row.kind, "symlink");
        assert_eq!(
            row.mode,
            portable_mode(&fs::symlink_metadata(&physical).unwrap())
        );
        assert_eq!(
            host.observation_modes
                .borrow()
                .expected_kind(&physical)
                .unwrap(),
            Some(ObservationNodeKind::Symlink)
        );
    }
}

#[test]
fn production_form_seeding_never_persists_secret_values() {
    let mut secret = ParamDecl::new("token");
    secret.secret = true;
    let mut spec = profile();
    spec.entries.push(CreateEntry {
        name: "Secret".to_owned(),
        kind: EntryKind::parse("command").unwrap(),
        mode: StorageMode::Copy,
        source: String::new(),
        workdir: "invoke".to_owned(),
        description: String::new(),
        payload: None,
        settings: EntrySettings {
            template: "printf {token}".to_owned(),
            parameters: vec![secret],
            ..EntrySettings::default()
        },
    });
    spec.forms.push(WalkerFormSeed {
        selector: "Secret".to_owned(),
        values: BTreeMap::from([("token".to_owned(), "must-not-persist".to_owned())]),
        ..WalkerFormSeed::default()
    });

    let mut host = RealWalkerHost::spawn(spec).unwrap();
    let observation = host.observe(&host.initial_state().unwrap()).unwrap();

    assert_eq!(observation.form_state["secret"]["values"], json!({}));
    assert!(
        !serde_json::to_string(&observation)
            .unwrap()
            .contains("must-not-persist")
    );
}

#[test]
fn form_seeding_uses_legacy_params_and_source_comment_declarations() {
    let legacy = CreateEntry {
        name: "Legacy params".to_owned(),
        kind: EntryKind::parse("command").unwrap(),
        mode: StorageMode::Copy,
        source: String::new(),
        workdir: "invoke".to_owned(),
        description: String::new(),
        payload: None,
        settings: EntrySettings {
            template: "printf {legacy}".to_owned(),
            params: vec!["legacy".to_owned()],
            parameters: Vec::new(),
            ..EntrySettings::default()
        },
    };
    let mut source_declaration = ParamDecl::new("SOURCE_ONLY");
    source_declaration.binding = ParameterBinding::Const;
    source_declaration.delivery = ParameterDelivery::Inject;
    source_declaration.default = Some(ParameterValue::String("before".to_owned()));
    let source = write_managed_params(
        "shell",
        "SOURCE_ONLY=before\nprintf '%s\\n' \"$SOURCE_ONLY\"\n",
        std::slice::from_ref(&source_declaration),
    )
    .unwrap();
    let source_only = CreateEntry {
        name: "Source comment".to_owned(),
        kind: EntryKind::parse("shell").unwrap(),
        mode: StorageMode::Copy,
        source: String::new(),
        workdir: "store".to_owned(),
        description: String::new(),
        payload: Some(EntryPayload {
            bytes: source.into_bytes(),
            stored_name: Some("script.sh".to_owned()),
            permissions: SourcePermissions::default(),
        }),
        settings: EntrySettings::default(),
    };
    let form = |selector: &str, name: &str| WalkerFormSeed {
        selector: selector.to_owned(),
        values: BTreeMap::from([(name.to_owned(), "saved".to_owned())]),
        preset: Some("fixture".to_owned()),
        last_run: Some(WalkerLastRunSeed {
            exit: 3,
            at: "2026-08-28T10:00:00+00:00".to_owned(),
            values: Some(BTreeMap::from([(name.to_owned(), "launched".to_owned())])),
        }),
        ..WalkerFormSeed::default()
    };
    let mut host = RealWalkerHost::spawn(WalkerSeedSpec {
        profile: "declarations".to_owned(),
        entries: vec![legacy, source_only],
        settings: BTreeMap::from([("lang".to_owned(), "en".to_owned())]),
        forms: vec![
            form("Legacy params", "legacy"),
            form("Source comment", "SOURCE_ONLY"),
        ],
        ..WalkerSeedSpec::default()
    })
    .unwrap();

    let observation = host.observe(&host.initial_state().unwrap()).unwrap();
    for (slug, name) in [
        ("legacy-params", "legacy"),
        ("source-comment", "SOURCE_ONLY"),
    ] {
        assert_eq!(observation.form_state[slug]["values"][name], "saved");
        assert_eq!(
            observation.form_state[slug]["presets"]["fixture"][name],
            "saved"
        );
        assert_eq!(
            observation.form_state[slug]["last_run"]["values"][name],
            "launched"
        );
        assert_eq!(observation.form_state[slug]["last_run"]["exit"], 3);
    }
}
