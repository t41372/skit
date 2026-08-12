//! CLI/state ports of Python v0.4 `tests/test_default_semantics_review_fixes.py`.
//!
//! These tests use the real CLI for machine JSON, preset history, run-save-preset, and source-edit
//! behavior. State seeding mirrors the Python tests only where they also seeded argstate directly.
//! A red behavioral assertion is a parity finding, not a reason to weaken this test or patch skit.

use std::{collections::BTreeMap, fs, path::PathBuf};

use assert_cmd::Command;
use serde_json::Value;
use skit_application::form_state::{FormStateRepository, FormStateService, prefill};
use skit_domain::{
    EntrySettings, Slug,
    parameters::{ParamDecl, ParameterBinding, ParameterDelivery, ParameterType, ParameterValue},
};
use skit_form::form_plan;
use skit_language::{ParseOutcome, parse_document};
use skit_store::FileFormStateStore;
use tempfile::TempDir;

const SECRET_LITERAL: &str = "sk-live-SUPERSECRET";

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
            .env("COLUMNS", "200")
            .env("TERM", "xterm-256color")
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

    fn json(&self, args: &[&str]) -> (String, Value) {
        let output = self.ok(args);
        let raw = String::from_utf8(output.stdout).unwrap();
        let value = serde_json::from_str(&raw)
            .unwrap_or_else(|error| panic!("stdout must be pure JSON: {error}\n{raw}"));
        (raw, value)
    }

    fn add_python(&self, name: &str, source: &str) -> PathBuf {
        let input = self.home.path().join(format!("{name}.py"));
        fs::write(&input, source).unwrap();
        self.ok(&["add", input.to_str().unwrap(), "--name", name, "--no-input"]);
        self.stored(name)
    }

    fn stored(&self, slug: &str) -> PathBuf {
        self.data
            .path()
            .join("scripts")
            .join(slug)
            .join("script.py")
    }

    fn state_store(&self) -> FileFormStateStore {
        FileFormStateStore::new(self.state.path())
    }
}

fn combined(output: &std::process::Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

fn slug(value: &str) -> Slug {
    Slug::parse(value.to_owned()).unwrap()
}

fn values(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn defaulted(name: &str, default: &str) -> ParamDecl {
    let mut declaration = ParamDecl::new(name);
    declaration.binding = ParameterBinding::Const;
    declaration.delivery = ParameterDelivery::Inject;
    declaration.parameter_type = ParameterType::Str;
    declaration.default = Some(ParameterValue::String(default.to_owned()));
    declaration
}

fn managed_default_source(default: &str) -> String {
    format!(
        r#"# /// script
# dependencies = []
#
# [tool.skit]
# schema = 1
#
# [[tool.skit.params]]
# name = "GREETING"
# kind = "const"
# type = "str"
# default = "{default}"
# ///
GREETING = "{default}"
print(GREETING)
"#
    )
}

fn source_with_city(default: &str, body_value: &str) -> String {
    format!(
        r#"# /// script
# dependencies = []
#
# [tool.skit]
# schema = 1
#
# [[tool.skit.params]]
# name = "CITY"
# kind = "const"
# type = "str"
# default = "{default}"
# ///
CITY = "{body_value}"
print(CITY)
"#
    )
}

fn parameter<'a>(document: &'a Value, name: &str) -> &'a Value {
    document["parameters"]
        .as_array()
        .unwrap_or_else(|| panic!("missing parameters array: {document}"))
        .iter()
        .find(|row| row["name"] == name)
        .unwrap_or_else(|| panic!("missing parameter {name}: {document}"))
}

fn managed_block(source: &str) -> &str {
    let first = source.find("# ///").expect("managed block opener");
    let second_relative = source[first + 4..]
        .find("# ///")
        .expect("managed block closer");
    let second = first + 4 + second_relative + "# ///".len();
    &source[..second]
}

