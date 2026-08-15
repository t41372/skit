use std::{fs,path::PathBuf,process::Output};

use assert_cmd::Command;
use serde_json::Value;
use tempfile::TempDir;

struct Sandbox{data:TempDir,state:TempDir,config:TempDir,home:TempDir}
impl Sandbox{
 fn new()->Self{Self{data:TempDir::new().unwrap(),state:TempDir::new().unwrap(),config:TempDir::new().unwrap(),home:TempDir::new().unwrap()}}
 fn command(&self)->Command{let mut c=assert_cmd::cargo::cargo_bin_cmd!("skit");c.env("SKIT_DATA_DIR",self.data.path()).env("SKIT_STATE_DIR",self.state.path()).env("SKIT_CONFIG_DIR",self.config.path()).env("SKIT_LANG","en").env("HOME",self.home.path()).env("USERPROFILE",self.home.path()).current_dir(self.home.path());c}
 fn run(&self,args:&[&str])->Output{self.command().args(args).output().unwrap()}
 fn source(&self)->PathBuf{let p=self.home.path().join("p.prompt.md");fs::write(&p,"{{a}}\n").unwrap();p}
}
fn combined(o:&Output)->String{format!("{}{}",String::from_utf8_lossy(&o.stdout),String::from_utf8_lossy(&o.stderr))}

#[test]
fn test_prompt_plan_unreadable_body_degrades_to_none_plan(){
 let s=Sandbox::new();let p=s.source();let added=s.run(&["add",p.to_str().unwrap(),"--name","p","--no-input"]);assert_eq!(added.status.code(),Some(0),"{}",combined(&added));fs::remove_file(s.data.path().join("scripts/p/prompt.md")).unwrap();
 let shown=s.run(&["show","p","--json"]);assert_eq!(shown.status.code(),Some(0),"{}",combined(&shown));let payload:Value=serde_json::from_slice(&shown.stdout).unwrap();assert_eq!(payload["param_source"],"none","unreadable prompt body must not keep claiming a usable prompt form");assert_eq!(payload["fields"],serde_json::json!([]));assert_eq!(payload["drift"],false);
}
