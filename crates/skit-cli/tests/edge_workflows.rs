use std::fs;

use assert_cmd::Command;
use serde_json::Value;
use tempfile::TempDir;

struct Sandbox {
    data: TempDir,
    state: TempDir,
    config: TempDir,
    home: TempDir,
}

impl Sandbox {
    fn new() -> Self {
        Self {
            data: TempDir::new().unwrap(),
            state: TempDir::new().unwrap(),
            config: TempDir::new().unwrap(),
            home: TempDir::new().unwrap(),
        }
    }

    fn command(&self) -> Command {
        let mut command = assert_cmd::cargo::cargo_bin_cmd!("skit");
        command
            .env("SKIT_DATA_DIR", self.data.path())
            .env("SKIT_STATE_DIR", self.state.path())
            .env("SKIT_CONFIG_DIR", self.config.path())
            .env("SKIT_LANG", "en")
            .env("HOME", self.home.path())
            .current_dir(self.home.path());
        command
    }

    fn ok(&self, args: &[&str]) -> Vec<u8> {
        let output = self.command().args(args).output().unwrap();
        assert!(
            output.status.success(),
            "args={args:?}\nstdout={}\nstderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
        );
        output.stdout
    }

    fn code(&self, args: &[&str], code: i32) {
        let output = self.command().args(args).output().unwrap();
        assert_eq!(
            output.status.code(),
            Some(code),
            "args={args:?}\nstdout={}\nstderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
        );
    }

    fn json(&self, args: &[&str]) -> Value {
        serde_json::from_slice(&self.ok(args)).unwrap()
    }

    fn source(&self, name: &str, text: &[u8]) -> String {
        let path = self.data.path().join(name);
        fs::write(&path, text).unwrap();
        path.to_str().unwrap().to_owned()
    }

    fn editor_pty(
        &self,
        args: &[&str],
        configure: impl FnOnce(&mut portable_pty::CommandBuilder),
    ) -> (u32, String) {
        use portable_pty::{CommandBuilder, PtySize, native_pty_system};
        use std::io::Read as _;

        let pair = native_pty_system()
            .openpty(PtySize {
                rows: 24,
                cols: 100,
                pixel_width: 0,
                pixel_height: 0,
            })
            .unwrap();
        let mut command = CommandBuilder::new(std::path::PathBuf::from(env!("CARGO_BIN_EXE_skit")));
        command.args(args);
        command.env("TERM", "xterm-256color");
        command.env("SKIT_DATA_DIR", self.data.path());
        command.env("SKIT_STATE_DIR", self.state.path());
        command.env("SKIT_CONFIG_DIR", self.config.path());
        command.env("SKIT_LANG", "en");
        command.env("HOME", self.home.path());
        command.cwd(self.home.path());
        configure(&mut command);
        let mut child = pair.slave.spawn_command(command).unwrap();
        drop(pair.slave);
        let mut reader = pair.master.try_clone_reader().unwrap();
        let drain = std::thread::spawn(move || {
            let mut output = Vec::new();
            reader.read_to_end(&mut output).unwrap();
            output
        });
        let status = child.wait().unwrap();
        drop(pair.master);
        let output = String::from_utf8_lossy(&drain.join().unwrap()).into_owned();
        (status.exit_code(), output)
    }
}

