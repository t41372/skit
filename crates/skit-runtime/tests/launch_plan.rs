use std::{collections::BTreeMap, path::PathBuf};

use skit_application::delivery::Assembly;
use skit_domain::{Entry, EntryKind, EntryMeta, EntrySettings, Slug, StorageMode};
use skit_runtime::{
    LaunchError, LaunchPaths, ProgramProbe, PromptRunner, build_launch_plan,
    render_command_template,
};

#[derive(Debug, Default)]
struct FakeProbe {
    programs: BTreeMap<String, PathBuf>,
    files: Vec<PathBuf>,
    dirs: Vec<PathBuf>,
    executable: Vec<PathBuf>,
}

impl ProgramProbe for FakeProbe {
    fn find_program(&self, name: &str) -> Option<PathBuf> {
        self.programs.get(name).cloned()
    }

    fn is_file(&self, path: &std::path::Path) -> bool {
        self.files.iter().any(|item| item == path)
    }

    fn is_dir(&self, path: &std::path::Path) -> bool {
        self.dirs.iter().any(|item| item == path)
    }

    fn is_executable(&self, path: &std::path::Path) -> bool {
        self.executable.iter().any(|item| item == path)
    }
}

fn entry(kind: &str) -> Entry {
    Entry {
        slug: Slug::parse("demo").unwrap(),
        meta: EntryMeta::minimal("Demo", EntryKind::parse(kind).unwrap()),
    }
}

fn paths(script: &str) -> LaunchPaths {
    LaunchPaths {
        script: PathBuf::from(script),
        entry_dir: PathBuf::from("/data/scripts/demo"),
        invoke_cwd: PathBuf::from("/invoke"),
    }
}

fn probe_for(script: &str) -> FakeProbe {
    FakeProbe {
        files: vec![PathBuf::from(script)],
        dirs: vec![
            PathBuf::from("/invoke"),
            PathBuf::from("/data/scripts/demo"),
        ],
        executable: vec![PathBuf::from(script)],
        ..FakeProbe::default()
    }
}

#[test]
fn python_uses_uv_no_project_and_keeps_reference_dependencies() {
    let mut entry = entry("python");
    entry.meta.mode = StorageMode::Reference;
    entry.meta.source = "/project/tool.py".to_owned();
    entry.meta.workdir = "invoke".to_owned();
    let settings = EntrySettings {
        requires_python: ">=3.13".to_owned(),
        dependencies: vec!["httpx>=0.28".to_owned(), "rich".to_owned()],
        ..EntrySettings::default()
    };
    settings.write_to_meta(&mut entry.meta);
    let mut probe = probe_for("/project/tool.py");
    probe
        .programs
        .insert("uv".to_owned(), PathBuf::from("/bin/uv"));

    let plan = build_launch_plan(
        &entry,
        &paths("/project/tool.py"),
        &Assembly {
            args: vec!["--count".to_owned(), "3".to_owned()],
            masked_args: vec!["--count".to_owned(), "3".to_owned()],
            ..Assembly::default()
        },
        None,
        None,
        &probe,
    )
    .unwrap();

    assert_eq!(plan.program, PathBuf::from("/bin/uv"));
    assert_eq!(
        plan.args,
        [
            "run",
            "--no-project",
            "--python",
            ">=3.13",
            "--with",
            "httpx>=0.28",
            "--with",
            "rich",
            "--script",
            "/project/tool.py",
            "--count",
            "3",
        ]
    );
    assert_eq!(plan.cwd, PathBuf::from("/invoke"));
}

#[test]
fn direct_and_interpreted_kinds_use_the_expected_program_shapes() {
    let cases = [
        ("shell", "bash", vec!["/copy/script.sh"]),
        ("fish", "fish", vec!["/copy/script.fish"]),
        ("powershell", "pwsh", vec!["-File", "/copy/script.ps1"]),
        ("ruby", "ruby", vec!["/copy/script.rb"]),
        ("perl", "perl", vec!["/copy/script.pl"]),
        ("lua", "lua", vec!["/copy/script.lua"]),
        ("r", "Rscript", vec!["/copy/script.r"]),
    ];

    for (kind, program, expected_prefix) in cases {
        let script = expected_prefix.last().unwrap();
        let mut probe = probe_for(script);
        probe
            .programs
            .insert(program.to_owned(), PathBuf::from(format!("/bin/{program}")));
        let plan = build_launch_plan(
            &entry(kind),
            &paths(script),
            &Assembly {
                args: vec!["tail".to_owned()],
                masked_args: vec!["tail".to_owned()],
                ..Assembly::default()
            },
            None,
            None,
            &probe,
        )
        .unwrap();
        assert_eq!(plan.program, PathBuf::from(format!("/bin/{program}")));
        let mut expected = expected_prefix
            .into_iter()
            .map(str::to_owned)
            .collect::<Vec<_>>();
        expected.push("tail".to_owned());
        assert_eq!(plan.args, expected, "kind={kind}");
    }

    let mut exe = entry("exe");
    exe.meta.mode = StorageMode::Reference;
    exe.meta.source = "/bin/demo".to_owned();
    exe.meta.workdir = "invoke".to_owned();
    let probe = probe_for("/bin/demo");
    let plan = build_launch_plan(
        &exe,
        &paths("/bin/demo"),
        &Assembly {
            args: vec!["x".to_owned()],
            masked_args: vec!["x".to_owned()],
            ..Assembly::default()
        },
        None,
        None,
        &probe,
    )
    .unwrap();
    assert_eq!(plan.program, PathBuf::from("/bin/demo"));
    assert_eq!(plan.args, ["x"]);
}

