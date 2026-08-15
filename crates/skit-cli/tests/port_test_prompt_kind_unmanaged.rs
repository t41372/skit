use std::{fs,path::PathBuf,process::Output};

use assert_cmd::Command;
use serde_json::{Value,json};
use skit_application::{CreateEntry,EntryMutationRepository as _,EntryPayload,SourcePermissions};
use skit_domain::{EntryKind,EntrySettings,StorageMode,parameters::synthesized_placeholder};
use skit_store::FileStore;
use tempfile::TempDir;

struct Sandbox{data:TempDir,state:TempDir,config:TempDir,home:TempDir}
impl Sandbox{
 fn new()->Self{Self{data:TempDir::new().unwrap(),state:TempDir::new().unwrap(),config:TempDir::new().unwrap(),home:TempDir::new().unwrap()}}
 fn command(&self)->Command{let mut c=assert_cmd::cargo::cargo_bin_cmd!("skit");c.env("SKIT_DATA_DIR",self.data.path()).env("SKIT_STATE_DIR",self.state.path()).env("SKIT_CONFIG_DIR",self.config.path()).env("SKIT_LANG","en").env("HOME",self.home.path()).env("USERPROFILE",self.home.path()).current_dir(self.home.path());c}
 fn run(&self,args:&[&str])->Output{self.command().args(args).output().unwrap()}
 fn params(&self,name:&str)->Value{let out=self.run(&["params",name,"--json"]);assert_eq!(out.status.code(),Some(0),"{}",combined(&out));serde_json::from_slice(&out.stdout).unwrap()}
 fn create_prompt(&self,body:&[u8],managed:&[&str],interpolate:bool){let source=self.home.path().join("p.prompt.md");fs::write(&source,body).unwrap();FileStore::new(self.data.path()).create(CreateEntry{name:"p".to_owned(),kind:EntryKind::parse("prompt").unwrap(),mode:StorageMode::Copy,source:source.display().to_string(),workdir:"invoke".to_owned(),description:String::new(),payload:Some(EntryPayload{bytes:body.to_vec(),stored_name:Some("prompt.md".to_owned()),permissions:SourcePermissions::default()}),settings:EntrySettings{params:managed.iter().map(|v|(*v).to_owned()).collect(),parameters:managed.iter().map(|v|synthesized_placeholder(v)).collect(),interpolate,..EntrySettings::default()}}).unwrap();}
 fn payload(&self)->PathBuf{self.data.path().join("scripts/p/prompt.md")}
}
fn combined(o:&Output)->String{format!("{}{}",String::from_utf8_lossy(&o.stdout),String::from_utf8_lossy(&o.stderr))}

#[test]
fn test_unmanaged_prompt_placeholders_is_body_minus_managed_in_order(){let s=Sandbox::new();s.create_prompt(b"{{a}} {{b}} {{c}}\n",&["b"],true);assert_eq!(s.params("p")["unmanaged"],json!(["a","c"]));}

#[test]
fn test_unmanaged_prompt_placeholders_empty_when_insertion_off(){let s=Sandbox::new();s.create_prompt(b"{{a}}\n",&["a"],false);assert_eq!(s.params("p")["unmanaged"],json!([]));}

#[test]
fn test_unmanaged_prompt_placeholders_empty_for_non_prompt(){let s=Sandbox::new();let out=s.run(&["add","--cmd","echo hi","--name","notaprompt"]);assert_eq!(out.status.code(),Some(0),"{}",combined(&out));assert_eq!(s.params("notaprompt")["unmanaged"],json!([]));}

#[test]
fn test_unmanaged_prompt_placeholders_empty_when_body_missing_or_undecodable(){let s=Sandbox::new();s.create_prompt(b"{{a}}\n",&["a"],true);fs::write(s.payload(),b"\xff\xfe not utf-8 {{a}}").unwrap();assert_eq!(s.params("p")["unmanaged"],json!([]));fs::remove_file(s.payload()).unwrap();assert_eq!(s.params("p")["unmanaged"],json!([]));}
