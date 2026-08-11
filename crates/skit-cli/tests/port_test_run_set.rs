//! Exact behavioral ports of Python v0.4 `tests/test_run_set.py`.
//!
//! Python `origin/main@206f9ef` is the oracle. These tests intentionally cross the CLI, form,
//! staging, launch, and persisted-state boundaries. A red assertion is a parity finding: do not
//! weaken it to match the Rust implementation.

use std::{
    collections::BTreeMap,
    fs,
    io::{Read as _, Write as _},
    path::{Path, PathBuf},
    process::Command as StdCommand,
    thread,
    time::Duration,
};

use assert_cmd::Command;
use portable_pty::{CommandBuilder, PtySize, native_pty_system};
use skit_domain::parameters::{ParamDecl, ParameterBinding, ParameterType};
use skit_language::inject_values;
use tempfile::TempDir;
use toml::Value;

const RAW_CONFLICT: &str =
    "--raw runs the script as-is; --set, --preset, and --save-preset do not apply.";

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
        self.configure_command(&mut command);
        command
    }

    fn configure_command(&self, command: &mut Command) {
        command
            .env("SKIT_DATA_DIR", self.data.path())
            .env("SKIT_STATE_DIR", self.state.path())
            .env("SKIT_CONFIG_DIR", self.config.path())
            .env("SKIT_LANG", "en")
            .env("HOME", self.home.path())
            .env("USERPROFILE", self.home.path())
            .env("XDG_CONFIG_HOME", self.home.path().join("xdg-config"))
            .env("XDG_DATA_HOME", self.home.path().join("xdg-data"))
            .env("XDG_STATE_HOME", self.home.path().join("xdg-state"))
            .env_remove("FORCE_COLOR")
            .env_remove("NO_COLOR")
            .env_remove("CLICOLOR")
            .env_remove("CLICOLOR_FORCE")
            .env_remove("PSModulePath")
            .current_dir(self.home.path());
    }

    fn output(&self, args: &[&str]) -> std::process::Output {
        self.command().args(args).output().unwrap()
    }

    fn ok(&self, args: &[&str]) -> std::process::Output {
        let output = self.output(args);
        assert_success(args, &output);
        output
    }

    fn register(&self, slug: &str) {
        fs::write(
            self.data.path().join("registry.toml"),
            format!("[entries.{slug}]\n"),
        )
        .unwrap();
    }

    fn entry_dir(&self, slug: &str) -> PathBuf {
        self.data.path().join("scripts").join(slug)
    }

    fn write_command_entry(&self, slug: &str, template: &str, metadata_tail: &str) {
        let directory = self.entry_dir(slug);
        fs::create_dir_all(&directory).unwrap();
        fs::write(
            directory.join("meta.toml"),
            format!(
                concat!(
                    "schema = 1\n",
                    "name = {slug:?}\n",
                    "kind = \"command\"\n",
                    "mode = \"copy\"\n",
                    "source = \"\"\n",
                    "source_hash = \"\"\n",
                    "added_at = \"2026-08-10T00:00:00Z\"\n",
                    "id = \"0123456789abcdef0123456789abcdef\"\n",
                    "workdir = \"invoke\"\n",
                    "description = \"\"\n",
                    "template = {template:?}\n",
                    "{metadata_tail}\n",
                ),
                slug = slug,
                template = template,
                metadata_tail = metadata_tail,
            ),
        )
        .unwrap();
        self.register(slug);
    }

    fn write_python_entry(&self, slug: &str, source: &str) {
        let directory = self.entry_dir(slug);
        fs::create_dir_all(&directory).unwrap();
        fs::write(directory.join("script.py"), source).unwrap();
        fs::write(
            directory.join("meta.toml"),
            format!(
                concat!(
                    "schema = 1\n",
                    "name = {slug:?}\n",
                    "kind = \"python\"\n",
                    "mode = \"copy\"\n",
                    "source = \"/original/{slug}.py\"\n",
                    "source_hash = \"\"\n",
                    "added_at = \"2026-08-10T00:00:00Z\"\n",
                    "id = \"1123456789abcdef0123456789abcdef\"\n",
                    "workdir = \"invoke\"\n",
                    "description = \"\"\n",
                ),
                slug = slug,
            ),
        )
        .unwrap();
        self.register(slug);
    }

    fn write_prompt_entry(&self, slug: &str, body: &str, metadata_tail: &str) {
        let directory = self.entry_dir(slug);
        fs::create_dir_all(&directory).unwrap();
        fs::write(directory.join("prompt.md"), body).unwrap();
        fs::write(
            directory.join("meta.toml"),
            format!(
                concat!(
                    "schema = 1\n",
                    "name = {slug:?}\n",
                    "kind = \"prompt\"\n",
                    "mode = \"copy\"\n",
                    "source = \"/original/{slug}.prompt.md\"\n",
                    "source_hash = \"\"\n",
                    "added_at = \"2026-08-10T00:00:00Z\"\n",
                    "id = \"2123456789abcdef0123456789abcdef\"\n",
                    "workdir = \"invoke\"\n",
                    "description = \"\"\n",
                    "{metadata_tail}\n",
                ),
                slug = slug,
                metadata_tail = metadata_tail,
            ),
        )
        .unwrap();
        self.register(slug);
    }

    fn write_exe_entry(&self, slug: &str, executable: &Path, metadata_tail: &str) {
        let directory = self.entry_dir(slug);
        fs::create_dir_all(&directory).unwrap();
        fs::write(
            directory.join("meta.toml"),
            format!(
                concat!(
                    "schema = 1\n",
                    "name = {slug:?}\n",
                    "kind = \"exe\"\n",
                    "mode = \"reference\"\n",
                    "source = {source:?}\n",
                    "source_hash = \"\"\n",
                    "added_at = \"2026-08-10T00:00:00Z\"\n",
                    "id = \"3123456789abcdef0123456789abcdef\"\n",
                    "workdir = \"invoke\"\n",
                    "description = \"\"\n",
                    "{metadata_tail}\n",
                ),
                slug = slug,
                source = executable.display().to_string(),
                metadata_tail = metadata_tail,
            ),
        )
        .unwrap();
        self.register(slug);
    }

    fn state_path(&self, slug: &str) -> PathBuf {
        self.state.path().join("values").join(format!("{slug}.toml"))
    }

    fn state_text(&self, slug: &str) -> String {
        fs::read_to_string(self.state_path(slug)).unwrap_or_default()
    }

    fn state_doc(&self, slug: &str) -> Option<Value> {
        let path = self.state_path(slug);
        path.exists()
            .then(|| toml::from_str(&fs::read_to_string(path).unwrap()).unwrap())
    }

    fn value(&self, slug: &str, name: &str) -> Option<String> {
        self.state_doc(slug)
            .and_then(|doc| doc.get("values").cloned())
            .and_then(|values| values.get(name).cloned())
            .and_then(|value| value.as_str().map(ToOwned::to_owned))
    }

    fn preset_value(&self, slug: &str, preset: &str, name: &str) -> Option<String> {
        self.state_doc(slug)
            .and_then(|doc| doc.get("presets").cloned())
            .and_then(|presets| presets.get(preset).cloned())
            .and_then(|preset| preset.get(name).cloned())
            .and_then(|value| value.as_str().map(ToOwned::to_owned))
    }

    fn last_exit(&self, slug: &str) -> Option<i64> {
        self.state_doc(slug)
            .and_then(|doc| doc.get("last_run").cloned())
            .and_then(|run| run.get("exit").cloned())
            .and_then(|value| value.as_integer())
    }

    fn seed_state(&self, slug: &str, body: &str) {
        let path = self.state_path(slug);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, body).unwrap();
    }

    fn compile_fake_uv(&self) -> PathBuf {
        let bin = self.home.path().join("fake-uv-bin");
        fs::create_dir_all(&bin).unwrap();
        let source = self.home.path().join("fake_uv.rs");
        fs::write(
            &source,
            r#"
use std::{env, fs, path::PathBuf};
fn main() {
    let args = env::args().skip(1).collect::<Vec<_>>();
    let script_index = args.iter().position(|arg| arg == "--script").expect("--script");
    let script = PathBuf::from(args.get(script_index + 1).expect("script path"));
    if let Some(path) = env::var_os("SKIT_CAPTURE_SCRIPT") {
        fs::copy(&script, path).expect("copy staged script");
    }
    if let Some(path) = env::var_os("SKIT_CAPTURE_ARGV") {
        fs::write(path, args.join("\n")).expect("write argv");
    }
}
"#,
        )
        .unwrap();
        let executable = bin.join(if cfg!(windows) { "uv.exe" } else { "uv" });
        let status = StdCommand::new("rustc")
            .arg(&source)
            .arg("-o")
            .arg(&executable)
            .status()
            .unwrap();
        assert!(status.success(), "failed to build fake uv");
        executable
    }

    fn compile_child_probe(&self) -> PathBuf {
        let source = self.home.path().join("child_probe.rs");
        fs::write(
            &source,
            r#"
use std::{env, fs, process};
fn main() {
    let args = env::args().skip(1).collect::<Vec<_>>();
    if let Some(path) = env::var_os("SKIT_CHILD_CAPTURE") {
        let mut text = args.join("\n");
        if let Ok(value) = env::var("MSG") {
            text.push_str("\nMSG=");
            text.push_str(&value);
        }
        fs::write(path, text).expect("write child capture");
    }
    if let Ok(code) = env::var("SKIT_CHILD_EXIT") {
        process::exit(code.parse::<i32>().expect("exit code"));
    }
}
"#,
        )
        .unwrap();
        let executable = self
            .home
            .path()
            .join(if cfg!(windows) { "child-probe.exe" } else { "child-probe" });
        let status = StdCommand::new("rustc")
            .arg(&source)
            .arg("-o")
            .arg(&executable)
            .status()
            .unwrap();
        assert!(status.success(), "failed to build child probe");
        executable
    }

    fn run_plain_pty(&self, args: &[&str], input: &[&[u8]]) -> (u32, String) {
        let pair = native_pty_system()
            .openpty(PtySize {
                rows: 24,
                cols: 100,
                pixel_width: 0,
                pixel_height: 0,
            })
            .unwrap();
        let mut command = CommandBuilder::new(PathBuf::from(env!("CARGO_BIN_EXE_skit")));
        command.args(args);
        command.env("TERM", "xterm-256color");
        command.env("SKIT_LANG", "en");
        command.env("SKIT_DATA_DIR", self.data.path());
        command.env("SKIT_STATE_DIR", self.state.path());
        command.env("SKIT_CONFIG_DIR", self.config.path());
        command.env("HOME", self.home.path());
        command.env("USERPROFILE", self.home.path());
        command.env("XDG_CONFIG_HOME", self.home.path().join("xdg-config"));
        command.env("XDG_DATA_HOME", self.home.path().join("xdg-data"));
        command.env("XDG_STATE_HOME", self.home.path().join("xdg-state"));
        command.cwd(self.home.path());
        let mut child = pair.slave.spawn_command(command).unwrap();
        drop(pair.slave);

        let mut reader = pair.master.try_clone_reader().unwrap();
        let drain = thread::spawn(move || {
            let mut bytes = Vec::new();
            reader.read_to_end(&mut bytes).unwrap();
            bytes
        });
        let mut writer = pair.master.take_writer().unwrap();
        thread::sleep(Duration::from_millis(80));
        for bytes in input {
            writer.write_all(bytes).unwrap();
            writer.flush().unwrap();
            thread::sleep(Duration::from_millis(80));
        }
        // EOF is part of the oracle: if Rust asks one field too many, the run aborts instead of
        // silently consuming an extra answer the Python form would never have requested.
        drop(writer);
        let status = child.wait().unwrap();
        let output = String::from_utf8_lossy(&drain.join().unwrap()).into_owned();
        (status.exit_code(), output)
    }
}

