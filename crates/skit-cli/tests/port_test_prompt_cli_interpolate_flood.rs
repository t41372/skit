use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command as ProcessCommand, Output},
};

use assert_cmd::Command;
use serde_json::{Value, json};
use skit_application::{CreateEntry, EntryMutationRepository as _, EntryPayload, SourcePermissions};
use skit_domain::{EntryKind, EntrySettings, StorageMode, parameters::synthesized_placeholder};
use skit_store::FileStore;
use tempfile::TempDir;

const FROZEN_AUTO_MANAGE_LIMIT: usize = 30;
const FROZEN_LIST_PREVIEW_LIMIT: usize = 20;

struct Sandbox {
    data: TempDir,
    state: TempDir,
    config: TempDir,
    home: TempDir,
    tools: TempDir,
}

impl Sandbox {
    fn new() -> Self {
        Self {
            data: TempDir::new().unwrap(),
            state: TempDir::new().unwrap(),
            config: TempDir::new().unwrap(),
            home: TempDir::new().unwrap(),
            tools: TempDir::new().unwrap(),
        }
    }

    fn command(&self) -> Command {
        self.command_lang("en")
    }

    fn command_lang(&self, lang: &str) -> Command {
        let mut command = assert_cmd::cargo::cargo_bin_cmd!("skit");
        command
            .env("SKIT_DATA_DIR", self.data.path())
            .env("SKIT_STATE_DIR", self.state.path())
            .env("SKIT_CONFIG_DIR", self.config.path())
            .env("SKIT_LANG", lang)
            .env("HOME", self.home.path())
            .env("USERPROFILE", self.home.path())
            .env("XDG_CONFIG_HOME", self.home.path().join("xdg-config"))
            .env("XDG_DATA_HOME", self.home.path().join("xdg-data"))
            .env("XDG_STATE_HOME", self.home.path().join("xdg-state"))
            .current_dir(self.home.path());
        command
    }

    fn run(&self, args: &[&str]) -> Output {
        self.command().args(args).output().unwrap()
    }

    fn source(&self, name: &str, body: &str) -> PathBuf {
        let path = self.home.path().join(name);
        fs::write(&path, body).unwrap();
        path
    }

    fn add_prompt(&self, name: &str, body: &str, no_interpolate: bool) {
        let source = self.source(&format!("{name}.prompt.md"), body);
        let mut args = vec!["add", source.to_str().unwrap(), "--name", name, "--no-input"];
        if no_interpolate {
            args.push("--no-interpolate");
        }
        let output = self.run(&args);
        assert_eq!(output.status.code(), Some(0), "{}", combined(&output));
    }

    fn show_json(&self, name: &str) -> Value {
        let output = self.run(&["show", name, "--json"]);
        assert_eq!(output.status.code(), Some(0), "{}", combined(&output));
        serde_json::from_slice(&output.stdout).unwrap()
    }

    fn create_prompt_managed_a(&self, body: &str, interpolate: bool) -> skit_domain::Entry {
        let source = self.home.path().join("p.prompt.md");
        fs::write(&source, body).unwrap();
        self.store().create(CreateEntry {
            name: "p".to_owned(),
            kind: EntryKind::parse("prompt").unwrap(),
            mode: StorageMode::Copy,
            source: source.display().to_string(),
            workdir: "invoke".to_owned(),
            description: String::new(),
            payload: Some(EntryPayload {
                bytes: body.as_bytes().to_vec(),
                stored_name: Some("prompt.md".to_owned()),
                permissions: SourcePermissions::default(),
            }),
            settings: EntrySettings {
                params: vec!["a".to_owned()],
                parameters: vec![synthesized_placeholder("a")],
                interpolate,
                ..EntrySettings::default()
            },
        }).unwrap()
    }

    fn store(&self) -> FileStore {
        FileStore::new(self.data.path())
    }

    fn prompt_path(&self) -> PathBuf {
        self.data.path().join("scripts/p/prompt.md")
    }
}

