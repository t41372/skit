use std::{fs::{self,File,OpenOptions},process::Command,thread,time::Duration};

use tempfile::TempDir;

#[test]
fn test_config_lock_serializes_a_real_subprocess(){
    let data=TempDir::new().unwrap();let state=TempDir::new().unwrap();let config=TempDir::new().unwrap();let home=TempDir::new().unwrap();
    let lock_path=config.path().join("config.lock");fs::create_dir_all(config.path()).unwrap();let lock=OpenOptions::new().read(true).write(true).create(true).truncate(false).open(&lock_path).unwrap();if lock.metadata().unwrap().len()==0{lock.set_len(1).unwrap();}lock.lock().unwrap();
    let mut child=Command::new(env!("CARGO_BIN_EXE_skit"));child.env("SKIT_DATA_DIR",data.path()).env("SKIT_STATE_DIR",state.path()).env("SKIT_CONFIG_DIR",config.path()).env("SKIT_LANG","en").env("HOME",home.path()).env("USERPROFILE",home.path()).args(["runner","add","child","--","child","{{prompt}}"]);let mut child=child.spawn().unwrap();
    thread::sleep(Duration::from_millis(150));assert!(child.try_wait().unwrap().is_none(),"subprocess ignored the process-wide config lock");
    lock.unlock().unwrap();let status=child.wait().unwrap();assert!(status.success());
    let raw=fs::read_to_string(config.path().join("config.toml")).unwrap();assert!(raw.contains("name = \"child\""),"{raw}");
}
