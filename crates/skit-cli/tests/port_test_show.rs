//! Exact behavioral ports of Python v0.4 `tests/test_show.py`.
//!
//! `show --json` is an agent-facing machine contract. The stable payload and field shapes are
//! compared as exact key sets, not subset smoke tests. Human secret tests assert both absence of
//! plaintext and the required masking/source cues.

use std::{collections::BTreeSet, fs, path::PathBuf};

use assert_cmd::Command;
use serde_json::{Value as JsonValue, json};
use tempfile::TempDir;

const PAYLOAD_KEYS: &[&str] = &[
    "name",
    "slug",
    "kind",
    "mode",
    "description",
    "source",
    "workdir",
    "interpreter",
    "missing",
    "dependencies",
    "requires_python",
    "needs",
    "template",
    "param_source",
    "param_origin",
    "degraded_reason",
    "drift",
    "fields",
    "presets",
    "last_run_at",
    "last_exit",
];

const FIELD_KEYS: &[&str] = &[
    "key",
    "label",
    "type",
    "source",
    "required",
    "secret",
    "multiple",
    "repeat",
    "degraded",
    "choices",
    "default",
    "help",
    "flag",
    "action",
    "env_source",
    "delivers_empty",
];

const ARGPARSE: &str = concat!(
    "import argparse\n",
    "ap = argparse.ArgumentParser()\n",
    "ap.add_argument('src')\n",
    "ap.add_argument('--width', type=int, default=800, help='target width')\n",
    "ap.add_argument('--fmt', choices=['png', 'jpg'], default='png')\n",
    "ap.add_argument('--force', action='store_true')\n",
    "ap.parse_args()\n",
);

const CLICK_MULTIPLE: &str = concat!(
    "import click\n",
    "@click.command()\n",
    "@click.option('--tag', multiple=True)\n",
    "def main(tag):\n",
    "    pass\n",
);

const DEGRADED: &str = concat!(
    "import argparse\n",
    "ap = argparse.ArgumentParser()\n",
    "sub = ap.add_subparsers()\n",
    "p = sub.add_parser('x')\n",
    "p.add_argument('--y')\n",
    "ap.parse_args()\n",
);

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

    fn entry_dir(&self, slug: &str) -> PathBuf {
        self.data.path().join("scripts").join(slug)
    }

    fn register(&self, slug: &str) {
        fs::write(
            self.data.path().join("registry.toml"),
            format!("[entries.{slug}]\n"),
        )
        .unwrap();
    }

    fn write_python_copy(
        &self,
        slug: &str,
        name: &str,
        source_text: &str,
        original: &std::path::Path,
        extra_meta: &str,
    ) -> PathBuf {
        let dir = self.entry_dir(slug);
        fs::create_dir_all(&dir).unwrap();
        let script = dir.join("script.py");
        fs::write(&script, source_text).unwrap();
        fs::write(
            dir.join("meta.toml"),
            format!(
                concat!(
                    "schema = 1\n",
                    "name = {name:?}\n",
                    "kind = \"python\"\n",
                    "mode = \"copy\"\n",
                    "source = {source:?}\n",
                    "source_hash = \"\"\n",
                    "added_at = \"2026-08-10T00:00:00+00:00\"\n",
                    "id = \"5123456789abcdef0123456789abcdef\"\n",
                    "workdir = \"invoke\"\n",
                    "description = \"\"\n",
                    "{extra_meta}\n",
                ),
                name = name,
                source = original.display().to_string(),
                extra_meta = extra_meta,
            ),
        )
        .unwrap();
        self.register(slug);
        script
    }

    fn write_python_reference(
        &self,
        slug: &str,
        name: &str,
        original: &std::path::Path,
        extra_meta: &str,
    ) {
        let dir = self.entry_dir(slug);
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("meta.toml"),
            format!(
                concat!(
                    "schema = 1\n",
                    "name = {name:?}\n",
                    "kind = \"python\"\n",
                    "mode = \"reference\"\n",
                    "source = {source:?}\n",
                    "source_hash = \"\"\n",
                    "added_at = \"2026-08-10T00:00:00+00:00\"\n",
                    "id = \"6123456789abcdef0123456789abcdef\"\n",
                    "workdir = \"invoke\"\n",
                    "description = \"\"\n",
                    "{extra_meta}\n",
                ),
                name = name,
                source = original.display().to_string(),
                extra_meta = extra_meta,
            ),
        )
        .unwrap();
        self.register(slug);
    }

    fn write_exe_reference(&self, slug: &str, source: &std::path::Path) {
        let dir = self.entry_dir(slug);
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("meta.toml"),
            format!(
                concat!(
                    "schema = 1\n",
                    "name = {slug:?}\n",
                    "kind = \"exe\"\n",
                    "mode = \"reference\"\n",
                    "source = {source:?}\n",
                    "source_hash = \"\"\n",
                    "added_at = \"2026-08-10T00:00:00+00:00\"\n",
                    "id = \"7123456789abcdef0123456789abcdef\"\n",
                    "workdir = \"invoke\"\n",
                    "description = \"\"\n",
                ),
                slug = slug,
                source = source.display().to_string(),
            ),
        )
        .unwrap();
        self.register(slug);
    }

    fn add_command(&self, name: &str, template: &str) {
        let output = self
            .command()
            .args(["add", "--cmd", template, "--name", name, "--no-input"])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "stdout={}\nstderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn show_output(&self, name: &str, json_mode: bool) -> std::process::Output {
        let mut command = self.command();
        command.args(["show", name]);
        if json_mode {
            command.arg("--json");
        }
        command.output().unwrap()
    }

    fn show_json(&self, name: &str) -> JsonValue {
        let output = self.show_output(name, true);
        assert!(
            output.status.success(),
            "stdout={}\nstderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice(&output.stdout).unwrap()
    }

    fn show_human(&self, name: &str) -> String {
        let output = self.show_output(name, false);
        assert!(
            output.status.success(),
            "stdout={}\nstderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).unwrap()
    }

    fn seed_state(&self, slug: &str, body: &str) {
        let path = self.state.path().join("values").join(format!("{slug}.toml"));
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, body).unwrap();
    }
}

