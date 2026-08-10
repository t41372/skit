//! Mechanical port of `origin/main@206f9ef:tests/test_launcher_fix.py`.
//!
//! Keep one Rust test per Python `def test_*`. Private Python helpers map to the public Rust
//! launch-planning and execution seams so the assertions still exercise production behavior.

use std::{
    cell::Cell,
    collections::BTreeMap,
    path::{Path, PathBuf},
};

use skit_application::delivery::Assembly;
use skit_domain::{Entry, EntryKind, EntryMeta, EntrySettings, Slug};
use skit_runtime::{
    LaunchError, LaunchPaths, LaunchPlan, ProgramProbe, build_launch_plan, execute_launch,
    render_command_template,
};
use tempfile::TempDir;

#[derive(Debug, Default)]
struct FakeProbe {
    programs: BTreeMap<String, PathBuf>,
    files: Vec<PathBuf>,
    dirs: Vec<PathBuf>,
    executable: Vec<PathBuf>,
    find_calls: Cell<usize>,
}

impl ProgramProbe for FakeProbe {
    fn find_program(&self, name: &str) -> Option<PathBuf> {
        self.find_calls.set(self.find_calls.get() + 1);
        self.programs.get(name).cloned()
    }

    fn is_file(&self, path: &Path) -> bool {
        self.files.iter().any(|candidate| candidate == path)
    }

    fn is_dir(&self, path: &Path) -> bool {
        self.dirs.iter().any(|candidate| candidate == path)
    }

    fn is_executable(&self, path: &Path) -> bool {
        self.executable.iter().any(|candidate| candidate == path)
    }
}

fn entry(kind: &str) -> Entry {
    Entry {
        slug: Slug::parse("demo").unwrap(),
        meta: EntryMeta::minimal("Demo", EntryKind::parse(kind).unwrap()),
    }
}

fn command_entry(template: &str) -> Entry {
    let mut entry = entry("command");
    entry.meta.workdir = "invoke".to_owned();
    EntrySettings {
        template: template.to_owned(),
        ..EntrySettings::default()
    }
    .write_to_meta(&mut entry.meta);
    entry
}

fn paths(script: impl Into<PathBuf>, invoke_cwd: impl Into<PathBuf>) -> LaunchPaths {
    LaunchPaths {
        script: script.into(),
        entry_dir: PathBuf::from("/data/scripts/demo"),
        invoke_cwd: invoke_cwd.into(),
    }
}

fn command_assembly(values: &[(&str, &str)]) -> Assembly {
    let values = values
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect::<BTreeMap<_, _>>();
    Assembly {
        command_values: values.clone(),
        masked_command_values: values,
        ..Assembly::default()
    }
}

#[test]
fn test_placeholder_value_with_double_braces_round_trips() {
    let rendered = render_command_template(
        "run --q {q}",
        &BTreeMap::from([("q".to_owned(), "{{ .name }}".to_owned())]),
    )
    .unwrap();

    #[cfg(windows)]
    assert_eq!(rendered, "run --q \"{{ .name }}\"");
    #[cfg(not(windows))]
    assert_eq!(rendered, "run --q '{{ .name }}'");
}

#[test]
fn test_placeholder_value_with_double_braces_inside_quoted_template_slot() {
    let rendered = render_command_template(
        "echo {msg}",
        &BTreeMap::from([("msg".to_owned(), "prefix{{inner}}suffix".to_owned())]),
    )
    .unwrap();
    assert!(rendered.contains("prefix{{inner}}suffix"), "{rendered}");
}

#[test]
fn test_template_escape_still_unescaped_alongside_a_corrupting_value() {
    let rendered = render_command_template(
        "echo {{literal}} {msg}",
        &BTreeMap::from([("msg".to_owned(), "{{escaped-looking}}".to_owned())]),
    )
    .unwrap();
    assert!(rendered.contains("{literal}"), "{rendered}");
    assert!(rendered.contains("{{escaped-looking}}"), "{rendered}");
}

#[cfg(unix)]
#[test]
fn test_run_entry_executes_correctly_with_double_brace_value() {
    let root = TempDir::new().unwrap();
    let outfile = root.path().join("out.txt");
    let template = format!("printf \"%s\" {{msg}} > \"{}\"", outfile.display());
    let entry = command_entry(&template);
    let probe = FakeProbe {
        programs: BTreeMap::from([("sh".to_owned(), PathBuf::from("/bin/sh"))]),
        dirs: vec![root.path().to_path_buf()],
        ..FakeProbe::default()
    };
    let plan = build_launch_plan(
        &entry,
        &paths("/unused", root.path()),
        &command_assembly(&[("msg", "prefix{{inner}}suffix")]),
        None,
        None,
        &probe,
    )
    .unwrap();

    assert_eq!(execute_launch(&plan).unwrap(), 0);
    assert_eq!(
        std::fs::read_to_string(outfile).unwrap(),
        "prefix{{inner}}suffix"
    );
}