#[test]
fn configuration_runner_and_completion_edges_are_explicit() {
    let sandbox = Sandbox::new();

    sandbox.ok(&["config"]);
    sandbox.ok(&["config", "--json"]);
    sandbox.ok(&["config", "form", "plain"]);
    sandbox.ok(&["config", "lang", "zh-TW", "--json"]);
    sandbox.ok(&["config", "form"]);
    sandbox.ok(&["config", "form", "--json"]);

    sandbox.ok(&["runner", "list"]);
    sandbox.ok(&["runner", "list", "--json", "--all"]);
    sandbox.ok(&["runner", "add", "agent", "agent", "{{prompt}}"]);
    sandbox.code(&["runner", "add", "agent", "other", "{{prompt}}"], 1);
    sandbox.ok(&[
        "runner",
        "add",
        "agent",
        "--force",
        "other",
        "run",
        "{{prompt}}",
    ]);
    sandbox.code(&["runner", "remove", "agent", "--no-input"], 2);
    sandbox.code(&["runner", "remove", "missing", "--yes"], 1);
    sandbox.ok(&["runner", "remove", "agent", "--yes"]);

    for shell in ["bash", "fish", "zsh", "elvish", "pwsh"] {
        sandbox
            .command()
            .env("SHELL", format!("/bin/{shell}"))
            .arg("--show-completion")
            .assert()
            .success();
    }
    sandbox
        .command()
        .env("SHELL", "/bin/unknown-shell")
        .arg("--show-completion")
        .assert()
        .code(2);
    sandbox
        .command()
        .env("SHELL", "/bin/unknown-shell")
        .env("PSModulePath", sandbox.home.path())
        .arg("--show-completion")
        .assert()
        .success();
    sandbox
        .command()
        .env("SHELL", "/bin/bash")
        .env("XDG_DATA_HOME", sandbox.home.path().join("share"))
        .arg("--install-completion")
        .assert()
        .success();
    assert!(
        sandbox
            .home
            .path()
            .join("share/bash-completion/completions/skit")
            .is_file()
    );
    let installed = sandbox
        .home
        .path()
        .join("share/bash-completion/completions/skit");
    fs::remove_file(&installed).unwrap();
    fs::create_dir(&installed).unwrap();
    sandbox
        .command()
        .env("SHELL", "/bin/bash")
        .env("XDG_DATA_HOME", sandbox.home.path().join("share"))
        .arg("--install-completion")
        .assert()
        .code(125);

    for (shell, relative) in [
        ("bash", ".local/share/bash-completion/completions/skit"),
        ("fish", ".config/fish/completions/skit.fish"),
        ("zsh", ".local/share/zsh/site-functions/_skit"),
        ("elvish", ".config/elvish/lib/skit.elv"),
        ("pwsh", "Documents/PowerShell/Completions/_skit.ps1"),
    ] {
        sandbox
            .command()
            .env_remove("XDG_DATA_HOME")
            .env_remove("XDG_CONFIG_HOME")
            .env("SHELL", format!("/bin/{shell}"))
            .arg("--install-completion")
            .assert()
            .success();
        assert!(
            sandbox.home.path().join(relative).is_file(),
            "shell={shell}"
        );
    }
    sandbox
        .command()
        .env_remove("HOME")
        .env_remove("USERPROFILE")
        .env("SHELL", "/bin/bash")
        .arg("--install-completion")
        .assert()
        .code(2);

    sandbox
        .command()
        .env("COMPLETE", "bash")
        .assert()
        .success()
        .stdout(predicates::str::contains("_skit"));
    sandbox.command().env("COMPLETE", "future").assert().code(2);

    sandbox.ok(&["add", "--cmd", "true", "--name", "Complete Me"]);
    sandbox
        .command()
        .env("COMPLETE", "bash")
        .env("_CLAP_COMPLETE_INDEX", "2")
        .env("_CLAP_COMPLETE_COMP_TYPE", "9")
        .env("_CLAP_COMPLETE_SPACE", "true")
        .args(["--", "skit", "run", ""])
        .assert()
        .success()
        .stdout(predicates::str::contains("complete-me"));
    let completion_state = sandbox.state.path().join("values");
    fs::create_dir_all(&completion_state).unwrap();
    fs::write(
        completion_state.join("complete-me.toml"),
        "[presets.fast]\n",
    )
    .unwrap();
    for (flag, expected) in [("--runner", "codex"), ("--preset", "fast")] {
        sandbox
            .command()
            .env("COMPLETE", "bash")
            .env("_CLAP_COMPLETE_INDEX", "4")
            .env("_CLAP_COMPLETE_COMP_TYPE", "9")
            .env("_CLAP_COMPLETE_SPACE", "true")
            .args(["--", "skit", "run", "complete-me", flag, ""])
            .assert()
            .success()
            .stdout(predicates::str::contains(expected));
    }
    let bad_root = sandbox.home.path().join("not-a-directory");
    fs::write(&bad_root, "file").unwrap();
    for (flag, variable) in [
        ("", "SKIT_DATA_DIR"),
        ("--runner", "SKIT_CONFIG_DIR"),
        ("--preset", "SKIT_STATE_DIR"),
    ] {
        let mut command = sandbox.command();
        command
            .env(variable, &bad_root)
            .env("COMPLETE", "bash")
            .env(
                "_CLAP_COMPLETE_INDEX",
                if flag.is_empty() { "2" } else { "4" },
            )
            .env("_CLAP_COMPLETE_COMP_TYPE", "9")
            .env("_CLAP_COMPLETE_SPACE", "true");
        if flag.is_empty() {
            command.args(["--", "skit", "run", ""]);
        } else {
            command.args(["--", "skit", "run", "complete-me", flag, ""]);
        }
        command.assert().success();
    }
    for (flag, variable, xdg) in [
        ("", "SKIT_DATA_DIR", "XDG_DATA_HOME"),
        ("--runner", "SKIT_CONFIG_DIR", "XDG_CONFIG_HOME"),
        ("--preset", "SKIT_STATE_DIR", "XDG_STATE_HOME"),
    ] {
        let mut command = sandbox.command();
        command
            .env_remove(variable)
            .env_remove(xdg)
            .env_remove("HOME")
            .env_remove("USERPROFILE")
            .env("COMPLETE", "bash")
            .env(
                "_CLAP_COMPLETE_INDEX",
                if flag.is_empty() { "2" } else { "4" },
            )
            .env("_CLAP_COMPLETE_COMP_TYPE", "9")
            .env("_CLAP_COMPLETE_SPACE", "true");
        if flag.is_empty() {
            command.args(["--", "skit", "run", ""]);
        } else {
            command.args(["--", "skit", "run", "complete-me", flag, ""]);
        }
        command.assert().success();
    }
}