#[test]
fn test_secret_source_literal_is_absent_from_reconcile_and_json() {
    let source = format!(
        r#"# /// script
# dependencies = []
#
# [tool.skit]
# schema = 1
#
# [[tool.skit.params]]
# name = "TOKEN"
# kind = "const"
# type = "str"
# secret = true
# ///
TOKEN = "{SECRET_LITERAL}"
print(TOKEN)
"#
    );

    let mut stored = ParamDecl::new("TOKEN");
    stored.binding = ParameterBinding::Const;
    stored.delivery = ParameterDelivery::Inject;
    stored.parameter_type = ParameterType::Str;
    stored.secret = true;
    let ParseOutcome::Parsed(document) = parse_document("python", &source) else {
        panic!("secret source must parse");
    };
    let report = document.reconcile(&[stored]);
    assert_eq!(report.ok.len(), 1);
    assert!(report.current_defaults.is_empty());

    let sandbox = Sandbox::new();
    sandbox.add_python("tok", &source);
    let (raw_params, params) = sandbox.json(&["params", "tok", "--json"]);
    assert_eq!(params["current_defaults"], serde_json::json!({}));
    let row = parameter(&params, "TOKEN");
    assert_eq!(row["secret"], true);
    assert!(
        row.get("default").is_none(),
        "secret row leaked a default: {row}"
    );
    assert!(!raw_params.contains(SECRET_LITERAL), "{raw_params}");

    let (raw_show, show) = sandbox.json(&["show", "tok", "--json"]);
    let shown = show["fields"]
        .as_array()
        .unwrap()
        .iter()
        .find(|field| field["key"] == "TOKEN")
        .unwrap_or_else(|| panic!("missing TOKEN field: {show}"));
    assert_eq!(shown["secret"], true);
    assert!(shown["default"].is_null());
    assert!(!raw_show.contains(SECRET_LITERAL), "{raw_show}");
}

#[test]
fn test_preset_from_last_saves_effective_values_after_an_all_defaults_run() {
    let sandbox = Sandbox::new();
    sandbox.add_python("greet", &managed_default_source("bonjour"));
    let repository = sandbox.state_store();
    let service = FormStateService::new(repository);
    let slug = slug("greet");
    let declarations = [defaulted("GREETING", "bonjour")];
    let accepted = values(&[("GREETING", "bonjour")]);

    service
        .save_last(
            &slug,
            &declarations,
            Some(&accepted),
            Some(Vec::new()),
            false,
        )
        .unwrap();
    service
        .record_run(
            &slug,
            0,
            "2026-07-09T14:30:05+00:00",
            &declarations,
            Some(&accepted),
        )
        .unwrap();
    let before = service.load(&slug);
    assert!(before.values.is_empty());
    assert_eq!(before.last_run.exit, Some(0));
    assert_eq!(before.last_run.values, Some(accepted.clone()));

    sandbox.ok(&["preset", "save", "greet", "p", "--from-last"]);
    assert_eq!(
        service.load(&slug).presets,
        BTreeMap::from([("p".to_owned(), accepted)])
    );
}

#[test]
fn test_preset_from_last_still_refuses_an_entry_that_never_ran() {
    let sandbox = Sandbox::new();
    sandbox.add_python("fresh", &managed_default_source("bonjour"));
    let service = FormStateService::new(sandbox.state_store());
    let output = sandbox.output(&["preset", "save", "fresh", "p", "--from-last"]);

    assert_eq!(output.status.code(), Some(1), "{}", combined(&output));
    assert!(
        combined(&output).contains("no remembered values yet"),
        "{}",
        combined(&output)
    );
    assert!(service.load(&slug("fresh")).presets.is_empty());
}

