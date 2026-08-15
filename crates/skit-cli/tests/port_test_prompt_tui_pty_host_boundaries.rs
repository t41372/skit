use std::{fs, process::Command};

use skit_application::{EntryMutationRepository as _, SourcePermissions};
use skit_store::{FileConfigStore, FileStore, PromptRunner};
use skit_ui::{KnownEntryKind, ReviewDefaults, ReviewState, SourceSnapshot};
use tempfile::TempDir;

#[path = "support/prompt_tui_pty.rs"]
mod prompt_tui_pty;
use prompt_tui_pty::TuiPty;

struct Sandbox { data:TempDir,state:TempDir,config:TempDir,home:TempDir }
impl Sandbox {
 fn new()->Self{Self{data:TempDir::new().unwrap(),state:TempDir::new().unwrap(),config:TempDir::new().unwrap(),home:TempDir::new().unwrap()}}
 fn config(&self)->FileConfigStore{FileConfigStore::new(self.config.path())}
 fn store(&self)->FileStore{FileStore::new(self.data.path())}
 fn prompt(&self,pin:&str){let path=self.home.path().join("p.prompt.md");fs::write(&path,"Do {{a}}\n").unwrap();let review=ReviewState::from_source(SourceSnapshot{path:path.clone(),source_record:path.display().to_string(),bytes:b"Do {{a}}\n".to_vec(),permissions:SourcePermissions::default(),is_regular:true,is_directory:false,is_draft:false},KnownEntryKind::Prompt,ReviewDefaults{name:Some("p".to_owned()),..ReviewDefaults::default()});let mut request=review.create_entry().unwrap();request.settings.runner=pin.to_owned();self.store().create(request).unwrap();}
 fn clear_runners(&self){let config=self.config();config.ensure_runners_seeded().unwrap();for name in config.runners().unwrap().into_iter().map(|r|r.name).collect::<Vec<_>>(){assert!(config.remove_runner(&name).unwrap());}}
 fn runner(&self,name:&str,marker:&str){self.config().set_runner(PromptRunner{name:name.to_owned(),argv:echo_argv(marker)},false).unwrap();}
 fn missing_runner(&self,name:&str){self.config().set_runner(PromptRunner{name:name.to_owned(),argv:vec![format!("skit-definitely-missing-{name}"),"{{prompt}}".to_owned()]},false).unwrap();}
 fn tui(&self)->TuiPty{TuiPty::spawn(self.data.path(),self.state.path(),self.config.path(),self.home.path())}
 fn cli(&self,args:&[&str])->std::process::Output{Command::new(env!("CARGO_BIN_EXE_skit")).args(args).env("SKIT_DATA_DIR",self.data.path()).env("SKIT_STATE_DIR",self.state.path()).env("SKIT_CONFIG_DIR",self.config.path()).env("SKIT_LANG","en").env("HOME",self.home.path()).env("USERPROFILE",self.home.path()).current_dir(self.home.path()).output().unwrap()}
 fn seed_run(&self,runner:&str,marker:&str){let out=self.cli(&["run","p","--set","a=1","--runner",runner,"--no-input","--plain"]);let shown=format!("{}{}",String::from_utf8_lossy(&out.stdout),String::from_utf8_lossy(&out.stderr));assert!(out.status.success(),"seed run failed: {shown}");assert!(shown.contains(marker),"seed run never reached marker child: {shown}");}
}
#[cfg(windows)] fn echo_argv(marker:&str)->Vec<String>{vec!["cmd.exe".to_owned(),"/C".to_owned(),"echo".to_owned(),marker.to_owned(),"{{prompt}}".to_owned()]}
#[cfg(not(windows))] fn echo_argv(marker:&str)->Vec<String>{vec!["/bin/echo".to_owned(),marker.to_owned(),"{{prompt}}".to_owned()]}
fn open(tui:&mut TuiPty)->usize{tui.wait_for("Library");let cp=tui.checkpoint();tui.send(b"\r");cp}
fn focus_runner(tui:&mut TuiPty){tui.send(b"\x1b[Z");}
fn submit_value(tui:&mut TuiPty,value:&str)->usize{tui.send(value.as_bytes());let cp=tui.checkpoint();tui.send(&[0x12]);cp}

