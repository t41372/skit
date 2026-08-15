use std::{collections::BTreeMap,path::PathBuf};

use skit_application::delivery::Assembly;
use skit_domain::{Entry,EntryKind,EntryMeta,Slug};
use skit_language::render_prompt_body;
use skit_runtime::{LaunchError,LaunchPaths,LaunchWarning,ProgramProbe,PromptRunner,build_launch_plan,build_launch_preview};

#[derive(Debug,Default)]
struct Probe{programs:BTreeMap<String,PathBuf>,files:Vec<PathBuf>,dirs:Vec<PathBuf>}
impl ProgramProbe for Probe{
 fn find_program(&self,name:&str)->Option<PathBuf>{self.programs.get(name).cloned()}
 fn is_file(&self,path:&std::path::Path)->bool{self.files.contains(&path.to_path_buf())}
 fn is_dir(&self,path:&std::path::Path)->bool{self.dirs.contains(&path.to_path_buf())}
 fn is_executable(&self,_:&std::path::Path)->bool{true}
}
fn entry()->Entry{let mut meta=EntryMeta::minimal("p",EntryKind::parse("prompt").unwrap());meta.workdir="invoke".to_owned();Entry{slug:Slug::parse("p").unwrap(),meta}}
fn paths()->LaunchPaths{LaunchPaths{script:PathBuf::from("/data/scripts/p/prompt.md"),entry_dir:PathBuf::from("/data/scripts/p"),invoke_cwd:PathBuf::from("/invoke")}}
fn probe(program:&str)->Probe{Probe{programs:BTreeMap::from([(program.to_owned(),PathBuf::from(format!("/bin/{program}")))]),files:vec![PathBuf::from("/data/scripts/p/prompt.md")],dirs:vec![PathBuf::from("/invoke"),PathBuf::from("/data/scripts/p")]}}
fn runner(name:&str,argv:&[&str])->PromptRunner{PromptRunner{name:name.to_owned(),argv:argv.iter().map(|v|(*v).to_owned()).collect()}}
fn assembly(extra:&[&str])->Assembly{Assembly{args:extra.iter().map(|v|(*v).to_owned()).collect(),masked_args:extra.iter().map(|v|(*v).to_owned()).collect(),..Assembly::default()}}

#[test]
fn test_build_renders_two_stages_and_appends_extra(){
 let rendered=render_prompt_body("Do {{a}}\n",&BTreeMap::from([("a".to_owned(),"X".to_owned())]),true);let r=runner("rec",&["rec-bin","{{prompt}}"]);let p=probe("rec-bin");let plan=build_launch_plan(&entry(),&paths(),&assembly(&["--model","opus"]),Some(&rendered),Some(&r),&p).unwrap();assert_eq!(plan.program,PathBuf::from("/bin/rec-bin"));assert_eq!(plan.args,["Do X\n","--model","opus"]);
}

#[test]
fn test_seeded_positional_runner_protects_dash_prefixed_prompt_and_keeps_extra(){let r=runner("claude",&["claude","--","{{prompt}}"]);let plan=build_launch_plan(&entry(),&paths(),&assembly(&["--model","opus"]),Some("--help"),Some(&r),&probe("claude")).unwrap();assert_eq!(plan.args,["--model","opus","--","--help"]);}

#[test]
fn test_seeded_opencode_binds_dash_prefixed_prompt_and_keeps_extra(){let r=runner("opencode",&["opencode","--prompt={{prompt}}"]);let plan=build_launch_plan(&entry(),&paths(),&assembly(&["--model","provider/model"]),Some("--version"),Some(&r),&probe("opencode")).unwrap();assert_eq!(plan.args,["--prompt=--version","--model","provider/model"]);}

#[test]
fn test_seeded_copilot_binds_dash_prefixed_prompt_and_keeps_extra(){let r=runner("copilot",&["copilot","--interactive={{prompt}}"]);let plan=build_launch_plan(&entry(),&paths(),&assembly(&["--model","gpt-5"]),Some("--version"),Some(&r),&probe("copilot")).unwrap();assert_eq!(plan.args,["--interactive=--version","--model","gpt-5"]);}

