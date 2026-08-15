#![cfg(windows)]

use std::{collections::BTreeMap,path::PathBuf};

use skit_application::delivery::Assembly;
use skit_domain::{Entry,EntryKind,EntryMeta,Slug};
use skit_runtime::{LaunchError,LaunchPaths,ProgramProbe,PromptRunner,build_launch_plan};

// Frozen Python explicitly monkeypatches ARGV_LIMIT to 60,000 and measures the fully quoted
// CreateProcessW command line in UTF-16LE bytes. Do not substitute the unrelated Win32 32,767-unit
// ceiling here; this is the skit safety margin contract.
const FROZEN_WINDOWS_ARGV_LIMIT_BYTES:usize=60_000;

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

// Independent Microsoft-CRT/list2cmdline quoting oracle. This is test code, not copied from the
// runtime implementation, and deliberately computes the byte size from the runner's configured
// argv token (`agent`), which is what Python check_argv_length receives before PATH resolution.
fn quote_windows(argument:&str)->String{
 let needs=argument.is_empty()||argument.contains([' ','\t','"']);if !needs{return argument.to_owned();}
 let mut out=String::from("\"");let mut slashes=0usize;
 for ch in argument.chars(){if ch=='\\'{slashes+=1;continue;}if ch=='"'{out.extend(std::iter::repeat_n('\\',slashes*2+1));out.push('"');}else{out.extend(std::iter::repeat_n('\\',slashes));out.push(ch);}slashes=0;}
 out.extend(std::iter::repeat_n('\\',slashes*2));out.push('"');out
}
fn quoted_utf16le_bytes(argv:&[String])->usize{
 argv.iter().map(|arg|quote_windows(arg)).collect::<Vec<_>>().join(" ").encode_utf16().count()*2+2
}

#[test]
fn test_check_argv_length_measures_windows_quoted_utf16(){
 let r=runner();
 // Backslashes before quotes expand under Windows quoting, so raw UTF-8 length stays below the
 // frozen limit while the actual UTF-16LE command line crosses it.
 let chunk="\\\"";
 let mut low=0usize;let mut high=30_000usize;
 while low+1<high{
   let mid=(low+high)/2;let prompt=chunk.repeat(mid);let argv=vec!["agent".to_owned(),prompt];
   if quoted_utf16le_bytes(&argv)<=FROZEN_WINDOWS_ARGV_LIMIT_BYTES{low=mid}else{high=mid}
 }
 let accepted=chunk.repeat(low);let rejected=chunk.repeat(high);
 let accepted_argv=vec!["agent".to_owned(),accepted.clone()];let rejected_argv=vec!["agent".to_owned(),rejected.clone()];
 assert!(accepted_argv.iter().map(|v|v.len()).sum::<usize>()<FROZEN_WINDOWS_ARGV_LIMIT_BYTES,"control must still fit a naive/raw byte count");
 assert!(quoted_utf16le_bytes(&accepted_argv)<=FROZEN_WINDOWS_ARGV_LIMIT_BYTES);
 assert!(quoted_utf16le_bytes(&rejected_argv)>FROZEN_WINDOWS_ARGV_LIMIT_BYTES);
 build_launch_plan(&entry(),&paths(),&Assembly::default(),Some(&accepted),Some(&r),&Probe).expect("the last independently measured <=60,000-byte Windows command line must launch");
 let error=build_launch_plan(&entry(),&paths(),&Assembly::default(),Some(&rejected),Some(&r),&Probe).expect_err("the first independently measured >60,000-byte Windows command line was accepted");
 assert!(matches!(error,LaunchError::PromptArgvTooLong{limit:60_000,unit:"bytes",..}),"wrong Windows argv refusal contract: {error:?}");
}
