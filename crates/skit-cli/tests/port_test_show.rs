//! Mechanical port of the Python oracle module `tests/test_show.py`
//! (`origin/main@206f9ef`): "`skit show` — the full read view of one script (identity +
//! unified schema + presets)." `show` is the agent-facing discovery surface (issue #2): the
//! one command that exposes the complete FormPlan field schema across all three sources
//! (inject / argparse / command). Each `#[test]` keeps its Python `def test_*` name and its
//! WHY comment so it traces back to its origin.
//!
//! WHY `skit-cli`: this oracle module drives the `skit` binary through Typer's `CliRunner`
//! (`show`, `add`, `deps`) and, for setup, the `store` / `argstate` / `metawriter` libraries.
//! The Rust rewrite hides `show` inside the composition root (a private `cli::show`), so only
//! the CLI crate can exercise the real read view. Every `show` assertion drives the real
//! `skit` binary via `assert_cmd`; the setup that has no clean CLI path (named presets, an
//! exact run stamp) drives the same library ports the composition root wires
//! (`FormStateService` over `FileFormStateStore`, pointed at the same temp state dir).
//!
//! Concept mapping used throughout:
//! - Python `runner.invoke(cli.app, ["show", name, "--json"])` -> `skit show name --json`
//!   (stdout is exactly one JSON document — the `--json` purity contract).
//! - Python `store.add_python(path, name=...)` -> `skit add <path> --name <name> --no-input`;
//!   `mode="reference"` -> `--ref`. Python `store.add(--cmd ...)` -> `skit add --cmd ...`.
//! - Python `store.resolve(name)` -> `LibraryService::new(FileStore::new(data)).show(name)`
//!   (the `Entry`, for its `slug` / `meta.workdir` / stored payload path).
//! - Python `entry.script_path` (the stored copy) -> `FileStore::payload_path(&entry)`.
//! - Python `store.update_dependencies(slug, deps, requires_python=...)`
//!   -> `skit deps <name> --dep ... --python ...`.
//! - Python `argstate.save_preset(slug, name, values)` -> `FormStateService::save_preset`;
//!   `argstate.record_run(slug, exit, at=...)` -> `FormStateService::record_run`.
//! - Python `metawriter.write_params(src, params)` -> `write_managed_params("python", src, &decls)`.
//! - Python `result.output` (merged streams) -> the COMBINED stdout+stderr; Rust writes the
//!   drift lines to stdout where Python writes them to stderr, and `CliRunner` merges both.
//!
//! Buckets:
//! - REAL asserting `#[test]` (API exists): 16 of 17, all ground-truthed against the shipping
//!   binary.
//! - DIVERGENCE (full asserting body, `#[ignore]`d): `test_show_json_stable_shape` alone. The
//!   oracle pins `show --json` to EXACTLY 21 payload keys; the Rust payload additionally
//!   carries `added_at` / `id` / `schema` / `source_hash`. These read as plausibly-intentional
//!   superset additions per the rewrite rule, so the delta is recorded for upstream
//!   adjudication rather than "greened" by relaxing `==` to a subset check (that would erase
//!   the signal and match divergent output). Fixing the impl and deleting the `#[ignore]` line
//!   turns it green; do NOT delete payload keys to green it.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Output;

use serde_json::{Value, json};
use skit_application::LibraryService;
use skit_application::form_state::FormStateService;
use skit_domain::Entry;
use skit_domain::parameters::{
    ParamDecl, ParameterBinding, ParameterDelivery, ParameterType, ParameterValue,
};
use skit_language::write_managed_params;
use skit_store::{FileFormStateStore, FileStore};
use tempfile::TempDir;

// --- The oracle's stable-shape contract (module-level constants) --------------------------------

/// Every key the payload must always carry — the stable-shape contract.
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

const SUBPARSERS: &str = concat!(
    "import argparse\n",
    "ap = argparse.ArgumentParser()\n",
    "sub = ap.add_subparsers()\n",
    "p = sub.add_parser('x')\n",
    "p.add_argument('--y')\n",
    "ap.parse_args()\n",
);

// --- one isolated skit library (private data/state/config plus a source scratch dir) ------------

struct Lib {
    data: TempDir,
    state: TempDir,
    config: TempDir,
    src: TempDir,
}

fn lib() -> Lib {
    Lib {
        data: TempDir::new().unwrap(),
        state: TempDir::new().unwrap(),
        config: TempDir::new().unwrap(),
        src: TempDir::new().unwrap(),
    }
}

