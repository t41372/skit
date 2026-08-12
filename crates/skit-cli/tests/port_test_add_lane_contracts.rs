//! Non-editor public-process ports from Python `tests/test_add_lane_contracts.py` at
//! `main@206f9ef`.
//!
//! The seven true editor-lane contracts live in `port_test_add_lane_editor.rs`, where a PTY and a
//! compiled editor probe make "editor never launched" observable. This file keeps stdin/path/read
//! lanes on real subprocesses and authoritative store/JSON state.

use std::{fs, path::{Path, PathBuf}, process::Output};

use assert_cmd::Command;
use skit_application::EntryRepository as _;
use skit_domain::StorageMode;
use skit_store::FileStore;
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
            .current_dir(self.home.path());
        command
    }

    fn store(&self) -> FileStore {
        FileStore::new(self.data.path())
    }

    fn run(&self, args: &[&str]) -> Output {
        self.command().args(args).output().unwrap()
    }

    fn run_stdin(&self, args: &[&str], input: &str) -> Captured {
        let mut command = self.command();
        let assertion = command.args(args).write_stdin(input).assert();
        let output = assertion.get_output();
        Captured {
            code: output.status.code().unwrap_or(-1),
            stdout: output.stdout.clone(),
            stderr: output.stderr.clone(),
        }
    }

    fn source(&self, name: &str, bytes: &[u8]) -> PathBuf {
        let path = self.home.path().join(name);
        fs::write(&path, bytes).unwrap();
        path
    }

    fn draft(&self, name: &str, bytes: &[u8]) -> PathBuf {
        let root = self.data.path().join("drafts");
        fs::create_dir_all(&root).unwrap();
        let path = root.join(name);
        fs::write(&path, bytes).unwrap();
        path
    }
}

