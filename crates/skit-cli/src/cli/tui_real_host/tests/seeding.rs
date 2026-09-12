//! Seeded profile, copy fact, and persistent token contracts.

use super::*;

fn add_external_source(host: &RealWalkerHost, name: &str) {
    let mut workflow = AddWorkflowState::new(Vec::new());
    let _ = workflow.reduce(AddAction::SetSourcePath(
        host.external_root.join(name).display().to_string(),
    ));
    let inspected = host
        .dispatch(Effect::Add(workflow.reduce(AddAction::Continue)))
        .unwrap();
    let inspected = serde_json::to_value(inspected).unwrap();
    let inspected: AddAction = serde_json::from_value(inspected["add"].clone()).unwrap();
    let _ = workflow.reduce(inspected);
    let committed = host
        .dispatch(Effect::Add(workflow.reduce(AddAction::Save)))
        .unwrap();
    assert!(matches!(
        committed,
        Action::Add(AddAction::CommitFinished { result: Ok(_), .. })
    ));
}

fn add_external_copy_payload(host: &RealWalkerHost, name: &str) -> PathBuf {
    let before = host
        .service
        .repository()
        .scan_entries()
        .unwrap()
        .into_iter()
        .map(|entry| entry.slug)
        .collect::<BTreeSet<_>>();
    add_external_source(host, name);
    let entry = host
        .service
        .repository()
        .scan_entries()
        .unwrap()
        .into_iter()
        .find(|entry| !before.contains(&entry.slug))
        .unwrap();
    host.service.repository().payload_path(&entry).unwrap()
}

#[test]
fn seeded_profiles_reopen_every_store_and_normalize_volatile_facts() {
    let mut first = RealWalkerHost::spawn(profile()).unwrap();
    let mut second = RealWalkerHost::spawn(profile()).unwrap();
    let first_external_before = outside_snapshot_portable(&first.external_root);
    let second_external_before = outside_snapshot_portable(&second.external_root);

    assert_ne!(first.roots().data, second.roots().data);
    let first_state = first.initial_state().unwrap();
    let second_state = second.initial_state().unwrap();
    let first_observation = first.observe(&first_state).unwrap();
    let second_observation = second.observe(&second_state).unwrap();
    assert_eq!(first_observation, second_observation);
    let encoded = serde_json::to_string_pretty(&first_observation).unwrap();
    assert!(!encoded.contains(&first.roots().data.display().to_string()));
    assert!(!encoded.contains(&second.roots().data.display().to_string()));
    assert!(encoded.contains("<entry-id:command>"));
    assert!(encoded.contains("<added-at:command>"));
    assert!(encoded.contains(&encode_hex(b"print('\xff')\n\xff")));
    let binary = first_observation
        .tree
        .iter()
        .find(|row| row.path == "data/scripts/binary-bytes/script.py")
        .unwrap();
    assert!(!binary.readonly);
    assert!(matches!(&binary.content, Some(ByteView::Hex(_))));
    #[cfg(unix)]
    assert_eq!(binary.mode, Some(0o4751));
    #[cfg(not(unix))]
    assert_eq!(binary.mode, None);
    assert_eq!(
        outside_snapshot_portable(&first.external_root),
        first_external_before
    );
    assert_eq!(
        outside_snapshot_portable(&second.external_root),
        second_external_before
    );
}

#[test]
fn newly_added_and_replaced_copy_payload_modes_stay_raw() {
    let mut host = RealWalkerHost::spawn(profile()).unwrap();
    let original = host
        .service
        .repository()
        .scan_entries()
        .unwrap()
        .into_iter()
        .map(|entry| entry.slug)
        .collect::<BTreeSet<_>>();
    add_external_source(&host, "outside.sh");
    let entry = host
        .service
        .repository()
        .scan_entries()
        .unwrap()
        .into_iter()
        .find(|entry| !original.contains(&entry.slug))
        .unwrap();
    let payload = host.service.repository().payload_path(&entry).unwrap();
    let tree_path = host.path_map.normalize_path(&payload);
    let tree_path = tree_path
        .strip_prefix(&format!("{}/", host.path_map.profile_label))
        .unwrap()
        .to_owned();

    let first = host.observe(&host.initial_state().unwrap()).unwrap();
    let first = first.tree.iter().find(|row| row.path == tree_path).unwrap();
    #[cfg(unix)]
    assert_eq!(first.mode, Some(0o640));
    #[cfg(not(unix))]
    assert_eq!(first.mode, None);

    fs::remove_file(&payload).unwrap();
    fs::write(&payload, b"replacement source\n").unwrap();
    set_unix_mode(&payload, 0o750).unwrap();
    let second = host.observe(&host.initial_state().unwrap()).unwrap();
    let second = second
        .tree
        .iter()
        .find(|row| row.path == tree_path)
        .unwrap();
    #[cfg(unix)]
    assert_eq!(second.mode, Some(0o750));
    #[cfg(not(unix))]
    assert_eq!(second.mode, None);
}

