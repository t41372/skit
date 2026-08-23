use std::{collections::BTreeMap, path::PathBuf};

use skit_application::delivery::Assembly;
use skit_domain::{Entry, EntryKind, EntryMeta, EntrySettings, Slug, StorageMode};
use skit_runtime::{
    LaunchError, LaunchPaths, ProgramProbe, PromptRunner, build_launch_plan, build_launch_preview,
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
fn python_can_use_a_verified_private_uv_path() {
    let mut entry = entry("python");
    let settings = EntrySettings {
        interpreter: "/data/bin/uv".to_owned(),
        ..EntrySettings::default()
    };
    settings.write_to_meta(&mut entry.meta);
    let mut probe = probe_for("/data/scripts/demo/script.py");
    probe
        .programs
        .insert("/data/bin/uv".to_owned(), PathBuf::from("/data/bin/uv"));

    let plan = build_launch_plan(
        &entry,
        &paths("/data/scripts/demo/script.py"),
        &Assembly::default(),
        None,
        None,
        &probe,
    )
    .unwrap();

    assert_eq!(plan.program, PathBuf::from("/data/bin/uv"));
}

#[test]
fn python_preview_uses_the_configured_program_name_without_path_lookup() {
    let mut preview = entry("python");
    preview.meta.workdir = "invoke".to_owned();
    let probe = probe_for("/data/scripts/demo/script.py");

    let plan = build_launch_preview(
        &preview,
        &paths("/data/scripts/demo/script.py"),
        &Assembly::default(),
        None,
        None,
        None,
        &probe,
    )
    .unwrap();

    assert_eq!(plan.program, PathBuf::from("uv"));
    assert!(plan.display.starts_with("uv run --no-project"));
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

    let mut copied = entry("exe");
    copied.meta.mode = StorageMode::Copy;
    copied.meta.source = "/deleted/original".to_owned();
    copied.meta.workdir = "invoke".to_owned();
    let probe = probe_for("/data/scripts/demo/script");
    let error = build_launch_plan(
        &copied,
        &paths("/data/scripts/demo/script"),
        &Assembly::default(),
        None,
        None,
        &probe,
    )
    .unwrap_err();
    assert!(matches!(
        error,
        LaunchError::TargetMissing { path }
            if path.as_path() == std::path::Path::new("/deleted/original")
    ));
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
    // bun takes the `run` subcommand; only node runs the script bare.
    assert_eq!(plan.args, ["run", "/copy/script.js"]);

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

    probe
        .programs
        .insert("deno".to_owned(), PathBuf::from("/bin/deno"));
    let plan = build_launch_plan(
        &entry("ts"),
        &paths("/copy/script.js"),
        &Assembly::default(),
        None,
        None,
        &probe,
    )
    .unwrap();
    assert_eq!(plan.program, PathBuf::from("/bin/deno"));
    assert_eq!(plan.args, ["run", "--allow-all", "/copy/script.js"]);
}

#[test]
fn command_template_preserves_v040_substitution_semantics() {
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

    assert_eq!(
        render_command_template(
            "tool --name \"{name}\"",
            &BTreeMap::from([("name".to_owned(), "a b; $HOME".to_owned())]),
        )
        .unwrap(),
        if cfg!(windows) {
            "tool --name \"\"a b; $HOME\"\""
        } else {
            "tool --name \"a b; \\$HOME\""
        }
    );
    assert_eq!(
        render_command_template(
            "tool --name '{name}'",
            &BTreeMap::from([("name".to_owned(), "it's $HOME".to_owned())]),
        )
        .unwrap(),
        if cfg!(windows) {
            "tool --name '\"it's $HOME\"'"
        } else {
            "tool --name 'it'\\''s $HOME'"
        }
    );
    assert_eq!(
        render_command_template("tool {drift}", &BTreeMap::new()).unwrap(),
        "tool {drift}"
    );
    assert_eq!(
        render_command_template(
            "tool {value}",
            &BTreeMap::from([("value".to_owned(), "{{kept}}".to_owned())]),
        )
        .unwrap(),
        if cfg!(windows) {
            "tool {{kept}}"
        } else {
            "tool '{{kept}}'"
        }
    );
    assert_eq!(
        render_command_template("tool {not-valid!}", &BTreeMap::new()).unwrap(),
        "tool {not-valid!}"
    );
    assert_eq!(
        render_command_template("tool 'literal'", &BTreeMap::new()).unwrap(),
        "tool 'literal'"
    );
}

#[cfg(not(windows))]
#[test]
fn command_template_tracks_nested_posix_quote_contexts() {
    assert_eq!(
        render_command_template(
            r#"printf "%s\n" "$(printf %s {value})""#,
            &BTreeMap::from([("value".to_owned(), "safe; printf INJECTED".to_owned(),)]),
        )
        .unwrap(),
        r#"printf "%s\n" "$(printf %s 'safe; printf INJECTED')""#
    );
    assert_eq!(
        render_command_template(
            r#"printf "%s\n" "$(printf %s "{value}")""#,
            &BTreeMap::from([("value".to_owned(), "$(printf PWNED)".to_owned())]),
        )
        .unwrap(),
        r#"printf "%s\n" "$(printf %s "\$(printf PWNED)")""#
    );
    assert_eq!(
        render_command_template(
            r#"printf "%s\n" "`printf %s '{value}'`""#,
            &BTreeMap::from([("value".to_owned(), "it's $HOME".to_owned())]),
        )
        .unwrap(),
        r#"printf "%s\n" "`printf %s 'it'\''s $HOME'`""#
    );
}

#[cfg(not(windows))]
#[test]
fn a_closed_command_substitution_restores_the_outer_quote_context() {
    assert_eq!(
        render_command_template(
            r#"tool "$(printf done) {value}""#,
            &BTreeMap::from([("value".to_owned(), "$HOME".to_owned())]),
        )
        .unwrap(),
        r#"tool "$(printf done) \$HOME""#,
    );
}

#[cfg(not(windows))]
#[test]
fn command_template_neutralizes_a_dangling_escape_before_a_value() {
    assert_eq!(
        render_command_template(
            r#"printf "%s\n" "foo\{name}""#,
            &BTreeMap::from([("name".to_owned(), "$(printf pwned)".to_owned())]),
        )
        .unwrap(),
        r#"printf "%s\n" "foo\\\$(printf pwned)""#
    );
    assert_eq!(
        render_command_template(
            r#"printf "%s\n" "\{{x}} {later}""#,
            &BTreeMap::from([("later".to_owned(), "$(x)".to_owned())]),
        )
        .unwrap(),
        r#"printf "%s\n" "\{x} \$(x)""#
    );
}

#[cfg(not(windows))]
#[test]
fn command_template_refuses_only_unrepresentable_nested_backtick_quotes() {
    assert!(matches!(
        render_command_template(
            r#"printf "%s\n" "`printf %s "{value}"`""#,
            &BTreeMap::from([("value".to_owned(), "$(printf PWNED)".to_owned())]),
        ),
        Err(LaunchError::UnsafeTemplatePlaceholder { .. })
    ));

    assert_eq!(
        render_command_template(
            r#"printf "%s\n" "$(printf %s "{value}")""#,
            &BTreeMap::from([("value".to_owned(), "$(printf safe)".to_owned())]),
        )
        .unwrap(),
        r#"printf "%s\n" "$(printf %s "\$(printf safe)")""#
    );
}

// Command templates lower through `sh -c` only under cfg(not(windows)); on Windows the
// same builder takes the render_windows_command_template arm, so asserting the sh program
// or its POSIX-rendered argv states a unix contract.
#[cfg(unix)]
#[test]
fn command_template_appends_extra_arguments_after_rendering() {
    let mut command = entry("command");
    command.meta.workdir = "invoke".to_owned();
    EntrySettings {
        template: "echo {name}".to_owned(),
        ..EntrySettings::default()
    }
    .write_to_meta(&mut command.meta);
    let mut probe = probe_for("/unused");
    probe
        .programs
        .insert("sh".to_owned(), PathBuf::from("/bin/sh"));
    let assembly = Assembly {
        args: vec!["tail".to_owned(), "two words".to_owned()],
        masked_args: vec!["tail".to_owned(), "two words".to_owned()],
        command_values: BTreeMap::from([("name".to_owned(), "hello".to_owned())]),
        masked_command_values: BTreeMap::from([("name".to_owned(), "hello".to_owned())]),
        ..Assembly::default()
    };

    let plan =
        build_launch_plan(&command, &paths("/unused"), &assembly, None, None, &probe).unwrap();

    assert_eq!(plan.program, PathBuf::from("/bin/sh"));
    assert_eq!(plan.args, ["-c", "echo hello tail 'two words'"]);
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
    let empty = PromptRunner {
        name: "empty".to_owned(),
        argv: Vec::new(),
    };
    assert!(matches!(
        build_launch_plan(
            &prompt,
            &paths("/copy/prompt.md"),
            &Assembly::default(),
            Some("body"),
            Some(&empty),
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
    let missing_shell = build_launch_plan(
        &entry("shell"),
        &paths("/copy/script.sh"),
        &Assembly::default(),
        None,
        None,
        &shell_probe,
    );
    // A missing shell is a typed refusal on every host, and the variant follows each host's
    // adjudicated policy (`resolve_interpreter`): unix resolves interpreters from PATH only, so
    // the refusal is ProgramNotFound; Windows falls from PATH to the configured bash path and
    // refuses as WindowsShellMissing.
    #[cfg(not(windows))]
    assert!(matches!(
        missing_shell,
        Err(LaunchError::ProgramNotFound { .. })
    ));
    #[cfg(windows)]
    assert!(matches!(
        missing_shell,
        Err(LaunchError::WindowsShellMissing { .. })
    ));

    let mut missing_file_probe = shell_probe;
    missing_file_probe
        .programs
        .insert("bash".to_owned(), PathBuf::from("/bin/bash"));
    missing_file_probe.files.clear();
    assert!(matches!(
        build_launch_plan(
            &entry("shell"),
            &paths("/copy/script.sh"),
            &Assembly::default(),
            None,
            None,
            &missing_file_probe,
        ),
        Err(LaunchError::TargetMissing { .. })
    ));

    assert!(matches!(
        build_launch_plan(
            &entry("js"),
            &paths("/copy/tool.txt"),
            &Assembly::default(),
            None,
            None,
            &probe,
        ),
        Err(LaunchError::JsRuntimeMissing { names }) if names == "deno, bun, node"
    ));
}

#[test]
fn an_executable_preview_still_checks_the_local_file_and_permission_bits() {
    let mut exe = entry("exe");
    exe.meta.mode = StorageMode::Reference;
    exe.meta.source = "/bin/demo".to_owned();
    exe.meta.workdir = "invoke".to_owned();

    // The preview probe answers program lookups itself but delegates every file question.
    let plan = build_launch_preview(
        &exe,
        &paths("/bin/demo"),
        &Assembly::default(),
        None,
        None,
        None,
        &probe_for("/bin/demo"),
    )
    .unwrap();
    assert_eq!(plan.program, PathBuf::from("/bin/demo"));

    let mut present = probe_for("/bin/demo");
    present.executable.clear();
    let error = build_launch_preview(
        &exe,
        &paths("/bin/demo"),
        &Assembly::default(),
        None,
        None,
        None,
        &present,
    )
    .unwrap_err();
    assert!(
        matches!(error, LaunchError::TargetNotExecutable { .. }),
        "{error:?}"
    );

    let mut directory = probe_for("/bin/demo");
    directory.files.clear();
    directory.executable.clear();
    directory.dirs.push(PathBuf::from("/bin/demo"));
    let error = build_launch_preview(
        &exe,
        &paths("/bin/demo"),
        &Assembly::default(),
        None,
        None,
        None,
        &directory,
    )
    .unwrap_err();
    assert!(
        matches!(error, LaunchError::TargetNotExecutable { .. }),
        "{error:?}"
    );
}

#[test]
fn a_protected_pi_prompt_preview_shows_the_added_newline() {
    let mut prompt = entry("prompt");
    prompt.meta.workdir = "invoke".to_owned();
    let runner = PromptRunner {
        name: "pi".to_owned(),
        argv: vec!["pi".to_owned(), "{{prompt}}".to_owned()],
    };
    let probe = probe_for("/data/scripts/demo/prompt.md");

    let plan = build_launch_preview(
        &prompt,
        &paths("/data/scripts/demo/prompt.md"),
        &Assembly::default(),
        Some("--version"),
        Some("--version"),
        Some(&runner),
        &probe,
    )
    .unwrap();

    // The runner is Pi, so skit keeps the body in message mode and the preview shows it.
    assert!(plan.display.contains("\n--version"));
}
