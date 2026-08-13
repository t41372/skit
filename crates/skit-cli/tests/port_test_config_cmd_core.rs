use std::{collections::BTreeSet, fs};
use assert_cmd::Command;
use serde_json::Value;
use tempfile::TempDir;

struct Sandbox{data:TempDir,state:TempDir,config:TempDir,home:TempDir}
impl Sandbox{
 fn new()->Self{Self{data:TempDir::new().unwrap(),state:TempDir::new().unwrap(),config:TempDir::new().unwrap(),home:TempDir::new().unwrap()}}
 fn command(&self)->Command{let mut c=assert_cmd::cargo::cargo_bin_cmd!("skit");c.env("SKIT_DATA_DIR",self.data.path()).env("SKIT_STATE_DIR",self.state.path()).env("SKIT_CONFIG_DIR",self.config.path()).env("SKIT_LANG","en").env("HOME",self.home.path()).env("USERPROFILE",self.home.path()).current_dir(self.home.path());c}
 fn json(&self,args:&[&str])->Value{let o=self.command().args(args).output().unwrap();assert!(o.status.success(),"{}{}",String::from_utf8_lossy(&o.stdout),String::from_utf8_lossy(&o.stderr));serde_json::from_slice(&o.stdout).unwrap()}
}

#[test]
fn test_bare_config_lists_all_keys(){let s=Sandbox::new();let o=s.command().arg("config").output().unwrap();let t=String::from_utf8_lossy(&o.stdout);assert!(o.status.success(),"{}",String::from_utf8_lossy(&o.stderr));for k in["lang","editor","mirror","form","after_run"]{assert!(t.contains(k),"{t}");}assert!(t.contains("off"),"{t}");assert!(t.contains("tui"),"{t}");}
#[test]
fn test_bare_config_json(){let s=Sandbox::new();let d=s.json(&["config","--json"]);let obj=d.as_object().unwrap();let actual=obj.keys().cloned().collect::<BTreeSet<_>>();let expected=["lang","editor","mirror","mirror.pypi","mirror.github","mirror.npm","form","after_run","shell.bash_path","js.runner"].into_iter().map(str::to_owned).collect::<BTreeSet<_>>();assert_eq!(actual,expected);assert_eq!(d["mirror"],"off");assert_eq!(d["mirror.pypi"],"off");assert_eq!(d["mirror.github"],"off");assert_eq!(d["mirror.npm"],"off");assert_eq!(d["form"],"tui");assert_eq!(d["after_run"],"exit");}
#[test]
fn test_read_one_key_json_emits_single_pair(){let s=Sandbox::new();assert_eq!(s.json(&["config","form","--json"]),serde_json::json!({"form":"tui"}));}
#[test]
fn test_set_one_key_json_emits_final_pair(){let s=Sandbox::new();assert_eq!(s.json(&["config","form","plain","--json"]),serde_json::json!({"form":"plain"}));let d:toml::Table=toml::from_str(&fs::read_to_string(s.config.path().join("config.toml")).unwrap()).unwrap();assert_eq!(d["form"].as_str(),Some("plain"));}
#[test]
fn test_unknown_key_exits_2(){let s=Sandbox::new();let o=s.command().args(["config","theme"]).output().unwrap();assert_eq!(o.status.code(),Some(2));}
#[test]
fn test_set_lang_writes_language_key(){let s=Sandbox::new();s.command().args(["config","lang","zh-CN"]).assert().success();let d:toml::Table=toml::from_str(&fs::read_to_string(s.config.path().join("config.toml")).unwrap()).unwrap();assert_eq!(d["language"].as_str(),Some("zh-CN"));}
#[test]
fn test_read_lang_shows_override(){let s=Sandbox::new();s.command().args(["config","lang","zh-CN"]).assert().success();let o=s.command().args(["config","lang"]).output().unwrap();assert!(o.status.success());assert!(String::from_utf8_lossy(&o.stdout).contains("zh-CN"));}
#[test]
fn test_lang_auto_clears(){let s=Sandbox::new();s.command().args(["config","lang","zh-CN"]).assert().success();s.command().args(["config","lang","auto"]).assert().success();let d:toml::Table=toml::from_str(&fs::read_to_string(s.config.path().join("config.toml")).unwrap()).unwrap();assert!(!d.contains_key("language"));}
#[test]
fn test_unknown_lang_exits_2(){let s=Sandbox::new();let o=s.command().args(["config","lang","xx-YY"]).output().unwrap();assert_eq!(o.status.code(),Some(2));}