fn assert_success(args: &[&str], output: &std::process::Output) {
    assert!(
        output.status.success(),
        "args={args:?}\nstdout={}\nstderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
}

fn combined(output: &std::process::Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

fn has_exact_line(output: &std::process::Output, expected: &str) -> bool {
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .chain(String::from_utf8_lossy(&output.stderr).lines())
        .any(|line| line == expected)
}

fn managed_trip_source() -> &'static str {
    concat!(
        "# /// script\n",
        "# [tool.skit]\n",
        "# schema = 1\n",
        "#\n",
        "# [[tool.skit.params]]\n",
        "# name = \"CITY\"\n",
        "# kind = \"const\"\n",
        "# type = \"str\"\n",
        "# default = \"Taipei\"\n",
        "#\n",
        "# [[tool.skit.params]]\n",
        "# name = \"TIMES\"\n",
        "# kind = \"const\"\n",
        "# type = \"int\"\n",
        "# default = 2\n",
        "# ///\n",
        "CITY = \"Taipei\"\n",
        "TIMES = 2\n",
        "print(CITY, TIMES)\n",
    )
}

fn managed_secret_source() -> &'static str {
    concat!(
        "# /// script\n",
        "# [tool.skit]\n",
        "# schema = 1\n",
        "#\n",
        "# [[tool.skit.params]]\n",
        "# name = \"KEY\"\n",
        "# kind = \"const\"\n",
        "# type = \"str\"\n",
        "# secret = true\n",
        "# ///\n",
        "KEY = \"old\"\n",
        "print(KEY)\n",
    )
}

