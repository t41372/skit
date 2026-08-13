//! Exact CLI/store ports from Python v0.4 `tests/test_shell_analyzer.py`.
use std::fs;
use assert_cmd::Command;
use skit_application::EntryRepository as _;
use skit_store::FileStore;
use tempfile::TempDir;

struct Sandbox{data:TempDir,state:TempDir,config:TempDir,home:TempDir}
impl Sandbox{
 fn new()->Self{let s=Self{data:TempDir::new().unwrap(),state:TempDir::new().unwrap(),config:TempDir::new().unwrap(),home:TempDir::new().unwrap()};fs::write(s.config.path().join("config.toml"),"[mirror]\nenabled = false\n").unwrap();s}
 fn cmd(&self)->Command{let mut c=assert_cmd::cargo::cargo_bin_cmd!("skit");c.env("SKIT_DATA_DIR",self.data.path()).env("SKIT_STATE_DIR",self.state.path()).env("SKIT_CONFIG_DIR",self.config.path()).env("SKIT_LANG","en").env("HOME",self.home.path()).env("USERPROFILE",self.home.path()).env("XDG_CONFIG_HOME",self.home.path().join("xdg-config")).env("XDG_DATA_HOME",self.home.path().join("xdg-data")).env("XDG_STATE_HOME",self.home.path().join("xdg-state")).env_remove("FORCE_COLOR").env_remove("NO_COLOR").env_remove("PSModulePath").current_dir(self.home.path());c}
 fn add(&self,name:&str,text:&str){let src=self.home.path().join(format!("{name}.sh"));fs::write(&src,text).unwrap();self.cmd().arg("add").arg(&src).args(["--kind","shell","--name",name,"--no-input"]).assert().success();}
 fn output(&self,args:&[&str])->std::process::Output{self.cmd().args(args).output().unwrap()}
 fn payload(&self,name:&str)->std::path::PathBuf{let store=FileStore::new(self.data.path());let e=store.resolve(name).unwrap();store.payload_path(&e).unwrap()}
}
fn text(o:&std::process::Output)->String{format!("{}{}",String::from_utf8_lossy(&o.stdout),String::from_utf8_lossy(&o.stderr))}

#[test]
fn test_envdefault_loud_drift_line(){let s=Sandbox::new();s.add("envdrift","#!/usr/bin/env bash\necho \"${PORT:-8080}\"\n");s.cmd().args(["params","envdrift","--manage","PORT"]).assert().success();let p=s.payload("envdrift");let before=fs::read_to_string(&p).unwrap();fs::write(&p,before.replace("echo \"${PORT:-8080}\"","echo hello")).unwrap();let o=s.output(&["params","envdrift"]);let t=text(&o);assert_eq!(o.status.code(),Some(0),"{t}");assert!(t.contains("PORT is no longer read from the environment"),"{t}");assert!(!t.contains("PORT: injection target no longer exists"),"{t}");}

#[test]
fn test_params_manage_writes_block_into_shell_copy(){let s=Sandbox::new();s.add("sh1","#!/usr/bin/env bash\nCITY=Taipei\necho $CITY\n");s.cmd().args(["params","sh1","--manage","CITY"]).assert().success();let t=fs::read_to_string(s.payload("sh1")).unwrap();assert!(t.contains("[tool.skit]"),"{t}");assert!(t.contains("name = \"CITY\""),"{t}");assert!(t.starts_with("#!/usr/bin/env bash\n"),"{t}");assert!(t.find("#!")<t.find("# /// script"));}

#[test]
fn test_params_show_lists_shell_const_and_unmanaged(){let s=Sandbox::new();s.add("sh2","#!/usr/bin/env bash\nCITY=Taipei\necho \"${MODE:-auto}\"\n");let o=s.output(&["params","sh2"]);let t=text(&o);assert_eq!(o.status.code(),Some(0),"{t}");assert!(t.contains("CITY"),"{t}");assert!(t.contains("MODE"),"{t}");}

#[test]
fn test_params_show_getopts_shell_stops_advertising_manage(){let s=Sandbox::new();s.add("gsh","#!/usr/bin/env bash\nOUT=hello\nwhile getopts \"n:v\" o; do :; done\necho \"$OUT\"\n");let o=s.output(&["params","gsh"]);let t=text(&o);assert_eq!(o.status.code(),Some(0),"{t}");assert!(!t.contains("--manage"),"{t}");assert!(!t.contains("Detected but not yet managed"),"{t}");assert!(t.contains("gsh has no managed parameters."),"{t}");}

#[test]
fn test_params_resync_reports_drift_after_edit(){let s=Sandbox::new();s.add("sh3","#!/usr/bin/env bash\nCITY=Taipei\necho $CITY\n");s.cmd().args(["params","sh3","--manage","CITY"]).assert().success();let p=s.payload("sh3");let before=fs::read_to_string(&p).unwrap();fs::write(&p,before.replace("CITY=Taipei","TOWN=Taipei").replace("$CITY","$TOWN")).unwrap();let o=s.output(&["params","sh3","--resync"]);let t=text(&o);assert_eq!(o.status.code(),Some(0),"{t}");assert!(t.contains("CITY"),"{t}");assert!(!fs::read_to_string(p).unwrap().contains("name = \"CITY\""));}
