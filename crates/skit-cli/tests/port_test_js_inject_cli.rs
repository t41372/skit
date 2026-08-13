//! Real CLI/store/staging ports from Python v0.4 `tests/test_js_inject.py` at `main@206f9ef`.
//!
//! A hermetic fake `node` observes the exact staged path, permissions, and bytes while the child is
//! running. It is not a JavaScript evaluator. Contracts that require actual JavaScript semantics
//! live in `port_test_js_inject_runtime.rs` and keep the Python runtime-availability gate.

#![cfg(unix)]

use std::{collections::BTreeMap, env, fs, os::unix::fs::PermissionsExt as _, path::Path};

use assert_cmd::Command;
use skit_application::{
    CreateEntry, EntryMutationRepository as _, EntryPayload, EntryRepository as _, SourcePermissions,
    payload_stored_name,
};
use skit_domain::{
    EntryKind, EntrySettings, StorageMode,
    parameters::{ParamDecl, ParameterBinding, ParameterDelivery, ParameterType, ParameterValue},
};
use skit_form::{FormSource, form_plan};
use skit_language::{ParseOutcome, parse_document, write_managed_params};
use skit_store::FileStore;
use tempfile::TempDir;

struct Sandbox {
    data: TempDir,
    state: TempDir,
    config: TempDir,
    home: TempDir,
    bin: TempDir,
}

impl Sandbox {
    fn new() -> Self {
        let sandbox = Self {
            data: TempDir::new().unwrap(),
            state: TempDir::new().unwrap(),
            config: TempDir::new().unwrap(),
            home: TempDir::new().unwrap(),
            bin: TempDir::new().unwrap(),
        };
        fs::write(
            sandbox.config.path().join("config.toml"),
            "[mirror]\nenabled = false\n",
        )
        .unwrap();
        sandbox.install_inspector_node();
        sandbox
    }

    fn store(&self) -> FileStore {
        FileStore::new(self.data.path())
    }

    fn command(&self) -> Command {
        let mut command = assert_cmd::cargo::cargo_bin_cmd!("skit");
        let inherited = env::var_os("PATH").unwrap_or_default();
        let mut paths = vec![self.bin.path().to_path_buf()];
        paths.extend(env::split_paths(&inherited));
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
            .env("PATH", env::join_paths(paths).unwrap())
            .env_remove("FORCE_COLOR")
            .env_remove("NO_COLOR")
            .current_dir(self.home.path());
        command
    }

    fn install_node(&self, body: &str) {
        let path = self.bin.path().join("node");
        fs::write(&path, body).unwrap();
        let mut permissions = fs::metadata(&path).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(path, permissions).unwrap();
    }

    fn install_inspector_node(&self) {
        self.install_node(
            r#"#!/bin/sh
if [ -n "${SKIT_TEST_MARKER:-}" ]; then : > "$SKIT_TEST_MARKER"; fi
printf 'FAKE_PATH=%s\n' "$1"
printf 'FAKE_MODE=%s\n' "$(LC_ALL=C ls -ld "$1" | cut -c1-10)"
printf '%s\n' 'FAKE_BODY_BEGIN'
cat "$1"
printf '%s\n' 'FAKE_BODY_END'
"#,
        );
    }

    fn install_rejecting_check_node(&self) {
        self.install_node(
            r#"#!/bin/sh
if [ "$1" = "--check" ]; then
  printf '%s\n' 'SyntaxError: boom' >&2
  exit 1
fi
if [ -n "${SKIT_TEST_MARKER:-}" ]; then : > "$SKIT_TEST_MARKER"; fi
exit 0
"#,
        );
    }

    fn create_managed_entry(&self, name: &str, kind: &str, origin: &str, source: &str) {
        let kind_value = EntryKind::parse(kind).unwrap();
        let ParseOutcome::Parsed(document) = parse_document(kind, source) else {
            panic!("test fixture must parse as {kind}");
        };
        let declaration = document
            .analysis()
            .candidates
            .into_iter()
            .next()
            .expect("test fixture must expose one candidate")
            .declaration;
        let managed = write_managed_params(kind, source, &[declaration]).unwrap();
        let settings = EntrySettings {
            interpreter: "node".to_owned(),
            ..EntrySettings::default()
        };
        self.store()
            .create(CreateEntry {
                name: name.to_owned(),
                kind: kind_value.clone(),
                mode: StorageMode::Copy,
                source: origin.to_owned(),
                workdir: "invoke".to_owned(),
                description: String::new(),
                payload: Some(EntryPayload {
                    bytes: managed.into_bytes(),
                    stored_name: Some(payload_stored_name(&kind_value, Path::new(origin))),
                    permissions: SourcePermissions::default(),
                }),
                settings,
            })
            .unwrap();
    }