#[test]
fn test_set_inject_values_non_interactive() {
    let sandbox = Sandbox::new();
    let source = managed_trip_source();
    sandbox.write_python_entry("trip", source);
    let uv = sandbox.compile_fake_uv();
    let capture = sandbox.home.path().join("captured.py");

    let output = sandbox
        .command()
        .env("PATH", uv.parent().unwrap())
        .env("SKIT_CAPTURE_SCRIPT", &capture)
        .args([
            "run",
            "trip",
            "--set",
            "CITY=Kaohsiung",
            "--set",
            "TIMES=3",
            "--no-input",
        ])
        .output()
        .unwrap();
    assert_success(&["run", "trip"], &output);

    let mut city = ParamDecl::new("CITY");
    city.binding = ParameterBinding::Const;
    city.parameter_type = ParameterType::Str;
    let mut times = ParamDecl::new("TIMES");
    times.binding = ParameterBinding::Const;
    times.parameter_type = ParameterType::Int;
    let values = BTreeMap::from([
        ("CITY".to_owned(), "Kaohsiung".to_owned()),
        ("TIMES".to_owned(), "3".to_owned()),
    ]);
    let expected = inject_values("python", source, &[city, times], &values).unwrap();
    assert_eq!(fs::read_to_string(&capture).unwrap(), expected);
    assert_eq!(sandbox.value("trip", "CITY").as_deref(), Some("Kaohsiung"));
    assert_eq!(sandbox.value("trip", "TIMES").as_deref(), Some("3"));
    assert_eq!(sandbox.last_exit("trip"), Some(0));
}

