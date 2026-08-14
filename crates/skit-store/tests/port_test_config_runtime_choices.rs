use std::fs;
use skit_store::FileConfigStore;
use tempfile::TempDir;

#[test]
fn test_bash_path_defaults_to_empty(){let r=TempDir::new().unwrap();assert_eq!(FileConfigStore::new(r.path()).get("shell.bash_path").unwrap(),"");}
#[test]
fn test_bash_path_round_trip(){let r=TempDir::new().unwrap();let bash=r.path().join("bash");fs::write(&bash,"").unwrap();let s=FileConfigStore::new(r.path());s.set("shell.bash_path",bash.to_str().unwrap()).unwrap();assert_eq!(s.get("shell.bash_path").unwrap(),bash.to_str().unwrap());}
#[test]
fn test_bash_path_strips_and_clears(){let r=TempDir::new().unwrap();let bash=r.path().join("bash");fs::write(&bash,"").unwrap();let s=FileConfigStore::new(r.path());s.set("shell.bash_path",&format!("  {}  ",bash.display())).unwrap();assert_eq!(s.get("shell.bash_path").unwrap(),bash.to_str().unwrap());s.set("shell.bash_path","").unwrap();assert_eq!(s.get("shell.bash_path").unwrap(),"");let text=fs::read_to_string(r.path().join("config.toml")).unwrap();let doc:toml::Table=toml::from_str(&text).unwrap();assert!(!doc.contains_key("shell"));}
#[test]
fn test_bash_path_garbage_normalizes_to_empty(){let r=TempDir::new().unwrap();fs::write(r.path().join("config.toml"),"[shell]\nbash_path = 123\n").unwrap();assert_eq!(FileConfigStore::new(r.path()).get("shell.bash_path").unwrap(),"");}
#[test]
fn test_bash_path_garbage_section_normalizes_to_empty(){let r=TempDir::new().unwrap();fs::write(r.path().join("config.toml"),"shell = \"not-a-table\"\n").unwrap();assert_eq!(FileConfigStore::new(r.path()).get("shell.bash_path").unwrap(),"");}
#[test]
fn test_bash_path_save_preserves_other_keys(){let r=TempDir::new().unwrap();let bash=r.path().join("bash");fs::write(&bash,"").unwrap();fs::write(r.path().join("config.toml"),"language = \"zh-CN\"\n").unwrap();let s=FileConfigStore::new(r.path());s.set("shell.bash_path",bash.to_str().unwrap()).unwrap();let d:toml::Table=toml::from_str(&fs::read_to_string(r.path().join("config.toml")).unwrap()).unwrap();assert_eq!(d["language"].as_str(),Some("zh-CN"));assert_eq!(d["shell"]["bash_path"].as_str(),Some(bash.to_str().unwrap()));}
#[test]
fn test_bash_path_clear_preserves_other_shell_keys(){let r=TempDir::new().unwrap();fs::write(r.path().join("config.toml"),"[shell]\nbash_path = \"/x\"\nother = \"keep\"\n").unwrap();let s=FileConfigStore::new(r.path());s.set("shell.bash_path","").unwrap();let d:toml::Table=toml::from_str(&fs::read_to_string(r.path().join("config.toml")).unwrap()).unwrap();assert!(d["shell"].get("bash_path").is_none());assert_eq!(d["shell"]["other"].as_str(),Some("keep"));}
#[test]
fn test_js_runner_defaults_to_empty(){let r=TempDir::new().unwrap();assert_eq!(FileConfigStore::new(r.path()).get("js.runner").unwrap(),"");}
#[test]
fn test_js_runner_round_trip(){let r=TempDir::new().unwrap();let s=FileConfigStore::new(r.path());for name in ["deno","bun","node"]{s.set("js.runner",name).unwrap();assert_eq!(s.get("js.runner").unwrap(),name);}}
#[test]
fn rust_additive_js_runner_round_trip_deno(){let r=TempDir::new().unwrap();let s=FileConfigStore::new(r.path());s.set("js.runner","deno").unwrap();assert_eq!(s.get("js.runner").unwrap(),"deno");}
#[test]
fn rust_additive_js_runner_round_trip_bun(){let r=TempDir::new().unwrap();let s=FileConfigStore::new(r.path());s.set("js.runner","bun").unwrap();assert_eq!(s.get("js.runner").unwrap(),"bun");}
#[test]
fn rust_additive_js_runner_round_trip_node(){let r=TempDir::new().unwrap();let s=FileConfigStore::new(r.path());s.set("js.runner","node").unwrap();assert_eq!(s.get("js.runner").unwrap(),"node");}
#[test]
fn test_js_runner_unknown_value_normalizes_to_empty(){let r=TempDir::new().unwrap();fs::write(r.path().join("config.toml"),"[js]\nrunner = \"carrier-pigeon\"\n").unwrap();assert_eq!(FileConfigStore::new(r.path()).get("js.runner").unwrap(),"");}
#[test]
fn test_js_runner_garbage_section_normalizes_to_empty(){let r=TempDir::new().unwrap();fs::write(r.path().join("config.toml"),"js = [\"not\", \"a\", \"table\"]\n").unwrap();assert_eq!(FileConfigStore::new(r.path()).get("js.runner").unwrap(),"");}
#[test]
fn test_js_runner_clears_and_drops_section(){let r=TempDir::new().unwrap();let s=FileConfigStore::new(r.path());s.set("js.runner","deno").unwrap();s.set("js.runner","").unwrap();assert_eq!(s.get("js.runner").unwrap(),"");let d:toml::Table=toml::from_str(&fs::read_to_string(r.path().join("config.toml")).unwrap()).unwrap();assert!(!d.contains_key("js"));}
#[test]
fn test_js_runner_save_preserves_other_keys(){let r=TempDir::new().unwrap();fs::write(r.path().join("config.toml"),"language = \"en\"\n").unwrap();let s=FileConfigStore::new(r.path());s.set("js.runner","bun").unwrap();let d:toml::Table=toml::from_str(&fs::read_to_string(r.path().join("config.toml")).unwrap()).unwrap();assert_eq!(d["language"].as_str(),Some("en"));assert_eq!(d["js"]["runner"].as_str(),Some("bun"));}
