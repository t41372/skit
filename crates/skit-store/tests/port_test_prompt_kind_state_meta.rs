use std::fs;

use skit_application::{EntryRepository as _, prompt_selection::PromptSelectionStore as _};
use skit_domain::{EntryKind, EntryMeta, EntrySettings};
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
    let mut settings=EntrySettings{interpolate:false,..EntrySettings::default()}; settings.write_to_meta(&mut meta);
    assert_eq!(meta.extra.get("interpolate"),Some(&serde_json::Value::Bool(false)));
    assert!(!EntrySettings::from_meta(&meta).interpolate);

    let mut default_meta=EntryMeta::minimal("p",kind.clone()); EntrySettings::default().write_to_meta(&mut default_meta);
    assert!(!default_meta.extra.contains_key("interpolate"),"default true must stay omitted for old metadata compatibility");
    let mut garbage=EntryMeta::minimal("p",kind); garbage.extra.insert("interpolate".to_owned(),serde_json::Value::String("no".to_owned()));
    assert!(EntrySettings::from_meta(&garbage).interpolate,"only genuine boolean false may disable insertion");
}

#[test]
fn test_meta_rejects_wrong_typed_runner_at_the_corruption_boundary() {
    let data=TempDir::new().unwrap(); let dir=data.path().join("scripts/p"); fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("prompt.md"),"hello\n").unwrap();
    fs::write(dir.join("meta.toml"),concat!("schema = 1\nname = \"p\"\nkind = \"prompt\"\nmode = \"copy\"\nsource = \"/source/p.prompt.md\"\nsource_hash = \"\"\nadded_at = \"2026-08-14T00:00:00Z\"\nid = \"0123456789abcdef0123456789abcdef\"\nworkdir = \"invoke\"\ndescription = \"\"\nrunner = 123\n")).unwrap();
    fs::write(data.path().join("registry.toml"),"[entries.p]\n").unwrap();
    assert!(FileStore::new(data.path()).resolve("p").is_err(),"wrong-typed runner was silently treated as an empty/default runner");
}