#[test]
fn test_set_makes_command_placeholders_runnable() {
    let sandbox = Sandbox::new();
    sandbox.ok(&[
        "add",
        "--cmd",
        "echo {target} {level}",
        "--name",
        "deploy",
        "--no-input",
    ]);
    let output = sandbox.output(&[
        "run",
        "deploy",
        "--set",
        "target=prod",
        "--set",
        "level=high",
        "--no-input",
    ]);
    assert_success(&["run", "deploy"], &output);
    assert!(combined(&output).contains("prod high"), "{}", combined(&output));
    assert_eq!(sandbox.value("deploy", "target").as_deref(), Some("prod"));
    assert_eq!(sandbox.value("deploy", "level").as_deref(), Some("high"));
}

#[test]
fn test_set_wins_over_preset() {
    let sandbox = Sandbox::new();
    sandbox.ok(&[
        "add",
        "--cmd",
        "echo {target}",
        "--name",
        "d2",
        "--no-input",
    ]);
    sandbox.seed_state("d2", "[presets.stage]\ntarget = \"staging\"\n");
    let output = sandbox.output(&[
        "run",
        "d2",
        "-p",
        "stage",
        "--set",
        "target=prod",
        "--no-input",
    ]);
    assert_success(&["run", "d2"], &output);
    assert_eq!(sandbox.value("d2", "target").as_deref(), Some("prod"));
    assert!(combined(&output).contains("prod"), "{}", combined(&output));
    assert!(!combined(&output).contains("staging"), "{}", combined(&output));
}

#[test]
fn test_set_satisfies_required_argparse_field() {
    let sandbox = Sandbox::new();
    let source = concat!(
        "import argparse\n",
        "ap = argparse.ArgumentParser()\n",
        "ap.add_argument('-o', '--output', required=True)\n",
        "ap.parse_args()\n",
    );
    sandbox.write_python_entry("ar", source);
    let uv = sandbox.compile_fake_uv();
    let argv_capture = sandbox.home.path().join("uv-argv.txt");
    let output = sandbox
        .command()
        .env("PATH", uv.parent().unwrap())
        .env("SKIT_CAPTURE_ARGV", &argv_capture)
        .args(["run", "ar", "--set", "output=x.png", "--no-input"])
        .output()
        .unwrap();
    assert_success(&["run", "ar"], &output);
    let argv = fs::read_to_string(argv_capture).unwrap();
    let lines = argv.lines().collect::<Vec<_>>();
    let output_pos = lines.iter().position(|item| *item == "--output").unwrap();
    assert_eq!(lines.get(output_pos + 1), Some(&"x.png"));
    assert_eq!(sandbox.value("ar", "output").as_deref(), Some("x.png"));
}

#[test]
fn test_set_saves_preset_with_dry_run_without_running() {
    let sandbox = Sandbox::new();
    sandbox.write_command_entry(
        "d3",
        "exit 99",
        concat!(
            "params = []\n",
            "[[parameters]]\n",
            "name = \"target\"\n",
            "delivery = \"env\"\n",
            "env_target = \"TARGET\"\n",
            "type = \"str\"\n",
        ),
    );
    let output = sandbox.output(&[
        "run",
        "d3",
        "--set",
        "target=stage",
        "--save-preset",
        "quick",
        "--dry-run",
        "--no-input",
    ]);
    assert_success(&["run", "d3"], &output);
    assert_eq!(sandbox.preset_value("d3", "quick", "target").as_deref(), Some("stage"));
    assert_eq!(sandbox.last_exit("d3"), None, "dry-run must not record a child exit");
    assert_eq!(sandbox.value("d3", "target"), None, "dry-run must not save last-used values");
}

#[test]
fn test_save_preset_on_field_less_entry_refused_saves_nothing() {
    let sandbox = Sandbox::new();
    sandbox.write_command_entry("noargs", "echo hi", "params = []\n");
    let output = sandbox.output(&["run", "noargs", "--save-preset", "nope", "--no-input"]);
    assert_eq!(output.status.code(), Some(2), "{}", combined(&output));
    assert!(
        combined(&output).contains("has no form fields, so there's nothing to save."),
        "{}",
        combined(&output)
    );
    assert!(sandbox.preset_value("noargs", "nope", "anything").is_none());
    assert_eq!(sandbox.last_exit("noargs"), None, "refused run must not launch");
}