#[test]
fn locale_fallbacks_use_config_then_environment_without_changing_json() {
    let sandbox = Sandbox::new();
    sandbox.ok(&["config", "lang", "zh-TW"]);
    sandbox
        .command()
        .env_remove("SKIT_LANG")
        .env("LC_ALL", "en_US.UTF-8")
        .arg("--help")
        .assert()
        .success()
        .stdout(predicates::str::contains(
            "程式、提示詞、執行檔與命令程式庫",
        ));

    let empty_config = TempDir::new().unwrap();
    sandbox
        .command()
        .env_remove("SKIT_LANG")
        .env("SKIT_CONFIG_DIR", empty_config.path())
        .env("LC_ALL", "zh_CN.UTF-8")
        .arg("--help")
        .assert()
        .success()
        .stdout(predicates::str::contains("脚本、提示词、程序与命令库"));
    sandbox
        .command()
        .env_remove("SKIT_LANG")
        .env("LC_ALL", "C")
        .env("LC_MESSAGES", "zh_TW.UTF-8")
        .env("LANG", "en_US.UTF-8")
        .env("SKIT_CONFIG_DIR", empty_config.path())
        .arg("--help")
        .assert()
        .success()
        .stdout(predicates::str::contains(
            "程式、提示詞、執行檔與命令程式庫",
        ));
}

#[test]
fn doctor_human_report_exposes_each_repair_axis() {
    let sandbox = Sandbox::new();
    let missing = sandbox.source("missing.sh", b"echo ok\n");
    sandbox.ok(&["add", &missing, "--ref", "--name", "Missing"]);
    fs::remove_file(&missing).unwrap();

    let drift = sandbox.source("drift.sh", b"NAME=world\necho \"$NAME\"\n");
    sandbox.ok(&["add", &drift, "--name", "Drift"]);
    sandbox.ok(&["params", "drift", "--manage", "NAME"]);
    let stored_drift = sandbox.data.path().join("scripts/drift/script.sh");
    let managed = fs::read_to_string(&stored_drift).unwrap();
    let changed = managed
        .lines()
        .filter(|line| !line.starts_with("NAME="))
        .collect::<Vec<_>>()
        .join("\n")
        + "\n";
    fs::write(stored_drift, changed).unwrap();

    sandbox.ok(&["add", "--cmd", "true", "--name", "Needs"]);
    sandbox.ok(&[
        "deps",
        "needs",
        "--need",
        "skit-command-that-does-not-exist",
    ]);
    let prompt = sandbox.source("prompt.md", b"Hello\n");
    sandbox.ok(&["add", &prompt, "--prompt", "--name", "Blocked"]);
    let blocked_meta = sandbox.data.path().join("scripts/blocked/meta.toml");
    let mut blocked = fs::read_to_string(&blocked_meta).unwrap();
    blocked.push_str("runner = \"missing-runner\"\n");
    fs::write(blocked_meta, blocked).unwrap();
    let python = sandbox.source("doctor.py", b"print('ok')\n");
    sandbox.ok(&["add", &python, "--name", "Python"]);

    fs::create_dir_all(sandbox.data.path().join("scripts/broken")).unwrap();
    fs::write(
        sandbox.data.path().join("scripts/broken/meta.toml"),
        "name = [broken",
    )
    .unwrap();
    fs::write(
        sandbox.config.path().join("config.toml"),
        concat!(
            "[[prompt.runners]]\n",
            "name = \"bad\"\n",
            "argv = [\"bad\"]\n",
        ),
    )
    .unwrap();

    let output = sandbox.ok(&["doctor", "--rebuild"]);
    let output = String::from_utf8(output).unwrap();
    for text in [
        "Registry rebuilt",
        "launch target is gone",
        "form definitions are out of sync",
        "missing external commands",
        "a run would refuse to start",
        "Ignored malformed runner row(s) in config: bad. Inspect and repair with: skit runner list --all",
    ] {
        assert!(
            output.contains(text),
            "missing doctor row: {text}\n{output}"
        );
    }
}