    fn create_drifted_entry(&self, name: &str) {
        let mut declaration = ParamDecl::new("WIDTH");
        declaration.binding = ParameterBinding::Const;
        declaration.delivery = ParameterDelivery::Inject;
        declaration.parameter_type = ParameterType::Int;
        declaration.default = Some(ParameterValue::Integer(800));
        let managed = write_managed_params("js", "const TALL = 800;\n", &[declaration]).unwrap();
        let kind = EntryKind::parse("js").unwrap();
        self.store()
            .create(CreateEntry {
                name: name.to_owned(),
                kind: kind.clone(),
                mode: StorageMode::Copy,
                source: format!("{name}.js"),
                workdir: "invoke".to_owned(),
                description: String::new(),
                payload: Some(EntryPayload {
                    bytes: managed.into_bytes(),
                    stored_name: Some(payload_stored_name(&kind, Path::new("fixture.js"))),
                    permissions: SourcePermissions::default(),
                }),
                settings: EntrySettings {
                    interpreter: "node".to_owned(),
                    ..EntrySettings::default()
                },
            })
            .unwrap();
    }

    fn run(&self, name: &str, key: &str, value: &str) -> std::process::Output {
        self.command()
            .args([
                "run",
                name,
                "--set",
                &format!("{key}={value}"),
                "--no-input",
            ])
            .output()
            .unwrap()
    }

    fn run_with_marker(
        &self,
        name: &str,
        key: &str,
        value: &str,
        marker: &Path,
    ) -> std::process::Output {
        self.command()
            .env("SKIT_TEST_MARKER", marker)
            .args([
                "run",
                name,
                "--set",
                &format!("{key}={value}"),
                "--no-input",
            ])
            .output()
            .unwrap()
    }

    fn staged_files(&self, name: &str) -> Vec<String> {
        let entry = self.store().resolve(name).unwrap();
        fs::read_dir(self.data.path().join("scripts").join(entry.slug.as_str()))
            .unwrap()
            .filter_map(Result::ok)
            .map(|item| item.file_name().to_string_lossy().into_owned())
            .filter(|name| name.starts_with(".run-"))
            .collect()
    }
}

fn output_text(output: &std::process::Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

fn tagged<'a>(text: &'a str, prefix: &str) -> &'a str {
    text.lines()
        .find_map(|line| line.strip_prefix(prefix))
        .unwrap_or_else(|| panic!("missing {prefix:?} in output:\n{text}"))
}

fn body(text: &str) -> &str {
    text.split_once("FAKE_BODY_BEGIN\n")
        .and_then(|(_, rest)| rest.split_once("FAKE_BODY_END"))
        .map(|(body, _)| body)
        .unwrap_or_else(|| panic!("fake runtime did not report staged body:\n{text}"))
}

#[test]
fn test_ts_temp_copy_has_ts_suffix() {
    let sandbox = Sandbox::new();
    sandbox.create_managed_entry("tscopy", "ts", "plain.ts", "const N: number = 5;\n");
    let output = sandbox.run("tscopy", "N", "7");
    let text = output_text(&output);
    assert_eq!(output.status.code(), Some(0), "{text}");
    assert!(tagged(&text, "FAKE_PATH=").ends_with(".ts"), "{text}");
    assert!(body(&text).contains("const N: number = 7;"), "{text}");
    assert!(sandbox.staged_files("tscopy").is_empty());
}

fn assert_module_flavor(origin: &str, kind: &str, expected: &str, name: &str) {
    let sandbox = Sandbox::new();
    let source = if kind == "js" {
        "const N = 5;\n"
    } else {
        "const N: number = 5;\n"
    };
    sandbox.create_managed_entry(name, kind, origin, source);
    let output = sandbox.run(name, "N", "7");
    let text = output_text(&output);
    assert_eq!(output.status.code(), Some(0), "origin={origin:?}: {text}");
    assert!(
        tagged(&text, "FAKE_PATH=").ends_with(expected),
        "origin={origin:?}, expected={expected:?}: {text}"
    );
    assert!(body(&text).contains("N = 7"), "origin={origin:?}: {text}");
    assert!(sandbox.staged_files(name).is_empty());
}

#[test]
fn test_injected_copy_carries_the_origins_module_flavor() {
    for (index, (origin, kind, expected)) in [
        ("tool.mjs", "js", ".mjs"),
        ("tool.cjs", "js", ".cjs"),
        ("plain.js", "js", ".js"),
        ("", "js", ".js"),
        ("tool.mts", "ts", ".mts"),
        ("tool.cts", "ts", ".cts"),
        ("plain.ts", "ts", ".ts"),
    ]
    .into_iter()
    .enumerate()
    {
        assert_module_flavor(origin, kind, expected, &format!("flavor{index}"));
    }
}

