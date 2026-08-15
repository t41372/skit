use std::{fs,path::PathBuf,process::Output};

use assert_cmd::Command;
use serde_json::Value;
use skit_application::{EntryMutationRepository as _,EntryRepository as _,SourcePermissions};
use skit_store::FileStore;
use skit_ui::{KnownEntryKind,ReviewDefaults,ReviewState,SourceSnapshot};
use tempfile::TempDir;

struct Sandbox{data:TempDir,state:TempDir,config:TempDir,home:TempDir}
impl Sandbox{
 fn new()->Self{Self{data:TempDir::new().unwrap(),state:TempDir::new().unwrap(),config:TempDir::new().unwrap(),home:TempDir::new().unwrap()}}
 fn command(&self)->Command{let mut c=assert_cmd::cargo::cargo_bin_cmd!("skit");c.env("SKIT_DATA_DIR",self.data.path()).env("SKIT_STATE_DIR",self.state.path()).env("SKIT_CONFIG_DIR",self.config.path()).env("SKIT_LANG","en").env("HOME",self.home.path()).env("USERPROFILE",self.home.path()).current_dir(self.home.path());c}
 fn run(&self,args:&[&str])->Output{self.command().args(args).output().unwrap()}
 fn source(&self,name:&str,body:&str)->PathBuf{let p=self.home.path().join(name);fs::write(&p,body).unwrap();p}
 fn show(&self,name:&str)->Value{let o=self.run(&["show",name,"--json"]);assert_eq!(o.status.code(),Some(0),"{}",combined(&o));serde_json::from_slice(&o.stdout).unwrap()}
 fn params(&self,name:&str)->Value{let o=self.run(&["params",name,"--json"]);assert_eq!(o.status.code(),Some(0),"{}",combined(&o));serde_json::from_slice(&o.stdout).unwrap()}
 fn review(&self,name:&str,body:&str)->ReviewState{ReviewState::from_source(SourceSnapshot{path:self.home.path().join(format!("{name}.prompt.md")),source_record:self.home.path().join(format!("{name}.prompt.md")).display().to_string(),bytes:body.as_bytes().to_vec(),permissions:SourcePermissions::default(),is_regular:true,is_directory:false,is_draft:false},KnownEntryKind::Prompt,ReviewDefaults::default())}
 fn meta(&self,name:&str)->String{fs::read_to_string(self.data.path().join("scripts").join(name).join("meta.toml")).unwrap()}
}
fn combined(o:&Output)->String{format!("{}{}",String::from_utf8_lossy(&o.stdout),String::from_utf8_lossy(&o.stderr))}

#[test]
fn test_add_prompt_manages_all_detected_by_default(){let s=Sandbox::new();let body="# T\n\nDo {{a}} then {{b}}. Sample {{a}}.\n";let request=s.review("job",body).create_entry().unwrap();let entry=FileStore::new(s.data.path()).create(request).unwrap();assert_eq!(entry.meta.kind.as_str(),"prompt");let view=s.show("job");assert_eq!(view["fields"].as_array().unwrap().iter().map(|f|f["key"].as_str().unwrap()).collect::<Vec<_>>(),["a","b"]);assert_eq!(view["workdir"],"invoke");assert_eq!(view["description"],"T");assert!(view["runner"].is_null());assert_eq!(fs::read(s.data.path().join("scripts/job/prompt.md")).unwrap(),body.as_bytes());}

#[test]
fn test_add_prompt_managed_subset_keeps_body_order(){let s=Sandbox::new();let mut review=s.review("sub","{{a}} {{b}} {{c}}\n");review.set_prompt_selection(&["c".to_owned(),"a".to_owned()]);FileStore::new(s.data.path()).create(review.create_entry().unwrap()).unwrap();assert_eq!(s.show("sub")["fields"].as_array().unwrap().iter().map(|f|f["key"].as_str().unwrap()).collect::<Vec<_>>(),["a","c"]);}

