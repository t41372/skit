use std::{collections::BTreeMap, path::{Path, PathBuf}};

use skit_application::delivery::Assembly;
use skit_domain::{Entry, EntryKind, EntryMeta, Slug};
use skit_runtime::{LaunchPaths, ProgramProbe, build_launch_plan};

#[derive(Default)]
struct Probe {
    programs: BTreeMap<String, PathBuf>,
    files: Vec<PathBuf>,
    dirs: Vec<PathBuf>,
}

impl ProgramProbe for Probe {
    fn find_program(&self, name: &str) -> Option<PathBuf> {
        self.programs.get(name).cloned()
    }
    fn is_file(&self, path: &Path) -> bool {
        self.files.iter().any(|item| item == path)
    }
    fn is_dir(&self, path: &Path) -> bool {
        self.dirs.iter().any(|item| item == path)
    }
}

#[test]
fn test_execute_env_delivery_writes_no_temp_copy() {
    let script = PathBuf::from("/data/scripts/exsh2/script.sh");
    let entry_dir = PathBuf::from("/data/scripts/exsh2");
    let invoke = PathBuf::from("/invoke");
    let mut entry = Entry {
        slug: Slug::parse("exsh2").unwrap(),
        meta: EntryMeta::minimal("exsh2", EntryKind::parse("shell").unwrap()),
    };
    entry.meta.workdir = "invoke".to_owned();
    let paths = LaunchPaths {
        script: script.clone(),
        entry_dir: entry_dir.clone(),
        invoke_cwd: invoke.clone(),
    };
    let probe = Probe {
        programs: BTreeMap::from([("bash".to_owned(), PathBuf::from("/bin/bash"))]),
        files: vec![script.clone()],
        dirs: vec![entry_dir, invoke],
    };
    let assembly = Assembly {
        env_values: BTreeMap::from([("MODE".to_owned(), "manual".to_owned())]),
        ..Assembly::default()
    };

    let plan = build_launch_plan(&entry, &paths, &assembly, None, None, &probe).unwrap();
    assert_eq!(plan.program, PathBuf::from("/bin/bash"));
    assert_eq!(plan.args, [script.display().to_string()]);
    assert_eq!(plan.env, BTreeMap::from([("MODE".to_owned(), "manual".to_owned())]));
    assert!(assembly.inject_values.is_empty(), "env-only execute handoff must carry no source rewrite request");
}
