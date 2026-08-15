use std::{fs, path::PathBuf, process::Output};

use assert_cmd::Command;
use serde_json::{Value, json};
use skit_application::{CreateEntry, EntryMutationRepository as _, EntryPayload, SourcePermissions};
use skit_domain::{EntryKind, EntrySettings, StorageMode, parameters::{ParamDecl, ParameterDelivery, synthesized_placeholder}};
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

    fn store(&self) -> FileStore {
        FileStore::new(self.data.path())
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

    fn run(&self, args: &[&str]) -> Output {
        self.command().args(args).output().unwrap()
    }

    fn json(&self, args: &[&str]) -> Value {
        let output = self.run(args);
        assert_eq!(output.status.code(), Some(0), "{}", combined(&output));
        serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
            panic!("stdout was not exactly one JSON document: {error}\n{}", combined(&output))
        })
    }

    fn create_prompt(
        &self,
        name: &str,
        body: &str,
        managed: &[&str],
        runner: &str,
        interpolate: bool,
    ) -> skit_domain::Entry {
        let source = self.home.path().join(format!("{name}.prompt.md"));
        fs::write(&source, body).unwrap();
        let parameters = managed
            .iter()
            .map(|name| synthesized_placeholder(name))
            .collect::<Vec<_>>();
        self.store()
            .create(CreateEntry {
                name: name.to_owned(),
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
                    params: managed.iter().map(|name| (*name).to_owned()).collect(),
                    parameters,
                    runner: runner.to_owned(),
                    interpolate,
                    ..EntrySettings::default()
                },
            })
            .unwrap()
    }

    fn prompt_path(&self, slug: &str) -> PathBuf {
        self.data.path().join("scripts").join(slug).join("prompt.md")
    }

    fn add_command(&self, name: &str, template: &str) {
        let output = self.run(&["add", "--cmd", template, "--name", name]);
        assert_eq!(output.status.code(), Some(0), "{}", combined(&output));
    }
}

