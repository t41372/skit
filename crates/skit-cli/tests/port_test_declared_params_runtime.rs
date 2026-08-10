//! Public CLI/runtime ports from Python v0.4 `tests/test_declared_params.py`.
//!
//! Secret and delivery assertions intentionally cross persistence/planning boundaries. A masked
//! label without the corresponding state or child-process behavior is not sufficient.

use std::{fs, path::PathBuf};

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
        command
    }

    fn output(&self, args: &[&str]) -> std::process::Output {
        self.command().args(args).output().unwrap()
    }

    fn ok(&self, args: &[&str]) -> std::process::Output {
        let output = self.output(args);
        assert!(
            output.status.success(),
            "args={args:?}\nstdout={}\nstderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
        );
        output
    }

    fn json(&self, args: &[&str]) -> Value {
        let output = self.ok(args);
        serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
            panic!(
                "stdout must be exactly one JSON document: {error}\nstdout={}\nstderr={}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr),
            )
        })
    }

    fn add_exe(&self, name: &str) {
        let source = self.home.path().join(format!("{name}-exe"));
        fs::create_dir(&source).unwrap();
        self.ok(&["add", source.to_str().unwrap(), "--exe", "--name", name]);
    }

    fn add_command(&self, name: &str, template: &str) {
        self.ok(&["add", "--cmd", template, "--name", name, "--no-input"]);
    }

    fn add_ruby(&self, name: &str) {
        let source = self.home.path().join(format!("{name}.rb"));
        fs::write(&source, "#!/usr/bin/env ruby\nputs 'hi'\n").unwrap();
        self.ok(&["add", source.to_str().unwrap(), "--name", name, "--no-input"]);
    }

    fn add_python(&self, name: &str, source: &str) {
        let path = self.home.path().join(format!("{name}.py"));
        fs::write(&path, source).unwrap();
        self.ok(&["add", path.to_str().unwrap(), "--name", name, "--no-input"]);
    }

    fn state_path(&self, slug: &str) -> PathBuf {
        self.state.path().join("values").join(format!("{slug}.toml"))
    }

    fn seed_raw_values(&self, slug: &str, pairs: &[(&str, &str)]) {
        let path = self.state_path(slug);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let mut body = String::from("[values]\n");
        for (name, value) in pairs {
            body.push_str(&format!("{name} = {value:?}\n"));
        }
        fs::write(path, body).unwrap();
    }
}

fn combined(output: &std::process::Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

fn strip_ansi(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut output = String::with_capacity(input.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == 0x1b && bytes.get(index + 1) == Some(&b'[') {
            index += 2;
            while index < bytes.len() {
                let byte = bytes[index];
                index += 1;
                if (0x40..=0x7e).contains(&byte) {
                    break;
                }
            }
            continue;
        }
        let ch = input[index..].chars().next().unwrap();
        output.push(ch);
        index += ch.len_utf8();
    }
    output
}

fn declared<'a>(document: &'a Value, name: &str) -> &'a Value {
    document["declared"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["name"] == name)
        .unwrap_or_else(|| panic!("missing declared {name}: {document}"))
}

#[test]
fn test_cli_add_flag_param_on_exe_then_run_set() {
    let sandbox = Sandbox::new();
    sandbox.add_exe("prog");
    let output = sandbox.ok(&[
        "params",
        "prog",
        "--add",
        "width",
        "--type",
        "width=int",
        "--deliver",
        "width=flag",
        "--flag",
        "width=--width",
        "--default",
        "width=800",
    ]);
    let human = strip_ansi(&combined(&output));
    assert!(
        human.contains("Declared parameters: width"),
        "Python's human summary disappeared:\n{human}"
    );
    let document = sandbox.json(&["params", "prog", "--json"]);
    let width = declared(&document, "width");
    assert_eq!(width["delivery"], "flag");
    assert_eq!(width["type"], "int");
    assert_eq!(width["flag"], "--width");
    assert_eq!(width["default"], 800);

    let dry = sandbox.ok(&[
        "run",
        "prog",
        "--set",
        "width=1024",
        "--dry-run",
        "--no-input",
    ]);
    let text = strip_ansi(&String::from_utf8_lossy(&dry.stdout)).replace('\n', "");
    assert!(text.contains("--width") && text.contains("1024"), "{text}");
}

