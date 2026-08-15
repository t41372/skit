#![cfg(windows)]

use std::{collections::BTreeMap,path::PathBuf};

use skit_application::delivery::Assembly;
use skit_domain::{Entry,EntryKind,EntryMeta,Slug};
use skit_runtime::{LaunchError,LaunchPaths,ProgramProbe,PromptRunner,build_launch_plan};

const WINDOWS_COMMAND_LINE_LIMIT:usize=32_767;

struct Probe;
impl ProgramProbe for Probe{
 fn find_program(&self,name:&str)->Option<PathBuf>{(name=="agent").then(||PathBuf::from(r"C:\Tools\Agent App\agent.exe"))}
 fn is_file(&self,path:&std::path::Path)->bool{path==std::path::Path::new(r"C:\data\p\prompt.md")}
 fn is_dir(&self,path:&std::path::Path)->bool{matches!(path.to_str(),Some(r"C:\invoke"|r"C:\data\p"))}
 fn is_executable(&self,_:&std::path::Path)->bool{true}
}
fn entry()->Entry{let mut meta=EntryMeta::minimal("p",EntryKind::parse("prompt").unwrap());meta.workdir="invoke".to_owned();Entry{slug:Slug::parse("p").unwrap(),meta}}
fn paths()->LaunchPaths{LaunchPaths{script:PathBuf::from(r"C:\data\p\prompt.md"),entry_dir:PathBuf::from(r"C:\data\p"),invoke_cwd:PathBuf::from(r"C:\invoke")}}
fn runner()->PromptRunner{PromptRunner{name:"agent".to_owned(),argv:vec!["agent".to_owned(),"{{prompt}}".to_owned()]}}

fn quote_windows(argument:&str)->String{
 let needs=argument.is_empty()||argument.contains([' ','\t','"']);if !needs{return argument.to_owned();}
 let mut out=String::from("\"");let mut slashes=0usize;
 for ch in argument.chars(){if ch=='\\'{slashes+=1;continue;}if ch=='"'{out.extend(std::iter::repeat_n('\\',slashes*2+1));out.push('"');}else{out.extend(std::iter::repeat_n('\\',slashes));out.push(ch);}slashes=0;}
 out.extend(std::iter::repeat_n('\\',slashes*2));out.push('"');out
}
fn units(program:&str,args:&[String])->usize{
 std::iter::once(program.to_owned()).chain(args.iter().cloned()).map(|arg|quote_windows(&arg)).collect::<Vec<_>>().join(" ").encode_utf16().count()+1
}

#[test]
fn test_check_argv_length_windows_uses_real_quoted_command_line_utf16_units(){
 let program=r"C:\Tools\Agent App\agent.exe";let r=runner();
 // Use quote/backslash-heavy chunks so raw string length and actual quoted command-line size diverge.
 let chunk="arg \\\" ";
 let mut low=0usize;let mut high=20_000usize;
 while low+1<high{let mid=(low+high)/2;let prompt=chunk.repeat(mid);let argv=vec![prompt];if units(program,&argv)<=WINDOWS_COMMAND_LINE_LIMIT{low=mid}else{high=mid}}
 let accepted=chunk.repeat(low);let rejected=chunk.repeat(high);
 assert!(units(program,&[accepted.clone()])<=WINDOWS_COMMAND_LINE_LIMIT);assert!(units(program,&[rejected.clone()])>WINDOWS_COMMAND_LINE_LIMIT);
 build_launch_plan(&entry(),&paths(),&Assembly::default(),Some(&accepted),Some(&r),&Probe).expect("the last Windows command line within 32,767 UTF-16 units must launch");
 assert!(matches!(build_launch_plan(&entry(),&paths(),&Assembly::default(),Some(&rejected),Some(&r),&Probe),Err(LaunchError::PromptArgvTooLong{..})),"the first independently measured over-limit command line was accepted");
}