#[test]
fn test_save_preset_deferred_until_a_real_run_is_accepted() {
    let sandbox = Sandbox::new();
    let child = sandbox.compile_child_probe();
    let capture = sandbox.home.path().join("child-ran.txt");
    sandbox.write_exe_entry(
        "e",
        &child,
        concat!(
            "[[parameters]]\n",
            "name = \"msg\"\n",
            "delivery = \"env\"\n",
            "env_target = \"MSG\"\n",
            "type = \"str\"\n",
            "required = true\n",
        ),
    );
    let output = sandbox
        .command()
        .env("SKIT_CHILD_CAPTURE", &capture)
        .args([
            "run",
            "e",
            "--set",
            "msg=hi",
            "--save-preset",
            "prod",
            "--no-input",
        ])
        .output()
        .unwrap();
    assert_success(&["run", "e"], &output);
    assert_eq!(fs::read_to_string(capture).unwrap().lines().last(), Some("MSG=hi"));
    assert_eq!(sandbox.preset_value("e", "prod", "msg").as_deref(), Some("hi"));
    assert_eq!(sandbox.last_exit("e"), Some(0));
}

#[test]
fn test_save_preset_not_written_when_launch_is_refused() {
    let sandbox = Sandbox::new();
    let missing = sandbox.home.path().join("missing-executable");
    sandbox.write_exe_entry(
        "e",
        &missing,
        concat!(
            "[[parameters]]\n",
            "name = \"msg\"\n",
            "delivery = \"env\"\n",
            "env_target = \"MSG\"\n",
            "type = \"str\"\n",
            "required = true\n",
        ),
    );
    let output = sandbox.output(&[
        "run",
        "e",
        "--set",
        "msg=hi",
        "--save-preset",
        "prod",
        "--no-input",
    ]);
    assert!(!output.status.success(), "launch refusal unexpectedly succeeded");
    assert_eq!(sandbox.preset_value("e", "prod", "msg"), None);
    assert_eq!(sandbox.last_exit("e"), None);
}

#[test]
fn test_save_preset_dry_run_validation_failure_writes_nothing() {
    let sandbox = Sandbox::new();
    sandbox.write_prompt_entry(
        "big",
        "Do {{a}}\n",
        "runner = \"offline\"\nparams = [\"a\"]\ninterpolate = true\n",
    );
    fs::write(
        sandbox.config.path().join("config.toml"),
        concat!(
            "[prompt]\n",
            "runners_seeded = true\n",
            "[[prompt.runners]]\n",
            "name = \"offline\"\n",
            "argv = [\"missing-agent\", \"{{prompt}}\"]\n",
        ),
    )
    .unwrap();
    let huge = if cfg!(windows) {
        "x".repeat(31_000)
    } else {
        "x".repeat(100_100)
    };
    let output = sandbox
        .command()
        .env("SKIT_BIG", huge)
        .args([
            "run",
            "big",
            "--set",
            "a={env:SKIT_BIG}",
            "--save-preset",
            "toolong",
            "--dry-run",
            "--no-input",
        ])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(125), "{}", combined(&output));
    assert_eq!(sandbox.preset_value("big", "toolong", "a"), None);
    assert_eq!(sandbox.last_exit("big"), None);
}

#[test]
fn test_set_secret_never_persisted_and_masked_in_dry_run() {
    let sandbox = Sandbox::new();
    sandbox.write_python_entry("api", managed_secret_source());
    let dry = sandbox.output(&[
        "run",
        "api",
        "--set",
        "KEY=s3cret-value",
        "--dry-run",
        "--no-input",
    ]);
    assert_success(&["run", "api", "--dry-run"], &dry);
    assert!(!combined(&dry).contains("s3cret-value"), "{}", combined(&dry));
    assert!(combined(&dry).contains("•••"), "{}", combined(&dry));

    let uv = sandbox.compile_fake_uv();
    let capture = sandbox.home.path().join("secret-captured.py");
    let run = sandbox
        .command()
        .env("PATH", uv.parent().unwrap())
        .env("SKIT_CAPTURE_SCRIPT", &capture)
        .args(["run", "api", "--set", "KEY=s3cret-value", "--no-input"])
        .output()
        .unwrap();
    assert_success(&["run", "api"], &run);
    assert!(fs::read_to_string(capture).unwrap().contains("s3cret-value"));
    let state = sandbox.state_text("api");
    assert!(!state.contains("s3cret-value"), "secret leaked to state:\n{state}");
    assert_eq!(sandbox.value("api", "KEY"), None);
}