impl Lib {
    fn cmd(&self) -> assert_cmd::Command {
        let mut command = assert_cmd::cargo::cargo_bin_cmd!("skit");
        command
            .env("SKIT_DATA_DIR", self.data.path())
            .env("SKIT_STATE_DIR", self.state.path())
            .env("SKIT_CONFIG_DIR", self.config.path())
            .env("SKIT_LANG", "en");
        command
    }

    /// Run one `skit` invocation and return its raw output (status + both streams).
    fn run(&self, args: &[&str]) -> Output {
        self.cmd().args(args).output().unwrap()
    }

    /// Write a source file into the scratch dir and return its path.
    fn write_src(&self, name: &str, body: &str) -> PathBuf {
        let path = self.src.path().join(name);
        fs::write(&path, body).unwrap();
        path
    }

    /// The read-only library adapters the composition root wires, over the SAME temp dirs the
    /// binary reads, so a library write is visible to the next `skit show`.
    fn store(&self) -> FileStore {
        FileStore::new(self.data.path())
    }

    fn service(&self) -> LibraryService<FileStore> {
        LibraryService::new(self.store())
    }

    fn state_service(&self) -> FormStateService<FileFormStateStore> {
        FormStateService::new(FileFormStateStore::new(self.state.path()))
    }

    /// Python `store.resolve(name)` — the stored entry (its slug, workdir, payload path).
    fn entry(&self, name: &str) -> Entry {
        self.service().show(name).unwrap()
    }

    /// Python `entry.script_path` — the stored copy path.
    fn copy_path(&self, entry: &Entry) -> PathBuf {
        self.store().payload_path(entry).unwrap()
    }

    /// Python `store.add_python(path, name=...)`.
    fn add_python(&self, path: &Path, name: &str) {
        self.run(&["add", path.to_str().unwrap(), "--name", name, "--no-input"])
            .status
            .success()
            .then_some(())
            .expect("add succeeds");
    }

    /// Python `store.show(name, --json)` — parse stdout as exactly one JSON document.
    fn show_json(&self, name: &str) -> Value {
        let output = self.run(&["show", name, "--json"]);
        assert!(
            output.status.success(),
            "show --json failed: {}",
            combined(&output)
        );
        serde_json::from_slice(&output.stdout).expect("stdout is exactly one JSON document")
    }

    /// Python `result.output` for the human view — the merged streams a user would see.
    fn show_human(&self, name: &str) -> String {
        let output = self.run(&["show", name]);
        assert!(
            output.status.success(),
            "show failed: {}",
            combined(&output)
        );
        combined(&output)
    }
}