fn combined(output: &Output) -> String {
    format!("{}{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr))
}

fn flat(output: &Output) -> String {
    combined(output).split_whitespace().collect::<Vec<_>>().join(" ")
}

#[test]
fn test_add_no_interpolate() {
    let sandbox = Sandbox::new();
    let source = sandbox.source("p.prompt.md", "{{a}} {{b}}\n");
    let output = sandbox.run(&["add", source.to_str().unwrap(), "--no-interpolate", "--no-input"]);
    assert_eq!(output.status.code(), Some(0), "{}", combined(&output));
    let view = sandbox.show_json("p");
    assert_eq!(view["interpolate"], false);
    assert_eq!(view["fields"], json!([]));
    assert!(combined(&output).to_lowercase().contains("insertion is off"), "{}", combined(&output));
}

#[test]
fn test_add_no_interpolate_refused_off_the_prompt_lanes() {
    let sandbox = Sandbox::new();
    let py = sandbox.source("s.py", "print(1)\n");
    let path = sandbox.run(&["add", py.to_str().unwrap(), "--no-interpolate", "--no-input"]);
    assert_eq!(path.status.code(), Some(2), "{}", combined(&path));
    assert!(combined(&path).contains("--no-interpolate only applies to prompt entries"), "{}", combined(&path));
    let command = sandbox.run(&["add", "--cmd", "echo hi", "-n", "c", "--no-interpolate"]);
    assert_eq!(command.status.code(), Some(2), "{}", combined(&command));
}

#[test]
fn test_add_no_interpolate_refused_up_front_on_non_prompt_path_lane() {
    for extra in [vec!["--exe"], vec!["--kind", "shell"]] {
        let sandbox = Sandbox::new();
        let source = sandbox.source("tool", "#!/bin/sh\necho hi\n");
        let mut args = vec!["add", source.to_str().unwrap()];
        args.extend(extra.iter().copied());
        args.extend(["--no-interpolate", "-n", "t", "--no-input"]);
        let output = sandbox.run(&args);
        assert_eq!(output.status.code(), Some(2), "extra={extra:?}\n{}", combined(&output));
        assert!(combined(&output).contains("--no-interpolate only applies to prompt entries"), "{}", combined(&output));
        assert!(!sandbox.data.path().join("scripts/t").exists());
    }
}

#[test]
fn test_add_no_interpolate_through_stdin_lane() {
    let sandbox = Sandbox::new();
    let output = sandbox
        .command()
        .args(["add", "-", "--prompt", "-n", "clip", "--no-interpolate"])
        .write_stdin("Body {{x}}\n")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0), "{}", combined(&output));
    let view = sandbox.show_json("clip");
    assert_eq!(view["interpolate"], false);
    assert_eq!(view["fields"], json!([]));
}

#[test]
fn test_params_interpolate_off_and_on() {
    let sandbox = Sandbox::new();
    sandbox.add_prompt("p", "{{a}}\n", false);
    let off = sandbox.run(&["params", "p", "--no-interpolate"]);
    assert_eq!(off.status.code(), Some(0), "{}", combined(&off));
    let human = sandbox.run(&["params", "p"]);
    assert!(combined(&human).contains("Variable insertion is off"), "{}", combined(&human));
    let payload: Value = serde_json::from_slice(&sandbox.run(&["params", "p", "--json"]).stdout).unwrap();
    assert_eq!(payload["interpolate"], false);
    assert_eq!(payload["unmanaged"], json!([]));
    let on = sandbox.run(&["params", "p", "--interpolate"]);
    assert_eq!(on.status.code(), Some(0), "{}", combined(&on));
    let view = sandbox.show_json("p");
    assert_eq!(view["interpolate"], true);
    assert_eq!(view["fields"].as_array().unwrap().iter().map(|field| field["key"].as_str().unwrap()).collect::<Vec<_>>(), ["a"]);
}

#[test]
fn test_params_interpolate_refused_on_non_prompt() {
    let sandbox = Sandbox::new();
    let added = sandbox.run(&["add", "--cmd", "echo {m}", "--name", "cmd"]);
    assert_eq!(added.status.code(), Some(0), "{}", combined(&added));
    let output = sandbox.run(&["params", "cmd", "--no-interpolate"]);
    assert_eq!(output.status.code(), Some(1), "{}", combined(&output));
    assert!(combined(&output).contains("--interpolate only applies to prompt entries"), "{}", combined(&output));
}

#[test]
fn test_params_unmanaged_listing_is_flood_capped_and_localizable() {
    for (extra, tail) in [(1_usize, "and 1 more candidate"), (7, "and 7 more candidates")] {
        let sandbox = Sandbox::new();
        let names = (0..FROZEN_LIST_PREVIEW_LIMIT + extra)
            .map(|index| format!("u{index}"))
            .collect::<Vec<_>>();
        sandbox.create_prompt_managed_a("{{a}}\n", true);
        let body = format!(
            "{{{{a}}}} {}\n",
            names.iter().map(|name| format!("{{{{{name}}}}}")).collect::<Vec<_>>().join(" ")
        );
        fs::write(sandbox.prompt_path(), body).unwrap();
        let output = sandbox.run(&["params", "p"]);
        assert_eq!(output.status.code(), Some(0), "{}", combined(&output));
        let shown = flat(&output);
        assert!(shown.contains(tail), "{shown}");
        assert!(shown.contains(&names[FROZEN_LIST_PREVIEW_LIMIT - 1]), "{shown}");
        assert!(!shown.contains(&names[FROZEN_LIST_PREVIEW_LIMIT]), "preview leaked hidden name: {shown}");
        let json_output = sandbox.run(&["params", "p", "--json"]);
        let payload: Value = serde_json::from_slice(&json_output.stdout).unwrap();
        assert_eq!(payload["unmanaged"], json!(names), "machine contract must keep the complete unmanaged set");
    }
}