#[test]
fn doctor_reports_missing_uv_for_an_empty_library() {
    let sandbox = Sandbox::new();
    let empty_path = TempDir::new().unwrap();
    sandbox
        .command()
        .env("PATH", empty_path.path())
        .arg("doctor")
        .assert()
        .code(1)
        .stdout(predicates::str::contains("ERROR uv: not found"));
}

#[test]
fn add_dependency_and_parameter_refusals_leave_no_partial_entry() {
    let sandbox = Sandbox::new();
    sandbox.code(&["add", "--no-input"], 2);
    sandbox.code(&["add", "--cmd", "echo ok", "--dep", "bad"], 2);
    sandbox.code(&["add", "--cmd", "echo ok", "--python", ">=3.13"], 2);
    sandbox
        .command()
        .write_stdin("print('x')\n")
        .args(["add", "-", "--ref", "--kind", "python"])
        .assert()
        .code(2);

    let unknown = sandbox.source("unknown", b"not executable\n");
    sandbox.code(&["add", &unknown], 2);
    let shell = sandbox.source("tool.sh", b"echo ok\n");
    sandbox.code(&["add", &shell, "--dep", "chalk"], 2);
    sandbox.code(&["add", &shell, "--python", ">=3.13"], 2);
    sandbox.code(&["add", &shell, "--runner", "agent"], 2);
    sandbox.code(
        &[
            "add",
            &shell,
            "--prompt",
            "--runner",
            "missing-runner",
            "--name",
            "Missing runner",
        ],
        2,
    );

    sandbox.ok(&["runner", "add", "agent", "sh", "-c", "printf %s {{prompt}}"]);
    sandbox.ok(&[
        "add",
        &shell,
        "--prompt",
        "--runner",
        "agent",
        "--name",
        "Prompt file",
    ]);
    let prompt = sandbox.ok(&["show", "prompt-file", "--json"]);
    let prompt: serde_json::Value = serde_json::from_slice(&prompt).unwrap();
    assert_eq!(prompt["runner"], "agent");
    assert!(
        prompt["runners_available"]
            .as_array()
            .unwrap()
            .iter()
            .any(|name| name == "agent")
    );

    let javascript = sandbox.source("tool.js", b"import 'chalk';\n");
    sandbox.ok(&["add", &javascript, "--ref"]);
    assert_eq!(
        sandbox.json(&["deps", "tool", "--json"])["dependencies"],
        serde_json::json!([])
    );
    assert_eq!(fs::read(&javascript).unwrap(), b"import 'chalk';\n");

    let explicit = sandbox.source("explicit.js", b"console.log('ok');\n");
    sandbox.code(&["add", &explicit, "--ref", "--dep", "chalk"], 2);
    assert!(!sandbox.data.path().join("scripts/explicit").exists());
}