#[test]
fn test_add_prompt_reference_mode_still_pins_invoke_workdir(){let s=Sandbox::new();let src=s.source("ref.prompt.md","hello {{x}}\n");let o=s.run(&["add",src.to_str().unwrap(),"--ref","--no-input"]);assert_eq!(o.status.code(),Some(0),"{}",combined(&o));let view=s.show("ref");assert_eq!(view["mode"],"reference");assert_eq!(view["workdir"],"invoke");assert_eq!(view["source"],src.display().to_string());assert!(!s.data.path().join("scripts/ref/prompt.md").exists());}

#[test]
fn test_add_prompt_name_strips_double_extension(){let s=Sandbox::new();let src=s.source("review.prompt.md","x\n");let o=s.run(&["add",src.to_str().unwrap(),"--no-input"]);assert_eq!(o.status.code(),Some(0),"{}",combined(&o));assert_eq!(s.show("review")["name"],"review");}

#[test]
fn test_add_prompt_missing_file(){let s=Sandbox::new();let missing=s.home.path().join("ghost.prompt.md");let o=s.run(&["add",missing.to_str().unwrap(),"--no-input"]);assert_eq!(o.status.code(),Some(1),"{}",combined(&o));assert!(combined(&o).contains("File not found"),"{}",combined(&o));}

#[test]
fn test_write_prompt_managed_and_runner_roundtrip(){
 let s=Sandbox::new();let req=s.review("p","{{a}} {{b}}\n").create_entry().unwrap();FileStore::new(s.data.path()).create(req).unwrap();
 let rm=s.run(&["params","p","--rm","a"]);assert_eq!(rm.status.code(),Some(0),"{}",combined(&rm));let pin=s.run(&["params","p","--runner","claude"]);assert_eq!(pin.status.code(),Some(0),"{}",combined(&pin));
 let view=s.params("p");assert_eq!(view["placeholders"],serde_json::json!(["b"]));assert_eq!(view["runner"],"claude");let persisted=s.meta("p");assert!(persisted.contains("params = [\"b\"]"),"managed subset was not persisted: {persisted}");assert!(persisted.contains("runner = \"claude\""),"runner pin was not persisted: {persisted}");
 let rm=s.run(&["params","p","--rm","b"]);assert_eq!(rm.status.code(),Some(0),"{}",combined(&rm));let clear=s.run(&["params","p","--runner",""]);assert_eq!(clear.status.code(),Some(0),"{}",combined(&clear));
 let view=s.params("p");assert_eq!(view["placeholders"],serde_json::json!([]));assert!(view["runner"].is_null());let persisted=s.meta("p");assert!(!persisted.lines().any(|line|line.trim_start().starts_with("params =")),"cleared managed list remained serialized instead of returning to the default/None state: {persisted}");assert!(!persisted.lines().any(|line|line.trim_start().starts_with("runner =")),"cleared runner remained serialized instead of returning to the default empty state: {persisted}");
}

#[test]
fn test_prompt_entries_pinned_to_filters_by_kind_and_runner(){let s=Sandbox::new();for (name,runner) in [("first","claude"),("second","codex"),("third","")]{let src=s.source(&format!("{name}.prompt.md"),"body\n");let mut args=vec!["add",src.to_str().unwrap(),"--name",name,"--no-input"];if !runner.is_empty(){args.extend(["--runner",runner]);}let o=s.run(&args);assert_eq!(o.status.code(),Some(0),"{}",combined(&o));}let cmd=s.run(&["add","--cmd","echo ok","--name","not-a-prompt"]);assert_eq!(cmd.status.code(),Some(0),"{}",combined(&cmd));let meta=s.data.path().join("scripts/not-a-prompt/meta.toml");let mut raw=fs::read_to_string(&meta).unwrap();raw.push_str("runner = \"claude\"\n");fs::write(&meta,raw).unwrap();let remove=s.run(&["runner","remove","claude","--yes"]);assert_eq!(remove.status.code(),Some(0),"{}",combined(&remove));assert!(combined(&remove).contains("1 prompt pins this runner"),"{}",combined(&remove));assert_eq!(s.show("first")["runner"],"claude");assert_eq!(s.show("second")["runner"],"codex");assert!(s.show("third")["runner"].is_null());}
