use std::{collections::BTreeMap, path::PathBuf};

use skit_application::delivery::Assembly;
use skit_domain::{Entry, EntryKind, EntryMeta, Slug};
use skit_runtime::{LaunchError, LaunchPaths, ProgramProbe, PromptRunner, build_launch_plan};

#[derive(Default)]
struct Probe { programs: BTreeMap<String, PathBuf>, files: Vec<PathBuf>, dirs: Vec<PathBuf> }
impl ProgramProbe for Probe {
    fn find_program(&self, name: &str) -> Option<PathBuf> { self.programs.get(name).cloned() }
    fn is_file(&self, path: &std::path::Path) -> bool { self.files.iter().any(|item| item == path) }
    fn is_dir(&self, path: &std::path::Path) -> bool { self.dirs.iter().any(|item| item == path) }
    fn is_executable(&self, _path: &std::path::Path) -> bool { true }
}
fn entry() -> Entry {
    let mut meta=EntryMeta::minimal("Prompt",EntryKind::parse("prompt").unwrap()); meta.workdir="invoke".to_owned(); Entry{slug:Slug::parse("p").unwrap(),meta}
}
fn paths()->LaunchPaths { LaunchPaths{script:PathBuf::from("/data/scripts/p/prompt.md"),entry_dir:PathBuf::from("/data/scripts/p"),invoke_cwd:PathBuf::from("/invoke")} }
fn probe(program:&str)->Probe { Probe{programs:BTreeMap::from([(program.to_owned(),PathBuf::from(format!("/bin/{program}")))]),files:vec![PathBuf::from("/data/scripts/p/prompt.md")],dirs:vec![PathBuf::from("/invoke"),PathBuf::from("/data/scripts/p")]} }

#[test]
fn test_fill_runner_argv_replaces_the_one_slot_raw() {
    let rendered="line1\nline2 with {braces} and {{more}}";
    let runner=PromptRunner{name:"agent".to_owned(),argv:vec!["agent".to_owned(),"--m={{prompt}}".to_owned(),"{lit}".to_owned()]};
    let plan=build_launch_plan(&entry(),&paths(),&Assembly::default(),Some(rendered),Some(&runner),&probe("agent")).unwrap();
    assert_eq!(plan.program,PathBuf::from("/bin/agent"));
    assert_eq!(plan.args,[format!("--m={rendered}"),"{lit}".to_owned()]);
}

#[test]
fn test_fill_runner_argv_leaves_foreign_holes_verbatim() {
    let runner=PromptRunner{name:"agent".to_owned(),argv:vec!["agent".to_owned(),"{{prompt}}".to_owned(),"{{other}}".to_owned(),"{single}".to_owned()]};
    let plan=build_launch_plan(&entry(),&paths(),&Assembly::default(),Some("X"),Some(&runner),&probe("agent")).unwrap();
    assert_eq!(plan.args,["X","{{other}}","{single}"]);
}

#[test]
fn test_fill_runner_argv_puts_extra_options_before_end_of_options() {
    for (template, expected) in [
        (vec!["agent","--","{{prompt}}"],vec!["--model","opus","--","task"]),
        (vec!["agent","--prompt={{prompt}}"],vec!["--prompt=task","--model","opus"]),
        (vec!["agent","--prompt","{{prompt}}","--","literal","--"],vec!["--prompt","task","--model","opus","--","literal","--"]),
        (vec!["agent","--marker=--","{{prompt}}"],vec!["--marker=--","task","--model","opus"]),
    ] {
        let runner=PromptRunner{name:"agent".to_owned(),argv:template.into_iter().map(str::to_owned).collect()};
        let assembly=Assembly{args:vec!["--model".to_owned(),"opus".to_owned()],masked_args:vec!["--model".to_owned(),"opus".to_owned()],..Assembly::default()};
        let plan=build_launch_plan(&entry(),&paths(),&assembly,Some("task"),Some(&runner),&probe("agent")).unwrap();
        assert_eq!(plan.args,expected, "runner={:?}",runner.argv);
    }
}

#[test]
fn test_check_argv_length_refuses_nul_before_subprocess() {
    let runner=PromptRunner{name:"agent".to_owned(),argv:vec!["agent".to_owned(),"{{prompt}}".to_owned()]};
    assert!(matches!(build_launch_plan(&entry(),&paths(),&Assembly::default(),Some("before\0after"),Some(&runner),&probe("agent")),Err(LaunchError::PromptContainsNul)));
}

#[test]
fn test_argv_length_uses_full_runner_not_just_prompt() {
    let runner=PromptRunner{name:"agent".to_owned(),argv:vec!["agent".to_owned(),"x".repeat(100_100),"{{prompt}}".to_owned()]};
    assert!(matches!(build_launch_plan(&entry(),&paths(),&Assembly::default(),Some("tiny"),Some(&runner),&probe("agent")),Err(LaunchError::PromptArgvTooLong{..})));
}