#[test]
fn test_params_unmanaged_tail_passes_through_the_i18n_boundary() {
    let sandbox = Sandbox::new();
    let names = (0..FROZEN_LIST_PREVIEW_LIMIT + 3)
        .map(|index| format!("u{index}"))
        .collect::<Vec<_>>();
    sandbox.create_prompt_managed_a("{{a}}\n", true);
    fs::write(
        sandbox.prompt_path(),
        format!("{{{{a}}}} {}", names.iter().map(|name| format!("{{{{{name}}}}}")).collect::<Vec<_>>().join(" ")),
    ).unwrap();
    let output = sandbox.command_lang("x-pseudo").args(["params", "p"]).output().unwrap();
    assert_eq!(output.status.code(), Some(0), "{}", combined(&output));
    let shown = combined(&output);
    assert!(shown.contains('⟦'), "pseudo locale was not used: {shown}");
    assert!(shown.contains("möré"), "unmanaged tail bypassed i18n: {shown}");
    assert!(!shown.contains("and 3 more"), "hard-coded English tail leaked: {shown}");
}

#[test]
fn test_show_reports_the_interpolate_switch() {
    let sandbox = Sandbox::new();
    sandbox.add_prompt("p", "{{a}}\n", false);
    let off = sandbox.run(&["params", "p", "--no-interpolate"]);
    assert_eq!(off.status.code(), Some(0), "{}", combined(&off));
    assert_eq!(sandbox.show_json("p")["interpolate"], false);
    let human = sandbox.run(&["show", "p"]);
    assert!(combined(&human).contains("Variable insertion: off"), "{}", combined(&human));
}

#[test]
fn test_doctor_skips_drift_for_an_insertion_off_prompt() {
    let sandbox = Sandbox::new();
    let entry = sandbox.create_prompt_managed_a("{{a}}\n", false);
    fs::write(sandbox.prompt_path(), "gone\n").unwrap();
    assert_eq!(entry.meta.kind.as_str(), "prompt");
    let output = sandbox.run(&["doctor", "--json"]);
    assert_eq!(output.status.code(), Some(0), "{}", combined(&output));
    let payload: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(payload["drift"], json!([]));
}

#[test]
fn test_run_insertion_off_prompt_rejects_set_and_sends_verbatim() {
    let sandbox = Sandbox::new();
    let probe = compile_probe(sandbox.tools.path());
    let add_runner = sandbox.run(&[
        "runner", "add", "probe", "--force", "--", probe.to_str().unwrap(), "{{prompt}}",
    ]);
    assert_eq!(add_runner.status.code(), Some(0), "{}", combined(&add_runner));
    let source = sandbox.source("p.prompt.md", "Keep {{a}} literal.\n");
    let added = sandbox.run(&[
        "add", source.to_str().unwrap(), "--name", "p", "--runner", "probe", "--no-interpolate", "--no-input",
    ]);
    assert_eq!(added.status.code(), Some(0), "{}", combined(&added));

    let bad_set = sandbox.run(&["run", "p", "--set", "a=1", "--no-input"]);
    assert_eq!(bad_set.status.code(), Some(2), "{}", combined(&bad_set));

    let capture = sandbox.tools.path().join("prompt-capture.txt");
    let run = sandbox.command().env("SKIT_PROMPT_CAPTURE", &capture).args(["run", "p", "--no-input"]).output().unwrap();
    assert_eq!(run.status.code(), Some(0), "{}", combined(&run));
    assert_eq!(fs::read_to_string(capture).unwrap(), "Keep {{a}} literal.\n");
}

fn compile_probe(root: &Path) -> PathBuf {
    let source = root.join("prompt_capture.rs");
    fs::write(
        &source,
        r#"
use std::{env, fs};
fn main() {
    let body = env::args_os().nth(1).expect("prompt body");
    fs::write(env::var_os("SKIT_PROMPT_CAPTURE").expect("capture"), body.to_string_lossy().as_bytes()).unwrap();
}
"#,
    ).unwrap();
    let executable = root.join(if cfg!(windows) { "prompt-capture.exe" } else { "prompt-capture" });
    let status = ProcessCommand::new("rustc").arg(source).arg("-o").arg(&executable).status().unwrap();
    assert!(status.success(), "failed to compile prompt capture endpoint");
    executable
}

#[test]
fn rust_additive_frozen_limits_are_explicit_not_implementation_derived() {
    assert_eq!(FROZEN_AUTO_MANAGE_LIMIT, 30);
    assert_eq!(FROZEN_LIST_PREVIEW_LIMIT, 20);
}