#[test]
fn live_payload_mode_fact_refuses_a_later_directory() {
    let mut host = RealWalkerHost::spawn(profile()).unwrap();
    let entry = host.service.show("Binary bytes").unwrap();
    let payload = host.service.repository().payload_path(&entry).unwrap();
    let state = host.initial_state().unwrap();
    host.observe(&state).unwrap();
    fs::remove_file(&payload).unwrap();
    fs::create_dir(&payload).unwrap();

    let error = host.observe(&state).unwrap_err();

    assert_eq!(
        error,
        format!(
            "observation mode provenance expected file at {}, but found directory",
            payload.display()
        )
    );
}

#[test]
fn seed_copy_mode_fact_refuses_a_directory_before_first_observe() {
    let mut host = RealWalkerHost::spawn(profile()).unwrap();
    let entry = host.service.show("Binary bytes").unwrap();
    let payload = host.service.repository().payload_path(&entry).unwrap();
    fs::remove_file(&payload).unwrap();
    fs::create_dir(&payload).unwrap();

    let error = host.observe(&host.initial_state().unwrap()).unwrap_err();

    assert_eq!(
        error,
        format!(
            "observation mode provenance expected file at {}, but found directory",
            payload.display()
        )
    );
}

#[cfg(unix)]
#[test]
fn seed_copy_mode_fact_refuses_a_symlink_before_first_observe() {
    let mut host = RealWalkerHost::spawn(profile()).unwrap();
    let entry = host.service.show("Binary bytes").unwrap();
    let payload = host.service.repository().payload_path(&entry).unwrap();
    fs::remove_file(&payload).unwrap();
    std::os::unix::fs::symlink(host.external_root.join("outside.sh"), &payload).unwrap();

    let error = host.observe(&host.initial_state().unwrap()).unwrap_err();

    assert_eq!(
        error,
        format!(
            "observation mode provenance expected file at {}, but found symlink",
            payload.display()
        )
    );
}

fn external_reference_lane_copy_host() -> (RealWalkerHost, PathBuf) {
    let mut spec = profile();
    spec.external_references.push(WalkerExternalReferenceSeed {
        request: CreateEntry {
            name: "External lane copy".to_owned(),
            kind: EntryKind::parse("shell").unwrap(),
            mode: StorageMode::Copy,
            source: String::new(),
            workdir: "origin".to_owned(),
            description: String::new(),
            payload: Some(EntryPayload {
                bytes: b"printf external lane copy\n".to_vec(),
                stored_name: Some("custom.bin".to_owned()),
                permissions: SourcePermissions {
                    readonly: false,
                    unix_mode: Some(0o640),
                },
            }),
            settings: EntrySettings::default(),
        },
        source: PathBuf::from("outside.sh"),
    });
    let host = RealWalkerHost::spawn(spec).unwrap();
    let entry = host.service.show("External lane copy").unwrap();
    let payload = host.service.repository().payload_path(&entry).unwrap();
    assert_eq!(
        payload.file_name().and_then(|name| name.to_str()),
        Some("custom.bin")
    );
    (host, payload)
}

#[test]
fn external_reference_lane_copy_fact_refuses_a_directory_before_first_observe() {
    let (mut host, payload) = external_reference_lane_copy_host();
    fs::remove_file(&payload).unwrap();
    fs::create_dir(&payload).unwrap();

    let error = host.observe(&host.initial_state().unwrap()).unwrap_err();

    assert_eq!(
        error,
        format!(
            "observation mode provenance expected file at {}, but found directory",
            payload.display()
        )
    );
}