#[test]
fn params_deps_presets_and_agent_commands_cover_mutation_and_refusal_axes() {
    let sandbox = Sandbox::new();
    sandbox.ok(&["add", "--cmd", "printf '%s' {name}", "--name", "Demo"]);
    sandbox.ok(&["params", "demo"]);
    sandbox.ok(&["params", "demo", "--json"]);
    sandbox.code(&["params", "demo", "--resync", "--add", "other"], 2);
    sandbox.code(&["params", "demo", "--add", "name"], 2);
    sandbox.code(&["params", "demo", "--type", "bad"], 2);
    sandbox.code(&["params", "demo", "--type", "missing=int"], 2);
    sandbox.code(&["params", "demo", "--type", "name=future"], 2);
    sandbox.code(&["params", "demo", "--deliver", "name=future"], 2);
    for flag in ["--required", "--optional", "--secret", "--no-secret"] {
        sandbox.code(&["params", "demo", flag, "missing"], 2);
    }
    sandbox.code(
        &[
            "params",
            "demo",
            "--default",
            "name=x",
            "--type",
            "name=int",
        ],
        2,
    );

    sandbox.ok(&[
        "params",
        "demo",
        "--add",
        "count",
        "--type",
        "count=int",
        "--default",
        "count=2",
        "--deliver",
        "count=flag",
        "--flag",
        "count=--count",
        "--help-text",
        "count=Count help",
        "--prompt",
        "count=Count",
        "--env-source",
        "count=COUNT_SOURCE",
        "--required",
        "count",
        "--secret",
        "count",
    ]);
    let params = sandbox.json(&["params", "demo", "--json"]);
    let count = params["parameters"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["name"] == "count")
        .unwrap();
    assert_eq!(count["type"], "int");
    assert_eq!(count["default"], 2);
    assert_eq!(count["delivery"], "flag");
    assert_eq!(count["flag"], "--count");
    assert_eq!(count["help"], "Count help");
    assert_eq!(count["prompt"], "Count");
    assert_eq!(count["env_source"], "COUNT_SOURCE");
    assert_eq!(count["required"], true);
    assert_eq!(count["secret"], true);
    sandbox.ok(&[
        "params",
        "demo",
        "--workdir",
        "invoke",
        "--template",
        "printf '%s' {name}",
    ]);
    sandbox.code(&["params", "demo", "--interpreter", "sh"], 2);
    sandbox.code(&["params", "demo", "--runner", ""], 1);
    sandbox.code(&["params", "demo", "--no-interpolate"], 1);
    sandbox.ok(&[
        "params",
        "demo",
        "--optional",
        "count",
        "--no-secret",
        "count",
    ]);
    let params = sandbox.json(&["params", "demo", "--json"]);
    let count = params["parameters"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["name"] == "count")
        .unwrap();
    assert!(!count["required"].as_bool().unwrap_or(false));
    assert!(!count["secret"].as_bool().unwrap_or(false));
    sandbox.ok(&["params", "demo", "--rm", "count"]);
    assert!(
        sandbox.json(&["params", "demo", "--json"])["parameters"]
            .as_array()
            .unwrap()
            .iter()
            .all(|row| row["name"] != "count")
    );

    sandbox.code(&["deps", "demo", "--dep", "x"], 2);
    sandbox
        .command()
        .args(["deps", "demo", "--clear", "--dep", "x"])
        .assert()
        .code(2)
        .stderr(predicates::str::contains("use --dep or --clear, not both"));
    sandbox.code(&["deps", "demo", "--clear-needs", "--need", "sh"], 2);
    sandbox.ok(&["deps", "demo", "--need", "sh"]);
    assert_eq!(
        sandbox.json(&["deps", "demo", "--json"])["needs"],
        serde_json::json!(["sh"])
    );
    sandbox.ok(&["deps", "demo", "--clear-needs"]);
    assert_eq!(
        sandbox.json(&["deps", "demo", "--json"])["needs"],
        serde_json::json!([])
    );

    let python = sandbox.source("deps.py", b"print('ok')\n");
    sandbox.ok(&[
        "add",
        &python,
        "--name",
        "Python deps",
        "--dep",
        "rich",
        "--python",
        ">=3.13",
    ]);
    sandbox.code(&["deps", "python-deps", "--clear", "--dep", "click"], 2);
    sandbox.ok(&["deps", "python-deps", "--dep", "click"]);
    sandbox.ok(&["deps", "python-deps", "--python", "none"]);
    sandbox.ok(&["deps", "python-deps", "--clear"]);
    sandbox.ok(&["deps", "python-deps", "--clear"]);
    let python_deps = sandbox.json(&["deps", "python-deps", "--json"]);
    assert_eq!(python_deps["dependencies"], serde_json::json!([]));
    assert_eq!(python_deps["requires_python"], "");
    sandbox.ok(&["params", "python-deps", "--workdir", "invoke"]);
    sandbox.code(&["params", "python-deps", "--normalize", "NAME"], 2);

    let javascript = sandbox.source("deps.js", b"console.log('ok');\n");
    sandbox.ok(&["add", &javascript, "--name", "JavaScript deps"]);
    sandbox.ok(&["deps", "javascript-deps", "--dep", "chalk"]);
    sandbox.ok(&["deps", "javascript-deps", "--clear"]);
    assert_eq!(
        sandbox.json(&["deps", "javascript-deps", "--json"])["dependencies"],
        serde_json::json!([])
    );

    sandbox.ok(&["run", "demo", "--set", "name=value", "--no-input"]);
    sandbox.ok(&["preset", "save", "demo", "current"]);
    sandbox.ok(&["preset", "save", "demo", "last", "--from-last"]);
    sandbox.ok(&["preset", "list", "demo"]);
    sandbox.ok(&["preset", "list", "demo", "--json"]);
    sandbox.ok(&["preset", "delete", "demo", "current", "--no-input"]);
    sandbox.code(&["preset", "delete", "demo", "missing", "--yes"], 1);
    sandbox.code(&["preset", "delete", "demo", "current", "--yes"], 1);

    sandbox.ok(&["agent", "install", "claude", "--project"]);
    sandbox.ok(&["agent", "install", "codex", "--project"]);
    sandbox.ok(&["agent", "install", "agents", "--project"]);
    sandbox.code(&["agent", "install", "future", "--project"], 2);
    sandbox.ok(&["agent", "install", "claude"]);
    sandbox.ok(&["agent", "install", "codex"]);
    sandbox.ok(&["agent", "install", "agents"]);
    sandbox.code(&["agent", "install", "future"], 2);
    assert!(
        sandbox
            .home
            .path()
            .join(".agents/skills/skit/SKILL.md")
            .is_file()
    );
}