#[test]
fn test_set_token_values_expand_at_assembly() {
    let sandbox = Sandbox::new();
    sandbox.ok(&[
        "add",
        "--cmd",
        "echo {where}",
        "--name",
        "d4",
        "--no-input",
    ]);
    let output = sandbox.output(&["run", "d4", "--set", "where={cwd}", "--no-input"]);
    assert_success(&["run", "d4"], &output);
    assert!(
        combined(&output).contains(&sandbox.home.path().display().to_string()),
        "{}",
        combined(&output)
    );
    assert_eq!(sandbox.value("d4", "where").as_deref(), Some("{cwd}"));
}

#[test]
fn test_set_malformed_exits_2_with_exact_message() {
    let sandbox = Sandbox::new();
    sandbox.write_python_entry("trip", managed_trip_source());
    for bad in ["NOVALUE", "=v"] {
        let output = sandbox.output(&["run", "trip", "--set", bad, "--no-input"]);
        assert_eq!(output.status.code(), Some(2), "{}", combined(&output));
        let expected = format!("Malformed --set (expected NAME=VALUE): {bad}");
        assert!(has_exact_line(&output, &expected), "{}", combined(&output));
        assert!(!combined(&output).contains("Unknown parameter"));
    }
    let output = sandbox.output(&[
        "run",
        "trip",
        "--set",
        "NOVALUE",
        "--set",
        "=v",
        "--no-input",
    ]);
    assert_eq!(output.status.code(), Some(2), "{}", combined(&output));
    assert!(
        has_exact_line(
            &output,
            "Malformed --set (expected NAME=VALUE): NOVALUE, =v"
        ),
        "{}",
        combined(&output)
    );
    assert_eq!(sandbox.last_exit("trip"), None);
}

#[test]
fn test_set_value_may_contain_equals_signs() {
    let sandbox = Sandbox::new();
    sandbox.ok(&[
        "add",
        "--cmd",
        "echo {CITY}",
        "--name",
        "trip",
        "--no-input",
    ]);
    let output = sandbox.output(&["run", "trip", "--set", "CITY=a=b", "--no-input"]);
    assert_success(&["run", "trip"], &output);
    assert_eq!(sandbox.value("trip", "CITY").as_deref(), Some("a=b"));
}

#[test]
fn test_set_key_is_stripped() {
    let sandbox = Sandbox::new();
    sandbox.ok(&[
        "add",
        "--cmd",
        "echo {CITY}",
        "--name",
        "trip",
        "--no-input",
    ]);
    let output = sandbox.output(&["run", "trip", "--set", " CITY =Kaohsiung", "--no-input"]);
    assert_success(&["run", "trip"], &output);
    assert_eq!(sandbox.value("trip", "CITY").as_deref(), Some("Kaohsiung"));
}

#[test]
fn test_set_unknown_name_exits_2_and_lists_valid() {
    let sandbox = Sandbox::new();
    sandbox.write_python_entry("trip", managed_trip_source());
    let output = sandbox.output(&[
        "run",
        "trip",
        "--set",
        "NOPE=1",
        "--set",
        "ALSO=2",
        "--no-input",
    ]);
    assert_eq!(output.status.code(), Some(2), "{}", combined(&output));
    assert!(
        has_exact_line(
            &output,
            "Unknown parameter for --set: ALSO, NOPE. This entry's parameters: CITY, TIMES"
        ),
        "{}",
        combined(&output)
    );
    assert_eq!(sandbox.last_exit("trip"), None);
}

#[test]
fn test_set_on_entry_without_fields_lists_a_dash() {
    let sandbox = Sandbox::new();
    let child = sandbox.compile_child_probe();
    sandbox.write_exe_entry("tool", &child, "");
    let output = sandbox.output(&["run", "tool", "--set", "X=1", "--no-input"]);
    assert_eq!(output.status.code(), Some(2), "{}", combined(&output));
    assert!(
        has_exact_line(
            &output,
            "Unknown parameter for --set: X. This entry's parameters: —"
        ),
        "{}",
        combined(&output)
    );
    assert_eq!(sandbox.last_exit("tool"), None);
}

#[test]
fn test_set_with_raw_is_a_usage_conflict() {
    let sandbox = Sandbox::new();
    sandbox.write_python_entry("trip", managed_trip_source());
    let output = sandbox.output(&[
        "run",
        "trip",
        "--raw",
        "--set",
        "CITY=x",
        "--no-input",
    ]);
    assert_eq!(output.status.code(), Some(2), "{}", combined(&output));
    assert!(has_exact_line(&output, RAW_CONFLICT), "{}", combined(&output));
    assert_eq!(sandbox.last_exit("trip"), None);
}

#[test]
fn test_preset_with_raw_is_a_usage_conflict() {
    let sandbox = Sandbox::new();
    sandbox.write_python_entry("trip", managed_trip_source());
    sandbox.seed_state("trip", "[presets.loud]\nCITY = \"Tainan\"\n");
    let output = sandbox.output(&[
        "run",
        "trip",
        "--raw",
        "-p",
        "loud",
        "--no-input",
    ]);
    assert_eq!(output.status.code(), Some(2), "{}", combined(&output));
    assert!(has_exact_line(&output, RAW_CONFLICT), "{}", combined(&output));
    assert_eq!(sandbox.last_exit("trip"), None);
}

