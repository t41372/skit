//! Ambient path and leak oracle fact contracts.

use super::*;

fn assert_clone_debug_eq<T: Clone + std::fmt::Debug + Eq>(value: &T) {
    assert_eq!(value, &value.clone(), "{value:?}");
}

#[test]
fn ambient_path_facts_keep_lexical_and_resolved_spellings() {
    let parent = tempfile::TempDir::new().unwrap();
    let lexical = parent.path().join(".");
    let resolved = fs::canonicalize(&lexical).unwrap();
    let absent = parent.path().join("absent");
    let paths = ambient_path_spellings([
        lexical.clone(),
        lexical.clone(),
        absent.clone(),
        PathBuf::new(),
    ]);

    let expected = BTreeSet::from([
        lexical.display().to_string(),
        resolved.display().to_string(),
        ordinary_path(&resolved).display().to_string(),
        absent.display().to_string(),
    ]);
    assert_eq!(paths, expected);
}

#[test]
fn ambient_path_facts_capture_the_process_without_entering_observation_json() {
    let mut host = RealWalkerHost::spawn(profile()).unwrap();
    let facts = host.leak_oracle_facts();
    let mut roots = vec![
        crate::cli::tui_walker_bundle::checkout_root().unwrap(),
        std::env::current_dir().unwrap(),
        std::env::temp_dir(),
    ];
    roots.extend(["HOME", "USERPROFILE"].into_iter().filter_map(|name| {
        std::env::var_os(name)
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
    }));
    for root in roots {
        assert!(facts.ambient_paths.contains(&root.display().to_string()));
    }

    let state = host.initial_state().unwrap();
    let observation = serde_json::to_value(host.observe(&state).unwrap()).unwrap();
    assert!(observation.get("ambient_paths").is_none());
    assert_eq!(host.leak_oracle_facts().ambient_paths, facts.ambient_paths);
    host.close().unwrap();
}

#[test]
fn c3_artifact_facts_capture_only_exact_projection_pairs() {
    let mut host = RealWalkerHost::spawn(profile()).unwrap();
    let draft_path = write_draft_at(
        &host,
        "skit-new-c3-artifact.py",
        b"print('artifact facts')\n",
        10,
    );
    host.path_map.refresh(&host.service).unwrap();
    let mut draft = serde_json::to_value(
        sorted_tui_drafts(&host.roots().data)
            .into_iter()
            .find(|draft| draft.path == draft_path)
            .unwrap(),
    )
    .unwrap();
    let raw_path = draft["path"].as_str().unwrap().to_owned();
    let raw_identity = skit_tui_walker_support::canonical_json_bytes(&draft["identity"]).unwrap();
    let raw_modified = draft["modified"].as_u64().unwrap();
    let mut path_only = draft["path"].clone();
    host.path_map.normalize_path_value(&mut path_only);

    let before = host.leak_oracle_facts();
    assert_clone_debug_eq(&before);
    assert_eq!(
        before
            .artifacts
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        ["<profile:fixture>"]
    );

    host.path_map.normalize_draft_summary(&mut draft).unwrap();

    let projected_path = draft["path"].as_str().unwrap();
    let projected_identity =
        skit_tui_walker_support::canonical_json_bytes(&draft["identity"]).unwrap();
    let projected_modified = draft["modified"].as_u64().unwrap();
    let after = host.leak_oracle_facts();
    let artifact = &after.artifacts[projected_path];
    assert_eq!(artifact.raw_path_spellings.len(), 1);
    assert!(artifact.raw_path_spellings.contains(&raw_path));
    assert_eq!(artifact.source_identities.len(), 1);
    let identity = artifact.source_identities.iter().next().unwrap();
    assert_eq!(identity.raw, raw_identity);
    assert_eq!(identity.projected, projected_identity);
    assert_eq!(artifact.modified_values.len(), 1);
    let modified = artifact.modified_values.iter().next().unwrap();
    assert_eq!(modified.raw, raw_modified);
    assert_eq!(modified.projected, projected_modified);

    host.path_map.normalize_draft_summary(&mut draft).unwrap();
    assert_eq!(host.leak_oracle_facts(), after);
}

#[test]
fn c3_renderer_draft_facts_keep_all_producer_names_and_observed_review_pairs() {
    let mut host = RealWalkerHost::spawn(profile()).unwrap();
    let draft = write_draft_at(
        &host,
        "skit-x.prompt.py",
        b"print('kind-sensitive name')\n",
        10,
    );
    host.path_map.refresh(&host.service).unwrap();
    let stable_path = host.path_map.normalize_path(&draft);
    let before = host.leak_oracle_facts();
    let renderer = &before.renderer_drafts[&stable_path];
    assert_eq!(renderer.raw_path, draft.display().to_string());
    assert_eq!(renderer.projected_path, stable_path);
    assert_eq!(renderer.raw_kind_picker_basename, "skit-x.prompt.py");
    assert_eq!(renderer.raw_lossy_basename, "skit-x.prompt.py");
    assert_eq!(renderer.projected_basename, "<draft:0>.py");
    assert!(renderer.review_names.is_empty());

    for (kind, raw_name) in [("python", "skit-x.prompt"), ("prompt", "skit-x")] {
        host.path_map.add_cause = AddProjectionCause::ReviewSource(Some(draft.clone()));
        let state = json!({"workflow": {"active": {"add": {
            "review_defaults": {"name": null},
            "review": {
                "kind": kind,
                "name": raw_name,
                "source": {"path": draft},
            },
        }}}});
        host.path_map.reconcile_add_provenance(&state).unwrap();
        let mut screen = json!({"add": state["workflow"]["active"]["add"].clone()});
        host.path_map.normalize_add_screen(&mut screen).unwrap();
        assert_eq!(screen["add"]["review"]["name"], "<draft:0>");
    }

    let after = host.leak_oracle_facts();
    assert!(before.renderer_drafts[&stable_path].review_names.is_empty());
    let review_names = &after.renderer_drafts[&stable_path].review_names;
    assert_eq!(review_names.len(), 2);
    for (kind, raw_name) in [("python", "skit-x.prompt"), ("prompt", "skit-x")] {
        assert!(review_names.iter().any(|fact| {
            fact.kind == kind
                && fact.raw_source_path == draft.display().to_string()
                && fact.projected_source_path == stable_path
                && fact.raw_name == raw_name
                && fact.projected_name == "<draft:0>"
        }));
    }
}

#[cfg(target_os = "linux")]
#[test]
fn c3_renderer_draft_facts_keep_distinct_non_utf_producer_basenames() {
    let mut host = RealWalkerHost::spawn(profile()).unwrap();
    let draft = write_non_utf_draft_at(&host);
    host.path_map.refresh(&host.service).unwrap();
    let stable_path = host.path_map.normalize_path(&draft);

    let facts = host.leak_oracle_facts();
    let renderer = &facts.renderer_drafts[&stable_path];
    assert_eq!(renderer.raw_kind_picker_basename, "");
    assert_eq!(
        renderer.raw_lossy_basename,
        draft.file_name().unwrap().to_string_lossy()
    );
    assert_eq!(renderer.projected_basename, "<draft:0>.unknown");
}