#[cfg(unix)]
#[test]
fn external_reference_lane_copy_fact_refuses_a_symlink_before_first_observe() {
    let (mut host, payload) = external_reference_lane_copy_host();
    fs::remove_file(&payload).unwrap();
    std::os::unix::fs::symlink(host.external_root.join("outside.sh"), &payload).unwrap();

    let error = host.observe(&host.initial_state().unwrap()).unwrap_err();

    assert_eq!(
        error,
        format!(
            "observation mode provenance expected file at {}, but found symlink",
            payload.display()
        )
    );
}

#[test]
fn new_copy_mode_fact_refuses_a_directory_before_first_observe() {
    let mut host = RealWalkerHost::spawn(profile()).unwrap();
    let payload = add_external_copy_payload(&host, "outside.sh");
    fs::remove_file(&payload).unwrap();
    fs::create_dir(&payload).unwrap();

    let error = host.observe(&host.initial_state().unwrap()).unwrap_err();

    assert_eq!(
        error,
        format!(
            "observation mode provenance expected file at {}, but found directory",
            payload.display()
        )
    );
}

#[cfg(unix)]
#[test]
fn new_copy_mode_fact_refuses_a_symlink_before_first_observe() {
    let mut host = RealWalkerHost::spawn(profile()).unwrap();
    let payload = add_external_copy_payload(&host, "outside.sh");
    fs::remove_file(&payload).unwrap();
    std::os::unix::fs::symlink(host.external_root.join("outside.sh"), &payload).unwrap();

    let error = host.observe(&host.initial_state().unwrap()).unwrap_err();

    assert_eq!(
        error,
        format!(
            "observation mode provenance expected file at {}, but found symlink",
            payload.display()
        )
    );
}

