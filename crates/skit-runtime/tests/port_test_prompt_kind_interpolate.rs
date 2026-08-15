use std::{collections::BTreeMap,path::PathBuf};

use skit_application::delivery::Assembly;
use skit_domain::{Entry,EntryKind,EntryMeta,Slug};
use skit_language::render_prompt_body;
use skit_runtime::{LaunchPaths,ProgramProbe,PromptRunner,build_launch_plan,build_launch_preview};

struct Probe;
impl ProgramProbe for Probe{
    fn find_program(&self,name:&str)->Option<PathBuf>{(name=="agent").then(||PathBuf::from("/bin/agent"))}
    fn is_file(&self,path:&std::path::Path)->bool{path==std::path::Path::new("/data/scripts/p/prompt.md")}
    fn is_dir(&self,path:&std::path::Path)->bool{matches!(path.to_str(),Some("/invoke"|"/data/scripts/p"))}
    fn is_executable(&self,_:&std::path::Path)->bool{true}
}
fn entry()->Entry{let mut meta=EntryMeta::minimal("p",EntryKind::parse("prompt").unwrap());meta.workdir="invoke".to_owned();Entry{slug:Slug::parse("p").unwrap(),meta}}
fn paths()->LaunchPaths{LaunchPaths{script:PathBuf::from("/data/scripts/p/prompt.md"),entry_dir:PathBuf::from("/data/scripts/p"),invoke_cwd:PathBuf::from("/invoke")}}

#[test]
fn test_build_for_an_insertion_off_prompt_sends_the_body_verbatim(){
    let body="Keep {{a}} as-is\n"; let rendered=render_prompt_body(body,&BTreeMap::new(),false); assert_eq!(rendered,body);
    let runner=PromptRunner{name:"agent".to_owned(),argv:vec!["agent".to_owned(),"{{prompt}}".to_owned()]};
    let plan=build_launch_plan(&entry(),&paths(),&Assembly::default(),Some(&rendered),Some(&runner),&Probe).unwrap(); assert_eq!(plan.args,[body]);
    let preview=build_launch_preview(&entry(),&paths(),&Assembly::default(),Some(&rendered),Some(&rendered),Some(&runner),&Probe).unwrap(); assert!(preview.display.contains("Keep {{a}} as-is"),"{}",preview.display);
}
