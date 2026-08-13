//! Model/store ports from Python `tests/test_interpreters.py` at `main@206f9ef`.
//!
//! These tests use Rust's public typed metadata and repository mutation ports. They preserve the
//! Python contracts instead of reconstructing Python-only model helpers inside the test suite.

use skit_application::{
    CreateEntry, EntryMutationRepository as _, EntryPayload, EntryRepository as _, SourcePermissions,
};
use skit_domain::{
    EntryKind, EntryMeta, EntrySettings, StorageMode,
    parameters::{ParamDecl, ParameterDelivery},
};
use skit_store::FileStore;
use tempfile::TempDir;

#[test]
fn test_meta_round_trip_carries_interpreter_needs_parameters() {
    let mut parameter = ParamDecl::new("WIDTH");
    parameter.delivery = ParameterDelivery::Env;
    let settings = EntrySettings {
        interpreter: "zsh".to_owned(),
        needs: vec!["jq".to_owned(), "ffmpeg".to_owned()],
        parameters: vec![parameter.clone()],
        ..EntrySettings::default()
    };
    let mut meta = EntryMeta::minimal("e", EntryKind::parse("shell").unwrap());

    settings.write_to_meta(&mut meta);
    let restored = EntrySettings::from_meta(&meta);

    assert_eq!(restored.interpreter, "zsh");
    assert_eq!(restored.needs, vec!["jq".to_owned(), "ffmpeg".to_owned()]);
    assert_eq!(restored.parameters, vec![parameter]);
}

#[test]
fn test_meta_omits_empty_needs() {
    let mut meta = EntryMeta::minimal("e", EntryKind::parse("shell").unwrap());
    EntrySettings::default().write_to_meta(&mut meta);
    assert!(
        !meta.extra.contains_key("needs"),
        "empty needs must stay absent from minimal metadata: {:?}",
        meta.extra
    );
}

#[test]
fn test_update_needs_sets_and_clears() {
    let data = TempDir::new().unwrap();
    let store = FileStore::new(data.path());
    let entry = store
        .create(CreateEntry {
            name: "d".to_owned(),
            kind: EntryKind::parse("shell").unwrap(),
            mode: StorageMode::Copy,
            source: "/source/d.sh".to_owned(),
            workdir: "origin".to_owned(),
            description: String::new(),
            payload: Some(EntryPayload {
                bytes: b"#!/bin/bash\necho hi\n".to_vec(),
                stored_name: Some("script.sh".to_owned()),
                permissions: SourcePermissions::default(),
            }),
            settings: EntrySettings::default(),
        })
        .unwrap();

    let mut settings = EntrySettings::from_meta(&entry.meta);
    settings.needs = vec!["jq".to_owned(), "ffmpeg".to_owned()];
    let updated = store
        .update_settings(&entry, &settings, &entry.meta.workdir)
        .unwrap();
    assert_eq!(
        EntrySettings::from_meta(&updated.meta).needs,
        vec!["jq".to_owned(), "ffmpeg".to_owned()]
    );
    assert_eq!(
        EntrySettings::from_meta(&store.resolve("d").unwrap().meta).needs,
        vec!["jq".to_owned(), "ffmpeg".to_owned()]
    );

    let mut cleared_settings = EntrySettings::from_meta(&updated.meta);
    cleared_settings.needs.clear();
    let cleared = store
        .update_settings(&updated, &cleared_settings, &updated.meta.workdir)
        .unwrap();
    assert!(EntrySettings::from_meta(&cleared.meta).needs.is_empty());
    assert!(
        !cleared.meta.extra.contains_key("needs"),
        "clearing needs must restore minimal metadata: {:?}",
        cleared.meta.extra
    );
}