#[test]
fn test_seeded_pi_warns_and_prefixes_newline_for_parser_ambiguous_prompt(){for text in ["--help\nsecond line","-v","@README.md","config","install","list","remove","uninstall","update"]{let r=runner("pi",&["pi","{{prompt}}"]);let plan=build_launch_plan(&entry(),&paths(),&assembly(&["--model","fast"]),Some(text),Some(&r),&probe("pi")).unwrap();assert_eq!(plan.args,[format!("\n{text}"),"--model".to_owned(),"fast".to_owned()]);assert_eq!(plan.warnings,[LaunchWarning::PiPromptProtected]);}}

#[test]
fn test_seeded_pi_keeps_unambiguous_prompt_byte_exact(){for text in ["ordinary prompt","first line\nsecond line"," install","help"]{let r=runner("pi",&["pi","{{prompt}}"]);let plan=build_launch_plan(&entry(),&paths(),&Assembly::default(),Some(text),Some(&r),&probe("pi")).unwrap();assert_eq!(plan.args,[text]);assert!(plan.warnings.is_empty());}}

#[test]
fn test_user_edited_pi_command_keeps_the_compatibility_adapter(){let r=runner("my-pi",&["/opt/tools/pi.exe","--model","fast","{{prompt}}"]);let mut p=probe("/opt/tools/pi.exe");p.programs.insert("/opt/tools/pi.exe".to_owned(),PathBuf::from("/opt/tools/pi.exe"));let plan=build_launch_plan(&entry(),&paths(),&Assembly::default(),Some("@notes.md"),Some(&r),&p).unwrap();assert_eq!(plan.args,["--model","fast","\n@notes.md"]);assert_eq!(plan.warnings,[LaunchWarning::PiPromptProtected]);}

#[test]
fn test_seeded_cursor_selects_agent_before_passing_prompt(){for text in ["--help\nsecond line","status"]{let r=runner("cursor",&["cursor-agent","--","agent","{{prompt}}"]);let plan=build_launch_plan(&entry(),&paths(),&assembly(&["--model","gpt-5"]),Some(text),Some(&r),&probe("cursor-agent")).unwrap();assert_eq!(plan.args,["--model","gpt-5","--","agent",text]);}}

#[test]
fn test_build_refuses_nul_in_prompt_as_launch_error(){let r=runner("rec",&["rec-bin","{{prompt}}"]);assert!(matches!(build_launch_plan(&entry(),&paths(),&Assembly::default(),Some("bad\0prompt"),Some(&r),&probe("rec-bin")),Err(LaunchError::PromptContainsNul)));}

#[test]
fn test_build_over_long_render_is_a_clean_launch_error(){let r=runner("rec",&["rec-bin","{{prompt}}"]);let error=build_launch_plan(&entry(),&paths(),&Assembly::default(),Some(&"x".repeat(100_100)),Some(&r),&probe("rec-bin")).unwrap_err();let shown=error.to_string();assert!(matches!(error,LaunchError::PromptArgvTooLong{..}));assert!(shown.contains("bytes")||shown.contains("UTF-16"),"{shown}");assert!(!shown.contains("characters"),"{shown}");}

#[test]
fn test_describe_with_runner_shows_the_real_argv(){let r=runner("rec",&["rec-bin","{{prompt}}"]);let plan=build_launch_preview(&entry(),&paths(),&Assembly::default(),Some("Do •••\n"),Some("Do •••\n"),Some(&r),&probe("rec-bin")).unwrap();assert!(plan.display.contains("rec-bin"),"{}",plan.display);assert!(plan.display.contains("•••"),"{}",plan.display);}

#[test]
fn test_validate_argv_without_a_display_twin_returns_the_real_prompt(){let r=runner("rec",&["rec-bin","{{prompt}}"]);let plan=build_launch_plan(&entry(),&paths(),&assembly(&["--model","fast"]),Some("Do actual-value\n"),Some(&r),&probe("rec-bin")).unwrap();assert!(plan.display.contains("rec-bin"),"{}",plan.display);assert!(plan.display.contains("actual-value"),"{}",plan.display);assert!(plan.display.contains("--model fast"),"{}",plan.display);}