#[test]
fn editor_dependency_source_management_and_raw_run_edges_are_transactional() {
    let sandbox = Sandbox::new();
    let shell = sandbox.source("edit.sh", b"NAME=old\necho \"$NAME\"\n");
    sandbox.ok(&["add", &shell, "--name", "Edit copy"]);
    sandbox.ok(&["config", "editor", "true"]);
    sandbox.ok(&["edit", "edit-copy", "--no-input"]);
    sandbox.ok(&[
        "config",
        "editor",
        "sh -c 'printf changed\\n > \"$1\"' editor",
    ]);
    sandbox.ok(&["edit", "edit-copy", "--no-input"]);
    assert_eq!(
        fs::read_to_string(sandbox.data.path().join("scripts/edit-copy/script.sh")).unwrap(),
        "changedn"
    );
    // v0.4 ignores the editor's own exit status (some editors exit non-zero on an
    // unmodified close), so a non-zero editor still saves cleanly.
    sandbox.ok(&["config", "editor", "sh -c 'exit 3'"]);
    sandbox.ok(&["edit", "edit-copy", "--no-input"]);
    // An unbalanced-quote value becomes the program name; launching it fails (1).
    sandbox.ok(&["config", "editor", "'"]);
    sandbox.code(&["edit", "edit-copy", "--no-input"], 1);
    // Every candidate blank resolves the platform default `vi`; with an empty PATH
    // the launch fails as a failed operation, never a usage error.
    sandbox.ok(&["config", "editor", "   "]);
    let empty_path = TempDir::new().unwrap();
    sandbox
        .command()
        .env_remove("VISUAL")
        .env_remove("EDITOR")
        .env("PATH", empty_path.path())
        .args(["edit", "edit-copy", "--no-input"])
        .assert()
        .code(1);
    sandbox
        .command()
        .env_remove("VISUAL")
        .env("EDITOR", "")
        .env("PATH", empty_path.path())
        .args(["edit", "edit-copy", "--no-input"])
        .assert()
        .code(1);

    let reference = sandbox.source("reference.sh", b"echo reference\n");
    sandbox.ok(&["add", &reference, "--ref", "--name", "Reference"]);
    sandbox.ok(&["config", "editor", "true"]);
    sandbox.ok(&["edit", "reference", "--no-input"]);
    sandbox.ok(&["config", "editor", "false"]);
    sandbox.ok(&["edit", "reference", "--no-input"]);
    sandbox.ok(&["add", "--cmd", "echo ok", "--name", "No source"]);
    sandbox.code(&["edit", "no-source", "--no-input"], 1);
    sandbox.code(&["edit", "missing", "--no-input"], 1);

    let managed = sandbox.source("managed.sh", b"NAME=old\necho \"$NAME\"\n");
    sandbox.ok(&["add", &managed, "--name", "Managed"]);
    sandbox.code(&["params", "managed", "--manage", "missing"], 2);
    sandbox.ok(&["params", "managed", "--manage", "NAME"]);
    sandbox.ok(&["params", "managed", "--manage", "NAME"]);
    sandbox.ok(&["params", "managed", "--resync"]);
    sandbox.code(&["params", "reference", "--resync"], 1);
    sandbox.code(&["params", "managed", "--normalize", "missing"], 2);
    sandbox.ok(&["params", "managed", "--normalize", "NAME"]);

    sandbox.ok(&[
        "run",
        "managed",
        "--set",
        "NAME=new",
        "--dry-run",
        "--",
        "tail",
    ]);
    sandbox.ok(&["run", "managed", "--raw", "--", "one", "two"]);
    sandbox.ok(&["run", "managed", "--raw", "--forget-args"]);
    sandbox.code(&["run", "managed", "--raw", "--set", "NAME=x"], 2);

    let python = sandbox.source("invalid.py", &[0xff]);
    sandbox.ok(&[
        "add",
        &python,
        "--kind",
        "python",
        "--name",
        "Invalid Python",
    ]);
    sandbox.ok(&["deps", "invalid-python", "--dep", "rich"]);
    assert_eq!(
        fs::read(sandbox.data.path().join("scripts/invalid-python/script.py")).unwrap(),
        [0xff]
    );
    assert_eq!(
        sandbox.json(&["deps", "invalid-python", "--json"])["dependencies"],
        serde_json::json!(["rich"])
    );
    let javascript = sandbox.source("plain.js", b"console.log('ok');\n");
    sandbox.ok(&["add", &javascript, "--ref", "--name", "JS reference"]);
    sandbox.code(&["deps", "js-reference", "--dep", "chalk"], 2);
    sandbox.code(&["deps", "js-reference", "--python", ">=3.13"], 2);
}

