use std::fs;

use predicates::prelude::*;
use tempfile::TempDir;

struct Sandbox {
    data: TempDir,
    state: TempDir,
    config: TempDir,
}

impl Sandbox {
    fn new() -> Self {
        Self {
            data: TempDir::new().unwrap(),
            state: TempDir::new().unwrap(),
            config: TempDir::new().unwrap(),
        }
    }

    fn command(&self) -> assert_cmd::Command {
        let mut command = assert_cmd::cargo::cargo_bin_cmd!("skit");
        command
            .env("SKIT_DATA_DIR", self.data.path())
            .env("SKIT_STATE_DIR", self.state.path())
            .env("SKIT_CONFIG_DIR", self.config.path())
            .env("SKIT_LANG", "en");
        command
    }
}

#[test]
fn add_expands_the_user_home_before_it_resolves_a_source() {
    let sandbox = Sandbox::new();
    let source = sandbox.data.path().join("home-source.py");
    fs::write(&source, "print('home')\n").unwrap();

    sandbox
        .command()
        .env("HOME", sandbox.data.path())
        .env("USERPROFILE", sandbox.data.path())
        .args(["add", "~/home-source.py", "--name", "Home", "--no-input"])
        .assert()
        .success();

    assert_eq!(
        fs::read(sandbox.data.path().join("scripts/home/script.py")).unwrap(),
        b"print('home')\n"
    );
}

#[test]
fn prompt_without_a_path_reads_a_pipe_in_no_input_mode() {
    let sandbox = Sandbox::new();
    sandbox
        .command()
        .args(["add", "--prompt", "--name", "Review", "--no-input"])
        .write_stdin("Review {{subject}}.\n")
        .assert()
        .success();

    assert_eq!(
        fs::read(sandbox.data.path().join("scripts/review/prompt.md")).unwrap(),
        b"Review {{subject}}.\n"
    );
}

#[test]
fn piped_text_defaults_to_python_but_honors_a_known_shebang() {
    let sandbox = Sandbox::new();
    sandbox
        .command()
        .args(["add", "-", "--name", "Default pipe", "--no-input"])
        .write_stdin("print('python')\n")
        .assert()
        .success();
    assert_eq!(
        fs::read(sandbox.data.path().join("scripts/default-pipe/script.py")).unwrap(),
        b"print('python')\n"
    );

    sandbox
        .command()
        .args(["add", "-", "--name", "Shell pipe", "--no-input"])
        .write_stdin("#!/bin/sh\nprintf ok\n")
        .assert()
        .success();
    assert_eq!(
        fs::read(sandbox.data.path().join("scripts/shell-pipe/script.sh")).unwrap(),
        b"#!/bin/sh\nprintf ok\n"
    );
}

#[test]
fn an_unknown_piped_shebang_requires_an_explicit_kind() {
    let sandbox = Sandbox::new();
    sandbox
        .command()
        .args(["add", "-", "--name", "Unknown pipe", "--no-input"])
        .write_stdin("#!/usr/bin/awk -f\n{ print $0 }\n")
        .assert()
        .code(2)
        .stderr(predicate::str::contains(
            "piped text's #! names no interpreter skit knows",
        ));
    assert!(!sandbox.data.path().join("scripts/unknown-pipe").exists());
}