fn keys(value: &JsonValue) -> BTreeSet<String> {
    value
        .as_object()
        .unwrap()
        .keys()
        .map(ToOwned::to_owned)
        .collect()
}

fn set(items: &[&str]) -> BTreeSet<String> {
    items.iter().map(|item| (*item).to_owned()).collect()
}

fn fields_by_key(payload: &JsonValue) -> Vec<(&str, &JsonValue)> {
    payload["fields"]
        .as_array()
        .unwrap()
        .iter()
        .map(|field| (field["key"].as_str().unwrap(), field))
        .collect()
}

fn managed_api_source() -> &'static str {
    concat!(
        "# /// script\n",
        "# [tool.skit]\n",
        "# schema = 1\n",
        "#\n",
        "# [[tool.skit.params]]\n",
        "# name = \"KEY\"\n",
        "# kind = \"const\"\n",
        "# type = \"str\"\n",
        "# default = \"abc\"\n",
        "# secret = true\n",
        "# env_source = \"API_KEY\"\n",
        "#\n",
        "# [[tool.skit.params]]\n",
        "# name = \"CITY\"\n",
        "# kind = \"const\"\n",
        "# type = \"str\"\n",
        "# default = \"Taipei\"\n",
        "# prompt = \"Which city?\"\n",
        "# ///\n",
        "KEY = \"abc\"\n",
        "CITY = \"Taipei\"\n",
        "print(KEY, CITY)\n",
    )
}

fn managed_city_source() -> &'static str {
    concat!(
        "# /// script\n",
        "# [tool.skit]\n",
        "# schema = 1\n",
        "#\n",
        "# [[tool.skit.params]]\n",
        "# name = \"CITY\"\n",
        "# kind = \"const\"\n",
        "# type = \"str\"\n",
        "# ///\n",
        "CITY = \"x\"\n",
        "print(CITY)\n",
    )
}

