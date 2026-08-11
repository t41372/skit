//! Executable ports of the declared-parameter edit contracts in Python v0.4
//! `tests/test_params_edit.py` (`origin/main@206f9ef`).
//!
//! The Python API returns edited rows plus closed warning codes. Rust exposes the same user-facing
//! edit boundary through `skit params`; these tests therefore assert the persisted row *and* the
//! soft-warning outcome. A hard error is not accepted as an equivalent warning.

use std::fs;

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

    fn add_exe(&self, name: &str) {
        // Parameter editing does not execute the target. An explicit directory is accepted by the
        // add lane as an exe reference on every platform and avoids Unix-only chmod fixtures.
        let source = self.home.path().join(format!("{name}-program"));
        fs::create_dir(&source).unwrap();
        self.command()
            .args(["add", source.to_str().unwrap(), "--exe", "--name", name])
            .assert()
            .success();
    }

    fn add_command(&self, name: &str, template: &str) {
        self.command()
            .args(["add", "--cmd", template, "--name", name])
            .assert()
            .success();
    }

    fn params(&self, slug: &str) -> Value {
        let output = self.ok(&["params", slug, "--json"]);
        serde_json::from_slice(&output.stdout).unwrap()
    }
}

fn rows(document: &Value) -> &[Value] {
    document["parameters"].as_array().unwrap()
}

fn row<'a>(document: &'a Value, name: &str) -> &'a Value {
    rows(document)
        .iter()
        .find(|item| item["name"] == name)
        .unwrap_or_else(|| panic!("missing {name} in {document}"))
}

fn names(document: &Value) -> Vec<String> {
    rows(document)
        .iter()
        .map(|item| item["name"].as_str().unwrap().to_owned())
        .collect()
}

fn string<'a>(row: &'a Value, key: &str) -> &'a str {
    row[key].as_str().unwrap_or("")
}

fn boolean(row: &Value, key: &str) -> bool {
    row[key].as_bool().unwrap_or(false)
}