#[test]
fn no_input_onboarding_reports_source_facts_without_guessing_a_selection() {
    let sandbox = Sandbox::new();
    let source = sandbox.data.path().join("arguments.py");
    fs::write(
        &source,
        "OUTPUT = 'report.txt'\nimport sys\nprint(sys.argv, 'input.csv')\n",
    )
    .unwrap();

    sandbox
        .command()
        .args([
            "add",
            source.to_str().unwrap(),
            "--name",
            "Arguments",
            "--no-input",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("reads command-line arguments"))
        .stdout(predicate::str::contains("input.csv"));

    let stored =
        fs::read_to_string(sandbox.data.path().join("scripts/arguments/script.py")).unwrap();
    assert!(!stored.contains("[tool.skit]"), "{stored}");
}

#[test]
fn an_existing_python_metadata_fence_is_the_add_time_authority() {
    let sandbox = Sandbox::new();
    let source = sandbox.data.path().join("owned.py");
    let original = concat!(
        "# /// script\n",
        "# dependencies = [\"block-dep\"]\n",
        "# requires-python = \">=3.11\"\n",
        "# [tool.future]\n",
        "# value = \"keep\"\n",
        "# ///\n",
        "print('ok')\n",
    );
    fs::write(&source, original).unwrap();

    sandbox
        .command()
        .args(["add", source.to_str().unwrap(), "--name", "Owned"])
        .args(["--dep", "ignored-dep", "--python", ">=3.13", "--no-input"])
        .assert()
        .success();

    let stored = fs::read(sandbox.data.path().join("scripts/owned/script.py")).unwrap();
    assert_eq!(stored, original.as_bytes());
    let meta = fs::read_to_string(sandbox.data.path().join("scripts/owned/meta.toml")).unwrap();
    assert!(!meta.contains("ignored-dep"), "{meta}");
    assert!(!meta.contains(">=3.13"), "{meta}");
}

#[test]
fn a_complete_but_invalid_python_metadata_fence_is_not_replaced_on_add() {
    let sandbox = Sandbox::new();
    let source = sandbox.data.path().join("malformed.py");
    let original = b"# /// script\n# dependencies =\n# ///\nprint('ok')\n";
    fs::write(&source, original).unwrap();

    sandbox
        .command()
        .args(["add", source.to_str().unwrap(), "--name", "Malformed"])
        .args(["--dep", "ignored-dep", "--python", ">=3.13", "--no-input"])
        .assert()
        .success();

    assert_eq!(
        fs::read(sandbox.data.path().join("scripts/malformed/script.py")).unwrap(),
        original
    );
    let meta = fs::read_to_string(sandbox.data.path().join("scripts/malformed/meta.toml")).unwrap();
    assert!(!meta.contains("ignored-dep"), "{meta}");
    assert!(!meta.contains(">=3.13"), "{meta}");
}

#[test]
fn an_authoritative_python_fence_does_not_hide_invalid_explicit_flags() {
    let sandbox = Sandbox::new();
    let source = sandbox.data.path().join("owned.py");
    fs::write(
        &source,
        "# /// script\n# dependencies = []\n# ///\nprint('ok')\n",
    )
    .unwrap();

    sandbox
        .command()
        .args(["add", source.to_str().unwrap(), "--name", "Owned"])
        .args(["--dep", "requests=>2", "--no-input"])
        .assert()
        .code(2);
    assert!(!sandbox.data.path().join("scripts/owned").exists());
}

#[test]
fn edit_and_no_input_are_an_explicit_usage_conflict() {
    Sandbox::new()
        .command()
        .args(["add", "--edit", "--name", "Draft", "--no-input"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("standard input"));
}

#[cfg(unix)]
#[test]
fn edit_creates_a_python_draft_and_removes_it_after_copying() {
    use std::os::unix::fs::PermissionsExt as _;

    let sandbox = Sandbox::new();
    let editor = sandbox.config.path().join("editor.sh");
    fs::write(&editor, "#!/bin/sh\nprintf 'print(42)\\n' > \"$1\"\n").unwrap();
    let mut permissions = fs::metadata(&editor).unwrap().permissions();
    permissions.set_mode(0o700);
    fs::set_permissions(&editor, permissions).unwrap();
    fs::create_dir_all(sandbox.config.path()).unwrap();
    fs::write(
        sandbox.config.path().join("config.toml"),
        format!("editor = {:?}\n", editor.display().to_string()),
    )
    .unwrap();

    sandbox
        .command()
        .args(["add", "--edit", "--name", "Draft"])
        .assert()
        .success();

    assert_eq!(
        fs::read(sandbox.data.path().join("scripts/draft/script.py")).unwrap(),
        b"print(42)\n"
    );
    let drafts = sandbox.data.path().join("drafts");
    assert!(
        !drafts.exists() || fs::read_dir(drafts).unwrap().next().is_none(),
        "a copied draft must not remain after success"
    );
}

#[test]
fn add_lane_selectors_are_mutually_exclusive() {
    let sandbox = Sandbox::new();
    let source = sandbox.data.path().join("source.py");
    fs::write(&source, "print(1)\n").unwrap();
    sandbox
        .command()
        .args(["add", source.to_str().unwrap(), "--cmd", "echo {name}"])
        .assert()
        .code(2);
    sandbox
        .command()
        .args(["add", source.to_str().unwrap(), "--edit"])
        .assert()
        .code(2);
}

#[test]
fn copied_javascript_adds_static_external_imports_as_private_dependencies() {
    let sandbox = Sandbox::new();
    let source = sandbox.data.path().join("tool.mjs");
    fs::write(
        &source,
        "import chalk from 'chalk';\nimport local from './local.mjs';\n",
    )
    .unwrap();

    sandbox
        .command()
        .args(["add", source.to_str().unwrap(), "--name", "Color"])
        .assert()
        .success();

    let meta = fs::read_to_string(sandbox.data.path().join("scripts/color/meta.toml")).unwrap();
    assert!(meta.contains("dependencies = [\"chalk\"]"));
    assert!(!meta.contains("./local.mjs"));
}

#[test]
fn copied_python_excludes_modules_next_to_the_original_source() {
    let sandbox = Sandbox::new();
    let source_dir = sandbox.data.path().join("source");
    fs::create_dir(&source_dir).unwrap();
    fs::write(source_dir.join("helpers.py"), "VALUE = 1\n").unwrap();
    let source = source_dir.join("tool.py");
    fs::write(&source, "import helpers\nimport requests\n").unwrap();

    sandbox
        .command()
        .args(["add", source.to_str().unwrap(), "--name", "Local import"])
        .assert()
        .success();

    let stored =
        fs::read_to_string(sandbox.data.path().join("scripts/local-import/script.py")).unwrap();
    assert!(stored.contains("dependencies = [\"requests\"]"));
    assert!(!stored.contains("\"helpers\""));
}

#[test]
fn copied_module_extensions_resolve_as_present_in_list_and_show() {
    let sandbox = Sandbox::new();
    let source = sandbox.data.path().join("tool.mjs");
    fs::write(&source, "export default 1;\n").unwrap();

    sandbox
        .command()
        .args(["add", source.to_str().unwrap(), "--name", "Module"])
        .assert()
        .success();

    sandbox
        .command()
        .args(["list", "--json"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"missing\":false"));
    sandbox
        .command()
        .args(["show", "module", "--json"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"missing\":false"));
}

#[cfg(unix)]
#[test]
fn executable_sources_are_references_and_permission_inference_is_supported() {
    let sandbox = Sandbox::new();
    // Running a copied system binary is racy with macOS provenance enforcement: the first launch
    // can be killed after the xattr is attached asynchronously. The original executable still
    // exercises permission-based inference and reference-mode execution without that OS race.
    let source = std::path::Path::new("/bin/dash");

    sandbox
        .command()
        .args(["add", source.to_str().unwrap(), "--name", "Executable"])
        .assert()
        .success();
    let meta =
        fs::read_to_string(sandbox.data.path().join("scripts/executable/meta.toml")).unwrap();
    assert!(meta.contains("kind = \"exe\""));
    assert!(meta.contains("mode = \"reference\""));
    assert!(
        !sandbox
            .data
            .path()
            .join("scripts/executable/script")
            .exists()
    );
    sandbox
        .command()
        .args(["run", "executable", "--no-input"])
        .assert()
        .success()
        .stdout(predicate::str::contains("→ ").and(predicate::str::contains("dash")));
}

#[test]
fn an_explicit_executable_directory_is_recorded_then_refused_as_not_executable() {
    let sandbox = Sandbox::new();
    let source = sandbox.state.path().join("program-directory");
    fs::create_dir(&source).unwrap();

    sandbox
        .command()
        .args([
            "add",
            source.to_str().unwrap(),
            "--exe",
            "--name",
            "Directory program",
        ])
        .assert()
        .success();

    let metadata = fs::read_to_string(
        sandbox
            .data
            .path()
            .join("scripts/directory-program/meta.toml"),
    )
    .unwrap();
    assert!(metadata.contains("kind = \"exe\""));
    assert!(metadata.contains("mode = \"reference\""));
    assert!(metadata.contains("source_hash = \"\""));

    for extra in [Vec::<&str>::new(), vec!["--dry-run"]] {
        let mut arguments = vec!["run", "directory-program", "--no-input"];
        arguments.extend(extra);
        sandbox
            .command()
            .args(arguments)
            .assert()
            .code(126)
            .stderr(predicate::str::contains("not executable"));
    }
}

#[test]
fn add_refuses_empty_or_conflicting_lane_controls() {
    let sandbox = Sandbox::new();
    let source = sandbox.data.path().join("source.py");
    fs::write(&source, "print(1)\n").unwrap();

    sandbox
        .command()
        .args(["add", "--cmd", "", "--name", "Empty"])
        .assert()
        .code(2);
    sandbox
        .command()
        .args(["add", "--cmd", "true"])
        .assert()
        .code(2);
    for arguments in [
        vec!["add", source.to_str().unwrap(), "--prompt", "--exe"],
        vec![
            "add",
            source.to_str().unwrap(),
            "--prompt",
            "--kind",
            "python",
        ],
        vec!["add", "--cmd", "true", "--prompt"],
        vec!["add", "--cmd", "true", "--exe"],
    ] {
        sandbox.command().args(arguments).assert().code(2);
    }
}

#[test]
fn python_metadata_is_validated_before_add_and_written_to_the_stored_copy() {
    let sandbox = Sandbox::new();
    let source = sandbox.data.path().join("tool.py");
    fs::write(&source, "#!/usr/bin/env python3.12.1\nprint('ok')\n").unwrap();

    sandbox
        .command()
        .args([
            "add",
            source.to_str().unwrap(),
            "--name",
            "Python tool",
            "--dep",
            "requests>=2,<3",
        ])
        .assert()
        .success();
    let stored =
        fs::read_to_string(sandbox.data.path().join("scripts/python-tool/script.py")).unwrap();
    assert!(stored.contains("dependencies = [\"requests>=2,<3\"]"));
    assert!(stored.contains("requires-python = \">=3.12.1,<3.13\""));
    let meta =
        fs::read_to_string(sandbox.data.path().join("scripts/python-tool/meta.toml")).unwrap();
    assert!(!meta.contains("dependencies ="));
    assert!(!meta.contains("requires_python ="));

    for arguments in [
        vec![
            "add",
            source.to_str().unwrap(),
            "--name",
            "Bad requirement",
            "--dep",
            "requests=>2",
        ],
        vec![
            "add",
            source.to_str().unwrap(),
            "--name",
            "Bad version",
            "--python",
            "not-a-version",
        ],
    ] {
        sandbox.command().args(arguments).assert().code(2);
    }
    assert!(!sandbox.data.path().join("scripts/bad-requirement").exists());
    assert!(!sandbox.data.path().join("scripts/bad-version").exists());

    let stored_path = sandbox.data.path().join("scripts/python-tool/script.py");
    let before = fs::read(&stored_path).unwrap();
    sandbox
        .command()
        .args(["deps", "python-tool", "--dep", "requests=>2"])
        .assert()
        .code(2);
    sandbox
        .command()
        .args(["deps", "python-tool", "--python", "not-a-version"])
        .assert()
        .code(2);
    assert_eq!(fs::read(stored_path).unwrap(), before);
}

#[test]
fn add_keeps_settings_that_do_not_belong_in_the_source() {
    let sandbox = Sandbox::new();
    let source = sandbox.data.path().join("reference.py");
    fs::write(&source, "print('ok')\n").unwrap();

    sandbox
        .command()
        .args(["add"])
        .arg(&source)
        .args(["--name", "Reference", "--ref", "--python", ">=3.12"])
        .assert()
        .success();
    sandbox
        .command()
        .args(["show", "reference", "--json"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"requires_python\":\">=3.12\""));

    sandbox
        .command()
        .args([
            "add",
            "--prompt",
            "--name",
            "Literal prompt",
            "--no-interpolate",
            "--no-input",
        ])
        .write_stdin("Keep {{subject}} literal.\n")
        .assert()
        .success();
    sandbox
        .command()
        .args(["show", "literal-prompt", "--json"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"interpolate\":false"));
}

#[test]
fn a_registered_shebang_pins_the_non_python_interpreter() {
    let sandbox = Sandbox::new();
    let source = sandbox.data.path().join("tool");
    fs::write(&source, "#!/usr/bin/env -S deno run\nconsole.log('ok');\n").unwrap();

    sandbox
        .command()
        .args(["add", source.to_str().unwrap(), "--name", "Deno tool"])
        .assert()
        .success();
    let meta = fs::read_to_string(sandbox.data.path().join("scripts/deno-tool/meta.toml")).unwrap();
    assert!(meta.contains("kind = \"js\""));
    assert!(meta.contains("interpreter = \"deno\""));
}

#[test]
fn successful_adds_restore_the_complete_latest_main_summary() {
    let sandbox = Sandbox::new();
    let source = sandbox.data.path().join("summary.py");
    fs::write(
        &source,
        concat!(
            "# /// script\n",
            "# dependencies = [\"requests\"]\n",
            "# [tool.skit]\n",
            "# schema = 1\n",
            "#\n",
            "# [[tool.skit.params]]\n",
            "# name = \"VALUE\"\n",
            "# kind = \"const\"\n",
            "# type = \"int\"\n",
            "# default = 1\n",
            "# ///\n",
            "VALUE = 1\n",
            "print(VALUE)\n",
        ),
    )
    .unwrap();

    sandbox
        .command()
        .args([
            "add",
            source.to_str().unwrap(),
            "--name",
            "Summary",
            "--description",
            "Visible detail",
            "--no-input",
        ])
        .assert()
        .success()
        .stdout(
            predicate::str::contains("Added: Summary (copy mode)")
                .and(predicate::str::contains("Description: Visible detail"))
                .and(predicate::str::contains("Dependencies: requests"))
                .and(predicate::str::contains("Managed parameters: VALUE"))
                .and(predicate::str::contains("Run it: skit run Summary")),
        );

    sandbox
        .command()
        .args([
            "add",
            "--cmd",
            "printf '%s' {API_KEY}",
            "--name",
            "Secret command",
        ])
        .assert()
        .success()
        .stdout(
            predicate::str::contains(
                "Detected parameters: API_KEY (the run form asks for them; your last values are remembered)",
            )
            .and(predicate::str::contains("Added: Secret command"))
            .and(predicate::str::contains(
                "Secret parameter values are never saved by skit: API_KEY",
            ))
            .and(predicate::str::contains("Run it: skit run Secret command")),
        );
}
