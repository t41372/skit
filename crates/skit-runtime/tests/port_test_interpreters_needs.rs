//! Required-command preflight ports from Python `tests/test_interpreters.py` at `main@206f9ef`.

use std::{collections::BTreeMap, path::PathBuf};

use skit_application::delivery::Assembly;
use skit_domain::{Entry, EntryKind, EntryMeta, EntrySettings, Slug};
use skit_runtime::{LaunchError, LaunchPaths, ProgramProbe, build_launch_plan};

#[derive(Debug, Default)]
struct Probe {
    programs: BTreeMap<String, PathBuf>,
}

impl Probe {
    fn with_programs(names: &[&str]) -> Self {
        Self {
            programs: names
                .iter()
                .map(|name| ((*name).to_owned(), PathBuf::from(format!("/bin/{name}"))))
                .collect(),
        }
    }
}

impl ProgramProbe for Probe {
    fn find_program(&self, name: &str) -> Option<PathBuf> {
        self.programs.get(name).cloned()
    }

    fn is_file(&self, path: &std::path::Path) -> bool {
        path == std::path::Path::new("/data/scripts/demo/script.sh")
    }

    fn is_dir(&self, path: &std::path::Path) -> bool {
        matches!(path.to_str(), Some("/data/scripts/demo" | "/invoke"))
    }

    fn is_executable(&self, path: &std::path::Path) -> bool {
        self.is_file(path)
    }
}

fn entry(needs: &[&str]) -> Entry {
    let mut entry = Entry {
        slug: Slug::parse("demo").unwrap(),
        meta: EntryMeta::minimal("Demo", EntryKind::parse("shell").unwrap()),
    };
    entry.meta.workdir = "invoke".to_owned();
    EntrySettings {
        needs: needs.iter().map(|value| (*value).to_owned()).collect(),
        ..EntrySettings::default()
    }
    .write_to_meta(&mut entry.meta);
    entry
}

fn paths() -> LaunchPaths {
    LaunchPaths {
        script: PathBuf::from("/data/scripts/demo/script.sh"),
        entry_dir: PathBuf::from("/data/scripts/demo"),
        invoke_cwd: PathBuf::from("/invoke"),
    }
}

#[test]
fn test_preflight_missing_needs() {
    let error = build_launch_plan(
        &entry(&["ffmpeg"]),
        &paths(),
        &Assembly::default(),
        None,
        None,
        &Probe::with_programs(&["bash"]),
    )
    .unwrap_err();
    assert!(
        matches!(&error, LaunchError::MissingNeed { name } if name == "ffmpeg"),
        "missing declared command was not the launch refusal: {error:?}"
    );
}

#[test]
fn test_preflight_all_needs_present() {
    let plan = build_launch_plan(
        &entry(&["ffmpeg"]),
        &paths(),
        &Assembly::default(),
        None,
        None,
        &Probe::with_programs(&["bash", "ffmpeg"]),
    )
    .unwrap();
    assert_eq!(plan.program, PathBuf::from("/bin/bash"));
}
