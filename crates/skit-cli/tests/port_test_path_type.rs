//! Mechanical port of `tests/test_path_type.py` from the Python oracle
//! (`main@206f9ef`). The Rust rewrite splits the original module across domain, language,
//! application, UI, CLI, and TUI boundaries. Each Python test name remains unique and executable.

use std::{collections::BTreeMap, fs};

use assert_cmd::Command;
use serde_json::{Value, json};
use skit_application::value_preparation::validate_form_value;
use skit_domain::parameters::{
    ParamDecl, ParameterBinding, ParameterDelivery, ParameterType, ParameterValue, coerce_default,
};
use skit_language::{ParseOutcome, ReconcileReport, managed_params, parse_document};
use skit_ui::{FormControl, FormInputKind, RunFieldRole, RunFormView};
use tempfile::TempDir;

const SCRIPT: &str = "SRC = \"./data.csv\"\nRETRIES = 3\nprint(SRC, RETRIES)\n";

fn map(value: Value) -> BTreeMap<String, Value> {
    serde_json::from_value(value).unwrap()
}

fn path_declaration(name: &str) -> ParamDecl {
    ParamDecl {
        binding: ParameterBinding::Const,
        delivery: ParameterDelivery::Inject,
        parameter_type: ParameterType::Path,
        ..ParamDecl::new(name)
    }
}

fn reconcile(source: &str, stored: &[ParamDecl]) -> ReconcileReport {
    match parse_document("python", source) {
        ParseOutcome::Parsed(document) => document.reconcile(stored),
        _ => ReconcileReport::from_syntax_error(stored),
    }
}

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
            String::from_utf8_lossy(&output.stderr)
        );
        output.stdout
    }

    fn json(&self, args: &[&str]) -> Value {
        serde_json::from_slice(&self.ok(args)).unwrap()
    }

    fn source(&self, name: &str, text: &str) -> String {
        let path = self.data.path().join(name);
        fs::write(&path, text).unwrap();
        path.to_str().unwrap().to_owned()
    }
}

#[test]
fn test_path_is_an_allowed_type() {
    assert_eq!(ParameterType::Path.as_str(), "path");
    assert_eq!(
        serde_json::from_value::<ParameterType>(json!("path")).unwrap(),
        ParameterType::Path
    );
}

#[test]
fn test_unknown_type_still_degrades_to_str() {
    assert_eq!(
        ParamDecl::from_block_map(&map(json!({
            "name": "X",
            "kind": "const",
            "type": "pathlike"
        })))
        .parameter_type,
        ParameterType::Str
    );
    assert_eq!(
        ParamDecl::from_meta_map(&map(json!({
            "name": "X",
            "type": "pathlike"
        })))
        .parameter_type,
        ParameterType::Str
    );
}

#[test]
fn test_block_round_trip_carries_path() {
    let declaration = path_declaration("SRC");
    assert_eq!(
        ParamDecl::from_block_map(&declaration.to_block_map()).parameter_type,
        ParameterType::Path
    );
}

#[test]
fn test_meta_round_trip_carries_path() {
    let declaration = ParamDecl {
        parameter_type: ParameterType::Path,
        ..ParamDecl::new("src")
    };
    assert_eq!(
        ParamDecl::from_meta_map(&declaration.to_meta_map()).parameter_type,
        ParameterType::Path
    );
}

#[test]
fn test_coerce_default_path_keeps_raw_string() {
    assert_eq!(
        coerce_default("./no such file.csv", ParameterType::Path).unwrap(),
        ParameterValue::String("./no such file.csv".to_owned())
    );
}

#[test]
fn test_edit_declared_accepts_path_type() {
    let sandbox = Sandbox::new();
    sandbox.ok(&[
        "add",
        "--cmd",
        "printf '%s' {src}",
        "--name",
        "Path edit",
        "--no-input",
    ]);
    sandbox.ok(&["params", "path-edit", "--type", "src=path"]);
    let record = sandbox.json(&["params", "path-edit", "--json"]);
    assert_eq!(record["parameters"][0]["type"], "path");
}

#[test]
fn test_reconcile_path_over_str_const_is_refinement() {
    let report = reconcile(SCRIPT, &[path_declaration("SRC")]);
    assert!(!report.has_drift());
    assert!(report.changed.is_empty());
    assert_eq!(
        report
            .usable()
            .into_iter()
            .map(|declaration| declaration.name.as_str())
            .collect::<Vec<_>>(),
        ["SRC"]
    );
}

#[test]
fn test_reconcile_path_over_int_const_is_drift() {
    let report = reconcile(SCRIPT, &[path_declaration("RETRIES")]);
    assert!(report.has_drift());
    assert_eq!(report.changed.len(), 1);
    assert_eq!(report.changed[0].stored.name, "RETRIES");
    assert_eq!(
        report.changed[0].current.declaration.parameter_type,
        ParameterType::Int
    );
}