#[cfg(unix)]
#[test]
fn live_payload_mode_fact_refuses_a_later_symlink() {
    let mut host = RealWalkerHost::spawn(profile()).unwrap();
    let entry = host.service.show("Binary bytes").unwrap();
    let payload = host.service.repository().payload_path(&entry).unwrap();
    let state = host.initial_state().unwrap();
    host.observe(&state).unwrap();
    fs::remove_file(&payload).unwrap();
    std::os::unix::fs::symlink(host.external_root.join("outside.sh"), &payload).unwrap();

    let error = host.observe(&state).unwrap_err();

    assert_eq!(
        error,
        format!(
            "observation mode provenance expected file at {}, but found symlink",
            payload.display()
        )
    );
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
#[test]
fn stable_sandbox_child_roots_keep_their_validated_private_modes() {
    let parent = tempfile::TempDir::new().unwrap();
    let namespace =
        StableSandboxNamespace::explicit(parent.path().join(STABLE_SANDBOX_NAMESPACE)).unwrap();
    let mut host =
        RealWalkerHost::spawn_stable_in(profile(), safe_profile("mode-provenance"), namespace)
            .unwrap();

    let observation = host.observe(&host.initial_state().unwrap()).unwrap();

    for path in ["data", "state", "config", "home", "cwd", "external"] {
        let row = observation
            .tree
            .iter()
            .find(|row| row.path == path)
            .unwrap();
        #[cfg(unix)]
        assert_eq!(row.mode, Some(0o700));
        #[cfg(not(unix))]
        assert_eq!(row.mode, None);
    }
    host.close().unwrap();
}

#[test]
fn repeated_profile_spawns_keep_exact_surface_order_and_bytes() {
    let mut expected = None;
    for _ in 0..12 {
        let mut spec = profile();
        spec.external.extend([
            WalkerExternalSeed::File {
                path: PathBuf::from("alpha.py"),
                bytes: b"print('alpha')\n".to_vec(),
                readonly: false,
                unix_mode: 0o640,
            },
            WalkerExternalSeed::File {
                path: PathBuf::from("beta.py"),
                bytes: b"print('beta')\n".to_vec(),
                readonly: false,
                unix_mode: 0o640,
            },
        ]);
        let mut host = RealWalkerHost::spawn(spec).unwrap();
        add_external_source(&host, "alpha.py");
        add_external_source(&host, "beta.py");
        let entries = host.service.repository().scan_entries().unwrap();
        let activity = entries
            .iter()
            .filter(|entry| matches!(entry.slug.as_str(), "alpha" | "beta"))
            .map(|entry| (entry.slug.as_str(), entry.meta.added_at.as_str()))
            .collect::<BTreeMap<_, _>>();
        assert_eq!(activity["alpha"], "2026-08-28T12:35:00+00:00");
        assert_eq!(activity["beta"], "2026-08-28T12:35:01+00:00");
        let observation = host.observe(&host.initial_state().unwrap()).unwrap();
        let bytes = serde_json::to_vec(&observation).unwrap();
        if let Some(expected) = &expected {
            assert_eq!(&bytes, expected);
        } else {
            expected = Some(bytes);
        }
    }
}

#[test]
fn persistent_tokens_expose_same_slug_and_same_path_replacements() {
    let mut host = RealWalkerHost::spawn(profile()).unwrap();
    // Both drafts need distinct modified times. The sorted scan refuses equal times.
    let draft = write_draft_at(&host, "skit-new-same.py", b"same bytes\n", 10);
    write_draft_at(&host, "skit-new-z.py", b"zz bytes\n", 20);
    let first = host.observe(&host.initial_state().unwrap()).unwrap();
    let first_meta = first
        .tree
        .iter()
        .find(|row| row.path == "data/scripts/command/meta.toml")
        .and_then(|row| row.content.as_ref())
        .unwrap();
    let first_meta = serde_json::to_value(first_meta).unwrap();
    assert_eq!(first_meta["encoding"], "utf8");
    let first_meta = first_meta["data"].as_str().unwrap();
    let first_meta: toml::Value = toml::from_str(first_meta).unwrap();
    assert_eq!(first_meta["id"].as_str(), Some("<entry-id:command>"));
    assert_eq!(first_meta["added_at"].as_str(), Some("<added-at:command>"));
    assert_identity_sentinel(&first.drafts[0]["identity"], 0, 0);
    {
        let paths = &mut host.path_map;
        let next_draft = paths.next_draft;
        paths.register_draft_path(&draft);
        assert_eq!(paths.next_draft, next_draft);
    }

    let old_draft = OpenOptions::new().read(true).open(&draft).unwrap();
    fs::remove_file(&draft).unwrap();
    fs::write(&draft, b"same bytes\n").unwrap();
    let command_entry = host.service.show("Command").unwrap();
    host.service.remove(&command_entry).unwrap();
    host.service.add(command("Command")).unwrap();

    let second = host.observe(&host.initial_state().unwrap()).unwrap();
    drop(old_draft);
    let second_meta = second
        .tree
        .iter()
        .find(|row| row.path == "data/scripts/command/meta.toml")
        .and_then(|row| row.content.as_ref())
        .unwrap();
    let second_meta = serde_json::to_value(second_meta).unwrap();
    assert_eq!(second_meta["encoding"], "utf8");
    let second_meta = second_meta["data"].as_str().unwrap();
    let second_meta: toml::Value = toml::from_str(second_meta).unwrap();

    assert_eq!(second_meta["id"].as_str(), Some("<entry-id:command:2>"));
    assert_eq!(
        second_meta["added_at"].as_str(),
        Some("<added-at:command:2>")
    );
    assert_identity_sentinel(&second.drafts[0]["identity"], 2, 0);
}

#[test]
fn absent_config_document_never_creates_a_dangling_tree_reference() {
    let mut host = RealWalkerHost::spawn(WalkerSeedSpec {
        profile: "empty".to_owned(),
        ..WalkerSeedSpec::default()
    })
    .unwrap();

    let observation = host.observe(&host.initial_state().unwrap()).unwrap();

    assert_eq!(observation.config["document_ref"], Value::Null);
    assert!(
        !observation
            .tree
            .iter()
            .any(|row| row.path == "config/config.toml")
    );
}

#[test]
fn an_empty_profile_name_uses_the_stable_default_artifact_label() {
    let mut host = RealWalkerHost::spawn(WalkerSeedSpec::default()).unwrap();

    assert_eq!(host.profile, "default");
    assert_eq!(host.path_map.profile_label, "<profile:default>");
    assert!(host.observe(&host.initial_state().unwrap()).is_ok());
}