#[test]
fn test_show_json_argparse_full_schema() {
    let sandbox = Sandbox::new();
    let original = sandbox.home.path().join("job.py");
    fs::write(&original, ARGPARSE).unwrap();
    sandbox.write_python_copy("resize", "resize", ARGPARSE, &original, "");
    let payload = sandbox.show_json("resize");

    assert_eq!(payload["name"], "resize");
    assert_eq!(payload["slug"], "resize");
    assert_eq!(payload["kind"], "python");
    assert_eq!(payload["mode"], "copy");
    assert_eq!(payload["source"], original.display().to_string());
    assert_eq!(payload["workdir"], "invoke");
    assert_eq!(payload["missing"], false);
    assert!(payload["template"].is_null());
    assert_eq!(payload["param_source"], "argparse");
    assert_eq!(payload["param_origin"], "reader");
    assert_eq!(payload["degraded_reason"], "");
    assert_eq!(payload["drift"], false);
    assert_eq!(payload["presets"], json!([]));
    assert!(payload["last_run_at"].is_null());
    assert!(payload["last_exit"].is_null());

    let fields = fields_by_key(&payload);
    assert_eq!(
        fields.iter().map(|(name, _)| *name).collect::<Vec<_>>(),
        ["src", "width", "fmt", "force"]
    );
    assert_eq!(
        fields[0].1,
        &json!({
            "key": "src",
            "label": "src",
            "type": "str",
            "source": "flag",
            "required": true,
            "secret": false,
            "multiple": false,
            "repeat": false,
            "degraded": false,
            "choices": [],
            "default": null,
            "help": "",
            "flag": "",
            "action": "",
            "env_source": "",
            "delivers_empty": false,
        })
    );
    let width = fields.iter().find(|(name, _)| *name == "width").unwrap().1;
    assert_eq!(width["type"], "int");
    assert_eq!(width["default"], "800");
    assert_eq!(width["help"], "target width");
    assert_eq!(width["flag"], "--width");
    assert_eq!(width["required"], false);
    let fmt = fields.iter().find(|(name, _)| *name == "fmt").unwrap().1;
    assert_eq!(fmt["type"], "choice");
    assert_eq!(fmt["choices"], json!(["png", "jpg"]));
    let force = fields.iter().find(|(name, _)| *name == "force").unwrap().1;
    assert_eq!(force["type"], "bool");
    assert_eq!(force["action"], "store_true");
    assert_eq!(force["default"], "false");
}

#[test]
fn test_show_json_stable_shape() {
    let sandbox = Sandbox::new();
    let original = sandbox.home.path().join("shape.py");
    fs::write(&original, ARGPARSE).unwrap();
    sandbox.write_python_copy("shape", "shape", ARGPARSE, &original, "");
    let payload = sandbox.show_json("shape");
    assert_eq!(keys(&payload), set(PAYLOAD_KEYS));
    for field in payload["fields"].as_array().unwrap() {
        assert_eq!(keys(field), set(FIELD_KEYS));
    }
}

#[test]
fn test_show_json_repeat_true_for_a_click_multiple_option() {
    let sandbox = Sandbox::new();
    let original = sandbox.home.path().join("tagger.py");
    fs::write(&original, CLICK_MULTIPLE).unwrap();
    sandbox.write_python_copy("tagger", "tagger", CLICK_MULTIPLE, &original, "");
    let payload = sandbox.show_json("tagger");
    let fields = fields_by_key(&payload);
    let tag = fields.iter().find(|(name, _)| *name == "tag").unwrap().1;
    assert_eq!(tag["multiple"], true);
    assert_eq!(tag["repeat"], true);
}

