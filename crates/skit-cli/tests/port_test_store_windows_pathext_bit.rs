#![cfg(windows)]
use std::fs;
use skit_application::EntryRepository as _;
use skit_store::FileStore;
use tempfile::TempDir;

fn add(filename:&str,name:&str,data:&TempDir,state:&TempDir,config:&TempDir,home:&TempDir)->String{
    let source=home.path().join(filename); fs::write(&source,b"payload").unwrap();
    let mut cmd=assert_cmd::cargo::cargo_bin_cmd!("skit");
    let out=cmd.env("SKIT_DATA_DIR",data.path()).env("SKIT_STATE_DIR",state.path()).env("SKIT_CONFIG_DIR",config.path()).env("SKIT_LANG","en").env("HOME",home.path()).env("USERPROFILE",home.path()).env("PATHEXT",".COM;.EXE;.BAT;.CMD").current_dir(home.path()).arg("add").arg(&source).args(["--name",name,"--no-input"]).output().unwrap();
    assert!(out.status.success(),"stdout={} stderr={}",String::from_utf8_lossy(&out.stdout),String::from_utf8_lossy(&out.stderr));
    FileStore::new(data.path()).resolve(name).unwrap().meta.kind.as_str().to_owned()
}

#[test]
fn test_infer_kind_windows_uses_pathext_not_execute_bit(){
    let data=TempDir::new().unwrap(); let state=TempDir::new().unwrap(); let config=TempDir::new().unwrap(); let home=TempDir::new().unwrap();
    fs::write(config.path().join("config.toml"),"[mirror]\nenabled = false\n").unwrap();
    assert_eq!(add("tool.exe","exe-tool",&data,&state,&config,&home),"exe");
    assert_eq!(add("run.BAT","bat-tool",&data,&state,&config,&home),"exe");
    assert_eq!(add("notes.txt","text-tool",&data,&state,&config,&home),"unknown");
}
