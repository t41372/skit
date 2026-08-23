use skit_domain::{Entry, EntryKind, EntryMeta, EntrySettings, Slug, StorageMode};
use skit_language::effective_entry_settings;

fn entry(kind: &str, mode: StorageMode, settings: EntrySettings) -> Entry {
    let mut meta = EntryMeta::minimal("Demo", EntryKind::parse(kind).unwrap());
    meta.mode = mode;
    settings.write_to_meta(&mut meta);
    Entry {
        slug: Slug::parse("demo").unwrap(),
        meta,
    }
}

#[test]
fn only_a_python_copy_reads_blank_effective_axes_from_its_source() {
    let source = br#"# /// script
# dependencies = ["httpx"]
# requires-python = ">=3.12"
# ///
print(1)
"#;
    let copy = entry("python", StorageMode::Copy, EntrySettings::default());
    let effective = effective_entry_settings(&copy, Some(source));
    assert_eq!(effective.dependencies, ["httpx"]);
    assert_eq!(effective.requires_python, ">=3.12");

    for candidate in [
        entry("python", StorageMode::Reference, EntrySettings::default()),
        entry("shell", StorageMode::Copy, EntrySettings::default()),
    ] {
        let effective = effective_entry_settings(&candidate, Some(source));
        assert!(effective.dependencies.is_empty());
        assert!(effective.requires_python.is_empty());
    }
}

#[test]
fn stored_effective_axes_and_unrelated_settings_stay_authoritative() {
    let stored = EntrySettings {
        dependencies: vec!["requests".to_owned()],
        requires_python: ">=3.11".to_owned(),
        runner: "kept".to_owned(),
        ..EntrySettings::default()
    };
    let copy = entry("python", StorageMode::Copy, stored.clone());
    let decoy = br#"# /// script
# dependencies = ["decoy"]
# requires-python = ">=9.9"
# ///
"#;
    assert_eq!(effective_entry_settings(&copy, Some(decoy)), stored);
    assert_eq!(effective_entry_settings(&copy, None), stored);
}