#[test]
fn javascript_runtime_order_is_deno_then_bun_then_node_and_a_pin_wins() {
    let mut probe = probe_for("/copy/script.js");
    probe
        .programs
        .insert("node".to_owned(), PathBuf::from("/bin/node"));
    probe
        .programs
        .insert("bun".to_owned(), PathBuf::from("/bin/bun"));
    let plan = build_launch_plan(
        &entry("js"),
        &paths("/copy/script.js"),
        &Assembly::default(),
        None,
        None,
        &probe,
    )
    .unwrap();
    assert_eq!(plan.program, PathBuf::from("/bin/bun"));
    assert_eq!(plan.args, ["/copy/script.js"]);

    let mut pinned = entry("js");
    let settings = EntrySettings {
        interpreter: "node".to_owned(),
        ..EntrySettings::default()
    };
    settings.write_to_meta(&mut pinned.meta);
    let plan = build_launch_plan(
        &pinned,
        &paths("/copy/script.js"),
        &Assembly::default(),
        None,
        None,
        &probe,
    )
    .unwrap();
    assert_eq!(plan.program, PathBuf::from("/bin/node"));
}

#[test]
fn command_template_quotes_unquoted_values_and_refuses_quoted_placeholders() {
    assert_eq!(
        render_command_template(
            "tool --name {name} --count {count}",
            &BTreeMap::from([
                ("name".to_owned(), "a b; echo no".to_owned()),
                ("count".to_owned(), "3".to_owned()),
            ]),
        )
        .unwrap(),
        if cfg!(windows) {
            "tool --name \"a b; echo no\" --count 3"
        } else {
            "tool --name 'a b; echo no' --count 3"
        }
    );

    assert!(matches!(
        render_command_template(
            "tool --name \"{name}\"",
            &BTreeMap::from([("name".to_owned(), "value".to_owned())]),
        ),
        Err(LaunchError::UnsafeTemplatePlaceholder { .. })
    ));
}

#[test]
fn prompt_runner_is_argv_only_and_requires_one_prompt_token() {
    let mut prompt = entry("prompt");
    prompt.meta.workdir = "invoke".to_owned();
    let runner = PromptRunner {
        name: "agent".to_owned(),
        argv: vec![
            "agent".to_owned(),
            "run".to_owned(),
            "{{prompt}}".to_owned(),
        ],
    };
    let mut probe = probe_for("/copy/prompt.md");
    probe
        .programs
        .insert("agent".to_owned(), PathBuf::from("/bin/agent"));

    let plan = build_launch_plan(
        &prompt,
        &paths("/copy/prompt.md"),
        &Assembly::default(),
        Some("Do the task."),
        Some(&runner),
        &probe,
    )
    .unwrap();

    assert_eq!(plan.program, PathBuf::from("/bin/agent"));
    assert_eq!(plan.args, ["run", "Do the task."]);

    let invalid = PromptRunner {
        name: "bad".to_owned(),
        argv: vec!["agent".to_owned(), "run".to_owned()],
    };
    assert!(matches!(
        build_launch_plan(
            &prompt,
            &paths("/copy/prompt.md"),
            &Assembly::default(),
            Some("body"),
            Some(&invalid),
            &probe,
        ),
        Err(LaunchError::InvalidPromptRunner { .. })
    ));
}

#[test]
fn needs_workdir_and_target_checks_fail_before_spawn() {
    let mut shell = entry("shell");
    shell.meta.source = "/old/script.sh".to_owned();
    shell.meta.workdir = "origin".to_owned();
    let settings = EntrySettings {
        needs: vec!["jq".to_owned()],
        ..EntrySettings::default()
    };
    settings.write_to_meta(&mut shell.meta);
    let mut probe = probe_for("/copy/script.sh");
    probe
        .programs
        .insert("bash".to_owned(), PathBuf::from("/bin/bash"));

    assert!(matches!(
        build_launch_plan(
            &shell,
            &paths("/copy/script.sh"),
            &Assembly::default(),
            None,
            None,
            &probe,
        ),
        Err(LaunchError::MissingNeed { name }) if name == "jq"
    ));

    probe
        .programs
        .insert("jq".to_owned(), PathBuf::from("/bin/jq"));
    let plan = build_launch_plan(
        &shell,
        &paths("/copy/script.sh"),
        &Assembly::default(),
        None,
        None,
        &probe,
    )
    .unwrap();
    assert_eq!(plan.cwd, PathBuf::from("/invoke"));
}

#[test]
fn unknown_kinds_and_missing_runtimes_are_typed_refusals() {
    let probe = probe_for("/copy/tool.txt");
    assert!(matches!(
        build_launch_plan(
            &entry("future-kind"),
            &paths("/copy/tool.txt"),
            &Assembly::default(),
            None,
            None,
            &probe,
        ),
        Err(LaunchError::UnknownKind { .. })
    ));

    let shell_probe = probe_for("/copy/script.sh");
    assert!(matches!(
        build_launch_plan(
            &entry("shell"),
            &paths("/copy/script.sh"),
            &Assembly::default(),
            None,
            None,
            &shell_probe,
        ),
        Err(LaunchError::ProgramNotFound { .. })
    ));
}