#[test]
fn test_save_preset_with_raw_is_a_usage_conflict() {
    let sandbox = Sandbox::new();
    sandbox.write_python_entry("trip", managed_trip_source());
    let output = sandbox.output(&[
        "run",
        "trip",
        "--raw",
        "--save-preset",
        "ghost",
        "--no-input",
    ]);
    assert_eq!(output.status.code(), Some(2), "{}", combined(&output));
    assert!(has_exact_line(&output, RAW_CONFLICT), "{}", combined(&output));
    assert_eq!(sandbox.preset_value("trip", "ghost", "CITY"), None);
    assert_eq!(sandbox.last_exit("trip"), None);
}

#[test]
fn test_raw_never_replays_last_extra_args() {
    let sandbox = Sandbox::new();
    let child = sandbox.compile_child_probe();
    let capture = sandbox.home.path().join("raw-child.txt");
    sandbox.write_exe_entry("j", &child, "");

    let first = sandbox
        .command()
        .env("SKIT_CHILD_CAPTURE", &capture)
        .args(["run", "j", "--no-input", "--", "--verbose", "x.png"])
        .output()
        .unwrap();
    assert_success(&["run", "j"], &first);
    assert_eq!(fs::read_to_string(&capture).unwrap(), "--verbose\nx.png");

    fs::remove_file(&capture).unwrap();
    let raw = sandbox
        .command()
        .env("SKIT_CHILD_CAPTURE", &capture)
        .args(["run", "j", "--raw", "--no-input"])
        .output()
        .unwrap();
    assert_success(&["run", "j", "--raw"], &raw);
    assert_eq!(fs::read_to_string(&capture).unwrap(), "");
    assert_eq!(sandbox.last_exit("j"), Some(0));

    fs::remove_file(&capture).unwrap();
    let reused = sandbox
        .command()
        .env("SKIT_CHILD_CAPTURE", &capture)
        .args(["run", "j", "--no-input"])
        .output()
        .unwrap();
    assert_success(&["run", "j"], &reused);
    assert_eq!(fs::read_to_string(&capture).unwrap(), "--verbose\nx.png");
    let stdout = String::from_utf8_lossy(&reused.stdout);
    let stderr = String::from_utf8_lossy(&reused.stderr);
    assert!(stderr.contains("Reusing your last arguments"), "stderr={stderr}");
    assert!(!stdout.contains("Reusing your last arguments"), "stdout={stdout}");
}

#[test]
fn test_set_bad_typed_value_exits_125() {
    let sandbox = Sandbox::new();
    sandbox.write_command_entry(
        "trip",
        "echo {TIMES}",
        concat!(
            "params = [\"TIMES\"]\n",
            "[[parameters]]\n",
            "name = \"TIMES\"\n",
            "delivery = \"placeholder\"\n",
            "type = \"int\"\n",
            "required = true\n",
        ),
    );
    let output = sandbox.output(&["run", "trip", "--set", "TIMES=abc", "--no-input"]);
    assert_eq!(output.status.code(), Some(125), "{}", combined(&output));
    assert!(
        has_exact_line(&output, "TIMES needs a whole number — you typed 'abc'."),
        "{}",
        combined(&output)
    );
    assert_eq!(sandbox.last_exit("trip"), None);
}

#[test]
fn test_set_bad_value_fails_before_the_form_opens() {
    let sandbox = Sandbox::new();
    sandbox.write_command_entry(
        "trip",
        "echo {CITY} {TIMES}",
        concat!(
            "params = [\"CITY\", \"TIMES\"]\n",
            "[[parameters]]\n",
            "name = \"CITY\"\n",
            "delivery = \"placeholder\"\n",
            "type = \"str\"\n",
            "default = \"Taipei\"\n",
            "[[parameters]]\n",
            "name = \"TIMES\"\n",
            "delivery = \"placeholder\"\n",
            "type = \"int\"\n",
            "default = 2\n",
        ),
    );
    let (code, output) = sandbox.run_plain_pty(
        &["run", "trip", "--plain", "--set", "TIMES=abc"],
        &[b"SHOULD-NOT-BE-ASKED\n"],
    );
    assert_eq!(code, 125, "{output}");
    assert!(
        !output.contains("CITY [") && !output.contains("CITY: "),
        "the form opened before --set validation:\n{output}"
    );
    assert!(output.lines().any(|line| line.contains("TIMES needs a whole number")), "{output}");
    assert_eq!(sandbox.last_exit("trip"), None);
}

