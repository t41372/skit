use std::{fs,path::{Path,PathBuf},process::{Command as ProcessCommand,Output}};

use assert_cmd::Command;
use tempfile::TempDir;

struct Sandbox{data:TempDir,state:TempDir,config:TempDir,home:TempDir,tools:TempDir}
impl Sandbox{
 fn new()->Self{Self{data:TempDir::new().unwrap(),state:TempDir::new().unwrap(),config:TempDir::new().unwrap(),home:TempDir::new().unwrap(),tools:TempDir::new().unwrap()}}
 fn command(&self)->Command{let mut c=assert_cmd::cargo::cargo_bin_cmd!("skit");c.env("SKIT_DATA_DIR",self.data.path()).env("SKIT_STATE_DIR",self.state.path()).env("SKIT_CONFIG_DIR",self.config.path()).env("SKIT_LANG","en").env("HOME",self.home.path()).env("USERPROFILE",self.home.path()).current_dir(self.home.path());c}
 fn run(&self,args:&[&str])->Output{self.command().args(args).output().unwrap()}
 fn source(&self,name:&str,body:&str)->PathBuf{let p=self.home.path().join(name);fs::write(&p,body).unwrap();p}
 fn add_runner(&self,name:&str,argv:&[&str]){let mut a=vec!["runner","add",name,"--force","--"];a.extend(argv.iter().copied());let o=self.run(&a);assert_eq!(o.status.code(),Some(0),"{}",combined(&o));}
 fn add_prompt(&self,name:&str,body:&str,pin:Option<&str>){let p=self.source(&format!("{name}.prompt.md"),body);let mut a=vec!["add",p.to_str().unwrap(),"--name",name,"--no-input"];if let Some(pin)=pin{a.extend(["--runner",pin]);}let o=self.run(&a);assert_eq!(o.status.code(),Some(0),"{}",combined(&o));}
}
fn combined(o:&Output)->String{format!("{}{}",String::from_utf8_lossy(&o.stdout),String::from_utf8_lossy(&o.stderr))}
fn compile_recorder(root:&Path,name:&str)->(PathBuf,PathBuf){let src=root.join(format!("{name}.rs"));let cap=root.join(format!("{name}.capture"));fs::write(&src,r#"use std::{env,fs};fn main(){let args=env::args_os().skip(1).map(|v|v.to_string_lossy().into_owned()).collect::<Vec<_>>();fs::write(env::var_os("SKIT_RR_CAPTURE").unwrap(),args.join("\u{001e}")).unwrap();}"#).unwrap();let exe=root.join(if cfg!(windows){format!("{name}.exe")}else{name.to_owned()});assert!(ProcessCommand::new("rustc").arg(src).arg("-o").arg(&exe).status().unwrap().success());(exe,cap)}

#[test]
fn test_build_resolves_the_pin_when_no_override_is_given(){let s=Sandbox::new();let(exe,cap)=compile_recorder(s.tools.path(),"pinned");s.add_runner("pinned",&[exe.to_str().unwrap(),"--from-pin","{{prompt}}"]);s.add_prompt("p","body",Some("pinned"));let o=s.command().env("SKIT_RR_CAPTURE",&cap).args(["run","p","--no-input"]).output().unwrap();assert_eq!(o.status.code(),Some(0),"{}",combined(&o));assert_eq!(fs::read_to_string(cap).unwrap().split('\u{001e}').collect::<Vec<_>>(),["--from-pin","body"]);}

#[test]
fn test_build_without_pin_or_override_is_exit_126(){let s=Sandbox::new();s.add_prompt("p","body",None);let o=s.run(&["run","p","--no-input"]);assert_eq!(o.status.code(),Some(126),"{}",combined(&o));assert!(combined(&o).contains("No runner selected"),"{}",combined(&o));}

#[test]
fn test_build_with_unconfigured_pin_is_exit_126(){let s=Sandbox::new();let(exe,_)=compile_recorder(s.tools.path(),"gone");s.add_runner("gone",&[exe.to_str().unwrap(),"{{prompt}}"]);s.add_prompt("p","body",Some("gone"));s.run(&["runner","remove","gone","--yes"]);let o=s.run(&["run","p","--no-input"]);assert_eq!(o.status.code(),Some(126),"{}",combined(&o));assert!(combined(&o).contains("gone"),"{}",combined(&o));}

#[test]
fn test_build_missing_binary_is_exit_126(){let s=Sandbox::new();s.add_runner("missing",&["definitely-not-installed","{{prompt}}"]);s.add_prompt("p","body",Some("missing"));let o=s.command().env("PATH","").args(["run","p","--no-input"]).output().unwrap();assert_eq!(o.status.code(),Some(126),"{}",combined(&o));assert!(combined(&o).contains("definitely-not-installed"),"{}",combined(&o));}

#[test]
fn test_build_missing_body_is_exit_127(){let s=Sandbox::new();let(exe,_)=compile_recorder(s.tools.path(),"r");s.add_runner("r",&[exe.to_str().unwrap(),"{{prompt}}"]);s.add_prompt("p","body",Some("r"));fs::remove_file(s.data.path().join("scripts/p/prompt.md")).unwrap();let o=s.run(&["run","p","--no-input"]);assert_eq!(o.status.code(),Some(127),"{}",combined(&o));}

#[test]
fn rust_additive_resolve_runner_precedence(){let s=Sandbox::new();let(pin,pin_cap)=compile_recorder(s.tools.path(),"pin");let(over,over_cap)=compile_recorder(s.tools.path(),"over");s.add_runner("pin",&[pin.to_str().unwrap(),"{{prompt}}"]);s.add_runner("over",&[over.to_str().unwrap(),"{{prompt}}"]);s.add_prompt("p","body",Some("pin"));let o=s.command().env("SKIT_RR_CAPTURE",&over_cap).args(["run","p","--runner","over","--no-input"]).output().unwrap();assert_eq!(o.status.code(),Some(0),"{}",combined(&o));assert_eq!(fs::read_to_string(over_cap).unwrap(),"body");assert!(!pin_cap.exists(),"stored pin won over explicit override");}

#[test]
fn rust_additive_resolve_runner_without_pin_or_override_does_not_use_last_runner(){let s=Sandbox::new();let(exe,cap)=compile_recorder(s.tools.path(),"remembered");s.add_runner("remembered",&[exe.to_str().unwrap(),"{{prompt}}"]);s.add_prompt("p","body",None);fs::create_dir_all(s.state.path()).unwrap();fs::write(s.state.path().join("prompt.toml"),"last_runner = \"remembered\"\n").unwrap();let o=s.command().env("SKIT_RR_CAPTURE",&cap).args(["run","p","--no-input"]).output().unwrap();assert_eq!(o.status.code(),Some(126),"{}",combined(&o));assert!(!cap.exists());}

#[test]
fn test_describe_resolves_a_pinned_multi_token_runner(){let s=Sandbox::new();s.add_runner("balanced",&["agent","--model","balanced","{{prompt}}"]);s.add_prompt("p","body",Some("balanced"));let o=s.run(&["run","p","--dry-run","--no-input"]);assert_eq!(o.status.code(),Some(0),"{}",combined(&o));let shown=combined(&o);assert!(shown.contains("agent")&&shown.contains("--model")&&shown.contains("balanced")&&shown.contains("body"),"{shown}");}

#[test]
fn test_describe_unresolvable_pin_degrades_to_the_name_stub(){let s=Sandbox::new();let p=s.source("p.prompt.md","body");s.run(&["add",p.to_str().unwrap(),"--name","p","--no-input"]);let meta=s.data.path().join("scripts/p/meta.toml");let mut raw=fs::read_to_string(&meta).unwrap();raw.push_str("runner = \"gone\"\n");fs::write(&meta,raw).unwrap();let o=s.run(&["show","p"]);assert_eq!(o.status.code(),Some(0),"{}",combined(&o));assert!(combined(&o).contains("Runner: gone"),"{}",combined(&o));}