struct Captured {
    code: i32,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

impl Captured {
    fn text(&self) -> String {
        format!(
            "{}{}",
            String::from_utf8_lossy(&self.stdout),
            String::from_utf8_lossy(&self.stderr)
        )
    }
}

fn text(output: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

fn flat(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn assert_success(output: &Output) {
    assert_eq!(output.status.code(), Some(0), "{}", text(output));
}

fn add_path(sandbox: &Sandbox, path: &Path, tail: &[&str]) -> Output {
    let mut command = sandbox.command();
    command.arg("add").arg(path).args(tail).output().unwrap()
}

#[test]
fn test_selector_collisions_are_refused_one_voice() {
    let sandbox = Sandbox::new();
    let real = sandbox.source("real.py", b"print(1)\n");
    let path = real.to_str().unwrap();
    let cases = [
        (vec!["add", path, "--cmd", "echo {x}"], "a file path", None),
        (vec!["add", "-", "--cmd", "echo {x}"], "stdin ('-')", Some("print(1)\n")),
        (vec!["add", "--edit", path], "--edit", None),
        (vec!["add", "--edit", "-"], "--edit", Some("print(1)\n")),
    ];

    for (argv, needle, input) in cases {
        let captured = if let Some(input) = input {
            sandbox.run_stdin(&argv, input)
        } else {
            let output = sandbox.run(&argv);
            Captured {
                code: output.status.code().unwrap_or(-1),
                stdout: output.stdout,
                stderr: output.stderr,
            }
        };
        let shown = flat(&captured.text());
        assert_eq!(captured.code, 2, "argv={argv:?}: {shown}");
        assert!(
            shown.contains("each pick a different way to add"),
            "argv={argv:?}: {shown}"
        );
        assert!(shown.contains(needle), "argv={argv:?}: {shown}");
        assert!(sandbox.store().scan().unwrap().entries.is_empty(), "argv={argv:?}");
        let drafts = sandbox.data.path().join("drafts");
        assert!(
            !drafts.exists() || fs::read_dir(drafts).unwrap().next().is_none(),
            "selector refusal materialized a draft for {argv:?}"
        );
    }
}

#[test]
fn test_stdin_versioned_python_shebang_lands_as_python() {
    let sandbox = Sandbox::new();
    let output = sandbox.run_stdin(
        &["add", "-", "-n", "v"],
        "#!/usr/bin/env python3.12\nprint(1)\n",
    );

    assert_eq!(output.code, 0, "{}", output.text());
    assert_eq!(sandbox.store().resolve("v").unwrap().meta.kind.as_str(), "python");
    let show = sandbox.run(&["show", "v", "--json"]);
    assert_success(&show);
    let value: serde_json::Value = serde_json::from_slice(&show.stdout).unwrap();
    assert_eq!(value["kind"], "python");
}

#[test]
fn test_stdin_prompt_bogus_runner_refused_before_any_draft() {
    let sandbox = Sandbox::new();
    let output = sandbox.run_stdin(
        &[
            "add", "-", "--prompt", "--runner", "bogus", "-n", "p",
        ],
        "x {{u}}\n",
    );
    let shown = flat(&output.text());

    assert_eq!(output.code, 2, "{shown}");
    assert!(shown.contains("Unknown runner"), "{shown}");
    assert!(sandbox.store().scan().unwrap().entries.is_empty());
    let drafts = sandbox.data.path().join("drafts");
    assert!(!drafts.exists() || fs::read_dir(drafts).unwrap().next().is_none());
}

#[test]
fn test_prompt_no_input_piped_still_adds() {
    let sandbox = Sandbox::new();
    let output = sandbox.run_stdin(
        &["add", "--prompt", "-n", "pp", "--no-input"],
        "Summarize {{url}}\n",
    );

    assert_eq!(output.code, 0, "{}", output.text());
    let entry = sandbox.store().resolve("pp").unwrap();
    assert_eq!(entry.meta.kind.as_str(), "prompt");
    assert_eq!(
        skit_domain::EntrySettings::from_meta(&entry.meta).params,
        ["url"]
    );
}

#[test]
fn test_path_add_of_a_drafts_home_file_unlinks_it_on_copy() {
    let sandbox = Sandbox::new();
    let draft = sandbox.draft("skit-new-resumeme.py", b"print('resume')\n");

    let output = add_path(
        &sandbox,
        &draft,
        &["-n", "res", "--no-input"],
    );

    assert_success(&output);
    assert_eq!(sandbox.store().resolve("res").unwrap().meta.mode, StorageMode::Copy);
    assert!(!draft.exists(), "successful copy resume left its owned draft behind");
}

#[test]
fn test_path_add_of_a_drafts_home_file_refuses_reference() {
    let sandbox = Sandbox::new();
    let draft = sandbox.draft("skit-new-keepme.py", b"print('keep')\n");

    let output = add_path(
        &sandbox,
        &draft,
        &["-n", "kep", "--ref", "--no-input"],
    );
    let shown = flat(&text(&output));

    assert_eq!(output.status.code(), Some(2), "{shown}");
    assert!(shown.contains("one of skit's own kept drafts"), "{shown}");
    assert!(shown.contains("Drop --ref"), "{shown}");
    assert!(draft.exists());
    assert!(sandbox.store().resolve("kep").is_err());
}

#[test]
fn test_path_add_of_a_normal_file_never_unlinks_the_original() {
    let sandbox = Sandbox::new();
    let source = sandbox.source("mine.py", b"print('mine')\n");

    let output = add_path(&sandbox, &source, &["-n", "mine", "--no-input"]);

    assert_success(&output);
    assert_eq!(sandbox.store().resolve("mine").unwrap().meta.mode, StorageMode::Copy);
    assert!(source.exists(), "copy add moved or deleted the user's original file");
}

#[test]
fn test_shell_getopts_add_prints_the_read_notice() {
    let sandbox = Sandbox::new();
    let source = sandbox.source(
        "flags.sh",
        b"#!/usr/bin/env bash\nwhile getopts \"n:v\" opt; do :; done\n",
    );

    let output = add_path(&sandbox, &source, &["-n", "flags", "--no-input"]);

    assert_success(&output);
    assert!(
        text(&output).contains("skit read this script's own arguments"),
        "{}",
        text(&output)
    );
}

#[test]
fn test_shell_dynamic_getopts_add_prints_the_passthrough_notice() {
    let sandbox = Sandbox::new();
    let source = sandbox.source(
        "dyn.sh",
        b"#!/usr/bin/env bash\nOPTS=\"n:v\"\nwhile getopts \"$OPTS\" opt; do :; done\n",
    );

    let output = add_path(&sandbox, &source, &["-n", "dyn", "--no-input"]);
    let shown = text(&output);

    assert_success(&output);
    assert!(shown.contains("parses its own arguments"), "{shown}");
    assert!(shown.contains("getopts"), "{shown}");
}

#[test]
fn test_js_parseargs_add_prints_the_read_notice() {
    let sandbox = Sandbox::new();
    let source = sandbox.source(
        "cli.js",
        concat!(
            "#!/usr/bin/env node\n",
            "import { parseArgs } from 'node:util'\n",
            "const { values } = parseArgs({ options: { name: { type: 'string' } } })\n",
            "console.log(values)\n",
        )
        .as_bytes(),
    );

    let output = add_path(&sandbox, &source, &["-n", "jscli", "--no-input"]);

    assert_success(&output);
    assert!(
        text(&output).contains("skit read this script's own arguments"),
        "{}",
        text(&output)
    );
}

#[test]
fn test_params_python_argparse_read_view_is_plain() {
    let sandbox = Sandbox::new();
    let source = sandbox.source(
        "ap.py",
        concat!(
            "import argparse\n",
            "OUT = 'hi'\n",
            "p = argparse.ArgumentParser()\n",
            "p.add_argument('--n')\n",
            "p.parse_args()\n",
            "print(OUT)\n",
        )
        .as_bytes(),
    );
    assert_success(&add_path(&sandbox, &source, &["-n", "ap", "--no-input"]));

    let plain = sandbox.run(&["params", "ap"]);
    assert_success(&plain);
    let shown = text(&plain);
    assert!(shown.contains("has no managed parameters."), "{shown}");
    assert!(!shown.contains("--manage"), "{shown}");
    let json = sandbox.run(&["params", "ap", "--json"]);
    assert_success(&json);
    let value: serde_json::Value = serde_json::from_slice(&json.stdout).unwrap();
    assert_eq!(value["unmanaged"], serde_json::json!([]));
}

#[test]
fn test_params_python_constants_only_still_offers_manage() {
    let sandbox = Sandbox::new();
    let source = sandbox.source("co.py", b"OUT = 'hi'\nprint(OUT)\n");
    assert_success(&add_path(&sandbox, &source, &["-n", "co", "--no-input"]));

    let plain = sandbox.run(&["params", "co"]);
    assert_success(&plain);
    assert!(text(&plain).contains("--manage"), "{}", text(&plain));
    let json = sandbox.run(&["params", "co", "--json"]);
    assert_success(&json);
    let value: serde_json::Value = serde_json::from_slice(&json.stdout).unwrap();
    assert_eq!(value["unmanaged"], serde_json::json!(["OUT"]));
}

#[test]
fn test_manage_flip_note_names_the_reader_form_then_stays_quiet() {
    let sandbox = Sandbox::new();
    let both = sandbox.source(
        "both.sh",
        b"#!/usr/bin/env bash\nCITY=Taipei\nwhile getopts \"n:v\" opt; do :; done\necho $CITY\n",
    );
    assert_success(&add_path(&sandbox, &both, &["-n", "both", "--no-input"]));

    let first = sandbox.run(&["params", "both", "--manage", "CITY"]);
    assert_success(&first);
    let first_text = text(&first);
    assert!(
        first_text.contains("The run form now asks for the managed parameters"),
        "{first_text}"
    );
    assert!(first_text.contains("getopts"), "{first_text}");

    let second = sandbox.source(
        "second.sh",
        b"#!/usr/bin/env bash\nCITY=Taipei\nPORT=8080\nwhile getopts \"n:v\" opt; do :; done\necho $CITY $PORT\n",
    );
    assert_success(&add_path(
        &sandbox,
        &second,
        &["-n", "second", "--no-input"],
    ));
    assert_success(&sandbox.run(&["params", "second", "--manage", "CITY"]));
    let again = sandbox.run(&["params", "second", "--manage", "PORT"]);
    assert_success(&again);
    assert!(
        !text(&again).contains("The run form now asks for the managed parameters"),
        "{}",
        text(&again)
    );
}

#[test]
fn test_manage_flip_json_stdout_is_exactly_one_document() {
    let sandbox = Sandbox::new();
    let source = sandbox.source(
        "j.sh",
        b"#!/usr/bin/env bash\nCITY=Taipei\nwhile getopts \"n:v\" opt; do :; done\necho $CITY\n",
    );
    assert_success(&add_path(
        &sandbox,
        &source,
        &["-n", "jflip", "--no-input"],
    ));

    let result = sandbox.run(&[
        "params", "jflip", "--manage", "CITY", "--json",
    ]);

    assert_success(&result);
    let value: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
    assert!(
        value["params"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["name"] == "CITY")
    );
    assert!(!String::from_utf8_lossy(&result.stdout).contains("The run form now asks"));
}