#[test]
fn test_cli_exe_show_table_and_json() {
    let sandbox = Sandbox::new();
    sandbox.add_exe("prog");
    sandbox.ok(&[
        "params",
        "prog",
        "--add",
        "width",
        "--deliver",
        "width=flag",
        "--flag",
        "width=--width",
        "--type",
        "width=int",
        "--default",
        "width=800",
    ]);
    let human = sandbox.ok(&["params", "prog"]);
    let text = strip_ansi(&String::from_utf8_lossy(&human.stdout));
    assert!(text.contains("width") && text.contains("flag"), "{text}");
    let document = sandbox.json(&["params", "prog", "--json"]);
    assert_eq!(declared(&document, "width")["delivery"], "flag");
}

#[test]
fn test_cli_exe_show_without_declared_is_plain_message() {
    let sandbox = Sandbox::new();
    sandbox.add_exe("prog");
    let output = sandbox.ok(&["params", "prog"]);
    let text = strip_ansi(&String::from_utf8_lossy(&output.stdout));
    assert!(text.contains("has no managed parameters"), "{text}");
}

#[test]
fn test_cli_python_manage_with_json_emits_the_final_read_view() {
    let sandbox = Sandbox::new();
    sandbox.add_python("job", "CITY = \"Taipei\"\nprint(CITY)\n");
    let document = sandbox.json(&["params", "job", "--manage", "CITY", "--json"]);
    assert_eq!(
        document["params"]
            .as_array()
            .unwrap()
            .iter()
            .map(|row| row["name"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["CITY"]
    );
}

#[test]
fn test_cli_add_choice_placeholder_on_command_then_run() {
    let sandbox = Sandbox::new();
    sandbox.add_command("conv", "echo {size}");
    sandbox.ok(&[
        "params",
        "conv",
        "--add",
        "size",
        "--type",
        "size=choice",
        "--choices",
        "size=s,m,l",
        "--default",
        "size=m",
        "--optional",
        "size",
    ]);
    let document = sandbox.json(&["params", "conv", "--json"]);
    let size = declared(&document, "size");
    assert_eq!(size["delivery"], "placeholder");
    assert_eq!(size["type"], "choice");
    assert_eq!(size["choices"], serde_json::json!(["s", "m", "l"]));
    assert_eq!(size["default"], "m");
    assert_eq!(size["required"], false);

    let run = sandbox.ok(&["run", "conv", "--no-input"]);
    assert!(
        strip_ansi(&String::from_utf8_lossy(&run.stdout)).contains('m'),
        "the declared default did not fill the placeholder:\n{}",
        combined(&run)
    );
}

#[test]
fn test_cli_command_show_enriched_and_env_rider() {
    let sandbox = Sandbox::new();
    sandbox.add_command("c", "echo {msg}");
    sandbox.ok(&[
        "params",
        "c",
        "--add",
        "msg",
        "--type",
        "msg=str",
        "--default",
        "msg=hi",
        "--optional",
        "msg",
    ]);
    sandbox.ok(&[
        "params",
        "c",
        "--add",
        "RETRIES",
        "--deliver",
        "RETRIES=env",
    ]);
    let human = sandbox.ok(&["params", "c"]);
    let text = strip_ansi(&String::from_utf8_lossy(&human.stdout));
    assert!(
        text.contains("msg") && text.contains("optional") && text.contains("RETRIES"),
        "{text}"
    );
    let document = sandbox.json(&["params", "c", "--json"]);
    let names = document["declared"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| row["name"].as_str().unwrap())
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(names, std::collections::BTreeSet::from(["RETRIES", "msg"]));
}

#[test]
fn test_cli_command_env_rider_only_no_placeholders() {
    let sandbox = Sandbox::new();
    sandbox.add_command("noph", "echo hi");
    sandbox.ok(&[
        "params",
        "noph",
        "--add",
        "RETRIES",
        "--deliver",
        "RETRIES=env",
    ]);
    let output = sandbox.ok(&["params", "noph"]);
    let text = strip_ansi(&String::from_utf8_lossy(&output.stdout));
    assert!(text.contains("RETRIES"), "{text}");
}

#[test]
fn test_cli_python_declared_op_is_refused() {
    let sandbox = Sandbox::new();
    sandbox.add_python("py", "CITY = \"x\"\nprint(CITY)\n");
    let output = sandbox.output(&["params", "py", "--add", "WIDTH"]);
    assert_eq!(output.status.code(), Some(1), "{}", combined(&output));
    assert!(
        strip_ansi(&combined(&output)).contains("manages its parameters from the script itself"),
        "{}",
        combined(&output)
    );
}

#[test]
fn test_cli_secret_override_persists_value_now_that_it_isnt_secret() {
    let sandbox = Sandbox::new();
    sandbox.add_command("auth", "echo {token_file}");
    sandbox.ok(&[
        "params",
        "auth",
        "--add",
        "token_file",
        "--no-secret",
        "token_file",
    ]);
    let document = sandbox.json(&["params", "auth", "--json"]);
    assert_eq!(declared(&document, "token_file")["secret"], false);

    sandbox.ok(&[
        "run",
        "auth",
        "--set",
        "token_file=creds.json",
        "--no-input",
    ]);
    let state = sandbox.json(&["params", "auth", "--json"]);
    assert_eq!(state["last_values"]["token_file"], "creds.json");
}

#[test]
fn test_cli_secret_declared_env_purges_prior_plaintext() {
    let sandbox = Sandbox::new();
    sandbox.add_command("prog", "echo hi");
    sandbox.ok(&[
        "params",
        "prog",
        "--add",
        "TOKEN",
        "--deliver",
        "TOKEN=env",
    ]);
    sandbox.seed_raw_values("prog", &[("TOKEN", "plaintext")]);
    assert_eq!(
        sandbox.json(&["params", "prog", "--json"])["last_values"]["TOKEN"],
        "plaintext",
        "the fixture must prove plaintext existed before the secret promotion"
    );

    let output = sandbox.ok(&["params", "prog", "--secret", "TOKEN"]);
    let state = sandbox.json(&["params", "prog", "--json"]);
    assert!(
        state["last_values"].get("TOKEN").is_none(),
        "secret promotion left prior plaintext behind: {state}"
    );
    assert!(
        strip_ansi(&combined(&output)).contains("Removed previously stored plaintext"),
        "state was scrubbed but the Python disclosure warning disappeared:\n{}",
        combined(&output)
    );
}

#[test]
fn test_cli_declared_secret_env_source_resolves_without_prompting() {
    let sandbox = Sandbox::new();
    let out = sandbox.home.path().join("token-output.txt");
    let template = if cfg!(windows) {
        format!("echo %TOKEN% > \"{}\"", out.display())
    } else {
        let escaped = out.display().to_string().replace('\'', "'\\''");
        format!("printf '%s' \"$TOKEN\" > '{escaped}'")
    };
    sandbox.add_command("svc", &template);
    sandbox.ok(&[
        "params",
        "svc",
        "--add",
        "TOKEN",
        "--deliver",
        "TOKEN=env",
        "--secret",
        "TOKEN",
        "--env-source",
        "TOKEN=SVC_TOKEN",
    ]);
    let mut command = sandbox.command();
    let output = command
        .env("SVC_TOKEN", "from-env")
        .args(["run", "svc", "--no-input"])
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", combined(&output));
    assert_eq!(fs::read_to_string(&out).unwrap().trim(), "from-env");
    let state = sandbox.json(&["params", "svc", "--json"]);
    assert!(
        state["last_values"].get("TOKEN").is_none(),
        "secret leaked into state: {state}"
    );
}

#[test]
fn test_cli_run_set_env_and_placeholder_dry_run() {
    let sandbox = Sandbox::new();
    sandbox.add_command("dr", "echo {msg}");
    sandbox.ok(&[
        "params",
        "dr",
        "--add",
        "RETRIES",
        "--deliver",
        "RETRIES=env",
        "--default",
        "RETRIES=3",
    ]);
    let output = sandbox.ok(&[
        "run",
        "dr",
        "--set",
        "msg=hello",
        "--dry-run",
        "--no-input",
    ]);
    let text = strip_ansi(&String::from_utf8_lossy(&output.stdout));
    assert!(text.contains("RETRIES=3"), "{text}");
}

#[test]
fn test_cli_command_env_show_json_source_env() {
    let sandbox = Sandbox::new();
    sandbox.add_command("cj", "echo {m}");
    sandbox.ok(&["params", "cj", "--add", "N", "--deliver", "N=env"]);
    let document = sandbox.json(&["show", "cj", "--json"]);
    let field = document["fields"]
        .as_array()
        .unwrap()
        .iter()
        .find(|field| field["key"] == "N")
        .unwrap();
    assert_eq!(field["source"], "env");
}

#[test]
fn test_cli_exe_show_masks_secret_default_and_last_value() {
    let sandbox = Sandbox::new();
    sandbox.add_exe("prog");
    sandbox.ok(&[
        "params",
        "prog",
        "--add",
        "a",
        "--secret",
        "a",
        "--add",
        "b",
        "--secret",
        "b",
        "--default",
        "b=x",
    ]);
    sandbox.seed_raw_values("prog", &[("a", "stale")]);

    let output = sandbox.ok(&["params", "prog"]);
    let text = strip_ansi(&String::from_utf8_lossy(&output.stdout));
    assert!(text.contains("•••"), "secret rows were not visibly masked:\n{text}");
    assert!(
        !text.contains("Last value: stale") && !text.contains("stale"),
        "legacy plaintext leaked through the human view:\n{text}"
    );
    assert!(
        !text.contains("Current default: x"),
        "secret definition default leaked through the human view:\n{text}"
    );
}

#[test]
fn test_cli_command_show_masks_secret_placeholder_and_undeclared() {
    let sandbox = Sandbox::new();
    sandbox.add_command("lg", "echo {password} {other}");
    sandbox.ok(&[
        "params",
        "lg",
        "--secret",
        "password",
        "--default",
        "password=seed",
        "--required",
        "password",
    ]);
    sandbox.seed_raw_values("lg", &[("password", "stale")]);

    let output = sandbox.ok(&["params", "lg"]);
    let text = strip_ansi(&String::from_utf8_lossy(&output.stdout));
    assert!(text.contains("•••"), "secret placeholder was not masked:\n{text}");
    assert!(
        !text.contains("Last value: stale") && !text.contains("stale"),
        "legacy placeholder plaintext leaked:\n{text}"
    );
    assert!(
        !text.contains("Current default: seed") && !text.contains("seed"),
        "secret placeholder default leaked:\n{text}"
    );
    assert!(text.contains("other"), "undeclared placeholder disappeared:\n{text}");
}

#[test]
fn test_declared_add_on_interpreted_kind_delivers_at_run() {
    let sandbox = Sandbox::new();
    sandbox.add_ruby("rb2");
    sandbox.ok(&[
        "params",
        "rb2",
        "--add",
        "SIZE",
        "--flag",
        "SIZE=--size",
    ]);
    let output = sandbox.ok(&[
        "run",
        "rb2",
        "--set",
        "SIZE=5",
        "--dry-run",
        "--no-input",
    ]);
    let text = strip_ansi(&String::from_utf8_lossy(&output.stdout)).replace('\n', "");
    assert!(text.contains("--size") && text.contains('5'), "{text}");
}

#[test]
fn test_declared_table_is_shown_for_an_interpreted_meta_kind() {
    let sandbox = Sandbox::new();
    sandbox.add_ruby("rb3");
    sandbox.ok(&["params", "rb3", "--add", "GREETING"]);
    let output = sandbox.ok(&["params", "rb3"]);
    let text = strip_ansi(&String::from_utf8_lossy(&output.stdout));
    assert!(text.contains("GREETING"), "{text}");
    assert!(!text.contains("has no managed parameters"), "{text}");
}

#[test]
fn test_declared_param_on_an_interpreted_kind_actually_delivers() {
    let sandbox = Sandbox::new();
    sandbox.add_ruby("rb4");
    sandbox.ok(&[
        "params",
        "rb4",
        "--add",
        "GREETING",
        "--flag",
        "GREETING=--greeting",
    ]);
    let output = sandbox.ok(&[
        "run",
        "rb4",
        "--set",
        "GREETING=world",
        "--dry-run",
        "--no-input",
    ]);
    let text = strip_ansi(&String::from_utf8_lossy(&output.stdout)).replace('\n', "");
    assert!(
        text.contains("--greeting") && text.contains("world"),
        "{text}"
    );
}