#[test]
fn test_show_json_inject_secret_and_state() {
    let sandbox = Sandbox::new();
    let original = sandbox.home.path().join("api.py");
    fs::write(&original, managed_api_source()).unwrap();
    sandbox.write_python_copy("api", "api", managed_api_source(), &original, "");
    sandbox.seed_state(
        "api",
        concat!(
            "[presets.fast]\n",
            "CITY = \"Tainan\"\n",
            "[last_run]\n",
            "at = \"2026-07-11T00:00:00+00:00\"\n",
            "exit = 3\n",
        ),
    );
    let payload = sandbox.show_json("api");
    assert_eq!(payload["param_source"], "inject");
    assert_eq!(payload["param_origin"], "managed");
    assert_eq!(payload["presets"], json!(["fast"]));
    assert_eq!(payload["last_run_at"], "2026-07-11T00:00:00+00:00");
    assert_eq!(payload["last_exit"], 3);
    let fields = fields_by_key(&payload);
    let key = fields.iter().find(|(name, _)| *name == "KEY").unwrap().1;
    assert_eq!(key["source"], "inject");
    assert_eq!(key["secret"], true);
    assert_eq!(key["env_source"], "API_KEY");
    assert_eq!(key["default"], "abc");
    let city = fields.iter().find(|(name, _)| *name == "CITY").unwrap().1;
    assert_eq!(city["label"], "Which city?");
}

#[test]
fn test_show_json_command_kind() {
    let sandbox = Sandbox::new();
    sandbox.add_command("deploy", "echo {target} {level}");
    let payload = sandbox.show_json("deploy");
    assert_eq!(payload["kind"], "command");
    assert_eq!(payload["template"], "echo {target} {level}");
    assert_eq!(payload["param_source"], "command");
    assert_eq!(payload["param_origin"], "command");
    let fields = fields_by_key(&payload);
    assert_eq!(
        fields.iter().map(|(name, _)| *name).collect::<BTreeSet<_>>(),
        BTreeSet::from(["target", "level"])
    );
    let target = fields.iter().find(|(name, _)| *name == "target").unwrap().1;
    assert_eq!(target["source"], "placeholder");
    assert_eq!(target["required"], true);
}

#[test]
fn test_show_json_deps_and_missing_reference() {
    let sandbox = Sandbox::new();
    let original = sandbox.home.path().join("job.py");
    fs::write(&original, "print(1)\n").unwrap();
    sandbox.write_python_reference(
        "ref",
        "ref",
        &original,
        "dependencies = [\"requests>=2,<3\"]\nrequires_python = \">=3.12\"",
    );
    fs::remove_file(&original).unwrap();
    let payload = sandbox.show_json("ref");
    assert_eq!(payload["mode"], "reference");
    assert_eq!(payload["missing"], true);
    assert_eq!(payload["dependencies"], json!(["requests>=2,<3"]));
    assert_eq!(payload["requires_python"], ">=3.12");
}

#[test]
fn test_show_json_degraded_parser() {
    let sandbox = Sandbox::new();
    let original = sandbox.home.path().join("multi.py");
    fs::write(&original, DEGRADED).unwrap();
    sandbox.write_python_copy("multi", "multi", DEGRADED, &original, "");
    let payload = sandbox.show_json("multi");
    assert_eq!(payload["param_source"], "argparse");
    assert_eq!(payload["degraded_reason"], "subparsers");
    assert_eq!(payload["fields"], json!([]));
}

#[test]
fn test_show_json_drift() {
    let sandbox = Sandbox::new();
    let original = sandbox.home.path().join("stale.py");
    fs::write(&original, managed_city_source()).unwrap();
    let script = sandbox.write_python_copy("stale", "stale", managed_city_source(), &original, "");
    let moved = fs::read_to_string(&script)
        .unwrap()
        .replace("CITY = \"x\"", "TOWN = \"x\"");
    fs::write(script, moved).unwrap();
    let payload = sandbox.show_json("stale");
    assert_eq!(payload["drift"], true);
}

#[test]
fn test_show_human_argparse_table() {
    let sandbox = Sandbox::new();
    let original = sandbox.home.path().join("resize.py");
    fs::write(&original, ARGPARSE).unwrap();
    sandbox.write_python_copy("resize", "resize", ARGPARSE, &original, "");
    let output = sandbox.show_human("resize");
    for expected in [
        "resize",
        "width",
        "target width",
        "png, jpg",
        "yes",
        "Source:",
        "Run it: skit run resize",
    ] {
        assert!(output.contains(expected), "missing {expected:?} in:\n{output}");
    }
}

