use std::fs;

use skit_application::{
    CreateEntry, EntryMutationRepository as _, EntryPayload, EntryRepository as _,
    SourcePermissions, prompt_selection::PromptSelectionStore as _,
};
use skit_domain::{EntryKind, EntryMeta, EntrySettings, StorageMode};
use skit_store::{FilePromptSelectionStore, FileStore};
use tempfile::TempDir;

#[test]
fn test_last_runner_roundtrip_and_corruption_degrades() {
    let root=TempDir::new().unwrap(); let state=FilePromptSelectionStore::new(root.path());
    assert_eq!(state.load_last_runner(),"");
    state.save_last_runner("codex").unwrap(); assert_eq!(state.load_last_runner(),"codex");
    fs::write(root.path().join("prompt.toml"),"not = [toml").unwrap(); assert_eq!(state.load_last_runner(),"");
    fs::write(root.path().join("prompt.toml"),"last_runner = 3").unwrap(); assert_eq!(state.load_last_runner(),"");
}

#[test]
fn test_meta_interpolate_round_trip_and_garbage_tolerance() {
    let kind=EntryKind::parse("prompt").unwrap();
    let mut meta=EntryMeta::minimal("p",kind.clone());
    let settings=EntrySettings{interpolate:false,..EntrySettings::default()}; settings.write_to_meta(&mut meta);
    assert_eq!(meta.extra.get("interpolate"),Some(&serde_json::Value::Bool(false)));
    assert!(!EntrySettings::from_meta(&meta).interpolate);

    let mut default_meta=EntryMeta::minimal("p",kind.clone()); EntrySettings::default().write_to_meta(&mut default_meta);
    assert!(!default_meta.extra.contains_key("interpolate"),"default true must stay omitted for old metadata compatibility");
    let mut garbage=EntryMeta::minimal("p",kind); garbage.extra.insert("interpolate".to_owned(),serde_json::Value::String("no".to_owned()));
    assert!(EntrySettings::from_meta(&garbage).interpolate,"only genuine boolean false may disable insertion");
}

#[test]
fn test_meta_rejects_wrong_typed_runner_at_the_corruption_boundary() {
    let data=TempDir::new().unwrap();
    let store=FileStore::new(data.path());
    let entry=store.create(CreateEntry{
        name:"p".to_owned(),
        kind:EntryKind::parse("prompt").unwrap(),
        mode:StorageMode::Copy,
        source:"/source/p.prompt.md".to_owned(),
        workdir:"invoke".to_owned(),
        description:String::new(),
        payload:Some(EntryPayload{bytes:b"hello\n".to_vec(),stored_name:Some("prompt.md".to_owned()),permissions:SourcePermissions::default()}),
        settings:EntrySettings::default(),
    }).unwrap();
    // Prove the control fixture is a fully valid registry + entry before corrupting one field.
    assert_eq!(store.resolve(entry.slug.as_str()).unwrap().meta.name,"p");

    let meta_path=data.path().join("scripts/p/meta.toml");
    let mut raw=fs::read_to_string(&meta_path).unwrap();
    raw.push_str("runner = 123\n");
    fs::write(&meta_path,raw).unwrap();

    let error=store.resolve("p").expect_err("wrong-typed runner was silently treated as an empty/default runner");
    let shown=error.to_string();
    assert!(shown.to_ascii_lowercase().contains("runner"),"corruption error did not identify the runner field: {shown}");
}
