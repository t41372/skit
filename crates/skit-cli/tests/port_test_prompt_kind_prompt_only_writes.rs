use std::process::Output;

use assert_cmd::Command;
use tempfile::TempDir;

struct Sandbox{data:TempDir,state:TempDir,config:TempDir,home:TempDir}
impl Sandbox{fn new()->Self{Self{data:TempDir::new().unwrap(),state:TempDir::new().unwrap(),config:TempDir::new().unwrap(),home:TempDir::new().unwrap()}}fn run(&self,args:&[&str])->Output{let mut c=assert_cmd::cargo::cargo_bin_cmd!("skit");c.env("SKIT_DATA_DIR",self.data.path()).env("SKIT_STATE_DIR",self.state.path()).env("SKIT_CONFIG_DIR",self.config.path()).env("SKIT_LANG","en").env("HOME",self.home.path()).env("USERPROFILE",self.home.path()).current_dir(self.home.path()).args(args).output().unwrap()}}
fn combined(o:&Output)->String{format!("{}{}",String::from_utf8_lossy(&o.stdout),String::from_utf8_lossy(&o.stderr))}

#[test]
fn test_write_prompt_helpers_refuse_non_prompt(){let s=Sandbox::new();let added=s.run(&["add","--cmd","echo {x}","--name","cmd"]);assert_eq!(added.status.code(),Some(0),"{}",combined(&added));for args in [vec!["params","cmd","--runner","claude"],vec!["params","cmd","--no-interpolate"]]{let out=s.run(&args);assert_eq!(out.status.code(),Some(1),"args={args:?}\n{}",combined(&out));assert!(combined(&out).contains("only applies to prompt entries"),"{}",combined(&out));}let show=s.run(&["show","cmd","--json"]);assert_eq!(show.status.code(),Some(0));let json:serde_json::Value=serde_json::from_slice(&show.stdout).unwrap();assert!(json.get("runner").is_none());assert!(json.get("interpolate").is_none());}
