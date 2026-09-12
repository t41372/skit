//! Path spelling contracts.

use super::*;

#[cfg(target_os = "linux")]
#[test]
fn tree_observation_escapes_non_utf8_paths_without_dropping_bytes() {
    use std::os::unix::ffi::OsStringExt as _;

    let mut host = RealWalkerHost::spawn(profile()).unwrap();
    let path = host
        .roots()
        .state
        .join(std::ffi::OsString::from_vec(b"opaque-\xff".to_vec()));
    fs::write(&path, b"\x00\xff\x10").unwrap();

    let observation = host.observe(&host.initial_state().unwrap()).unwrap();

    assert!(observation.tree.iter().any(|row| {
        row.path == "state/opaque-\\xff"
            && matches!(&row.content, Some(ByteView::Hex(bytes)) if bytes == "00ff10")
    }));
}

#[cfg(unix)]
#[test]
fn tree_observation_keeps_a_leaked_temporary_file() {
    let mut host = RealWalkerHost::spawn(profile()).unwrap();
    fs::write(host.roots().data.join("leaked.tmp-1234"), b"visible").unwrap();

    let observation = host.observe(&host.initial_state().unwrap()).unwrap();

    assert!(
        observation
            .tree
            .iter()
            .any(|row| row.path == "data/leaked.tmp-1234")
    );
}
/// Every case of the Windows verbatim-prefix rule, as text.
///
/// The rule decides which spelling the fixture looks up and compares. No Windows host runs
/// these tests, so the cases are literal text and they run on every platform.
#[test]
fn the_verbatim_prefix_rule_keeps_the_ordinary_spelling_of_one_location() {
    for (recorded, ordinary) in [
        (r"\\?\C:\a\b", r"C:\a\b"),
        (r"\\?\c:\", r"c:\"),
        (r"\\?\UNC\srv\share\x", r"\\srv\share\x"),
        (r"C:\a\b", r"C:\a\b"),
        (r"\\srv\share\x", r"\\srv\share\x"),
        (r"\\?\Volume{0}\x", r"\\?\Volume{0}\x"),
        (r"\\?\C:", r"\\?\C:"),
        (r"\\?\1:\a", r"\\?\1:\a"),
        (r"\\?\", r"\\?\"),
        ("/tmp/a/b", "/tmp/a/b"),
        ("", ""),
    ] {
        assert_eq!(ordinary_windows_path(recorded), ordinary, "{recorded}");
        assert_eq!(
            *ordinary_path(Path::new(recorded)),
            *Path::new(ordinary),
            "{recorded}"
        );
        assert!(
            names_the_same_path(recorded, Path::new(ordinary)),
            "{recorded}"
        );
        assert!(
            !names_the_same_path(recorded, Path::new("/other")),
            "{recorded}"
        );
    }
}

#[cfg(unix)]
#[test]
fn one_path_that_is_not_unicode_keeps_its_exact_spelling() {
    use std::os::unix::ffi::OsStringExt as _;

    let path = PathBuf::from(std::ffi::OsString::from_vec(b"/tmp/opaque-\xff".to_vec()));

    assert_eq!(*ordinary_path(&path), *path);
}

#[cfg(windows)]
#[test]
fn one_path_that_is_not_unicode_keeps_its_exact_spelling() {
    use std::os::windows::ffi::OsStringExt as _;

    let path = PathBuf::from(std::ffi::OsString::from_wide(&[0xd800]));

    assert_eq!(*ordinary_path(&path), *path);
}

#[cfg(unix)]
#[test]
fn resolved_profile_paths_keep_the_declared_fixture_spelling() {
    let host = RealWalkerHost::spawn(profile()).unwrap();
    let parent = tempfile::TempDir::new().unwrap();
    let alias = parent.path().join("profile-alias");
    std::os::unix::fs::symlink(host.sandbox_root(), &alias).unwrap();
    let mut paths = PathMap::new(
        &host.profile,
        &alias,
        &host.service,
        &host.file_picker_tree.files,
    )
    .unwrap();
    let draft = alias.join("data/.drafts/skit-new-first.py");
    paths.register_draft_path(&draft);
    let resolved = host.sandbox_root().join("data/.drafts/skit-new-first.py");
    let projected = paths.normalize_path(&draft);
    assert!(paths.is_registered_path(&resolved));
    assert_eq!(paths.normalize_path(&resolved), projected);
    assert_eq!(
        paths.normalize_host_text(resolved.to_str().unwrap()),
        projected
    );
    let facts = paths.leak_oracle_facts();
    let root = &facts.artifacts["<profile:fixture>"].raw_path_spellings;
    assert!(root.contains(alias.to_str().unwrap()));
    assert!(root.contains(host.sandbox_root().to_str().unwrap()));
}

/// The fixture owns one path in the verbatim spelling of it.
///
/// Windows `fs::canonicalize` returns the verbatim form, so a path that the product recorded
/// arrives in that form. The lookup keys stay in the ordinary spelling.
#[test]
fn a_registered_path_accepts_the_windows_verbatim_spelling_of_itself() {
    let mut host = RealWalkerHost::spawn(profile()).unwrap();
    let draft = Path::new(r"C:\sandbox\data\.drafts\skit-new-first.py");
    host.path_map.register_draft_path(draft);
    host.path_map
        .register_owned_transient_path(Path::new(r"C:\sandbox\state\run.tmp"), "run", "");
    let stable = host.path_map.normalize_path(draft);

    for (recorded, expected) in [
        (r"\\?\C:\sandbox\data\.drafts\skit-new-first.py", &stable),
        (r"C:\sandbox\data\.drafts\skit-new-first.py", &stable),
    ] {
        assert!(host.path_map.is_registered_path(Path::new(recorded)));
        assert_eq!(host.path_map.normalize_path(Path::new(recorded)), *expected);
    }
    assert!(
        host.path_map
            .is_registered_path(Path::new(r"\\?\C:\sandbox\state\run.tmp"))
    );
    let stranger = Path::new(r"\\?\C:\sandbox\data\.drafts\stranger.py");
    let outside = host.path_map.normalize_path(stranger);
    assert!(!host.path_map.is_registered_path(stranger));
    assert_ne!(outside, stable);
    assert!(outside.contains("stranger.py"), "{outside}");
}

/// One Add Commit that the product recorded in the Windows verbatim spelling.
///
/// The product resolves `source_record` and `CreateEntry::source`, so both arrive with the
/// `\\?\` prefix while the draft path keeps skit's own spelling. The oracle must accept the
/// pair, and no verbatim text may reach the projected artifact.
#[test]
fn a_commit_accepts_the_windows_verbatim_spelling_of_its_recorded_source() {
    let mut host = RealWalkerHost::spawn(profile()).unwrap();
    let draft = Path::new(r"C:\sandbox\data\.drafts\skit-new-first.py");
    let recorded = r"\\?\C:\sandbox\data\.drafts\skit-new-first.py";
    host.path_map.register_draft_path(draft);
    let stable_path = host.path_map.normalize_path(draft);
    let raw_name = review_default_name(draft, "python");
    let stable_name = review_default_name(Path::new(&stable_path), "python");
    host.path_map.add_provenance.review_source_path = Some(draft.to_path_buf());
    set_review_name_projection(&mut host, draft, &raw_name, &stable_name);

    let commit_value = |record: &str| {
        let mut entry = serde_json::to_value(profile().entries[1].clone()).unwrap();
        entry["name"] = json!(raw_name);
        entry["kind"] = json!("python");
        entry["mode"] = json!("copy");
        entry["source"] = json!(record);
        json!({"add": [{"commit": {
            "request": 0,
            "entry": entry,
            "source": {
                "path": draft.display().to_string(),
                "source_record": record,
                "bytes": [1, 2, 3],
                "permissions": {"readonly": false, "unix_mode": 384},
                "executable": false,
                "is_regular": true,
                "is_directory": false,
                "is_draft": true,
                "identity": null,
            },
        }}]})
    };
    let mut effect: Effect =
        deserialize_canonical(commit_value(recorded), "verbatim Commit fixture").unwrap();
    host.path_map.project_typed_effect(&mut effect).unwrap();
    let projected = serde_json::to_value(effect).unwrap();

    let commit = &projected["add"][0]["commit"];
    assert_eq!(commit["entry"]["name"], json!(stable_name));
    assert_eq!(commit["entry"]["source"], json!(stable_path));
    assert_eq!(commit["source"]["path"], json!(stable_path));
    assert_eq!(commit["source"]["source_record"], json!(stable_path));

    let mut stranger: Effect = deserialize_canonical(
        commit_value(r"\\?\C:\sandbox\data\.drafts\stranger.py"),
        "verbatim Commit stranger fixture",
    )
    .unwrap();
    assert!(host.path_map.project_typed_effect(&mut stranger).is_err());
}
