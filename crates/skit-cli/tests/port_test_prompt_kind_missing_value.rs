use std::{fs,path::PathBuf,process::Output};

use assert_cmd::Command;
use tempfile::TempDir;

struct Sandbox{data:TempDir,state:TempDir,config:TempDir,home:TempDir}
impl Sandbox{fn new()->Self{Self{data:TempDir::new().unwrap(),state:TempDir::new().unwrap(),config:TempDir::new().unwrap(),home:TempDir::new().unwrap()}}fn command(&self)->Command{let mut c=assert_cmd::cargo::cargo_bin_cmd!("skit");c.env("SKIT_DATA_DIR",self.data.path()).env("SKIT_STATE_DIR",self.state.path()).env("SKIT_CONFIG_DIR",self.config.path()).env("SKIT_LANG","en").env("HOME",self.home.path()).env("USERPROFILE",self.home.path()).current_dir(self.home.path());c}fn run(&self,args:&[&str])->Output{self.command().args(args).output().unwrap()}fn source(&self,name:&str,body:&str)->PathBuf{let p=self.home.path().join(name);fs::write(&p,body).unwrap();p}}
fn combined(o:&Output)->String{format!("{}{}",String::from_utf8_lossy(&o.stdout),String::from_utf8_lossy(&o.stderr))}

#[test]
fn test_render_body_missing_managed_value_raises(){let s=Sandbox::new();let p=s.source("p.prompt.md","{{target}}\n");let added=s.run(&["add",p.to_str().unwrap(),"--name","p","--runner","claude","--no-input"]);assert_eq!(added.status.code(),Some(0),"{}",combined(&added));let out=s.command().env("PATH","").args(["run","p","--no-input"]).output().unwrap();assert_eq!(out.status.code(),Some(125),"{}",combined(&out));let shown=combined(&out);assert!(shown.contains("target")&&shown.contains("required"),"{shown}");assert!(!shown.contains('→'),"missing managed value reached launch transparency: {shown}");}