#[test]
fn test_preset_from_last_pins_the_default_that_actually_ran() {
    let sandbox = Sandbox::new();
    let stored = sandbox.add_python("history", &managed_default_source("A"));
    let service = FormStateService::new(sandbox.state_store());
    let slug = slug("history");
    let old_declarations = [defaulted("GREETING", "A")];
    let ran = values(&[("GREETING", "A")]);
    service
        .save_last(&slug, &old_declarations, Some(&ran), None, false)
        .unwrap();
    service
        .record_run(
            &slug,
            0,
            "2026-07-09T14:30:05+00:00",
            &old_declarations,
            Some(&ran),
        )
        .unwrap();

    let current = fs::read_to_string(&stored).unwrap();
    fs::write(
        &stored,
        current.replace("GREETING = \"A\"", "GREETING = \"B\""),
    )
    .unwrap();
    let updated = fs::read_to_string(&stored).unwrap();
    let plan = form_plan("python", &updated, &EntrySettings::default());
    assert_eq!(
        prefill(&plan.declarations(), &BTreeMap::new(), None),
        values(&[("GREETING", "B")])
    );

    sandbox.ok(&["preset", "save", "history", "p", "--from-last"]);
    assert_eq!(
        service.load(&slug).presets.get("p"),
        Some(&values(&[("GREETING", "A")]))
    );
}

#[test]
fn test_preset_from_legacy_run_without_snapshot_refuses_to_guess() {
    let sandbox = Sandbox::new();
    sandbox.add_python("legacy-history", &managed_default_source("B"));
    let repository = sandbox.state_store();
    let service = FormStateService::new(repository.clone());
    let slug = slug("legacy-history");
    repository
        .update(&slug, |state| {
            state.last_run.at = Some("2026-07-09T14:30:05+00:00".to_owned());
            state.last_run.exit = Some(0);
            state.last_run.values = None;
        })
        .unwrap();

    let output = sandbox.output(&["preset", "save", "legacy-history", "p", "--from-last"]);
    assert_eq!(output.status.code(), Some(1), "{}", combined(&output));
    assert!(
        combined(&output).contains("run it once first"),
        "{}",
        combined(&output)
    );
    assert!(service.load(&slug).presets.is_empty());
}

#[test]
fn test_run_save_preset_stores_a_default_equal_value_verbatim() {
    let sandbox = Sandbox::new();
    sandbox.add_python("pinned", &managed_default_source("bonjour"));
    let output = sandbox.output(&[
        "run",
        "pinned",
        "--set",
        "GREETING=bonjour",
        "--save-preset",
        "p",
        "--no-input",
        "--dry-run",
    ]);
    assert!(output.status.success(), "{}", combined(&output));

    let state = FormStateService::new(sandbox.state_store()).load(&slug("pinned"));
    assert_eq!(
        state.presets,
        BTreeMap::from([("p".to_owned(), values(&[("GREETING", "bonjour")]))])
    );
    assert!(state.values.is_empty());
}

#[test]
fn test_resync_and_secret_in_one_edit_drops_the_refreshed_literal() {
    let sandbox = Sandbox::new();
    let stored = sandbox.add_python("secret-edit", &source_with_city("old", "sk-live-source"));

    let output = sandbox.output(&["params", "secret-edit", "--resync", "--secret", "CITY"]);
    assert!(output.status.success(), "{}", combined(&output));
    let (_, document) = sandbox.json(&["params", "secret-edit", "--json"]);
    let city = parameter(&document, "CITY");
    assert_eq!(city["secret"], true);

    let source = fs::read_to_string(stored).unwrap();
    let block = managed_block(&source);
    assert!(block.contains("secret = true"), "{block}");
    assert!(!block.contains("default ="), "{block}");
    assert!(!block.contains("sk-live-source"), "{block}");
}

#[test]
fn test_final_no_secret_in_same_edit_keeps_the_public_default() {
    let sandbox = Sandbox::new();
    let stored = sandbox.add_python("public-edit", &source_with_city("old", "new"));

    let output = sandbox.output(&[
        "params",
        "public-edit",
        "--resync",
        "--secret",
        "CITY",
        "--no-secret",
        "CITY",
    ]);
    assert!(output.status.success(), "{}", combined(&output));
    let (_, document) = sandbox.json(&["params", "public-edit", "--json"]);
    let city = parameter(&document, "CITY");
    assert!(!city["secret"].as_bool().unwrap_or(false));
    assert_eq!(city["default"], "new");

    let source = fs::read_to_string(stored).unwrap();
    let block = managed_block(&source);
    assert!(!block.contains("secret = true"), "{block}");
    assert!(block.contains("default = \"new\""), "{block}");
}