#[cfg(unix)]
fn shell_plan(root: &TempDir, command: &str) -> LaunchPlan {
    LaunchPlan {
        program: PathBuf::from("/bin/sh"),
        args: vec!["-c".to_owned(), command.to_owned()],
        env: BTreeMap::new(),
        cwd: root.path().to_path_buf(),
        display: String::new(),
        warnings: Vec::new(),
    }
}

#[cfg(unix)]
#[test]
fn test_normalize_exit_code_maps_negative_returncode_to_128_plus_n() {
    let root = TempDir::new().unwrap();
    assert_eq!(execute_launch(&shell_plan(&root, "kill -SEGV $$")).unwrap(), 139);
    assert_eq!(execute_launch(&shell_plan(&root, "kill -TERM $$")).unwrap(), 143);
    assert_eq!(execute_launch(&shell_plan(&root, "exit 0")).unwrap(), 0);
    assert_eq!(execute_launch(&shell_plan(&root, "exit 2")).unwrap(), 2);
}

#[cfg(unix)]
#[test]
fn test_run_entry_normalizes_signal_killed_child_to_shell_convention() {
    let root = TempDir::new().unwrap();
    let entry = command_entry("kill -TERM $$");
    let probe = FakeProbe {
        programs: BTreeMap::from([("sh".to_owned(), PathBuf::from("/bin/sh"))]),
        dirs: vec![root.path().to_path_buf()],
        ..FakeProbe::default()
    };
    let plan = build_launch_plan(
        &entry,
        &paths("/unused", root.path()),
        &Assembly::default(),
        None,
        None,
        &probe,
    )
    .unwrap();
    assert_eq!(execute_launch(&plan).unwrap(), 143);
}

#[test]
fn test_build_python_missing_script_raises_before_calling_ensure_uv() {
    let mut python = entry("python");
    python.meta.workdir = "invoke".to_owned();
    let probe = FakeProbe {
        programs: BTreeMap::from([("uv".to_owned(), PathBuf::from("/fake/uv"))]),
        dirs: vec![PathBuf::from("/invoke")],
        ..FakeProbe::default()
    };

    let error = build_launch_plan(
        &python,
        &paths("/data/scripts/demo/script.py", "/invoke"),
        &Assembly::default(),
        None,
        None,
        &probe,
    )
    .unwrap_err();
    assert!(matches!(error, LaunchError::TargetMissing { .. }));
    assert_eq!(probe.find_calls.get(), 0);
}

#[test]
fn test_build_python_healthy_script_still_calls_ensure_uv() {
    let mut python = entry("python");
    python.meta.workdir = "invoke".to_owned();
    let script = PathBuf::from("/data/scripts/demo/script.py");
    let probe = FakeProbe {
        programs: BTreeMap::from([("uv".to_owned(), PathBuf::from("/fake/uv"))]),
        files: vec![script.clone()],
        dirs: vec![PathBuf::from("/invoke")],
        ..FakeProbe::default()
    };

    let plan = build_launch_plan(
        &python,
        &paths(script, "/invoke"),
        &Assembly::default(),
        None,
        None,
        &probe,
    )
    .unwrap();
    assert_eq!(probe.find_calls.get(), 1);
    assert_eq!(plan.program, PathBuf::from("/fake/uv"));
}

#[cfg(not(windows))]
#[test]
fn test_placeholder_value_with_space_is_quoted_as_one_word() {
    let rendered = render_command_template(
        "ffmpeg -i {input} out.mp4",
        &BTreeMap::from([("input".to_owned(), "My Movie.mp4".to_owned())]),
    )
    .unwrap();
    assert_eq!(rendered, "ffmpeg -i 'My Movie.mp4' out.mp4");
}

#[cfg(not(windows))]
#[test]
fn test_placeholder_value_with_shell_metacharacters_cannot_inject() {
    let rendered = render_command_template(
        "echo {msg}",
        &BTreeMap::from([("msg".to_owned(), "a; rm -rf x".to_owned())]),
    )
    .unwrap();
    assert_eq!(rendered, "echo 'a; rm -rf x'");
}

#[cfg(unix)]
#[test]
fn test_run_entry_placeholder_value_with_space_reaches_child_intact() {
    let root = TempDir::new().unwrap();
    let outfile = root.path().join("out.txt");
    let template = format!(
        "printf \"%s|%s|\" {{a}} {{b}} > \"{}\"",
        outfile.display()
    );
    let entry = command_entry(&template);
    let probe = FakeProbe {
        programs: BTreeMap::from([("sh".to_owned(), PathBuf::from("/bin/sh"))]),
        dirs: vec![root.path().to_path_buf()],
        ..FakeProbe::default()
    };
    let plan = build_launch_plan(
        &entry,
        &paths("/unused", root.path()),
        &command_assembly(&[("a", "My Movie.mp4"), ("b", "second")]),
        None,
        None,
        &probe,
    )
    .unwrap();

    assert_eq!(execute_launch(&plan).unwrap(), 0);
    assert_eq!(
        std::fs::read_to_string(outfile).unwrap(),
        "My Movie.mp4|second|"
    );
}