fn combined(output: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

fn flat(output: &Output) -> String {
    combined(output).split_whitespace().collect::<Vec<_>>().join(" ")
}

#[test]
fn test_params_read_view_shows_unmanaged_and_gone() {
    let sandbox = Sandbox::new();
    let entry = sandbox.create_prompt("p", "{{a}} {{b}}\n", &["a"], "", true);
    fs::write(sandbox.prompt_path(entry.slug.as_str()), "{{b}} {{c}} only\n").unwrap();
    let output = sandbox.run(&["params", "p"]);
    assert_eq!(output.status.code(), Some(0), "{}", combined(&output));
    let shown = combined(&output);
    assert!(shown.contains("Prompt placeholders"), "{shown}");
    assert!(shown.contains("Detected but not yet managed: b, c"), "{shown}");
    assert!(shown.contains("No longer in the prompt") && shown.contains('a'), "{shown}");
}

#[test]
fn test_params_json_carries_runner_and_unmanaged() {
    let sandbox = Sandbox::new();
    sandbox.create_prompt("p", "{{a}} {{b}}\n", &["a"], "claude", true);
    let payload = sandbox.json(&["params", "p", "--json"]);
    assert_eq!(payload["placeholders"], json!(["a"]));
    assert_eq!(payload["unmanaged"], json!(["b"]));
    assert_eq!(payload["runner"], "claude");
}

#[test]
fn test_params_add_manages_a_body_placeholder() {
    let sandbox = Sandbox::new();
    sandbox.create_prompt("p", "{{a}} {{b}}\n", &["a"], "", true);
    let output = sandbox.run(&["params", "p", "--add", "b"]);
    assert_eq!(output.status.code(), Some(0), "{}", combined(&output));
    let payload = sandbox.json(&["params", "p", "--json"]);
    assert_eq!(payload["placeholders"], json!(["a", "b"]), "body order must win");
    let declared = payload["declared"].as_array().unwrap();
    assert_eq!(declared.len(), 1, "only newly declared b should have an explicit row: {payload}");
    assert_eq!(declared[0]["name"], "b");
    assert_eq!(declared[0]["delivery"], "placeholder");
}

#[test]
fn test_params_rm_unmanages_even_without_a_declared_row() {
    let sandbox = Sandbox::new();
    sandbox.create_prompt("p", "{{a}} {{b}}\n", &["a", "b"], "", true);
    let output = sandbox.run(&["params", "p", "--rm", "b"]);
    assert_eq!(output.status.code(), Some(0), "{}", combined(&output));
    let shown = combined(&output);
    let payload = sandbox.json(&["params", "p", "--json"]);
    assert_eq!(payload["placeholders"], json!(["a"]));
    assert!(!shown.contains("not-declared") && !shown.contains("isn't declared"), "{shown}");
}

#[test]
fn test_params_add_unknown_name_becomes_env_rider() {
    let sandbox = Sandbox::new();
    sandbox.create_prompt("p", "{{a}}\n", &["a"], "", true);
    let output = sandbox.run(&["params", "p", "--add", "EXTRA"]);
    assert_eq!(output.status.code(), Some(0), "{}", combined(&output));
    let payload = sandbox.json(&["params", "p", "--json"]);
    assert_eq!(payload["placeholders"], json!(["a"]));
    let declared = payload["declared"].as_array().unwrap();
    assert_eq!(declared.len(), 1, "{payload}");
    assert_eq!(declared[0]["name"], "EXTRA");
    assert_eq!(declared[0]["delivery"], "env");
}

#[test]
fn test_params_deliver_placeholder_is_allowed_on_prompts() {
    let sandbox = Sandbox::new();
    let mut row = ParamDecl::new("a");
    row.delivery = ParameterDelivery::Env;
    let source = sandbox.home.path().join("p.prompt.md");
    fs::write(&source, "{{a}}\n").unwrap();
    sandbox.store().create(CreateEntry {
        name: "p".to_owned(),
        kind: EntryKind::parse("prompt").unwrap(),
        mode: StorageMode::Copy,
        source: source.display().to_string(),
        workdir: "invoke".to_owned(),
        description: String::new(),
        payload: Some(EntryPayload {
            bytes: b"{{a}}\n".to_vec(),
            stored_name: Some("prompt.md".to_owned()),
            permissions: SourcePermissions::default(),
        }),
        settings: EntrySettings {
            params: vec!["a".to_owned()],
            parameters: vec![row],
            ..EntrySettings::default()
        },
    }).unwrap();
    let output = sandbox.run(&["params", "p", "--deliver", "a=placeholder"]);
    assert_eq!(output.status.code(), Some(0), "{}", combined(&output));
    let payload = sandbox.json(&["params", "p", "--json"]);
    assert_eq!(payload["declared"][0]["delivery"], "placeholder");
}

#[test]
fn test_params_runner_pin_and_clear() {
    let sandbox = Sandbox::new();
    sandbox.create_prompt("p", "Do {{a}}\n", &["a"], "", true);
    fs::create_dir_all(sandbox.state.path()).unwrap();
    fs::write(sandbox.state.path().join("prompt.toml"), "last_runner = \"opencode\"\n").unwrap();

    let pin = sandbox.run(&["params", "p", "--runner", "claude"]);
    assert_eq!(pin.status.code(), Some(0), "{}", combined(&pin));
    assert_eq!(sandbox.json(&["show", "p", "--json"])["runner"], "claude");
    assert_eq!(fs::read_to_string(sandbox.state.path().join("prompt.toml")).unwrap(), "last_runner = \"opencode\"\n");

    let clear = sandbox.run(&["params", "p", "--runner", ""]);
    assert_eq!(clear.status.code(), Some(0), "{}", combined(&clear));
    assert!(sandbox.json(&["show", "p", "--json"])["runner"].is_null());
    assert_eq!(fs::read_to_string(sandbox.state.path().join("prompt.toml")).unwrap(), "last_runner = \"opencode\"\n");
    assert!(combined(&clear).contains("asks at run time"), "{}", combined(&clear));
}

#[test]
fn test_params_runner_pin_with_json_emits_the_read_view() {
    let sandbox = Sandbox::new();
    sandbox.create_prompt("p", "Do {{a}}\n", &["a"], "", true);
    let payload = sandbox.json(&["params", "p", "--runner", "claude", "--json"]);
    assert_eq!(payload["runner"], "claude");
    assert_eq!(sandbox.json(&["show", "p", "--json"])["runner"], "claude");
}

#[test]
fn test_params_workdir_with_json_emits_the_read_view() {
    let sandbox = Sandbox::new();
    sandbox.create_prompt("p", "Do {{a}}\n", &["a"], "", true);
    let payload = sandbox.json(&["params", "p", "--workdir", "origin", "--json"]);
    assert!(payload.get("params").is_some(), "own-op JSON must be the full params read view: {payload}");
    assert!(payload["runner"].is_null());
    assert_eq!(sandbox.json(&["show", "p", "--json"])["workdir"], "origin");
}

#[test]
fn test_params_interpolate_with_json_emits_the_read_view() {
    let sandbox = Sandbox::new();
    sandbox.create_prompt("p", "Do {{a}}\n", &["a"], "", true);
    let payload = sandbox.json(&["params", "p", "--no-interpolate", "--json"]);
    assert_eq!(payload["interpolate"], false);
    assert_eq!(sandbox.json(&["show", "p", "--json"])["interpolate"], false);
}

#[test]
fn test_params_runner_pin_validates_the_name() {
    let sandbox = Sandbox::new();
    sandbox.create_prompt("p", "Do {{a}}\n", &["a"], "", true);
    let output = sandbox.run(&["params", "p", "--runner", "ghost"]);
    assert_eq!(output.status.code(), Some(1), "{}", combined(&output));
    assert!(combined(&output).contains("isn't configured"), "{}", combined(&output));
    assert!(sandbox.json(&["show", "p", "--json"])["runner"].is_null());
}

#[test]
fn test_params_runner_pin_refused_on_non_prompt() {
    let sandbox = Sandbox::new();
    sandbox.add_command("cmd", "echo {m}");
    let output = sandbox.run(&["params", "cmd", "--runner", "claude"]);
    assert_eq!(output.status.code(), Some(1), "{}", combined(&output));
    assert!(combined(&output).contains("--runner only applies to prompt entries"), "{}", combined(&output));
}

#[test]
fn test_show_json_prompt_additions() {
    let sandbox = Sandbox::new();
    sandbox.create_prompt("p", "Do {{a}}\n", &["a"], "claude", true);
    let payload = sandbox.json(&["show", "p", "--json"]);
    assert_eq!(payload["kind"], "prompt");
    assert_eq!(payload["runner"], "claude");
    assert!(payload["runners_available"].as_array().unwrap().iter().any(|name| name == "claude"));
    assert_eq!(payload["workdir"], "invoke");
    assert_eq!(payload["fields"].as_array().unwrap().iter().map(|field| field["key"].as_str().unwrap()).collect::<Vec<_>>(), ["a"]);
    assert_eq!(payload["fields"].as_array().unwrap().iter().map(|field| field["source"].as_str().unwrap()).collect::<Vec<_>>(), ["placeholder"]);
}

#[test]
fn test_show_json_non_prompt_has_no_runner_keys() {
    let sandbox = Sandbox::new();
    sandbox.add_command("cmd", "echo {m}");
    let payload = sandbox.json(&["show", "cmd", "--json"]);
    assert!(payload.get("runner").is_none(), "{payload}");
    assert!(payload.get("runners_available").is_none(), "{payload}");
}

#[test]
fn test_show_human_prints_the_runner_line() {
    let sandbox = Sandbox::new();
    sandbox.create_prompt("p", "Do {{a}}\n", &["a"], "claude", true);
    let pinned = sandbox.run(&["show", "p"]);
    assert_eq!(pinned.status.code(), Some(0), "{}", combined(&pinned));
    assert!(combined(&pinned).contains("Runner: claude"), "{}", combined(&pinned));
    let cleared = sandbox.run(&["params", "p", "--runner", ""]);
    assert_eq!(cleared.status.code(), Some(0), "{}", combined(&cleared));
    let unpinned = sandbox.run(&["show", "p"]);
    assert!(combined(&unpinned).contains("asks at run time"), "{}", combined(&unpinned));
}

#[test]
fn test_show_human_no_fields_names_prompt_and_command_receivers() {
    let sandbox = Sandbox::new();
    sandbox.create_prompt("plain", "No fields\n", &[], "", true);
    let prompt = sandbox.run(&["show", "plain"]);
    assert_eq!(prompt.status.code(), Some(0), "{}", combined(&prompt));
    let prompt_text = combined(&prompt);
    assert!(prompt_text.contains("arguments after -- go to the selected agent"), "{prompt_text}");
    assert!(!prompt_text.contains("pass straight through to the script"), "{prompt_text}");
    sandbox.add_command("cmd", "echo ready");
    let command = sandbox.run(&["show", "cmd"]);
    assert!(combined(&command).contains("arguments after -- are appended to the command"), "{}", combined(&command));
}

#[test]
fn test_doctor_reports_prompt_drift_and_bad_runner_rows() {
    let sandbox = Sandbox::new();
    let entry = sandbox.create_prompt("p", "{{a}}\n", &["a"], "", true);
    fs::write(sandbox.prompt_path(entry.slug.as_str()), "no holes\n").unwrap();
    fs::create_dir_all(sandbox.config.path()).unwrap();
    fs::write(
        sandbox.config.path().join("config.toml"),
        "[prompt]\nrunners_seeded = true\nrunners = [{ name = \"broken\", argv = [\"x\"] }]\n",
    ).unwrap();
    let payload = sandbox.json(&["doctor", "--json"]);
    assert!(payload["drift"].as_array().unwrap().iter().any(|slug| slug == "p"), "{payload}");
    assert_eq!(payload["runner_rows_invalid"], json!(["broken"]));
    let human = sandbox.run(&["doctor"]);
    let shown = flat(&human);
    assert!(shown.contains("broken"), "{shown}");
    assert!(shown.contains("Inspect and repair with: skit runner list --all"), "{shown}");
}

#[test]
fn test_doctor_healthy_prompt_reports_no_drift() {
    let sandbox = Sandbox::new();
    sandbox.create_prompt("p", "{{a}}\n", &["a"], "", true);
    let payload = sandbox.json(&["doctor", "--json"]);
    assert_eq!(payload["drift"], json!([]));
    assert_eq!(payload["runner_rows_invalid"], json!([]));
}

#[test]
fn test_doctor_skips_a_prompt_whose_body_is_gone() {
    let sandbox = Sandbox::new();
    let entry = sandbox.create_prompt("p", "{{a}}\n", &["a"], "", true);
    fs::remove_file(sandbox.prompt_path(entry.slug.as_str())).unwrap();
    let payload = sandbox.json(&["doctor", "--json"]);
    assert_eq!(payload["drift"], json!([]));
    assert!(payload["missing"].as_array().unwrap().iter().any(|slug| slug == "p"), "{payload}");
}
