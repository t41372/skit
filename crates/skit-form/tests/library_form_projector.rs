use skit_application::library_detail::LibraryFormProjector as _;
use skit_domain::{Entry, EntryKind, EntryMeta, EntrySettings, Slug};
use skit_form::FormLibraryProjector;

fn prompt(settings: EntrySettings) -> Entry {
    let mut meta = EntryMeta::minimal("Prompt", EntryKind::parse("prompt").unwrap());
    settings.write_to_meta(&mut meta);
    Entry {
        slug: Slug::parse("prompt").unwrap(),
        meta,
    }
}

#[test]
fn prompt_bytes_project_fields_and_drift_without_filesystem_access() {
    let entry = prompt(EntrySettings {
        params: vec!["name".to_owned()],
        ..EntrySettings::default()
    });
    let projector = FormLibraryProjector;

    let current = projector.project(&entry, Some(b"Hello {{name}}\r\n"));
    assert_eq!(current.declarations.len(), 1);
    assert_eq!(current.declarations[0].name, "name");
    assert!(!current.drifted);

    let invalid = projector.project(&entry, Some(&[0xff]));
    assert_eq!(invalid.declarations.len(), 1);
    assert!(invalid.drifted);
}

#[test]
fn missing_source_keeps_metadata_only_forms_available() {
    let entry = prompt(EntrySettings {
        params: vec!["name".to_owned()],
        interpolate: false,
        ..EntrySettings::default()
    });
    let facts = FormLibraryProjector.project(&entry, None);
    assert!(facts.declarations.is_empty());
    assert!(!facts.drifted);
}