#[test]
fn test_set_empty_value_on_required_placeholder_exits_125() {
    let sandbox = Sandbox::new();
    sandbox.ok(&[
        "add",
        "--cmd",
        "echo {target}",
        "--name",
        "d5",
        "--no-input",
    ]);
    let output = sandbox.output(&["run", "d5", "--set", "target=", "--no-input"]);
    assert_eq!(output.status.code(), Some(125), "{}", combined(&output));
    assert_eq!(sandbox.last_exit("d5"), None);
}

#[test]
fn test_interactive_form_skips_set_fields() {
    let sandbox = Sandbox::new();
    sandbox.write_command_entry(
        "trip",
        "echo {CITY} {TIMES}",
        concat!(
            "params = [\"CITY\", \"TIMES\"]\n",
            "[[parameters]]\n",
            "name = \"CITY\"\n",
            "delivery = \"placeholder\"\n",
            "type = \"str\"\n",
            "default = \"Taipei\"\n",
            "[[parameters]]\n",
            "name = \"TIMES\"\n",
            "delivery = \"placeholder\"\n",
            "type = \"int\"\n",
            "default = 2\n",
        ),
    );
    sandbox.seed_state("trip", "[values]\nCITY = \"old-city\"\n");
    let (code, output) = sandbox.run_plain_pty(
        &["run", "trip", "--plain", "--set", "TIMES=9"],
        &[b"form-city\n"],
    );
    assert_eq!(code, 0, "{output}");
    assert!(output.contains("CITY [old-city]"), "{output}");
    assert!(!output.contains("TIMES ["), "fixed TIMES must never be asked:\n{output}");
    assert_eq!(sandbox.value("trip", "CITY").as_deref(), Some("form-city"));
    assert_eq!(sandbox.value("trip", "TIMES").as_deref(), Some("9"));
}

#[test]
fn test_interactive_all_fields_set_skips_the_form_entirely() {
    let sandbox = Sandbox::new();
    sandbox.write_command_entry(
        "trip",
        "echo {CITY} {TIMES}",
        concat!(
            "params = [\"CITY\", \"TIMES\"]\n",
            "[[parameters]]\n",
            "name = \"CITY\"\n",
            "delivery = \"placeholder\"\n",
            "type = \"str\"\n",
            "[[parameters]]\n",
            "name = \"TIMES\"\n",
            "delivery = \"placeholder\"\n",
            "type = \"int\"\n",
        ),
    );
    let (code, output) = sandbox.run_plain_pty(
        &[
            "run",
            "trip",
            "--plain",
            "--set",
            "CITY=x",
            "--set",
            "TIMES=1",
        ],
        &[],
    );
    assert_eq!(code, 0, "{output}");
    assert!(!output.contains("CITY: ") && !output.contains("TIMES: "), "{output}");
    assert_eq!(sandbox.value("trip", "CITY").as_deref(), Some("x"));
    assert_eq!(sandbox.value("trip", "TIMES").as_deref(), Some("1"));
}

#[test]
fn test_save_preset_no_fields_refused_before_any_form() {
    let sandbox = Sandbox::new();
    sandbox.write_prompt_entry("plainp", "Just do the thing.\n", "interpolate = true\n");
    fs::write(
        sandbox.config.path().join("config.toml"),
        concat!(
            "[prompt]\n",
            "runners_seeded = true\n",
            "[[prompt.runners]]\n",
            "name = \"offline\"\n",
            "argv = [\"missing-agent\", \"{{prompt}}\"]\n",
        ),
    )
    .unwrap();
    let (code, output) = sandbox.run_plain_pty(
        &["run", "plainp", "--plain", "--save-preset", "x"],
        &[],
    );
    assert_eq!(code, 2, "{output}");
    assert!(output.contains("has no form fields"), "{output}");
    assert!(!output.contains("Prompt runner"), "runner picker opened before refusal:\n{output}");
    assert!(
        fs::read_dir(sandbox.state.path())
            .map(|mut entries| entries.next().is_none())
            .unwrap_or(true),
        "refused invocation left a state fingerprint"
    );
}

#[test]
fn test_save_preset_persists_when_ctrl_c_ends_an_accepted_run() {
    let sandbox = Sandbox::new();
    let template = if cfg!(windows) {
        "exit /b 130"
    } else {
        "kill -INT $$"
    };
    sandbox.write_command_entry(
        "e",
        template,
        concat!(
            "params = []\n",
            "[[parameters]]\n",
            "name = \"msg\"\n",
            "delivery = \"env\"\n",
            "env_target = \"MSG\"\n",
            "type = \"str\"\n",
            "required = true\n",
        ),
    );
    let output = sandbox.output(&[
        "run",
        "e",
        "--set",
        "msg=hi",
        "--save-preset",
        "prod",
        "--no-input",
    ]);
    assert_eq!(output.status.code(), Some(130), "{}", combined(&output));
    assert_eq!(sandbox.preset_value("e", "prod", "msg").as_deref(), Some("hi"));
    assert_eq!(sandbox.last_exit("e"), Some(130));
}
