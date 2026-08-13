use std::{fs,time::UNIX_EPOCH};
use skit_application::{CreateEntry,EntryMutationRepository as _,EntryPayload,EntryRepository as _,SourcePermissions};
use skit_domain::{EntryKind,EntrySettings,StorageMode,parameters::ParamDecl};
use skit_store::FileStore;
use tempfile::TempDir;
use toml::{Table,Value};

fn add(store:&FileStore,name:&str,kind:&str)->skit_domain::Entry{
    store.create(CreateEntry{name:name.into(),kind:EntryKind::parse(kind).unwrap(),mode:StorageMode::Copy,source:format!("/origin/{name}"),workdir:"invoke".into(),description:"old".into(),payload:Some(EntryPayload{bytes:if kind=="prompt"{b"Summarize {{topic}}\n".to_vec()}else{b"print(1)\n".to_vec()},stored_name:Some(if kind=="prompt"{"prompt.md".into()}else{"script.py".into()}),permissions:SourcePermissions::default()}),settings:EntrySettings::default()}).unwrap()
}

fn assert_fresh(root:&TempDir,store:&FileStore,name:&str){
    let e=store.resolve(name).unwrap();
    let meta=root.path().join("scripts").join(e.slug.as_str()).join("meta.toml");
    let ns=i64::try_from(fs::metadata(meta).unwrap().modified().unwrap().duration_since(UNIX_EPOCH).unwrap().as_nanos()).unwrap();
    let doc:Table=toml::from_str(&fs::read_to_string(root.path().join("registry.toml")).unwrap()).unwrap();
    let row=doc["entries"][e.slug.as_str()].as_table().unwrap();
    assert_eq!(row["name"].as_str(),Some(e.meta.name.as_str()));
    assert_eq!(row["description"].as_str(),Some(e.meta.description.as_str()));
    assert_eq!(row["mtime_ns"].as_integer(),Some(ns));
    assert!(matches!(row.get("skit_cache"),Some(Value::Table(_))));
    let before=fs::read(root.path().join("registry.toml")).unwrap();
    let _=store.scan().unwrap();
    assert_eq!(fs::read(root.path().join("registry.toml")).unwrap(),before);
}

#[test]
fn test_a_meta_mutator_leaves_a_row_the_next_listing_serves_untouched(){
    let root=TempDir::new().unwrap(); let store=FileStore::new(root.path());

    let e=add(&store,"needs","python"); let mut s=EntrySettings::from_meta(&e.meta); s.needs=vec!["ffmpeg".into()]; store.update_settings(&e,&s,"invoke").unwrap(); assert_fresh(&root,&store,"needs");
    let e=add(&store,"params","python"); let mut s=EntrySettings::from_meta(&e.meta); s.parameters=vec![ParamDecl::new("CITY")]; store.update_settings(&e,&s,"invoke").unwrap(); assert_fresh(&root,&store,"params");
    let e=add(&store,"deps","python"); let mut s=EntrySettings::from_meta(&e.meta); s.dependencies=vec!["httpx".into()]; store.update_settings(&e,&s,"invoke").unwrap(); assert_fresh(&root,&store,"deps");
    let e=add(&store,"workdir","python"); store.update_settings(&e,&EntrySettings::from_meta(&e.meta),"store").unwrap(); assert_fresh(&root,&store,"workdir");
    let e=add(&store,"description","python"); store.describe(&e,"new").unwrap(); assert_fresh(&root,&store,"description");
    let e=add(&store,"prompt","prompt"); let mut s=EntrySettings::from_meta(&e.meta); s.params=vec!["topic".into()]; store.update_settings(&e,&s,"invoke").unwrap(); assert_fresh(&root,&store,"prompt");
}