#[test]
fn test_show_human_masks_secret_default_and_names_env_source() {
    let sandbox = Sandbox::new();
    let source = managed_api_source();
    let original = sandbox.home.path().join("api.py");
    fs::write(&original, source).unwrap();
    sandbox.write_python_copy("api", "api", source, &original, "");
    let output = sandbox.show_human("api");
    assert!(!output.contains("abc"), "secret default leaked:\n{output}");
    assert!(output.contains("•••"), "secret mask missing:\n{output}");
    assert!(output.contains("$API_KEY"), "env source missing:\n{output}");
}

#[test]
fn test_show_human_secret_without_env_source() {
    let sandbox = Sandbox::new();
    let source = concat!(
        "# /// script\n",
        "# [tool.skit]\n",
        "# schema = 1\n",
        "# [[tool.skit.params]]\n",
        "# name = \"TOKEN\"\n",
        "# kind = \"const\"\n",
        "# type = \"str\"\n",
        "# secret = true\n",
        "# ///\n",
        "TOKEN = \"t\"\n",
        "print(TOKEN)\n",
    );
    let original = sandbox.home.path().join("tok.py");
    fs::write(&original, source).unwrap();
    sandbox.write_python_copy("tok", "tok", source, &original, "");
    let output = sandbox.show_human("tok");
    assert_eq!(output.matches("yes").count(), 1, "{output}");
    assert!(!output.contains('←'), "unexpected env-source arrow:\n{output}");
}

#[test]
fn test_show_human_command_kind() {
    let sandbox = Sandbox::new();
    sandbox.add_command("c1", "echo {a}");
    let output = sandbox.show_human("c1");
    assert!(output.contains("Command template: echo {a}"), "{output}");
    assert!(!output.contains("Source:"), "{output}");
    assert!(output.contains('a'), "{output}");
}

#[test]
fn test_show_human_no_fields_exe() {
    let sandbox = Sandbox::new();
    let executable = std::env::current_exe().unwrap();
    sandbox.write_exe_reference("tool", &executable);
    let output = sandbox.show_human("tool");
    assert!(output.contains("No form fields"), "{output}");
}

#[test]
fn test_show_human_description_deps_presets_and_drift() {
    let sandbox = Sandbox::new();
    let original = sandbox.home.path().join("trip.py");
    fs::write(&original, managed_city_source()).unwrap();
    let script = sandbox.write_python_copy(
        "trip",
        "trip",
        managed_city_source(),
        &original,
        "description = \"plan a trip\"\ndependencies = [\"rich>=15\"]\nrequires_python = \">=3.12\"",
    );
    sandbox.seed_state("trip", "[presets.quick]\nCITY = \"Tainan\"\n");
    let moved = fs::read_to_string(&script)
        .unwrap()
        .replace("CITY = \"x\"", "TOWN = \"x\"");
    fs::write(script, moved).unwrap();
    let output = sandbox.show_human("trip");
    for expected in [
        "plan a trip",
        "rich>=15",
        ">=3.12",
        "Presets: quick",
        "drifted from the script",
    ] {
        assert!(output.contains(expected), "missing {expected:?} in:\n{output}");
    }
}

#[test]
fn test_show_human_degraded_parser_notice() {
    let sandbox = Sandbox::new();
    let original = sandbox.home.path().join("multi.py");
    fs::write(&original, DEGRADED).unwrap();
    sandbox.write_python_copy("multi", "multi", DEGRADED, &original, "");
    let output = sandbox.show_human("multi");
    assert!(
        output
            .lines()
            .any(|line| line == "skit could not model this script's own arguments; pass them after -- instead."),
        "exact degraded notice missing:\n{output}"
    );
}

#[test]
fn test_show_human_missing_marker() {
    let sandbox = Sandbox::new();
    let original = sandbox.home.path().join("gone.py");
    fs::write(&original, "print(1)\n").unwrap();
    sandbox.write_python_reference("gone", "gone", &original, "");
    fs::remove_file(original).unwrap();
    let output = sandbox.show_human("gone");
    assert!(output.contains("⚠ missing:"), "{output}");
}

#[test]
fn test_show_not_found_exits_1() {
    let sandbox = Sandbox::new();
    let output = sandbox.show_output("ghost", false);
    assert_eq!(output.status.code(), Some(1));
}
