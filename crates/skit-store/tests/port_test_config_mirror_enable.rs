use std::fs;
use skit_store::FileConfigStore;
use tempfile::TempDir;

#[test]
fn test_update_mirror_axes_off_on_empty_stays_off(){let r=TempDir::new().unwrap();let s=FileConfigStore::new(r.path());s.set("mirror.pypi","off").unwrap();assert!(!s.mirror().unwrap().enabled);}
#[test]
fn test_update_mirror_axes_clearing_the_last_url_disables(){let r=TempDir::new().unwrap();let s=FileConfigStore::new(r.path());s.set("mirror.npm","npmmirror").unwrap();s.set("mirror.npm","off").unwrap();let m=s.mirror().unwrap();assert!(!m.enabled);assert_eq!(m.npm,"");}
#[test]
fn test_enable_works_for_each_single_axis(){
 for (key,value) in [("pypi","https://x/simple"),("python_install","https://x/py/"),("uv_binary","https://x/uv"),("npm","https://x/npm")]{
  let r=TempDir::new().unwrap();let path=r.path().join("config.toml");fs::write(&path,format!("[mirror]\nenabled = false\n{key} = {value:?}\n")).unwrap();let s=FileConfigStore::new(r.path());s.set("mirror","on").unwrap();assert!(s.mirror().unwrap().enabled,"{key}");
 }
}
#[test]
fn rust_additive_enable_works_for_pypi_only(){let r=TempDir::new().unwrap();fs::write(r.path().join("config.toml"),"[mirror]\nenabled = false\npypi = \"https://x/simple\"\n").unwrap();let s=FileConfigStore::new(r.path());s.set("mirror","on").unwrap();assert!(s.mirror().unwrap().enabled);}
#[test]
fn rust_additive_enable_works_for_python_install_only(){let r=TempDir::new().unwrap();fs::write(r.path().join("config.toml"),"[mirror]\nenabled = false\npython_install = \"https://x/py/\"\n").unwrap();let s=FileConfigStore::new(r.path());s.set("mirror","on").unwrap();assert!(s.mirror().unwrap().enabled);}
#[test]
fn rust_additive_enable_works_for_uv_binary_only(){let r=TempDir::new().unwrap();fs::write(r.path().join("config.toml"),"[mirror]\nenabled = false\nuv_binary = \"https://x/uv\"\n").unwrap();let s=FileConfigStore::new(r.path());s.set("mirror","on").unwrap();assert!(s.mirror().unwrap().enabled);}
#[test]
fn rust_additive_enable_works_for_npm_only(){let r=TempDir::new().unwrap();fs::write(r.path().join("config.toml"),"[mirror]\nenabled = false\nnpm = \"https://x/npm\"\n").unwrap();let s=FileConfigStore::new(r.path());s.set("mirror","on").unwrap();assert!(s.mirror().unwrap().enabled);}
#[test]
fn test_enable_refuses_when_nothing_saved(){let r=TempDir::new().unwrap();let s=FileConfigStore::new(r.path());let e=s.set("mirror","on").unwrap_err();assert!(e.is_usage());assert!(!s.mirror().unwrap().enabled);}