fn output_text(output: &std::process::Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

fn exact_line(output: &std::process::Output, expected: &str) -> bool {
    output_text(output).lines().any(|line| line == expected)
}

#[test]
fn test_add_defaults_to_first_allowed_delivery_for_a_binary() {
    let sandbox = Sandbox::new();
    sandbox.add_exe("binary");
    sandbox.ok(&["params", "binary", "--add", "width"]);
    let document = sandbox.params("binary");
    let width = row(&document, "width");
    assert_eq!(
        (
            string(width, "name"),
            string(width, "delivery"),
            string(width, "binding"),
            string(width, "type"),
            boolean(width, "required"),
        ),
        ("width", "flag", "none", "str", false)
    );
}

#[test]
fn test_add_on_a_template_placeholder_name_becomes_a_required_placeholder() {
    let sandbox = Sandbox::new();
    sandbox.add_command("template", "echo {size}");
    let document = sandbox.params("template");
    let size = row(&document, "size");
    assert_eq!(
        (string(size, "delivery"), boolean(size, "required")),
        ("placeholder", true)
    );
}

// Python's pure `edit_declared(..., allowed_deliveries=("placeholder", "env"))` helper
// has no public Rust equivalent. Do not fake that internal helper through the CLI: Python's own
// CLI contract intentionally overrides it for command templates and is ported verbatim in
// `port_test_declared_params_cli::test_template_add_of_a_non_placeholder_name_creates_a_deliverable_env_row`.

#[test]
fn test_add_existing_name_warns_already_declared() {
    let sandbox = Sandbox::new();
    sandbox.add_exe("binary");
    sandbox.ok(&["params", "binary", "--add", "a"]);
    let output = sandbox.output(&["params", "binary", "--add", "a"]);
    let document = sandbox.params("binary");
    assert!(
        output.status.success()
            && exact_line(&output, "a is already declared; skipped.")
            && names(&document) == ["a"],
        "duplicate add must be a soft warning and leave one row\nstatus={:?}\noutput={}\nstate={document}",
        output.status.code(),
        output_text(&output),
    );
}

#[test]
fn test_rm_drops_the_row() {
    let sandbox = Sandbox::new();
    sandbox.add_exe("binary");
    sandbox.ok(&["params", "binary", "--add", "a", "--add", "b"]);
    sandbox.ok(&["params", "binary", "--rm", "a"]);
    assert_eq!(names(&sandbox.params("binary")), ["b"]);
}

#[test]
fn test_rm_unknown_name_warns_not_declared() {
    let sandbox = Sandbox::new();
    sandbox.add_exe("binary");
    sandbox.ok(&["params", "binary", "--add", "a"]);
    let output = sandbox.output(&["params", "binary", "--rm", "ghost"]);
    let document = sandbox.params("binary");
    assert!(
        output.status.success()
            && exact_line(&output, "ghost isn't a declared parameter; skipped.")
            && names(&document) == ["a"],
        "unknown remove must be a soft warning and preserve rows\nstatus={:?}\noutput={}\nstate={document}",
        output.status.code(),
        output_text(&output),
    );
}

#[test]
fn test_apply_order_is_rm_then_add_then_tweak() {
    let sandbox = Sandbox::new();
    sandbox.add_exe("binary");
    sandbox.ok(&["params", "binary", "--add", "a", "--type", "a=int"]);
    sandbox.ok(&[
        "params", "binary", "--rm", "a", "--add", "a", "--type", "a=float",
    ]);
    let document = sandbox.params("binary");
    assert_eq!(names(&document), ["a"]);
    assert_eq!(string(row(&document, "a"), "type"), "float");
}

#[test]
fn test_delivery_tweak_within_allowed_set() {
    let sandbox = Sandbox::new();
    sandbox.add_exe("binary");
    sandbox.ok(&["params", "binary", "--add", "a", "--deliver", "a=env"]);
    assert_eq!(
        string(row(&sandbox.params("binary"), "a"), "delivery"),
        "env"
    );
}

#[test]
fn test_delivery_outside_allowed_set_warns_bad_delivery() {
    let sandbox = Sandbox::new();
    sandbox.add_exe("binary");
    sandbox.ok(&["params", "binary", "--add", "a"]);
    let output = sandbox.output(&["params", "binary", "--deliver", "a=placeholder"]);
    let document = sandbox.params("binary");
    assert!(
        output.status.success()
            && exact_line(
                &output,
                "a: that delivery isn't available for this kind; skipped."
            )
            && string(row(&document, "a"), "delivery") == "flag",
        "invalid delivery must warn and roll back the row\nstatus={:?}\noutput={}\nstate={document}",
        output.status.code(),
        output_text(&output),
    );
}

#[test]
fn test_placeholder_delivery_on_a_non_placeholder_name_warns() {
    let sandbox = Sandbox::new();
    sandbox.add_command("template", "echo {other}");
    sandbox.ok(&["params", "template", "--add", "a", "--deliver", "a=env"]);
    let output = sandbox.output(&["params", "template", "--deliver", "a=placeholder"]);
    let document = sandbox.params("template");
    assert!(
        output.status.success()
            && exact_line(
                &output,
                "a isn't a template placeholder, so it can't use placeholder delivery; skipped."
            )
            && string(row(&document, "a"), "delivery") == "env",
        "non-placeholder delivery must warn and keep env\nstatus={:?}\noutput={}\nstate={document}",
        output.status.code(),
        output_text(&output),
    );
}

#[test]
fn test_placeholder_delivery_on_a_matching_placeholder_name_is_allowed() {
    let sandbox = Sandbox::new();
    sandbox.add_command("template", "echo {size}");
    sandbox.ok(&["params", "template", "--deliver", "size=env"]);
    sandbox.ok(&["params", "template", "--deliver", "size=placeholder"]);
    assert_eq!(
        string(row(&sandbox.params("template"), "size"), "delivery"),
        "placeholder"
    );
}

#[test]
fn test_type_tweak_valid() {
    let sandbox = Sandbox::new();
    sandbox.add_exe("binary");
    sandbox.ok(&["params", "binary", "--add", "a", "--type", "a=int"]);
    assert_eq!(string(row(&sandbox.params("binary"), "a"), "type"), "int");
}

#[test]
fn test_type_tweak_invalid_warns_bad_type() {
    let sandbox = Sandbox::new();
    sandbox.add_exe("binary");
    sandbox.ok(&["params", "binary", "--add", "a"]);
    let output = sandbox.output(&["params", "binary", "--type", "a=integer"]);
    let document = sandbox.params("binary");
    assert!(
        output.status.success()
            && exact_line(
                &output,
                "a: unknown type; skipped (use str, int, float, bool, choice, or path)."
            )
            && string(row(&document, "a"), "type") == "str",
        "bad type must warn and preserve the previous row\nstatus={:?}\noutput={}\nstate={document}",
        output.status.code(),
        output_text(&output),
    );
}

#[test]
fn test_choices_tweak_sets_the_tuple() {
    let sandbox = Sandbox::new();
    sandbox.add_exe("binary");
    sandbox.ok(&[
        "params",
        "binary",
        "--add",
        "a",
        "--type",
        "a=choice",
        "--choices",
        "a=x,y",
    ]);
    assert_eq!(
        row(&sandbox.params("binary"), "a")["choices"],
        serde_json::json!(["x", "y"])
    );
}

#[test]
fn test_default_coerced_to_the_declared_type() {
    let sandbox = Sandbox::new();
    sandbox.add_exe("binary");
    sandbox.ok(&[
        "params",
        "binary",
        "--add",
        "a",
        "--type",
        "a=int",
        "--default",
        "a=42",
    ]);
    assert_eq!(row(&sandbox.params("binary"), "a")["default"], 42);
}

#[test]
fn test_default_type_set_in_same_call_applies_before_coercion() {
    let sandbox = Sandbox::new();
    sandbox.add_exe("binary");
    sandbox.ok(&[
        "params",
        "binary",
        "--add",
        "a",
        "--type",
        "a=float",
        "--default",
        "a=1.5",
    ]);
    assert_eq!(row(&sandbox.params("binary"), "a")["default"], 1.5);
}

#[test]
fn test_default_bad_value_warns_bad_default_and_keeps_old() {
    let sandbox = Sandbox::new();
    sandbox.add_exe("binary");
    sandbox.ok(&[
        "params",
        "binary",
        "--add",
        "a",
        "--type",
        "a=int",
        "--default",
        "a=3",
    ]);
    let output = sandbox.output(&["params", "binary", "--default", "a=notanint"]);
    let document = sandbox.params("binary");
    assert!(
        output.status.success()
            && exact_line(&output, "a: the default doesn't fit its type; skipped.")
            && row(&document, "a")["default"] == 3,
        "bad default must warn and preserve old default\nstatus={:?}\noutput={}\nstate={document}",
        output.status.code(),
        output_text(&output),
    );
}

#[test]
fn test_flag_tweak_strips_and_sets_empty_for_positional() {
    let sandbox = Sandbox::new();
    sandbox.add_exe("binary");
    sandbox.ok(&["params", "binary", "--add", "a"]);
    sandbox.ok(&["params", "binary", "--flag", "a=  --out "]);
    let trimmed = string(row(&sandbox.params("binary"), "a"), "flag").to_owned();
    sandbox.ok(&["params", "binary", "--flag", "a="]);
    let empty = string(row(&sandbox.params("binary"), "a"), "flag").to_owned();
    assert_eq!((trimmed.as_str(), empty.as_str()), ("--out", ""));
}

#[test]
fn test_required_and_optional_tweaks() {
    let sandbox = Sandbox::new();
    sandbox.add_exe("binary");
    sandbox.ok(&["params", "binary", "--add", "a"]);
    sandbox.ok(&["params", "binary", "--required", "a"]);
    let required = boolean(row(&sandbox.params("binary"), "a"), "required");
    sandbox.ok(&["params", "binary", "--optional", "a"]);
    let optional = boolean(row(&sandbox.params("binary"), "a"), "required");
    assert_eq!((required, optional), (true, false));
}

#[test]
fn test_help_text_and_prompt_tweaks() {
    let sandbox = Sandbox::new();
    sandbox.add_exe("binary");
    sandbox.ok(&[
        "params",
        "binary",
        "--add",
        "a",
        "--help-text",
        "a=what it does",
        "--prompt",
        "a=A?",
    ]);
    let document = sandbox.params("binary");
    let a = row(&document, "a");
    assert_eq!(
        (string(a, "help"), string(a, "prompt")),
        ("what it does", "A?")
    );
}

#[test]
fn test_secret_and_env_source_together() {
    let sandbox = Sandbox::new();
    sandbox.add_exe("binary");
    sandbox.ok(&[
        "params",
        "binary",
        "--add",
        "tok",
        "--secret",
        "tok",
        "--env-source",
        "tok= API_TOKEN ",
    ]);
    let document = sandbox.params("binary");
    let tok = row(&document, "tok");
    assert!(boolean(tok, "secret"), "{tok}");
    assert_eq!(string(tok, "env_source"), "API_TOKEN", "{tok}");
}

#[test]
fn test_env_source_on_a_non_secret_param_warns_and_leaves_it_unset() {
    let sandbox = Sandbox::new();
    sandbox.add_exe("binary");
    sandbox.ok(&["params", "binary", "--add", "a"]);
    let output = sandbox.output(&["params", "binary", "--env-source", "a=VAR"]);
    let document = sandbox.params("binary");
    let expected = "a isn't secret; --env-source only applies to secret parameters (mark it with --secret first).";
    assert!(
        output.status.success()
            && exact_line(&output, expected)
            && string(row(&document, "a"), "env_source").is_empty(),
        "env-source on public row must warn and remain unset\nstatus={:?}\noutput={}\nstate={document}",
        output.status.code(),
        output_text(&output),
    );
}

#[test]
fn test_no_secret_clears_the_env_source() {
    let sandbox = Sandbox::new();
    sandbox.add_exe("binary");
    sandbox.ok(&[
        "params",
        "binary",
        "--add",
        "tok",
        "--secret",
        "tok",
        "--env-source",
        "tok=API_TOKEN",
    ]);
    sandbox.ok(&["params", "binary", "--no-secret", "tok"]);
    let document = sandbox.params("binary");
    let tok = row(&document, "tok");
    assert!(!boolean(tok, "secret"), "{tok}");
    assert_eq!(string(tok, "env_source"), "", "{tok}");
}

#[test]
fn test_tweak_on_unknown_name_warns_not_declared() {
    let sandbox = Sandbox::new();
    sandbox.add_exe("binary");
    sandbox.ok(&["params", "binary", "--add", "a"]);
    let output = sandbox.output(&["params", "binary", "--type", "ghost=int"]);
    let document = sandbox.params("binary");
    assert!(
        output.status.success()
            && exact_line(&output, "ghost isn't a declared parameter; skipped.")
            && names(&document) == ["a"],
        "unknown tweak must be a soft warning and preserve rows\nstatus={:?}\noutput={}\nstate={document}",
        output.status.code(),
        output_text(&output),
    );
}

#[test]
fn test_a_name_touched_by_two_ops_is_listed_once_and_both_apply() {
    let sandbox = Sandbox::new();
    sandbox.add_exe("binary");
    sandbox.ok(&["params", "binary", "--add", "a"]);
    sandbox.ok(&[
        "params",
        "binary",
        "--type",
        "a=int",
        "--default",
        "a=5",
        "--secret",
        "a",
        "--prompt",
        "a=A?",
    ]);
    let document = sandbox.params("binary");
    let a = row(&document, "a");
    assert_eq!(string(a, "type"), "int");
    assert_eq!(a["default"], 5);
    assert!(boolean(a, "secret"), "{a}");
    assert_eq!(string(a, "prompt"), "A?");
    assert_eq!(names(&document), ["a"]);
}

#[test]
fn test_choice_type_without_choices_reverts_and_warns() {
    let sandbox = Sandbox::new();
    sandbox.add_exe("binary");
    sandbox.ok(&["params", "binary", "--add", "a", "--help-text", "a=keep me"]);
    let output = sandbox.output(&[
        "params",
        "binary",
        "--type",
        "a=choice",
        "--help-text",
        "a=changed",
    ]);
    let document = sandbox.params("binary");
    let a = row(&document, "a");
    assert!(
        output.status.success()
            && exact_line(
                &output,
                "a: a choice parameter needs choices; set --choices a=a,b,c."
            )
            && string(a, "type") == "str"
            && string(a, "help") == "keep me",
        "invalid choice edit must roll back the whole row\nstatus={:?}\noutput={}\nstate={a}",
        output.status.code(),
        output_text(&output),
    );
}

#[test]
fn test_choice_type_with_choices_in_the_same_call_is_valid() {
    let sandbox = Sandbox::new();
    sandbox.add_exe("binary");
    sandbox.ok(&[
        "params",
        "binary",
        "--add",
        "a",
        "--type",
        "a=choice",
        "--choices",
        "a=r,g",
    ]);
    let document = sandbox.params("binary");
    let a = row(&document, "a");
    assert_eq!(string(a, "type"), "choice");
    assert_eq!(a["choices"], serde_json::json!(["r", "g"]));
}

#[test]
fn rust_additive_cli_edit_of_one_row_does_not_mutate_sibling_row() {
    // Rust has no caller-owned in-memory `edit_declared(list)` API. The executable behavioral
    // equivalent is that an edit addressed to `a` must not mutate the full persisted machine row
    // owned by sibling `b`. Forward-compatible metadata preservation is a separate storage
    // contract and is tested elsewhere; adding it here would manufacture a stronger, unrelated
    // failure and no longer represent this Python test.
    let sandbox = Sandbox::new();
    sandbox.add_exe("binary");
    sandbox.ok(&[
        "params", "binary", "--add", "a", "--add", "b", "--type", "b=int", "--prompt", "b=orig",
    ]);
    let before = sandbox.params("binary");
    let b_before = row(&before, "b").clone();

    sandbox.ok(&["params", "binary", "--prompt", "a=changed", "--secret", "a"]);
    let after = sandbox.params("binary");
    assert_eq!(
        row(&after, "b"),
        &b_before,
        "an edit addressed to a must not mutate sibling b"
    );
}