#[test]
fn draft_editor_failures_keep_recoverable_work_and_report_exact_causes() {
    let sandbox = Sandbox::new();
    sandbox.ok(&["config", "editor", "true"]);
    let (untouched_code, untouched) =
        sandbox.editor_pty(&["add", "--edit", "--name", "Empty"], |_| {});
    assert_eq!(untouched_code, 0, "{untouched}");
    assert!(
        untouched.contains("Nothing was written, so no script was added."),
        "{untouched}"
    );
    assert!(
        fs::read_dir(sandbox.data.path().join("drafts"))
            .unwrap()
            .next()
            .is_none(),
        "an untouched draft is litter, not recoverable work"
    );

    sandbox.ok(&["config", "editor", "false"]);
    let (failed_code, failed) =
        sandbox.editor_pty(&["add", "--edit", "--name", "Failed Editor"], |_| {});
    assert_eq!(failed_code, 2, "{failed}");
    // An unbalanced-quote value becomes the program name; the launch failure is a
    // failed operation (exit 1), and the draft is kept like every editor failure.
    sandbox.ok(&["config", "editor", "'"]);
    let (quote_code, quote) = sandbox.editor_pty(&["add", "--edit", "--name", "Bad Quote"], |_| {});
    assert_eq!(quote_code, 1, "{quote}");

    let editor = sandbox.source(
        "draft-editor.sh",
        b"#!/bin/sh\nprintf 'print(1)\\n' > \"$1\"\n",
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        fs::set_permissions(&editor, fs::Permissions::from_mode(0o755)).unwrap();
    }
    sandbox.ok(&["config", "editor", &editor]);
    sandbox.ok(&["config", "editor", "   "]);
    // Every candidate blank resolves the platform default `vi`; an empty PATH turns
    // the launch into the failed-operation refusal (exit 1) with the config hint.
    let empty_path = TempDir::new().unwrap();
    let (empty_code, empty) =
        sandbox.editor_pty(&["add", "--edit", "--name", "Empty command"], |command| {
            command.env_remove("VISUAL");
            command.env("EDITOR", "");
            command.env("PATH", empty_path.path());
        });
    assert_eq!(empty_code, 1, "{empty}");
    let (visual_code, visual) =
        sandbox.editor_pty(&["add", "--edit", "--name", "Visual draft"], |command| {
            command.env("VISUAL", &editor);
            command.env_remove("EDITOR");
        });
    assert_eq!(visual_code, 0, "{visual}");
    sandbox.ok(&["config", "editor", &editor]);
    sandbox.ok(&["add", "--cmd", "true", "--name", "Duplicate"]);
    let (duplicate_code, duplicate) =
        sandbox.editor_pty(&["add", "--edit", "--name", "Duplicate"], |_| {});
    assert_eq!(duplicate_code, 1, "{duplicate}");
    assert!(
        fs::read_dir(sandbox.data.path().join("drafts"))
            .unwrap()
            .filter_map(Result::ok)
            .count()
            >= 2
    );
}