#[test]
fn test_missing_pinned_binary_cannot_block_a_different_pick(){
 const WORKING:&str="PTY-WORKING-OVERRIDE";let s=Sandbox::new();s.clear_runners();s.missing_runner("broken");s.runner("working",WORKING);s.prompt("broken");let mut tui=s.tui();let cp=open(&mut tui);let visible=tui.wait_for_after(cp,"Run p");assert!(visible.contains("broken"),"broken pin was not honestly prefilled before user override: {visible}");focus_runner(&mut tui);tui.send(b"\r");tui.send(b"\x1b[B");tui.send(b"\r");tui.send(b"\t");let cp=submit_value(&mut tui,"hello");let out=tui.wait_for_after(cp,WORKING);assert!(out.contains("prompt.md"),"working override did not receive prompt: {out}");assert!(!out.contains("skit-definitely-missing-broken"));
}

#[test]
fn test_selected_prompt_runner_preflight_failure_returns_to_library(){
 let s=Sandbox::new();s.clear_runners();s.missing_runner("missing");s.prompt("");let mut tui=s.tui();let cp=open(&mut tui);tui.wait_for_after(cp,"Run p");let cp=submit_value(&mut tui,"hello");let out=tui.wait_for_after(cp,"Library");assert!(out.contains("missing"),"preflight refusal lost selected runner identity: {out}");assert!(out.to_lowercase().contains("not")||out.to_lowercase().contains("cannot"),"preflight refusal was not actionable: {out}");assert!(!out.contains("hello\nprompt"),"a child appears to have run despite preflight refusal: {out}");
}

#[test]
fn test_rerun_unpinned_prompt_falls_back_to_the_form(){
 let s=Sandbox::new();s.clear_runners();s.runner("codex","PTY-SEED-UNPINNED");s.runner("claude","PTY-CLAUDE-UNPINNED");s.prompt("");s.seed_run("codex","PTY-SEED-UNPINNED");let mut tui=s.tui();tui.wait_for("Library");let cp=tui.checkpoint();tui.send(b"r");let out=tui.wait_for_after(cp,"Run p");assert!(out.contains("Runner"),"unpinned rerun silently chose an agent instead of returning to the form: {out}");
}

#[test]
fn test_rerun_pinned_prompt_skips_the_form_and_uses_the_pin(){
 const PIN:&str="PTY-PINNED-RERUN";let s=Sandbox::new();s.clear_runners();s.runner("codex","PTY-SEED-PINNED");s.runner("claude",PIN);s.prompt("claude");s.seed_run("codex","PTY-SEED-PINNED");let mut tui=s.tui();tui.wait_for("Library");let cp=tui.checkpoint();tui.send(b"r");let out=tui.wait_for_after(cp,PIN);assert!(!out.contains("Run p"),"pinned rerun reopened the form instead of using its pin: {out}");assert!(out.contains("prompt.md"));
}

#[test]
fn test_exit_mode_pending_run_carries_the_runner(){
 const MARKER:&str="PTY-EXIT-RUNNER";let s=Sandbox::new();s.clear_runners();s.runner("codex",MARKER);s.prompt("");s.config().set("after_run","exit").unwrap();let mut tui=s.tui();let cp=open(&mut tui);tui.wait_for_after(cp,"Run p");let cp=submit_value(&mut tui,"v");let out=tui.wait_for_after(cp,MARKER);assert!(out.contains("prompt.md"),"selected runner was lost before exit-mode launch: {out}");let exit_cp=tui.checkpoint();let exited=tui.wait_for_exit_after(exit_cp);assert!(!exited.contains("Run p"),"exit-mode frontend reopened the form after launch: {exited}");
}

#[test]
fn test_form_submit_with_a_runner_removed_mid_flight_is_honest(){
 const MARKER:&str="PTY-REMOVED-SHOULD-NOT-RUN";let s=Sandbox::new();s.clear_runners();s.runner("codex",MARKER);s.prompt("codex");let mut tui=s.tui();let cp=open(&mut tui);tui.wait_for_after(cp,"Run p");assert!(s.config().remove_runner("codex").unwrap());let cp=submit_value(&mut tui,"x");let out=tui.wait_for_after(cp,"Library");assert!(out.contains("codex")&&out.contains("no longer configured"),"mid-flight runner removal was not explained honestly: {out}");assert!(!out.contains(MARKER),"removed runner still launched: {out}");
}
