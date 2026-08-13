use std::collections::BTreeMap;
use skit_store::FileConfigStore;
use tempfile::TempDir;

#[test]
fn test_full_mirror_saves_all_four_vectors(){
 let r=TempDir::new().unwrap();let s=FileConfigStore::new(r.path());
 s.set_many(&BTreeMap::from([("mirror.pypi".into(),"tsinghua".into()),("mirror.github".into(),"nju".into()),("mirror.npm".into(),"npmmirror".into())])).unwrap();
 assert!(s.mirror_configured().unwrap());let m=s.mirror().unwrap();assert!(m.enabled);
 assert_eq!(m.pypi,"https://pypi.tuna.tsinghua.edu.cn/simple");assert_eq!(m.python_install,"https://mirror.nju.edu.cn/github-release/astral-sh/python-build-standalone/");assert_eq!(m.uv_binary,"https://mirror.nju.edu.cn/github-release/astral-sh/uv");assert_eq!(m.npm,"https://registry.npmmirror.com");
}
