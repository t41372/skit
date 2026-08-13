use std::collections::BTreeMap;
use skit_store::FileConfigStore;
use tempfile::TempDir;

fn full() -> (TempDir, FileConfigStore) {
    let root=TempDir::new().unwrap(); let store=FileConfigStore::new(root.path());
    store.set_many(&BTreeMap::from([
        ("mirror.pypi".into(),"tsinghua".into()),
        ("mirror.github".into(),"nju".into()),
        ("mirror.npm".into(),"npmmirror".into()),
    ])).unwrap(); (root,store)
}

#[test]
fn test_full_mirror_saves_all_four_vectors(){let (_r,s)=full();let m=s.mirror().unwrap();assert!(m.enabled);assert_eq!(m.pypi,"https://pypi.tuna.tsinghua.edu.cn/simple");assert_eq!(m.python_install,"https://mirror.nju.edu.cn/github-release/astral-sh/python-build-standalone/");assert_eq!(m.uv_binary,"https://mirror.nju.edu.cn/github-release/astral-sh/uv");assert_eq!(m.npm,"https://registry.npmmirror.com");}
#[test]
fn test_axis_choice_readers(){let (_r,s)=full();let v=s.settings().unwrap();assert_eq!(v["mirror.pypi"],"tsinghua");assert_eq!(v["mirror.github"],"nju");assert_eq!(v["mirror.npm"],"npmmirror");}
#[test]
fn test_axis_choice_readers_are_blind_to_the_master_switch(){let (_r,s)=full();s.set("mirror","off").unwrap();let v=s.settings().unwrap();assert_eq!(v["mirror.pypi"],"tsinghua");assert_eq!(v["mirror.github"],"nju");assert_eq!(v["mirror.npm"],"npmmirror");}
#[test]
fn test_update_mirror_axes_fresh_url_auto_enables(){let r=TempDir::new().unwrap();let s=FileConfigStore::new(r.path());s.set("mirror.pypi","tsinghua").unwrap();let m=s.mirror().unwrap();assert!(m.enabled);assert_eq!(m.pypi,"https://pypi.tuna.tsinghua.edu.cn/simple");}
#[test]
fn test_update_mirror_axes_enabled_stays_on_while_a_url_remains(){let (_r,s)=full();s.set("mirror.pypi","off").unwrap();let m=s.mirror().unwrap();assert!(m.enabled);assert_eq!(m.pypi,"");assert_eq!(m.npm,"https://registry.npmmirror.com");}
#[test]
fn test_update_mirror_axes_paused_stays_paused_and_preserves_others(){let (_r,s)=full();s.set("mirror","off").unwrap();s.set("mirror.npm","https://new.example/npm").unwrap();let m=s.mirror().unwrap();assert!(!m.enabled);assert_eq!(m.npm,"https://new.example/npm");assert_eq!(m.pypi,"https://pypi.tuna.tsinghua.edu.cn/simple");}
#[test]
fn test_update_mirror_axes_none_leaves_axes_untouched(){let (_r,s)=full();s.set("mirror.npm","https://new.example/npm").unwrap();let m=s.mirror().unwrap();assert_eq!(m.pypi,"https://pypi.tuna.tsinghua.edu.cn/simple");assert_eq!(m.python_install,"https://mirror.nju.edu.cn/github-release/astral-sh/python-build-standalone/");assert_eq!(m.uv_binary,"https://mirror.nju.edu.cn/github-release/astral-sh/uv");}
#[test]
fn test_enable_restores_saved_urls(){let (_r,s)=full();s.set("mirror","off").unwrap();s.set("mirror","on").unwrap();let m=s.mirror().unwrap();assert!(m.enabled);assert_eq!(m.pypi,"https://pypi.tuna.tsinghua.edu.cn/simple");}
