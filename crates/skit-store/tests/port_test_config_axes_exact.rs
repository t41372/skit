use skit_store::FileConfigStore;
use tempfile::TempDir;

#[test]
fn test_axes_are_independent(){
 let r=TempDir::new().unwrap();let s=FileConfigStore::new(r.path());s.set("mirror.pypi","aliyun").unwrap();let m=s.mirror().unwrap();assert_eq!((m.python_install.as_str(),m.uv_binary.as_str(),m.npm.as_str()),("","",""));
 let r=TempDir::new().unwrap();let s=FileConfigStore::new(r.path());s.set("mirror.npm","npmmirror").unwrap();let m=s.mirror().unwrap();assert_eq!((m.pypi.as_str(),m.python_install.as_str(),m.uv_binary.as_str()),("","",""));
 let r=TempDir::new().unwrap();let s=FileConfigStore::new(r.path());s.set("mirror.github","nju").unwrap();let m=s.mirror().unwrap();assert_eq!((m.pypi.as_str(),m.npm.as_str()),("",""));
}
#[test]
fn test_github_release_urls_expand_from_one_base(){let r=TempDir::new().unwrap();let s=FileConfigStore::new(r.path());s.set("mirror.github","https://my.mirror/gh/").unwrap();let m=s.mirror().unwrap();assert_eq!(m.python_install,"https://my.mirror/gh/astral-sh/python-build-standalone/");assert_eq!(m.uv_binary,"https://my.mirror/gh/astral-sh/uv");}
