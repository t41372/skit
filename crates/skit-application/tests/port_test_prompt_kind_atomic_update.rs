use skit_application::{CreateEntry,EntryMutationRepository as _,EntryRepository as _,LibraryService,SourcePermissions,UpdateEntry};
use skit_domain::{EntryKind,EntrySettings,StorageMode,parameters::synthesized_placeholder};
use skit_store::FileStore;
use tempfile::TempDir;

#[test]
fn test_write_prompt_managed_and_runner_lock_atomically(){
 let data=TempDir::new().unwrap();let source=data.path().join("source.prompt.md");std::fs::write(&source,"{{a}} {{b}}\n").unwrap();let store=FileStore::new(data.path());let service=LibraryService::new(store.clone());
 let entry=service.add(CreateEntry{name:"p".to_owned(),kind:EntryKind::parse("prompt").unwrap(),mode:StorageMode::Copy,source:source.display().to_string(),workdir:"invoke".to_owned(),description:String::new(),payload:Some(skit_application::EntryPayload{bytes:b"{{a}} {{b}}\n".to_vec(),stored_name:Some("prompt.md".to_owned()),permissions:SourcePermissions::default()}),settings:EntrySettings::default()}).unwrap();
 let claimed=service.claim_identity(&entry).unwrap();let settings=EntrySettings{params:vec!["b".to_owned()],parameters:vec![synthesized_placeholder("b")],runner:"codex".to_owned(),..EntrySettings::default()};
 service.update_entry(&claimed,UpdateEntry{name:entry.meta.name.clone(),description:entry.meta.description.clone(),settings:settings.clone(),workdir:entry.meta.workdir.clone(),source:None,expected_source_hash:entry.meta.source_hash.clone()}).unwrap();
 let loaded=service.show("p").unwrap();let got=EntrySettings::from_meta(&loaded.meta);assert_eq!(got.params,["b"]);assert_eq!(got.runner,"codex");assert_eq!(got.parameters.len(),1);assert_eq!(got.parameters[0].name,"b");
}