#[test]
fn run_pipeline_materializes_javascript_and_preserves_trusted_command_semantics() {
    let sandbox = Sandbox::new();
    let tools = TempDir::new().unwrap();
    let node = tools.path().join("node");
    let npm = tools.path().join("npm");
    fs::write(&node, "#!/bin/sh\nexit 0\n").unwrap();
    fs::write(&npm, "#!/bin/sh\n/bin/mkdir -p node_modules\nexit 0\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        fs::set_permissions(&node, fs::Permissions::from_mode(0o755)).unwrap();
        fs::set_permissions(&npm, fs::Permissions::from_mode(0o755)).unwrap();
    }

    let javascript = sandbox.source("launch.js", b"console.log('ok');\n");
    sandbox.ok(&["add", &javascript, "--name", "Launch JS"]);
    sandbox.ok(&["deps", "launch-js", "--dep", "chalk"]);
    for _ in 0..2 {
        sandbox
            .command()
            .env("PATH", tools.path())
            .args(["run", "launch-js", "--no-input"])
            .assert()
            .success();
    }
    assert!(
        sandbox
            .data
            .path()
            .join("scripts/launch-js/node_modules")
            .is_dir()
    );
    let custom_runtime = tools.path().join("custom-js");
    fs::write(&custom_runtime, "#!/bin/sh\nexit 0\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        fs::set_permissions(&custom_runtime, fs::Permissions::from_mode(0o755)).unwrap();
    }
    let unsupported = sandbox.source("unsupported.js", b"console.log('custom');\n");
    sandbox.ok(&["add", &unsupported, "--name", "Unsupported runtime"]);
    sandbox.ok(&["deps", "unsupported-runtime", "--dep", "chalk"]);
    sandbox.ok(&[
        "params",
        "unsupported-runtime",
        "--interpreter",
        custom_runtime.to_str().unwrap(),
    ]);
    sandbox
        .command()
        .env("PATH", tools.path())
        .args(["run", "unsupported-runtime", "--no-input"])
        .assert()
        .success();
    assert!(
        sandbox
            .data
            .path()
            .join("scripts/unsupported-runtime/node_modules")
            .is_dir()
    );

    let reference = sandbox.source("reference.js", b"console.log('ref');\n");
    sandbox.ok(&["add", &reference, "--ref", "--name", "Reference JS"]);
    let meta = sandbox.data.path().join("scripts/reference-js/meta.toml");
    let mut metadata = fs::read_to_string(&meta).unwrap();
    metadata.push_str("dependencies = [\"chalk\"]\n");
    fs::write(&meta, metadata).unwrap();
    sandbox
        .command()
        .env("PATH", tools.path())
        .args(["run", "reference-js", "--no-input"])
        .assert()
        .code(125);
    let plain_reference = sandbox.source("plain-reference.js", b"console.log('ref');\n");
    sandbox.ok(&[
        "add",
        &plain_reference,
        "--ref",
        "--name",
        "Plain reference",
    ]);
    sandbox
        .command()
        .env("PATH", tools.path())
        .args(["run", "plain-reference", "--no-input", "--dry-run"])
        .assert()
        .success();

    let missing_source = sandbox.source("gone.js", b"console.log('gone');\n");
    sandbox.ok(&["add", &missing_source, "--ref", "--name", "Gone JS"]);
    fs::remove_file(missing_source).unwrap();
    sandbox
        .command()
        .env("PATH", tools.path())
        .args(["run", "gone-js", "--no-input"])
        .assert()
        .code(125);

    let prompt = sandbox.source("runner.prompt.md", b"Hello\n");
    sandbox.ok(&["add", &prompt, "--prompt", "--name", "Runner failure"]);
    sandbox.code(
        &["run", "runner-failure", "--runner", "missing", "--no-input"],
        126,
    );

    sandbox.ok(&["add", "--cmd", "true", "--name", "Mirrored"]);
    sandbox.ok(&[
        "config",
        "mirror.pypi",
        "https://packages.example.invalid/simple",
    ]);
    sandbox.ok(&["config", "mirror", "on"]);
    sandbox
        .command()
        .env_remove("UV_DEFAULT_INDEX")
        .env_remove("UV_INDEX_URL")
        .args(["run", "mirrored", "--no-input"])
        .assert()
        .success();

    let injected = sandbox.source("injected.sh", b"NAME=old\necho \"$NAME\"\n");
    sandbox.ok(&["add", &injected, "--name", "Injected"]);
    sandbox.ok(&["params", "injected", "--manage", "NAME"]);
    sandbox.ok(&[
        "run",
        "injected",
        "--set",
        "NAME=new",
        "--no-input",
        "--dry-run",
    ]);

    sandbox.ok(&["add", "--cmd", "echo '{value}'", "--name", "Unsafe command"]);
    sandbox.ok(&["run", "unsafe-command", "--set", "value=x", "--no-input"]);
}

#[test]
fn first_python_run_announces_private_uv_before_a_local_refused_download() {
    let sandbox = Sandbox::new();
    let empty_path = TempDir::new().unwrap();
    let python = sandbox.source("bootstrap.py", b"print('ok')\n");
    sandbox.ok(&["add", &python, "--name", "Bootstrap"]);
    sandbox.ok(&["config", "mirror.github", "https://127.0.0.1:9"]);
    sandbox.ok(&["config", "mirror", "on"]);
    sandbox
        .command()
        .env("PATH", empty_path.path())
        .args(["run", "bootstrap", "--no-input"])
        .assert()
        // A non-interactive stream keeps the version 0.4 zero-action first run
        // (`src/skit/uvman.py:72-73`), and a failed bootstrap exits 125 like every launch failure
        // (`src/skit/langs/launch.py:57-63` into `src/skit/flows.py:868`).
        .code(125)
        .stderr(predicates::str::contains("First run — downloading uv"))
        .stderr(predicates::prelude::PredicateBooleanExt::not(
            predicates::str::contains("Download uv"),
        ));
}