macro_rules! flavor_case {
    ($name:ident, $origin:literal, $kind:literal, $expected:literal) => {
        #[test]
        fn $name() {
            assert_module_flavor($origin, $kind, $expected, stringify!($name));
        }
    };
}
flavor_case!(rust_additive_js_inject_flavor_mjs, "tool.mjs", "js", ".mjs");
flavor_case!(rust_additive_js_inject_flavor_cjs, "tool.cjs", "js", ".cjs");
flavor_case!(rust_additive_js_inject_flavor_js, "plain.js", "js", ".js");
flavor_case!(rust_additive_js_inject_flavor_no_origin, "", "js", ".js");
flavor_case!(rust_additive_js_inject_flavor_mts, "tool.mts", "ts", ".mts");
flavor_case!(rust_additive_js_inject_flavor_cts, "tool.cts", "ts", ".cts");
flavor_case!(rust_additive_js_inject_flavor_ts, "plain.ts", "ts", ".ts");

#[test]
fn test_injected_copy_is_0600() {
    let sandbox = Sandbox::new();
    sandbox.create_managed_entry(
        "secret",
        "js",
        "secret.js",
        "const API_KEY = \"changeme\";\n",
    );
    let output = sandbox.run("secret", "API_KEY", "s3cr3t");
    let text = output_text(&output);
    assert_eq!(output.status.code(), Some(0), "{text}");
    assert_eq!(tagged(&text, "FAKE_MODE="), "-rw-------", "{text}");
    assert!(body(&text).contains("const API_KEY = \"s3cr3t\";"), "{text}");
    assert!(sandbox.staged_files("secret").is_empty());
}

#[test]
fn test_execute_runs_a_js_entry_offline_plan() {
    let sandbox = Sandbox::new();
    sandbox.create_managed_entry(
        "offline",
        "js",
        "offline.js",
        "const WIDTH = 800;\nconsole.log(WIDTH);\n",
    );
    let entry = sandbox.store().resolve("offline").unwrap();
    let source = fs::read_to_string(
        sandbox
            .data
            .path()
            .join("scripts")
            .join(entry.slug.as_str())
            .join("script.js"),
    )
    .unwrap();
    let plan = form_plan("js", &source, &EntrySettings::from_meta(&entry.meta));
    assert_eq!(plan.source, FormSource::Inject);
    assert_eq!(
        plan.fields
            .iter()
            .map(|field| field.declaration.name.as_str())
            .collect::<Vec<_>>(),
        ["WIDTH"]
    );
}

#[test]
fn test_execute_maps_a_drifted_js_definition_to_drift() {
    let sandbox = Sandbox::new();
    sandbox.create_drifted_entry("drifted");
    let marker = sandbox.home.path().join("launched");
    let output = sandbox.run_with_marker("drifted", "WIDTH", "1200", &marker);
    let text = output_text(&output);
    assert_eq!(output.status.code(), Some(125), "{text}");
    assert!(text.contains("--resync"), "Python drift guidance was lost: {text}");
    assert!(!marker.exists(), "drifted JS launched before injection refusal");
    assert!(sandbox.staged_files("drifted").is_empty());
}

#[test]
fn test_execute_refuses_a_bad_value_before_launch() {
    let sandbox = Sandbox::new();
    sandbox.create_managed_entry(
        "badvalue",
        "js",
        "badvalue.js",
        "const WIDTH = 800;\n",
    );
    let marker = sandbox.home.path().join("launched");
    let output = sandbox.run_with_marker("badvalue", "WIDTH", "abc", &marker);
    let text = output_text(&output);
    assert_eq!(output.status.code(), Some(125), "{text}");
    assert!(!marker.exists(), "bad JS value reached the child");
    assert!(sandbox.staged_files("badvalue").is_empty());
}

#[test]
fn test_gate2_failure_removes_the_temp_copy() {
    let sandbox = Sandbox::new();
    sandbox.install_rejecting_check_node();
    sandbox.create_managed_entry(
        "gate2",
        "js",
        "gate2.js",
        "const T = \"hi\";\n",
    );
    let marker = sandbox.home.path().join("launched");
    let output = sandbox.run_with_marker("gate2", "T", "x", &marker);
    let text = output_text(&output);
    assert_eq!(
        output.status.code(),
        Some(125),
        "node syntax gate did not refuse before child launch: {text}"
    );
    assert!(!marker.exists(), "gate2 rejection happened only after the child launched");
    assert!(sandbox.staged_files("gate2").is_empty());
}
