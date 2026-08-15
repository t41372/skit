use std::{
    fs::{self,File,OpenOptions},
    path::PathBuf,
    process::{Child,Command,Output,Stdio},
    sync::mpsc,
    thread,
    time::Duration,
};

use serde_json::Value;
use tempfile::TempDir;

struct Sandbox{data:TempDir,state:TempDir,config:TempDir,home:TempDir}
impl Sandbox{
 fn new()->Self{Self{data:TempDir::new().unwrap(),state:TempDir::new().unwrap(),config:TempDir::new().unwrap(),home:TempDir::new().unwrap()}}
 fn configure(&self,c:&mut Command){c.env("SKIT_DATA_DIR",self.data.path()).env("SKIT_STATE_DIR",self.state.path()).env("SKIT_CONFIG_DIR",self.config.path()).env("SKIT_LANG","en").env("HOME",self.home.path()).env("USERPROFILE",self.home.path()).current_dir(self.home.path());}
 fn run(&self,args:&[&str])->Output{let mut c=Command::new(env!("CARGO_BIN_EXE_skit"));self.configure(&mut c);c.args(args).output().unwrap()}
 fn spawn(&self,args:&[&str])->Child{let mut c=Command::new(env!("CARGO_BIN_EXE_skit"));self.configure(&mut c);c.args(args).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap()}
 fn add_prompt(&self){let p=self.home.path().join("p.prompt.md");fs::write(&p,"{{a}} {{b}}\n").unwrap();let o=self.run(&["add",p.to_str().unwrap(),"--name","p","--no-input"]);assert_eq!(o.status.code(),Some(0),"{}",combined(&o));}
 fn show(&self)->Value{let o=self.run(&["show","p","--json"]);assert_eq!(o.status.code(),Some(0),"{}",combined(&o));serde_json::from_slice(&o.stdout).unwrap()}
 fn params(&self)->Value{let o=self.run(&["params","p","--json"]);assert_eq!(o.status.code(),Some(0),"{}",combined(&o));serde_json::from_slice(&o.stdout).unwrap()}
 fn meta_lock(&self)->File{let path=self.data.path().join(".locks/p.meta.lock");fs::create_dir_all(path.parent().unwrap()).unwrap();let f=OpenOptions::new().read(true).write(true).create(true).truncate(false).open(path).unwrap();if f.metadata().unwrap().len()==0{f.set_len(1).unwrap();}f.lock().unwrap();f}
}
fn combined(o:&Output)->String{format!("{}{}",String::from_utf8_lossy(&o.stdout),String::from_utf8_lossy(&o.stderr))}
fn assert_blocked(child:&mut Child){thread::sleep(Duration::from_millis(120));assert!(child.try_wait().unwrap().is_none(),"metadata writer ignored p.meta.lock");}
fn wait_ok(child:Child)->Output{let o=child.wait_with_output().unwrap();assert_eq!(o.status.code(),Some(0),"{}",combined(&o));o}
fn fields(v:&Value)->Vec<&str>{v["fields"].as_array().unwrap().iter().map(|f|f["key"].as_str().unwrap()).collect()}

#[test]
fn rust_additive_prompt_store_writers_share_entry_lock_across_processes(){let s=Sandbox::new();s.add_prompt();let lock=s.meta_lock();let mut child=s.spawn(&["params","p","--runner","claude"]);assert_blocked(&mut child);lock.unlock().unwrap();wait_ok(child);assert_eq!(s.show()["runner"],"claude");}

#[test]
fn rust_additive_concurrent_prompt_runner_update_preserves_disjoint_metadata(){let s=Sandbox::new();s.add_prompt();let lock=s.meta_lock();let mut runner=s.spawn(&["params","p","--runner","claude"]);let mut desc=s.spawn(&["describe","p","updated description"]);assert_blocked(&mut runner);assert_blocked(&mut desc);lock.unlock().unwrap();wait_ok(runner);wait_ok(desc);let view=s.show();assert_eq!(view["runner"],"claude");assert_eq!(view["description"],"updated description");}

#[test]
fn rust_additive_concurrent_prompt_managed_update_preserves_disjoint_metadata(){let s=Sandbox::new();s.add_prompt();let lock=s.meta_lock();let mut managed=s.spawn(&["params","p","--rm","a"]);let mut desc=s.spawn(&["describe","p","managed changed"]);assert_blocked(&mut managed);assert_blocked(&mut desc);lock.unlock().unwrap();wait_ok(managed);wait_ok(desc);assert_eq!(fields(&s.show()),["b"]);assert_eq!(s.show()["description"],"managed changed");}

#[test]
fn test_write_prompt_interpolate_keeps_managed_list(){let s=Sandbox::new();s.add_prompt();let before=s.params()["placeholders"].clone();let o=s.run(&["params","p","--no-interpolate"]);assert_eq!(o.status.code(),Some(0),"{}",combined(&o));let after=s.params();assert_eq!(after["placeholders"],before);assert_eq!(after["interpolate"],false);}

#[test]
fn rust_additive_concurrent_prompt_interpolate_update_preserves_disjoint_metadata(){let s=Sandbox::new();s.add_prompt();let lock=s.meta_lock();let mut interpolate=s.spawn(&["params","p","--no-interpolate"]);let mut desc=s.spawn(&["describe","p","still here"]);assert_blocked(&mut interpolate);assert_blocked(&mut desc);lock.unlock().unwrap();wait_ok(interpolate);wait_ok(desc);let view=s.show();assert_eq!(view["interpolate"],false);assert_eq!(view["description"],"still here");}

#[test]
fn rust_additive_concurrent_prompt_managed_and_interpolate_preserve_each_other(){let s=Sandbox::new();s.add_prompt();let lock=s.meta_lock();let mut managed=s.spawn(&["params","p","--rm","a"]);let mut interpolate=s.spawn(&["params","p","--no-interpolate"]);assert_blocked(&mut managed);assert_blocked(&mut interpolate);lock.unlock().unwrap();wait_ok(managed);wait_ok(interpolate);let params=s.params();assert_eq!(params["placeholders"],serde_json::json!(["b"]));assert_eq!(params["interpolate"],false);}

#[test]
fn rust_additive_concurrent_prompt_writers_conflict_on_same_field_last_writer_wins(){let s=Sandbox::new();s.add_prompt();let lock=s.meta_lock();let first=s.spawn(&["params","p","--runner","claude"]);let second=s.spawn(&["params","p","--runner","codex"]);let(tx,rx)=mpsc::channel();for(name,child)in[("claude",first),("codex",second)]{let tx=tx.clone();thread::spawn(move||{let o=child.wait_with_output().unwrap();tx.send((name.to_owned(),o)).unwrap();});}thread::sleep(Duration::from_millis(120));lock.unlock().unwrap();let(a,oa)=rx.recv().unwrap();let(b,ob)=rx.recv().unwrap();assert_eq!(oa.status.code(),Some(0),"{}",combined(&oa));assert_eq!(ob.status.code(),Some(0),"{}",combined(&ob));assert_ne!(a,b);assert_eq!(s.show()["runner"],b,"final metadata did not match the writer that completed last");}