#[test]
fn test_resync_preserves_declared_path() {
    let sandbox = Sandbox::new();
    let source = sandbox.source(
        "path.py",
        concat!(
            "# /// script\n",
            "# [tool.skit]\n",
            "# schema = 1\n",
            "#\n",
            "# [[tool.skit.params]]\n",
            "# name = \"SRC\"\n",
            "# kind = \"const\"\n",
            "# type = \"path\"\n",
            "# prompt = \"Which file? \"\n",
            "# ///\n",
            "SRC = \"./data.csv\"\n",
            "print(SRC)\n",
        ),
    );
    sandbox.ok(&["add", &source, "--name", "Path resync", "--no-input"]);
    sandbox.ok(&["params", "path-resync", "--resync"]);

    let stored = fs::read_to_string(
        sandbox
            .data
            .path()
            .join("scripts/path-resync/script.py"),
    )
    .unwrap();
    let declarations = managed_params("python", &stored);
    assert_eq!(declarations.len(), 1);
    assert_eq!(declarations[0].parameter_type, ParameterType::Path);
    assert_eq!(declarations[0].prompt, "Which file? ");
}

#[test]
fn test_resync_still_corrects_real_type_drift() {
    let sandbox = Sandbox::new();
    let source = sandbox.source(
        "drift.py",
        concat!(
            "# /// script\n",
            "# [tool.skit]\n",
            "# schema = 1\n",
            "#\n",
            "# [[tool.skit.params]]\n",
            "# name = \"RETRIES\"\n",
            "# kind = \"const\"\n",
            "# type = \"path\"\n",
            "# ///\n",
            "RETRIES = 3\n",
            "print(RETRIES)\n",
        ),
    );
    sandbox.ok(&["add", &source, "--name", "Path drift", "--no-input"]);
    sandbox.ok(&["params", "path-drift", "--resync"]);

    let stored = fs::read_to_string(sandbox.data.path().join("scripts/path-drift/script.py"))
        .unwrap();
    let declarations = managed_params("python", &stored);
    assert_eq!(declarations.len(), 1);
    assert_eq!(declarations[0].parameter_type, ParameterType::Int);
}

#[test]
fn test_formfield_carries_path_for_every_delivery() {
    let declarations = [
        ParamDecl {
            binding: ParameterBinding::Const,
            delivery: ParameterDelivery::Inject,
            parameter_type: ParameterType::Path,
            ..ParamDecl::new("inject")
        },
        ParamDecl {
            delivery: ParameterDelivery::Flag,
            parameter_type: ParameterType::Path,
            ..ParamDecl::new("flag")
        },
        ParamDecl {
            binding: ParameterBinding::EnvDefault,
            delivery: ParameterDelivery::Env,
            parameter_type: ParameterType::Path,
            ..ParamDecl::new("env")
        },
        ParamDecl {
            delivery: ParameterDelivery::Placeholder,
            parameter_type: ParameterType::Path,
            ..ParamDecl::new("placeholder")
        },
    ];
    let view = RunFormView::from_declarations(
        "path-fields",
        "Path fields",
        &declarations,
        &BTreeMap::new(),
        &[],
        "",
        &BTreeMap::new(),
        "",
    );
    let kinds = view
        .fields()
        .iter()
        .filter(|field| matches!(field.role, RunFieldRole::Parameter { .. }))
        .map(|field| match &field.control {
            FormControl::Text(control) => control.kind,
            FormControl::Checkbox { .. } | FormControl::Choice(_) => {
                panic!("path parameters must use text controls")
            }
        })
        .collect::<Vec<_>>();
    assert_eq!(kinds, vec![FormInputKind::Path; 4]);
}

#[test]
fn test_degraded_flag_field_still_renders_free_text() {
    let declaration = ParamDecl {
        parameter_type: ParameterType::Path,
        degraded: true,
        ..ParamDecl::new("src")
    };
    let view = RunFormView::from_declarations(
        "degraded-path",
        "Degraded path",
        &[declaration],
        &BTreeMap::new(),
        &[],
        "",
        &BTreeMap::new(),
        "",
    );
    let kind = match &view.fields()[0].control {
        FormControl::Text(control) => control.kind,
        FormControl::Checkbox { .. } | FormControl::Choice(_) => {
            panic!("a degraded path must stay a free-text control")
        }
    };
    assert_eq!(kind, FormInputKind::Text);
}

#[test]
fn test_validate_value_path_is_free_text() {
    let declaration = ParamDecl {
        parameter_type: ParameterType::Path,
        ..ParamDecl::new("src")
    };
    assert!(
        validate_form_value(&declaration, "./definitely/not/created/yet.csv").is_ok()
    );
}
