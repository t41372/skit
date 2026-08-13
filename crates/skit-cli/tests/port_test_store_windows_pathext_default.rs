#![cfg(windows)]
use std::fs;
use skit_application::EntryRepository as _;
use skit_store::FileStore;
use tempfile::TempDir;

#[test]
fn test_infer_kind_windows_falls_back_to_default_pathext() {
    let data=TempDir::new().unwrap(); let state=TempDir::new().unwrap(); let config=TempDir::new().unwrap(); let home=TempDir::new().unwrap();
    fs::write(config.path().join("config.toml"),"[mirror]\nenabled = false\n").unwrap();
    let source=home.path().join("go.bat"); fs::write(&source,b"payload").unwrap();
    let mut cmd=assert_cmd::cargo::cargo_bin_cmd!("skit");
    let out=cmd.env("SKIT_DATA_DIR",data.path()).env("SKIT_STATE_DIR",state.path()).env("SKIT_CONFIG_DIR",config.path()).env("SKIT_LANG","en").env("HOME",home.path()).env("USERPROFILE",home.path()).env_remove("PATHEXT").current_dir(home.path()).arg("add").arg(&source).args(["--name","default-bat","--no-input"]).output().unwrap();
    assert!(out.status.success(),"stdout={} stderr={}",String::from_utf8_lossy(&out.stdout),String::from_utf8_lossy(&out.stderr));
    assert_eq!(FileStore::new(data.path()).resolve("default-bat").unwrap().meta.kind.as_str(),"exe");
}