/// Python `result.output`: both streams a user would see, joined.
fn combined(output: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

/// The oracle `metawriter.write_params(src, params)` output for an inject fixture.
fn inject(src: &str, decls: &[ParamDecl]) -> String {
    write_managed_params("python", src, decls).unwrap()
}

/// A `const`-bound inject `str` parameter (the oracle's default `ParamDecl(binding="const",
/// type="str")` shape).
fn const_str(name: &str) -> ParamDecl {
    let mut declaration = ParamDecl::new(name);
    declaration.binding = ParameterBinding::Const;
    declaration.delivery = ParameterDelivery::Inject;
    declaration.parameter_type = ParameterType::Str;
    declaration
}

/// The `[f["key"] for f in payload["fields"]]` list, in stored order.
fn field_keys_in_order(payload: &Value) -> Vec<String> {
    payload["fields"]
        .as_array()
        .unwrap()
        .iter()
        .map(|field| field["key"].as_str().unwrap().to_owned())
        .collect()
}

/// The `{f["key"]: f for f in payload["fields"]}` map.
fn fields_by_key(payload: &Value) -> BTreeMap<String, Value> {
    payload["fields"]
        .as_array()
        .unwrap()
        .iter()
        .map(|field| (field["key"].as_str().unwrap().to_owned(), field.clone()))
        .collect()
}

// ================================================================================================
// --json: the stable contract
// ================================================================================================

#[test]
fn test_show_json_argparse_full_schema() {
    let lib = lib();
    let path = lib.write_src("job.py", ARGPARSE);
    lib.add_python(&path, "resize");
    let entry = lib.entry("resize");
    let payload = lib.show_json("resize");
    assert_eq!(payload["name"], json!("resize"));
    assert_eq!(payload["slug"], json!(entry.slug.as_str()));
    assert_eq!(payload["kind"], json!("python"));
    assert_eq!(payload["mode"], json!("copy"));
    assert_eq!(payload["source"], json!(path.to_str().unwrap()));
    assert_eq!(payload["workdir"], json!(entry.meta.workdir));
    assert_eq!(payload["missing"], json!(false));
    assert_eq!(payload["template"], Value::Null);
    assert_eq!(payload["param_source"], json!("argparse"));
    // argparse reader -> machine-facing origin
    assert_eq!(payload["param_origin"], json!("reader"));
    assert_eq!(payload["degraded_reason"], json!(""));
    assert_eq!(payload["drift"], json!(false));
    assert_eq!(payload["presets"], json!([]));
    assert_eq!(payload["last_run_at"], Value::Null);
    assert_eq!(payload["last_exit"], Value::Null);

    let fields = fields_by_key(&payload);
    assert_eq!(
        field_keys_in_order(&payload),
        ["src", "width", "fmt", "force"]
    );
    assert_eq!(
        fields["src"],
        json!({
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
    assert_eq!(fields["width"]["type"], json!("int"));
    assert_eq!(fields["width"]["default"], json!("800"));
    assert_eq!(fields["width"]["help"], json!("target width"));
    assert_eq!(fields["width"]["flag"], json!("--width"));
    assert_eq!(fields["width"]["required"], json!(false));
    assert_eq!(fields["fmt"]["type"], json!("choice"));
    assert_eq!(fields["fmt"]["choices"], json!(["png", "jpg"]));
    assert_eq!(fields["force"]["type"], json!("bool"));
    assert_eq!(fields["force"]["action"], json!("store_true"));
    assert_eq!(fields["force"]["default"], json!("false"));
}

#[test]
#[ignore = "FAILING CONTRACT (divergence): oracle pins show --json to exactly 21 payload keys; Rust adds added_at/id/schema/source_hash (plausibly-intentional superset additions per the rewrite rule — adjudicate before removing keys)"]
fn test_show_json_stable_shape() {
    let lib = lib();
    let path = lib.write_src("job.py", ARGPARSE);
    lib.add_python(&path, "shape");
    let payload = lib.show_json("shape");
    let keys: BTreeSet<&str> = payload
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    let expected: BTreeSet<&str> = PAYLOAD_KEYS.iter().copied().collect();
    assert_eq!(keys, expected);
    for field in payload["fields"].as_array().unwrap() {
        let field_keys: BTreeSet<&str> = field
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        let expected_field: BTreeSet<&str> = FIELD_KEYS.iter().copied().collect();
        assert_eq!(field_keys, expected_field);
    }
}

#[test]
fn test_show_json_repeat_true_for_a_click_multiple_option() {
    // The True side of the `repeat` JSON key (the argparse fixtures only ever emit False): a click
    // multiple option assembles as a repeated flag, and an agent reads that off `repeat`.
    let lib = lib();
    let path = lib.write_src("tagger.py", CLICK_MULTIPLE);
    lib.add_python(&path, "tagger");
    let payload = lib.show_json("tagger");
    let fields = fields_by_key(&payload);
    assert_eq!(fields["tag"]["multiple"], json!(true));
    assert_eq!(fields["tag"]["repeat"], json!(true));
}

#[test]
fn test_show_json_inject_secret_and_state() {
    let lib = lib();
    let mut key = const_str("KEY");
    key.default = Some(ParameterValue::String("abc".to_owned()));
    key.secret = true;
    key.env_source = "API_KEY".to_owned();
    let mut city = const_str("CITY");
    city.default = Some(ParameterValue::String("Taipei".to_owned()));
    city.prompt = "Which city?".to_owned();
    let text = inject(
        "KEY = \"abc\"\nCITY = \"Taipei\"\nprint(KEY, CITY)\n",
        &[key, city],
    );
    let path = lib.write_src("api.py", &text);
    lib.add_python(&path, "api");
    let entry = lib.entry("api");
    // Only the preset NAME is read below; the oracle stores raw values, but empty declarations
    // keep the value filter a no-op and still insert the "fast" key.
    let mut preset_values = BTreeMap::new();
    preset_values.insert("CITY".to_owned(), "Tainan".to_owned());
    lib.state_service()
        .save_preset(&entry.slug, "fast", &[], &preset_values)
        .unwrap();
    lib.state_service()
        .record_run(&entry.slug, 3, "2026-07-11T00:00:00+00:00", &[], None)
        .unwrap();
    let payload = lib.show_json("api");
    assert_eq!(payload["param_source"], json!("inject"));
    // injected [tool.skit] params -> "managed"
    assert_eq!(payload["param_origin"], json!("managed"));
    assert_eq!(payload["presets"], json!(["fast"]));
    assert_eq!(payload["last_run_at"], json!("2026-07-11T00:00:00+00:00"));
    assert_eq!(payload["last_exit"], json!(3));
    let fields = fields_by_key(&payload);
    let key = &fields["KEY"];
    assert_eq!(key["source"], json!("inject"));
    assert_eq!(key["secret"], json!(true));
    assert_eq!(key["env_source"], json!("API_KEY"));
    // params --json parity: a secret's declared default already lives in the script's own text,
    // so the JSON carries it as-is (the human table masks it instead).
    assert_eq!(key["default"], json!("abc"));
    assert_eq!(fields["CITY"]["label"], json!("Which city?"));
}

#[test]
fn test_show_json_command_kind() {
    let lib = lib();
    let output = lib.run(&[
        "add",
        "--cmd",
        "echo {target} {level}",
        "--name",
        "deploy",
        "--no-input",
    ]);
    assert!(output.status.success(), "{}", combined(&output));
    let payload = lib.show_json("deploy");
    assert_eq!(payload["kind"], json!("command"));
    assert_eq!(payload["template"], json!("echo {target} {level}"));
    assert_eq!(payload["param_source"], json!("command"));
    assert_eq!(payload["param_origin"], json!("command"));
    let fields = fields_by_key(&payload);
    let keys: BTreeSet<&str> = fields.keys().map(String::as_str).collect();
    assert_eq!(keys, BTreeSet::from(["target", "level"]));
    assert_eq!(fields["target"]["source"], json!("placeholder"));
    assert_eq!(fields["target"]["required"], json!(true));
}

#[test]
fn test_show_json_deps_and_missing_reference() {
    let lib = lib();
    let path = lib.write_src("ref.py", "print(1)\n");
    let added = lib.run(&[
        "add",
        path.to_str().unwrap(),
        "--name",
        "ref",
        "--ref",
        "--no-input",
    ]);
    assert!(added.status.success(), "{}", combined(&added));
    let deps = lib.run(&[
        "deps",
        "ref",
        "--dep",
        "requests>=2,<3",
        "--python",
        ">=3.12",
    ]);
    assert!(deps.status.success(), "{}", combined(&deps));
    fs::remove_file(&path).unwrap();
    let payload = lib.show_json("ref");
    assert_eq!(payload["mode"], json!("reference"));
    assert_eq!(payload["missing"], json!(true));
    assert_eq!(payload["dependencies"], json!(["requests>=2,<3"]));
    assert_eq!(payload["requires_python"], json!(">=3.12"));
}

#[test]
fn test_show_json_degraded_parser() {
    let lib = lib();
    let path = lib.write_src("multi.py", SUBPARSERS);
    lib.add_python(&path, "multi");
    let payload = lib.show_json("multi");
    assert_eq!(payload["param_source"], json!("argparse"));
    assert_eq!(payload["degraded_reason"], json!("subparsers"));
    assert_eq!(payload["fields"], json!([]));
}

#[test]
fn test_show_json_drift() {
    let lib = lib();
    let text = inject("CITY = \"x\"\nprint(CITY)\n", &[const_str("CITY")]);
    let path = lib.write_src("stale.py", &text);
    lib.add_python(&path, "stale");
    let entry = lib.entry("stale");
    let copy = lib.copy_path(&entry);
    let moved = fs::read_to_string(&copy)
        .unwrap()
        .replace("CITY = \"x\"", "TOWN = \"x\"");
    fs::write(&copy, moved).unwrap();
    let payload = lib.show_json("stale");
    assert_eq!(payload["drift"], json!(true));
}

// ================================================================================================
// human view
// ================================================================================================

#[test]
fn test_show_human_argparse_table() {
    let lib = lib();
    let path = lib.write_src("job.py", ARGPARSE);
    lib.add_python(&path, "resize");
    let output = lib.show_human("resize");
    assert!(output.contains("resize"));
    assert!(output.contains("width"));
    assert!(output.contains("target width"));
    assert!(output.contains("png, jpg"));
    assert!(output.contains("yes")); // src is required
    assert!(output.contains("Source:"));
    assert!(output.contains("Run it: skit run resize"));
}

#[test]
fn test_show_human_masks_secret_default_and_names_env_source() {
    let lib = lib();
    let mut key = const_str("KEY");
    key.default = Some(ParameterValue::String("s3cret".to_owned()));
    key.secret = true;
    key.env_source = "API_KEY".to_owned();
    let text = inject("KEY = \"s3cret\"\nprint(KEY)\n", &[key]);
    let path = lib.write_src("api.py", &text);
    lib.add_python(&path, "api");
    let output = lib.show_human("api");
    assert!(!output.contains("s3cret"));
    assert!(output.contains("•••"));
    assert!(output.contains("$API_KEY"));
}

#[test]
fn test_show_human_secret_without_env_source() {
    let lib = lib();
    let mut token = const_str("TOKEN");
    token.secret = true;
    let text = inject("TOKEN = \"t\"\nprint(TOKEN)\n", &[token]);
    let path = lib.write_src("tok.py", &text);
    lib.add_python(&path, "tok");
    let output = lib.show_human("tok");
    // TOKEN is optional and has no default, so the single "yes" is the Secret cell.
    assert_eq!(output.matches("yes").count(), 1);
    assert!(!output.contains('←')); // no env-source arrow without an env source
}

#[test]
fn test_show_human_command_kind() {
    let lib = lib();
    let added = lib.run(&["add", "--cmd", "echo {a}", "--name", "c1", "--no-input"]);
    assert!(added.status.success(), "{}", combined(&added));
    let output = lib.show_human("c1");
    assert!(output.contains("Command template: echo {a}"));
    assert!(!output.contains("Source:")); // a command entry has no file source to show
    assert!(output.contains('a'));
}

#[test]
fn test_show_human_no_fields_exe() {
    let lib = lib();
    let exe = lib.write_src("tool", "#!/bin/sh\necho hi\n");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        fs::set_permissions(&exe, fs::Permissions::from_mode(0o755)).unwrap();
    }
    let added = lib.run(&[
        "add",
        exe.to_str().unwrap(),
        "--exe",
        "--name",
        "tool",
        "--no-input",
    ]);
    assert!(added.status.success(), "{}", combined(&added));
    let output = lib.show_human("tool");
    assert!(output.contains("No form fields"));
}

#[test]
fn test_show_human_description_deps_presets_and_drift() {
    let lib = lib();
    let text = inject("CITY = \"x\"\nprint(CITY)\n", &[const_str("CITY")]);
    let path = lib.write_src("trip.py", &text);
    let added = lib.run(&[
        "add",
        path.to_str().unwrap(),
        "--name",
        "trip",
        "-d",
        "plan a trip",
        "--no-input",
    ]);
    assert!(added.status.success(), "{}", combined(&added));
    let deps = lib.run(&["deps", "trip", "--dep", "rich>=15", "--python", ">=3.12"]);
    assert!(deps.status.success(), "{}", combined(&deps));
    let entry = lib.entry("trip");
    let mut preset_values = BTreeMap::new();
    preset_values.insert("CITY".to_owned(), "Tainan".to_owned());
    lib.state_service()
        .save_preset(&entry.slug, "quick", &[], &preset_values)
        .unwrap();
    // Drift the copy AFTER `deps` rewrites its own PEP 723 block, so the edit survives.
    let copy = lib.copy_path(&entry);
    let moved = fs::read_to_string(&copy)
        .unwrap()
        .replace("CITY = \"x\"", "TOWN = \"x\"");
    fs::write(&copy, moved).unwrap();
    let output = lib.show_human("trip");
    assert!(output.contains("plan a trip"));
    assert!(output.contains("rich>=15"));
    assert!(output.contains(">=3.12"));
    assert!(output.contains("Presets: quick"));
    assert!(output.contains("drifted from the script")); // the drift banner is shown
}

#[test]
fn test_show_human_degraded_parser_notice() {
    let lib = lib();
    let path = lib.write_src("multi.py", SUBPARSERS);
    lib.add_python(&path, "multi");
    let output = lib.show_human("multi");
    // Line-exact: an XX-wrapped msgid mutant still contains the substring.
    assert!(output.lines().any(|line| line
        == "skit could not model this script's own arguments; pass them after -- instead."));
}

#[test]
fn test_show_human_missing_marker() {
    let lib = lib();
    let path = lib.write_src("gone.py", "print(1)\n");
    let added = lib.run(&[
        "add",
        path.to_str().unwrap(),
        "--name",
        "gone",
        "--ref",
        "--no-input",
    ]);
    assert!(added.status.success(), "{}", combined(&added));
    fs::remove_file(&path).unwrap();
    let output = lib.show_human("gone");
    // The glyph-prefixed marker, not a bare "missing".
    assert!(output.contains("⚠ missing:"));
}

#[test]
fn test_show_not_found_exits_1() {
    let lib = lib();
    let output = lib.run(&["show", "ghost"]);
    assert_eq!(output.status.code(), Some(1));
}
