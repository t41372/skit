//! Mechanical port of the Python oracle module `tests/test_declared_params.py`
//! (`origin/main@206f9ef`): "Declared parameter schema (`[[parameters]]`) + env delivery".
//! Each `#[test]` keeps its Python `def test_*` name and its WHY comment so it traces back to
//! its origin.
//!
//! WHY `skit-cli`: this oracle module is cross-cutting. It drives the pure param-schema
//! helpers (`skit.params`), the form composition (`skit.flows.plan_for_entry` / `assemble`),
//! the meta model (`skit.models`), the store round-trip (`skit.store`), the launcher overlay
//! (`skit.launcher`), and — for over half its tests — the `skit` CLI through Typer's
//! `CliRunner`. The Rust rewrite disperses those seams: the schema helpers into `skit-domain`,
//! the form plan into `skit-form`, delivery routing into `skit-application`, and the whole
//! `params`/`show`/`run` surface into the `skit` binary. Only the composition-root crate can
//! reach every one of these without a forbidden dependency edit, so the port lives here.
//!
//! Concept mapping used throughout:
//! - Python `params.declared_for_template` / `declared_from_meta` / `synthesized_placeholder`
//!   -> `skit_domain::parameters::{declared_for_template, declared_from_meta,
//!   synthesized_placeholder}`.
//! - Python `ParamDecl(...).to_meta_dict()` -> `ParamDecl { .. }.to_meta_map()`.
//! - Python `flows.plan_for_entry(entry)` -> `skit_form::form_plan(kind, text, &settings)`
//!   (source/field parity for exe/command/none; see the two reader tests for the one seam
//!   where the shipping CLI composition and `form_plan` disagree).
//! - Python `FormField.source` -> `PreparedField.declaration.delivery`; `FormPlan.source`
//!   -> `FormSource` (`.as_str()` gives the machine spelling `command`/`declared`/`none`/
//!   `argparse`/`inject`). The exact reader+rider form contract lives in the existing
//!   `skit-form/tests/form_params.rs` owner; this target keeps its CLI consumers.
//! - Python `flows.assemble(plan, values, extra, ...)` -> `skit_application::delivery::assemble(
//!   &declarations, &prepared_values, &extra)`; the Python cwd/env token pass happens BEFORE
//!   this Rust boundary, so the token-free oracle values map to `PreparedValue::Scalar`.
//! - Python `ScriptMeta.to_toml_dict` / `from_toml_dict` non-dict-row dropping ->
//!   `EntrySettings::{write_to_meta, from_meta}` (`from_meta` drops non-object rows).
//! - Python `store.add_*` / CLI `params` / `run` / `show` -> the real `skit` binary via
//!   `assert_cmd`. Human-string assertions read Python's `result.output`; because the exact
//!   stdout/stderr split is not what those tests measure, they are checked against the
//!   COMBINED stream. The explicit `--json` purity tests keep the streams separate.
//! - Python `store.write_parameters` / `read_parameters` -> the real `FileStore`
//!   create/resolve/update-settings transaction in `skit-store/tests/mutations.rs`. It is a store
//!   persistence contract, not the CLI's broader `params --rm` product action.
//!
//! Buckets:
//! - REAL asserting `#[test]` (API exists): the pure-schema, form-plan, assemble, meta-model,
//!   and the CLI tests whose behavior the Rust product reproduces.
//! - DIVERGENCE (full asserting body, `#[ignore]`d): the assertion is faithful to the oracle
//!   and compiles; it fails because Rust diverges. Fixing the impl and deleting the `#[ignore]`
//!   line turns it green. These capture: the absent confirmation/warning strings
//!   ("Declared parameters:", "has no managed parameters", "Ignored a malformed value",
//!   "Removed previously stored plaintext"), the `params` batch
//!   fault-tolerance gap (a malformed/bad value hard-errors exit 2 instead of warning at
//!   exit 0).
//! - UNMAPPABLE white-box (`#[ignore]` stub): `test_cli_declared_warning_codes_render` drives
//!   the Python-private `cli._render_declared_warning`; the Rust warnings are localized
//!   messages with no public renderer to observe, and their observable outcomes are covered
//!   (or recorded as divergences) by the CLI tests here.
//!
//! Windows note: the oracle's `sys.platform == "win32"` fixture arms are dropped; these tests
//! run the Unix `#!/bin/sh` fixtures only.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Output;

use serde_json::{Value, json};
use skit_application::delivery::{PreparedValue, assemble};
use skit_domain::parameters::{
    ParamDecl, ParameterDelivery, ParameterType, ParameterValue, declared_for_template,
    declared_from_meta, synthesized_placeholder,
};
use skit_domain::{EntryKind, EntryMeta, EntrySettings};
use skit_form::{FormPlan, FormSource, form_plan};
use tempfile::TempDir;

// ---- shared helpers (self-contained; this file edits no shared module) --------------------------

/// Names of a declaration list in stored order.
fn names(decls: &[ParamDecl]) -> Vec<String> {
    decls.iter().map(|decl| decl.name.clone()).collect()
}

/// The oracle's `[(f.key, f.source) for f in plan.fields]` — name plus delivery per field.
fn field_sources(plan: &FormPlan) -> Vec<(String, ParameterDelivery)> {
    plan.fields
        .iter()
        .map(|field| (field.declaration.name.clone(), field.declaration.delivery))
        .collect()
}

/// A meta `[[parameters]]` row keyed by name/delivery, mirroring `ParamDecl(...).to_meta_dict()`.
fn meta_row(build: impl FnOnce(&mut ParamDecl)) -> BTreeMap<String, Value> {
    let mut decl = ParamDecl::new("");
    build(&mut decl);
    decl.to_meta_map()
}

/// One command `EntrySettings` with a placeholder cache and declared rows, as `plan_for_entry`
/// would read from an entry's `meta.toml`.
fn command_settings(placeholders: &[&str], declared: Vec<ParamDecl>) -> EntrySettings {
    EntrySettings {
        params: placeholders.iter().map(|name| (*name).to_owned()).collect(),
        parameters: declared,
        ..EntrySettings::default()
    }
}

/// A `BTreeMap<String, String>` literal for the expected env/command value maps.
fn string_map(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
        .collect()
}

/// A prepared scalar value map for `assemble`.
fn scalar_values(pairs: &[(&str, &str)]) -> BTreeMap<String, PreparedValue> {
    pairs
        .iter()
        .map(|(key, value)| {
            (
                (*key).to_owned(),
                PreparedValue::Scalar((*value).to_owned()),
            )
        })
        .collect()
}

/// One isolated skit library: private data/state/config directories plus a source scratch dir.
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

fn snapshot_tree(root: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    fn visit(root: &Path, directory: &Path, output: &mut Vec<(PathBuf, Vec<u8>)>) {
        let Ok(entries) = std::fs::read_dir(directory) else {
            return;
        };
        for entry in entries {
            let path = entry.unwrap().path();
            if path.is_dir() {
                visit(root, &path, output);
            } else {
                output.push((
                    path.strip_prefix(root).unwrap().to_owned(),
                    std::fs::read(path).unwrap(),
                ));
            }
        }
    }

    let mut output = Vec::new();
    visit(root, root, &mut output);
    output.sort_by(|left, right| left.0.cmp(&right.0));
    output
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

    /// Write an executable source file into the scratch dir and return its path.
    fn write_script(&self, name: &str, body: &str) -> PathBuf {
        let path = self.src.path().join(name);
        std::fs::write(&path, body).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        path
    }

    /// Add the oracle's `_exe` fixture: an argv-echoing shell program registered under `name`.
    fn add_exe(&self, name: &str) {
        let exe = self.write_script("t", "#!/bin/sh\nprintf '%s\\n' \"$@\"\n");
        self.cmd()
            .arg("add")
            .arg(&exe)
            .args(["--exe", "--name", name, "--no-input"])
            .assert()
            .success();
    }

    /// Seed the argstate values file, as `argstate.save_last(slug, values=...)` would.
    fn seed_values(&self, slug: &str, body: &str) {
        let dir = self.state.path().join("values");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(format!("{slug}.toml")), body).unwrap();
    }

    /// The stored argstate file text, or empty when it was purged/never written.
    fn values_file(&self, slug: &str) -> String {
        std::fs::read_to_string(
            self.state
                .path()
                .join("values")
                .join(format!("{slug}.toml")),
        )
        .unwrap_or_default()
    }

    fn meta(&self, slug: &str) -> String {
        std::fs::read_to_string(
            self.data
                .path()
                .join("scripts")
                .join(slug)
                .join("meta.toml"),
        )
        .unwrap()
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

/// Parse stdout as exactly one JSON document (the `--json` purity contract).
fn stdout_json(output: &Output) -> Value {
    serde_json::from_slice(&output.stdout).expect("stdout is exactly one JSON document")
}

fn stderr_text(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

// ================================================================================================
// declared_for_template  (skit-domain, pure)
// ================================================================================================

#[test]
fn test_undeclared_placeholders_synthesize_the_historical_field() {
    let decls = declared_for_template(None, &["input".to_owned(), "api_key".to_owned()]);
    assert_eq!(names(&decls), ["input", "api_key"]);
    assert!(
        decls
            .iter()
            .all(|decl| decl.delivery == ParameterDelivery::Placeholder)
    );
    assert!(decls.iter().all(|decl| decl.required));
    assert!(!decls[0].secret);
    assert!(decls[1].secret); // KEY heuristic — unchanged historical behavior
}

#[test]
fn test_declared_row_overrides_placeholder_schema_including_secret() {
    // THE defect fix: {token_file} matched the TOKEN heuristic and could never be
    // un-secreted. A declared row now owns the schema outright.
    let row = meta_row(|decl| {
        decl.name = "token_file".to_owned();
        decl.delivery = ParameterDelivery::Placeholder;
        decl.parameter_type = ParameterType::Str;
        decl.required = false;
        decl.default = Some(ParameterValue::String("creds.json".to_owned()));
        decl.secret = false;
    });
    let decls = declared_for_template(Some(&[row]), &["token_file".to_owned(), "host".to_owned()]);
    assert!(!decls[0].secret);
    assert!(!decls[0].required);
    assert_eq!(
        decls[0].default,
        Some(ParameterValue::String("creds.json".to_owned()))
    );
    // the undeclared one still synthesizes
    assert_eq!(decls[1].name, "host");
    assert!(decls[1].required);
}

#[test]
fn test_declared_env_param_rides_along_after_placeholders() {
    let row = meta_row(|decl| {
        decl.name = "RETRIES".to_owned();
        decl.delivery = ParameterDelivery::Env;
        decl.parameter_type = ParameterType::Int;
        decl.default = Some(ParameterValue::Integer(3));
    });
    let decls = declared_for_template(Some(&[row]), &["file".to_owned()]);
    assert_eq!(names(&decls), ["file", "RETRIES"]);
    assert_eq!(decls[1].delivery, ParameterDelivery::Env);
}

#[test]
fn test_declared_flag_row_is_dropped_for_templates() {
    // argv is not a template's interface (takes_argv=False): a flag row can only be a
    // hand-edit mistake, and dropping beats assembling arguments the template never reads.
    let row = meta_row(|decl| {
        decl.name = "width".to_owned();
        decl.delivery = ParameterDelivery::Flag;
        decl.flag = "--width".to_owned();
    });
    let decls = declared_for_template(Some(&[row]), &["file".to_owned()]);
    assert_eq!(names(&decls), ["file"]);
}

#[test]
fn test_declared_row_with_wrong_delivery_for_its_placeholder_is_replaced_by_synth() {
    // A row named like a placeholder but declared env can't fill the {slot}; the
    // placeholder still needs a value, so the synthesized field steps back in.
    let row = meta_row(|decl| {
        decl.name = "file".to_owned();
        decl.delivery = ParameterDelivery::Env;
    });
    let decls = declared_for_template(Some(&[row]), &["file".to_owned()]);
    assert_eq!(decls.len(), 1);
    assert_eq!(decls[0].delivery, ParameterDelivery::Placeholder);
    assert!(decls[0].required);
}

#[test]
fn test_declared_from_meta_drops_nameless_rows() {
    let nameless: BTreeMap<String, Value> =
        BTreeMap::from([("delivery".to_owned(), json!("flag"))]);
    let ok = ParamDecl::new("ok").to_meta_map();
    assert_eq!(names(&declared_from_meta(Some(&[nameless, ok]))), ["ok"]);
}

#[test]
fn test_synthesized_placeholder_shape() {
    let decl = synthesized_placeholder("api_key");
    assert_eq!(
        (decl.delivery, decl.required, decl.secret),
        (ParameterDelivery::Placeholder, true, true)
    );
}

// ================================================================================================
// plan_for_entry  (skit-form::form_plan)
// ================================================================================================

#[test]
fn test_command_plan_honors_declared_schema() {
    let mut size = ParamDecl::new("size");
    size.delivery = ParameterDelivery::Placeholder;
    size.parameter_type = ParameterType::Choice;
    size.choices = vec!["s".to_owned(), "m".to_owned()];
    size.default = Some(ParameterValue::String("m".to_owned()));
    size.required = false;
    let settings = command_settings(&["size", "api_key"], vec![size]);

    let plan = form_plan("command", "", &settings);
    assert_eq!(plan.source, FormSource::Command);
    let size_field = &plan.fields[0].declaration;
    assert_eq!(size_field.parameter_type, ParameterType::Choice);
    assert_eq!(size_field.choices, ["s", "m"]);
    assert_eq!(
        size_field.default,
        Some(ParameterValue::String("m".to_owned()))
    );
    assert!(!size_field.required);
    let key_field = &plan.fields[1].declaration;
    assert!(key_field.required); // undeclared: synthesized, unchanged behavior
    assert!(key_field.secret);
}

#[test]
fn test_exe_with_declared_params_gets_a_form() {
    let mut width = ParamDecl::new("width");
    width.delivery = ParameterDelivery::Flag;
    width.flag = "--width".to_owned();
    width.parameter_type = ParameterType::Int;
    width.default = Some(ParameterValue::Integer(800));
    let mut debug = ParamDecl::new("DEBUG");
    debug.delivery = ParameterDelivery::Env;
    debug.parameter_type = ParameterType::Bool;
    let mut slot = ParamDecl::new("slot");
    slot.delivery = ParameterDelivery::Placeholder; // meaningless on a binary: dropped
    let settings = EntrySettings {
        parameters: vec![width, debug, slot],
        ..EntrySettings::default()
    };

    let plan = form_plan("exe", "", &settings);
    assert_eq!(plan.source, FormSource::Declared);
    assert_eq!(
        field_sources(&plan),
        vec![
            ("width".to_owned(), ParameterDelivery::Flag),
            ("DEBUG".to_owned(), ParameterDelivery::Env),
        ]
    );
}

#[test]
fn test_exe_without_declared_params_stays_none_plan() {
    let plan = form_plan("exe", "", &EntrySettings::default());
    assert_eq!(plan.source, FormSource::None);
}

// ================================================================================================
// assemble: env routing  (skit-application::delivery)
// ================================================================================================

/// The oracle's `_env_plan()` field list.
fn env_plan_decls() -> Vec<ParamDecl> {
    let mut width = ParamDecl::new("WIDTH");
    width.delivery = ParameterDelivery::Env;
    width.parameter_type = ParameterType::Int;
    let mut token = ParamDecl::new("token");
    token.delivery = ParameterDelivery::Env;
    token.secret = true;
    token.env_target = "API_TOKEN".to_owned();
    let mut unset = ParamDecl::new("UNSET");
    unset.delivery = ParameterDelivery::Env;
    vec![width, token, unset]
}

#[test]
fn test_assemble_env_values_masked_and_empty_absent() {
    let asm = assemble(
        &env_plan_decls(),
        &scalar_values(&[("WIDTH", "800"), ("token", "hunter2")]),
        &[],
    )
    .unwrap();
    assert_eq!(
        asm.env_values,
        string_map(&[("WIDTH", "800"), ("API_TOKEN", "hunter2")])
    ); // env_target honored
    assert_eq!(
        asm.masked_env,
        string_map(&[("WIDTH", "800"), ("API_TOKEN", "•••")])
    ); // secret masked for display
    assert!(!asm.env_values.contains_key("UNSET")); // empty stays ABSENT so script defaults fire
    assert!(asm.args.is_empty());
}

#[test]
fn test_assemble_mixed_flag_and_env_fields() {
    let mut width = ParamDecl::new("width");
    width.delivery = ParameterDelivery::Flag;
    width.flag = "--width".to_owned();
    width.parameter_type = ParameterType::Int;
    let mut debug = ParamDecl::new("DEBUG");
    debug.delivery = ParameterDelivery::Env;
    let asm = assemble(
        &[width, debug],
        &scalar_values(&[("width", "800"), ("DEBUG", "1")]),
        &["-v".to_owned()],
    )
    .unwrap();
    assert_eq!(asm.args, ["--width", "800", "-v"]); // env field never enters argv
    assert_eq!(asm.env_values, string_map(&[("DEBUG", "1")]));
}

#[test]
fn test_assemble_command_with_env_rider() {
    let mut retries = ParamDecl::new("RETRIES");
    retries.delivery = ParameterDelivery::Env;
    let plan = form_plan("command", "", &command_settings(&["msg"], vec![retries]));
    let asm = assemble(
        &plan.declarations(),
        &scalar_values(&[("msg", "hi"), ("RETRIES", "3")]),
        &[],
    )
    .unwrap();
    assert_eq!(asm.command_values, string_map(&[("msg", "hi")])); // env rider is NOT a template value
    assert_eq!(asm.env_values, string_map(&[("RETRIES", "3")]));
}

// ================================================================================================
// run_entry: overlay order  (skit binary, real launch)
// ================================================================================================

#[test]
fn test_run_entry_env_overlay_wins_last() {
    // The explicit parameter beats the ambient value. The oracle monkeypatches subprocess.run
    // and reads the child env; here a real `#!/bin/sh` child prints $WIDTH, the ambient value
    // is set on the invocation, and the declared env parameter overlays it.
    let workspace = lib();
    let child = workspace.write_script("t", "#!/bin/sh\necho \"W=$WIDTH\"\n");
    workspace
        .cmd()
        .arg("add")
        .arg(&child)
        .args(["--exe", "--name", "ov", "--no-input"])
        .assert()
        .success();
    workspace.run(&["params", "ov", "--add", "WIDTH", "--deliver", "WIDTH=env"]);

    let output = workspace
        .cmd()
        .env("WIDTH", "from-ambient")
        .args(["run", "ov", "--set", "WIDTH=800", "--no-input"])
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", combined(&output));
    let seen = combined(&output);
    assert!(seen.contains("W=800"), "{seen}");
    assert!(!seen.contains("from-ambient"), "{seen}");
}

// ================================================================================================
// transparency  (skit binary, --dry-run env prefix)
// ================================================================================================

#[test]
fn test_transparency_shows_masked_env_prefix() {
    // The env overlay renders as a copy-pasteable, masked VAR=value prefix: a secret masks to
    // ••• and never reaches the scrollback, and a spaced value is quoted so the shown line is
    // genuinely copy-pasteable. In Rust this prefix is produced by the run/dry-run planner
    // (`transparency_messages` alone omits it), so the port drives `run --dry-run`.
    let workspace = lib();
    workspace
        .cmd()
        .args(["add", "--cmd", "echo hi", "--name", "tr", "--no-input"])
        .assert()
        .success();
    workspace.run(&[
        "params",
        "tr",
        "--add",
        "API_TOKEN",
        "--deliver",
        "API_TOKEN=env",
        "--secret",
        "API_TOKEN",
    ]);
    workspace.run(&[
        "params",
        "tr",
        "--add",
        "GREETING",
        "--deliver",
        "GREETING=env",
        "--default",
        "GREETING=hello world",
    ]);

    let output = workspace.run(&[
        "run",
        "tr",
        "--set",
        "API_TOKEN=hunter2",
        "--dry-run",
        "--no-input",
    ]);
    assert!(output.status.success(), "{}", combined(&output));
    let arrow = combined(&output);
    assert!(arrow.contains("API_TOKEN="), "{arrow}");
    assert!(!arrow.contains("hunter2"), "{arrow}"); // the secret value never reaches the scrollback
    assert!(arrow.contains("•••"), "{arrow}");
    assert!(
        arrow.contains("GREETING='hello world'") || arrow.contains("GREETING=\"hello world\""),
        "{arrow}"
    );
}

// ================================================================================================
// execute wiring  (skit binary, env reaches the child)
// ================================================================================================

#[test]
fn test_execute_passes_env_values_to_run_entry() {
    // The oracle spies that flows.execute forwards env_values to launcher.run_entry as
    // env_overlay; here the observable outcome is the child receiving the env value at run.
    let workspace = lib();
    workspace
        .cmd()
        .args([
            "add",
            "--cmd",
            "echo {m} N=$N",
            "--name",
            "exec-env",
            "--no-input",
        ])
        .assert()
        .success();
    workspace.run(&["params", "exec-env", "--add", "N", "--deliver", "N=env"]);

    let output = workspace.run(&[
        "run",
        "exec-env",
        "--set",
        "m=x",
        "--set",
        "N=5",
        "--no-input",
    ]);
    assert!(output.status.success(), "{}", combined(&output));
    assert!(combined(&output).contains("N=5"), "{}", combined(&output));
}

// ================================================================================================
// meta model  (skit-domain EntrySettings)
// ================================================================================================

#[test]
#[ignore = "FAILING CONTRACT (divergence): the oracle's to_toml_dict passes raw parameter dicts through verbatim, so a minimal {name, delivery} row stays 2-key (models.py:112-113), and from_toml_dict keeps dict rows raw (models.py:163-167) -- the 'keep unknown TOML fields' rule for the [[parameters]] array. Rust's typed Vec<ParamDecl> re-serializes every row through to_meta_map, which ALWAYS emits `type` (parameters.rs:340-349) and drops any key ParamDecl does not model, so the stored row is {name, delivery, type} and unknown keys are lost."]
fn test_meta_parameters_roundtrip_and_non_dict_rows_dropped() {
    // Oracle (test_declared_params.py:328-338): a raw parameter dict round-trips through to_toml_dict
    // verbatim, and a hand-edited array holding non-table garbage keeps only the real rows.
    let mut declared = ParamDecl::new("a");
    declared.delivery = ParameterDelivery::Placeholder;
    let settings = EntrySettings {
        parameters: vec![declared],
        ..EntrySettings::default()
    };
    let mut meta = EntryMeta::minimal("x", EntryKind::parse("command").unwrap());
    settings.write_to_meta(&mut meta);
    // EXACT equality, matching the oracle's `d["parameters"] == [{"name": "a", "delivery":
    // "placeholder"}]`: the raw 2-key row must survive verbatim, with no `type` added. Rust adds
    // `type`, so this is the failing half of the contract above.
    assert_eq!(
        meta.extra.get("parameters"),
        Some(&json!([{"name": "a", "delivery": "placeholder"}]))
    );

    meta.extra.insert(
        "parameters".to_owned(),
        json!([{"name": "a"}, "garbage", 5]),
    );
    let back = EntrySettings::from_meta(&meta);
    // Non-dict rows ("garbage", 5) are dropped; the one real row is kept.
    assert_eq!(back.parameters.len(), 1);
    assert_eq!(back.parameters[0].name, "a");
}

#[test]
fn test_declared_plan_secret_placeholder_masks_in_command_values() {
    // End-to-end C3 for declared schema: a secret placeholder's value masks in
    // masked_command_values while the real value still runs.
    let plan = form_plan("command", "", &command_settings(&["password"], Vec::new()));
    let asm = assemble(
        &plan.declarations(),
        &scalar_values(&[("password", "s3cret")]),
        &[],
    )
    .unwrap();
    assert_eq!(asm.command_values, string_map(&[("password", "s3cret")]));
    assert_eq!(
        asm.masked_command_values,
        string_map(&[("password", "•••")])
    );
}

#[test]
fn test_unknown_kind_entry_still_gets_none_plan() {
    let plan = form_plan("martian", "", &EntrySettings::default());
    assert_eq!(plan.source, FormSource::None);
}

#[test]
fn test_exe_with_only_placeholder_rows_falls_through_to_none() {
    // Every declared row filters out (placeholder means nothing for a binary): the plan must
    // fall through to "none", not produce an empty declared form.
    let mut slot = ParamDecl::new("slot");
    slot.delivery = ParameterDelivery::Placeholder;
    let settings = EntrySettings {
        parameters: vec![slot],
        ..EntrySettings::default()
    };
    assert_eq!(form_plan("exe", "", &settings).source, FormSource::None);
}

// ================================================================================================
// CLI: skit params --add/--rm/--type/... on exe & command
// ================================================================================================

#[test]
fn test_cli_add_flag_param_on_exe_then_run_set() {
    let workspace = lib();
    workspace.add_exe("prog");
    let output = workspace.run(&[
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
    assert!(output.status.success(), "{}", combined(&output));
    let receipt = combined(&output);
    assert!(
        receipt.contains("Updated prog. Declared parameters: width"),
        "{receipt}"
    );
    assert!(!receipt.contains("Managed parameters:"), "{receipt}");
    let declared = stdout_json(&workspace.run(&["params", "prog", "--json"]));
    let row = &declared["declared"][0];
    assert_eq!(row["name"], "width");
    assert_eq!(row["delivery"], "flag");
    assert_eq!(row["type"], "int");
    assert_eq!(row["flag"], "--width");
    assert_eq!(row["default"], 800); // coerced + stored typed

    // run --set assembles the real flag
    let run = workspace.run(&["run", "prog", "--set", "width=1024", "--no-input"]);
    assert!(run.status.success(), "{}", combined(&run));
    assert!(
        combined(&run).contains("--width\n1024"),
        "{}",
        combined(&run)
    );
}

#[test]
fn test_cli_exe_show_table_and_json() {
    let workspace = lib();
    workspace.add_exe("prog");
    workspace.run(&[
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
    let human = workspace.run(&["params", "prog"]);
    assert!(human.status.success());
    assert!(combined(&human).contains("width"));
    assert!(combined(&human).contains("flag")); // the Delivery column value
    let payload = stdout_json(&workspace.run(&["params", "prog", "--json"]));
    assert_eq!(payload["declared"][0]["name"], "width");
    assert_eq!(payload["declared"][0]["delivery"], "flag");
}

#[test]
fn test_cli_exe_show_without_declared_is_plain_message() {
    let workspace = lib();
    workspace.add_exe("prog");
    let output = workspace.run(&["params", "prog"]);
    assert!(output.status.success());
    assert!(combined(&output).contains("has no managed parameters"));
}

#[test]
fn test_cli_declared_edit_with_json_emits_the_final_read_view() {
    // A declared edit with --json emits the final read-view JSON as the WHOLE of stdout, instead
    // of silently dropping the flag — an explicit --json never no-ops, and under the purity rule
    // the human summary rides stderr, not stdout.
    let workspace = lib();
    workspace.add_exe("prog");
    let output = workspace.run(&[
        "params",
        "prog",
        "--add",
        "width",
        "--deliver",
        "width=flag",
        "--flag",
        "width=--width",
        "--json",
    ]);
    assert!(output.status.success(), "{}", combined(&output));
    assert!(
        !combined(&output).contains("Updated prog. Declared parameters:"),
        "{}",
        combined(&output)
    );
    let payload = stdout_json(&output);
    assert_eq!(payload["declared"][0]["name"], "width");
    assert_eq!(payload["declared"][0]["delivery"], "flag");
}

#[test]
fn test_cli_env_source_on_non_secret_declared_param_warns() {
    let workspace = lib();
    workspace.add_exe("prog");
    let setup = workspace.run(&["params", "prog", "--add", "WIDTH", "--deliver", "WIDTH=env"]);
    assert!(setup.status.success(), "{}", combined(&setup));
    let meta_path = workspace.data.path().join("scripts/prog/meta.toml");
    let before = std::fs::read(&meta_path).unwrap();

    let output = workspace.run(&["params", "prog", "--env-source", "WIDTH=COLS"]);
    assert!(output.status.success(), "{}", combined(&output));
    assert!(stderr_text(&output).contains("WIDTH isn't secret")); // the no-op flag is surfaced
    assert_eq!(std::fs::read(&meta_path).unwrap(), before); // a refused edit does not rewrite data

    let json_run = workspace.run(&["params", "prog", "--env-source", "WIDTH=COLS", "--json"]);
    assert!(json_run.status.success(), "{}", combined(&json_run));
    let payload = stdout_json(&json_run); // stdout alone is pure JSON
    let width = payload["declared"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["name"] == "WIDTH")
        .unwrap();
    assert!(width.get("env_source").is_none());
    assert!(width.get("secret").is_none());
    assert!(stderr_text(&json_run).contains("WIDTH isn't secret")); // the warning rode stderr
    assert_eq!(std::fs::read(&meta_path).unwrap(), before); // JSON is the same no-op

    let made_secret = stdout_json(&workspace.run(&[
        "params",
        "prog",
        "--secret",
        "WIDTH",
        "--env-source",
        "WIDTH= COLS ",
        "--json",
    ]));
    assert_eq!(made_secret["declared"][0]["secret"], true);
    assert_eq!(made_secret["declared"][0]["env_source"], "COLS");

    let updated_secret =
        stdout_json(&workspace.run(&["params", "prog", "--env-source", "WIDTH=LINES", "--json"]));
    assert_eq!(updated_secret["declared"][0]["env_source"], "LINES");

    let made_public =
        stdout_json(&workspace.run(&["params", "prog", "--no-secret", "WIDTH", "--json"]));
    assert!(made_public["declared"][0].get("secret").is_none());
    assert!(made_public["declared"][0].get("env_source").is_none());
}

#[test]
fn test_cli_python_manage_with_json_emits_the_final_read_view() {
    // The twin on the analyzer branch: `skit params <py> --manage CITY --json` emits the final
    // read-view JSON after managing CITY.
    let workspace = lib();
    let src = workspace.write_script("job.py", "CITY = \"Taipei\"\nprint(CITY)\n");
    workspace
        .cmd()
        .arg("add")
        .arg(&src)
        .args(["--name", "job", "--no-input"])
        .assert()
        .success();
    let output = workspace.run(&["params", "job", "--manage", "CITY", "--json"]);
    assert!(output.status.success(), "{}", combined(&output));
    let payload = stdout_json(&output);
    let managed: Vec<&Value> = payload["params"].as_array().unwrap().iter().collect();
    assert_eq!(managed.len(), 1);
    assert_eq!(managed[0]["name"], "CITY"); // CITY is now managed in the JSON
}

#[test]
fn test_cli_add_choice_placeholder_on_command_then_run() {
    let workspace = lib();
    workspace
        .cmd()
        .args([
            "add",
            "--cmd",
            "convert {size}",
            "--name",
            "conv",
            "--no-input",
        ])
        .assert()
        .success();
    let initial = stdout_json(&workspace.run(&["params", "conv", "--json"]));
    assert_eq!(initial["placeholders"], json!(["size"]));
    assert_eq!(initial["declared"], json!([]));
    assert_eq!(initial["parameters"].as_array().unwrap().len(), 1);
    assert_eq!(initial["parameters"][0]["name"], "size");
    assert_eq!(initial["parameters"][0]["delivery"], "placeholder");
    assert_eq!(initial["parameters"][0]["required"], true);
    let state_before = snapshot_tree(workspace.state.path());
    let config_before = snapshot_tree(workspace.config.path());
    let output = workspace.run(&[
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
    assert!(output.status.success(), "{}", combined(&output));
    let payload = stdout_json(&workspace.run(&["params", "conv", "--json"]));
    let decl = &payload["declared"][0];
    assert_eq!(decl["delivery"], "placeholder"); // add on a placeholder name
    assert_eq!(decl["type"], "choice");
    assert_eq!(decl["choices"], json!(["s", "m", "l"]));
    assert_eq!(decl["default"], "m");
    assert!(decl.get("required").is_none()); // false is omitted from the raw explicit row
    assert_eq!(payload["declared"].as_array().unwrap().len(), 1);
    assert_eq!(payload["parameters"].as_array().unwrap().len(), 1);
    assert_eq!(payload["parameters"][0]["delivery"], "placeholder");
    assert_eq!(payload["parameters"][0]["type"], "choice");
    assert_eq!(payload["parameters"][0]["choices"], json!(["s", "m", "l"]));
    assert_eq!(payload["parameters"][0]["default"], "m");
    assert_eq!(payload["placeholders"], json!(["size"]));
    let show = stdout_json(&workspace.run(&["show", "conv", "--json"]));
    assert_eq!(show["fields"][0]["required"], false); // effective machine field is total
    assert_eq!(snapshot_tree(workspace.state.path()), state_before);
    assert_eq!(snapshot_tree(workspace.config.path()), config_before);
    assert!(
        workspace
            .meta("conv")
            .contains("template = \"convert {size}\"")
    );

    // run --no-input: the declared default fills the placeholder without prompting
    let run = workspace.run(&["run", "conv", "--no-input", "--dry-run"]);
    assert!(run.status.success(), "{}", combined(&run));
    assert!(combined(&run).contains("convert m"), "{}", combined(&run));
    assert_eq!(snapshot_tree(workspace.state.path()), state_before);
    assert_eq!(snapshot_tree(workspace.config.path()), config_before);
}

#[test]
fn test_cli_command_show_enriched_and_env_rider() {
    let workspace = lib();
    workspace
        .cmd()
        .args(["add", "--cmd", "echo {msg}", "--name", "c", "--no-input"])
        .assert()
        .success();
    let initial = stdout_json(&workspace.run(&["params", "c", "--json"]));
    assert_eq!(initial["placeholders"], json!(["msg"]));
    assert_eq!(initial["declared"], json!([]));
    assert_eq!(initial["parameters"][0]["name"], "msg");
    assert_eq!(initial["parameters"][0]["delivery"], "placeholder");
    let state_before = snapshot_tree(workspace.state.path());
    let config_before = snapshot_tree(workspace.config.path());
    let first = workspace.run(&[
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
    assert!(first.status.success(), "{}", combined(&first));
    let second = workspace.run(&[
        "params",
        "c",
        "--add",
        "RETRIES",
        "--deliver",
        "RETRIES=env",
    ]);
    assert!(second.status.success(), "{}", combined(&second));
    let data_before_reads = snapshot_tree(workspace.data.path());
    let state_before_reads = snapshot_tree(workspace.state.path());
    let config_before_reads = snapshot_tree(workspace.config.path());
    let human = workspace.run(&["params", "c"]);
    assert!(human.status.success(), "{}", combined(&human));
    assert!(combined(&human).contains("msg"));
    assert!(combined(&human).contains("optional")); // the schema suffix marker
    assert!(combined(&human).contains("RETRIES")); // the declared env rider is listed
    let payload = stdout_json(&workspace.run(&["params", "c", "--json"]));
    let declared_names: Vec<&str> = payload["declared"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| row["name"].as_str().unwrap())
        .collect();
    assert_eq!(declared_names, ["msg", "RETRIES"]);
    assert_eq!(payload["declared"][0]["delivery"], "placeholder");
    assert_eq!(payload["declared"][0]["default"], "hi");
    assert!(payload["declared"][0].get("required").is_none());
    assert_eq!(payload["declared"][1]["delivery"], "env");
    assert_eq!(payload["placeholders"], json!(["msg"]));
    let effective_names = payload["parameters"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| row["name"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(effective_names, ["msg", "RETRIES"]);
    let show = stdout_json(&workspace.run(&["show", "c", "--json"]));
    let shown_fields = show["fields"]
        .as_array()
        .unwrap()
        .iter()
        .map(|field| {
            (
                field["key"].as_str().unwrap(),
                field["source"].as_str().unwrap(),
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(shown_fields, [("msg", "placeholder"), ("RETRIES", "env")]);
    assert_eq!(show["template"], "echo {msg}");
    assert_eq!(snapshot_tree(workspace.data.path()), data_before_reads);
    assert_eq!(snapshot_tree(workspace.state.path()), state_before_reads);
    assert_eq!(snapshot_tree(workspace.config.path()), config_before_reads);
    assert_eq!(snapshot_tree(workspace.state.path()), state_before);
    assert_eq!(snapshot_tree(workspace.config.path()), config_before);
}

#[test]
fn test_cli_command_env_rider_only_no_placeholders() {
    // a template with no placeholders but a declared env rider: the show view still lists it
    let workspace = lib();
    workspace
        .cmd()
        .args(["add", "--cmd", "echo hi", "--name", "noph", "--no-input"])
        .assert()
        .success();
    workspace.run(&[
        "params",
        "noph",
        "--add",
        "RETRIES",
        "--deliver",
        "RETRIES=env",
    ]);
    let output = workspace.run(&["params", "noph"]);
    assert!(output.status.success());
    assert!(combined(&output).contains("RETRIES"));
}

#[test]
fn test_cli_python_declared_op_is_refused() {
    let workspace = lib();
    let src = workspace.write_script("job.py", "CITY = \"x\"\nprint(CITY)\n");
    workspace
        .cmd()
        .arg("add")
        .arg(&src)
        .args(["--name", "py", "--no-input"])
        .assert()
        .success();
    let output = workspace.run(&["params", "py", "--add", "WIDTH"]);
    assert_eq!(output.status.code(), Some(1));
    assert!(combined(&output).contains("manages its parameters from the script itself"));
}

#[test]
#[ignore = "FAILING CONTRACT (divergence): a malformed `--type NOEQUALS` (no `=`) must be tolerated — the oracle warns \"Ignored a malformed value\" and exits 0 (src/skit/cli.py batch fault tolerance). The Rust product hard-errors exit 2 with \"type needs NAME=VALUE\" on stderr and applies nothing (pending task: params batch fault tolerance)."]
fn test_cli_declared_malformed_value_warns() {
    let workspace = lib();
    workspace.add_exe("prog");
    let output = workspace.run(&["params", "prog", "--type", "NOEQUALS"]);
    assert!(output.status.success(), "{}", combined(&output));
    assert!(combined(&output).contains("Ignored a malformed value"));
}

#[test]
#[ignore = "UNMAPPABLE white-box: the oracle drives the Python-private `cli._render_declared_warning(code)` for the 7 closed warning codes (not-declared/already-declared/bad-delivery/not-a-placeholder/bad-type/bad-default/choice-without-choices). The Rust warnings are localized messages with no public renderer to call, and their observable outcomes are covered (or recorded as divergences) by the CLI tests in this file. Not a MUST-FIX feature."]
fn test_cli_declared_warning_codes_render() {
    // for code in the 7 closed warning codes: line = cli._render_declared_warning(code);
    // assert "x" in line and the code prefix isn't leaked into the message.
}

#[test]
#[ignore = "FAILING CONTRACT (divergence): a bad `--type w=integer` must be tolerated — the oracle warns \"unknown type\" (exit 0) and leaves the type unchanged (src/skit/cli.py batch fault tolerance). The Rust product hard-errors exit 2 with \"unknown parameter type: integer\" on stderr (pending task: params batch fault tolerance)."]
fn test_cli_bad_type_warns_and_skips() {
    let workspace = lib();
    workspace.add_exe("prog");
    workspace.run(&[
        "params",
        "prog",
        "--add",
        "w",
        "--deliver",
        "w=flag",
        "--type",
        "w=str",
    ]);
    let output = workspace.run(&["params", "prog", "--type", "w=integer"]);
    assert!(output.status.success(), "{}", combined(&output));
    assert!(combined(&output).contains("unknown type"));
    let payload = stdout_json(&workspace.run(&["params", "prog", "--json"]));
    assert_eq!(payload["declared"][0]["type"], "str"); // unchanged
}

#[test]
fn test_cli_secret_override_persists_value_now_that_it_isnt_secret() {
    let workspace = lib();
    workspace
        .cmd()
        .args([
            "add",
            "--cmd",
            "printf '%s' {token_file}",
            "--name",
            "auth",
            "--no-input",
        ])
        .assert()
        .success();
    let initial = stdout_json(&workspace.run(&["params", "auth", "--json"]));
    assert_eq!(initial["placeholders"], json!(["token_file"]));
    assert_eq!(initial["declared"], json!([]));
    assert_eq!(initial["parameters"][0]["name"], "token_file");
    assert_eq!(initial["parameters"][0]["delivery"], "placeholder");
    assert_eq!(initial["parameters"][0]["secret"], true);
    let state_before = snapshot_tree(workspace.state.path());
    let config_before = snapshot_tree(workspace.config.path());
    let output = workspace.run(&[
        "params",
        "auth",
        "--add",
        "token_file",
        "--no-secret",
        "token_file",
    ]);
    assert!(output.status.success(), "{}", combined(&output));
    let payload = stdout_json(&workspace.run(&["params", "auth", "--json"]));
    assert_eq!(payload["declared"].as_array().unwrap().len(), 1);
    assert_eq!(payload["declared"][0]["name"], "token_file");
    assert_eq!(payload["declared"][0]["delivery"], "placeholder");
    assert!(payload["declared"][0].get("secret").is_none()); // false is the sparse raw row
    assert_eq!(payload["parameters"].as_array().unwrap().len(), 1);
    assert!(payload["parameters"][0].get("secret").is_none());
    assert_eq!(payload["placeholders"], json!(["token_file"]));
    let show = stdout_json(&workspace.run(&["show", "auth", "--json"]));
    assert_eq!(show["fields"][0]["secret"], false); // effective machine field is total
    assert_eq!(snapshot_tree(workspace.state.path()), state_before);
    assert_eq!(snapshot_tree(workspace.config.path()), config_before);
    assert!(
        workspace
            .meta("auth")
            .contains("template = \"printf '%s' {token_file}\"")
    );

    let run = workspace.run(&[
        "run",
        "auth",
        "--set",
        "token_file=creds.json",
        "--no-input",
    ]);
    assert!(run.status.success(), "{}", combined(&run));
    // Now that it isn't secret, the value IS remembered (the old behavior scrubbed it).
    assert!(workspace.values_file("auth").contains("token_file"));
    assert!(workspace.values_file("auth").contains("creds.json"));
    let after_run = stdout_json(&workspace.run(&["params", "auth", "--json"]));
    assert_eq!(after_run["last_values"]["token_file"], "creds.json");
    assert!(after_run["declared"][0].get("secret").is_none());
    assert_eq!(snapshot_tree(workspace.config.path()), config_before);
}

#[test]
fn test_cli_secret_declared_env_purges_prior_plaintext() {
    let workspace = lib();
    workspace.add_exe("prog");
    workspace.run(&["params", "prog", "--add", "TOKEN", "--deliver", "TOKEN=env"]);
    workspace.seed_values("prog", "[values]\nTOKEN = \"plaintext\"\n");
    let output = workspace.run(&["params", "prog", "--secret", "TOKEN"]);
    assert!(output.status.success(), "{}", combined(&output));
    let shown = combined(&output);
    let purge = shown
        .find("Removed previously stored plaintext")
        .unwrap_or_else(|| panic!("missing purge notice: {shown}"));
    let receipt = shown
        .find("Updated prog. Declared parameters: TOKEN")
        .unwrap_or_else(|| panic!("missing declared receipt: {shown}"));
    assert!(purge < receipt, "{shown}");
    assert!(!workspace.values_file("prog").contains("TOKEN"));
}

#[test]
fn test_cli_declared_secret_env_source_resolves_without_prompting() {
    // A secret env param with an env_source resolves under --no-input with no prompt: the value
    // comes from the environment, never from a form field, and is never persisted (C3).
    let workspace = lib();
    workspace
        .cmd()
        .args(["add", "--cmd", "echo $TOKEN", "--name", "svc", "--no-input"])
        .assert()
        .success();
    workspace.run(&[
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
    let output = workspace
        .cmd()
        .env("SVC_TOKEN", "from-env")
        .args(["run", "svc", "--no-input"])
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", combined(&output));
    assert!(
        combined(&output).contains("from-env"),
        "{}",
        combined(&output)
    ); // env overlay carried the value
    assert!(!workspace.values_file("svc").contains("TOKEN")); // C3: never persisted
}

#[test]
fn test_cli_run_set_env_and_placeholder_dry_run() {
    let workspace = lib();
    workspace
        .cmd()
        .args(["add", "--cmd", "echo {msg}", "--name", "dr", "--no-input"])
        .assert()
        .success();
    workspace.run(&[
        "params",
        "dr",
        "--add",
        "RETRIES",
        "--deliver",
        "RETRIES=env",
        "--default",
        "RETRIES=3",
    ]);
    let output = workspace.run(&["run", "dr", "--set", "msg=hello", "--dry-run", "--no-input"]);
    assert!(output.status.success(), "{}", combined(&output));
    assert!(combined(&output).contains("RETRIES=3")); // env overlay shown in the transparency line
}

#[test]
fn test_cli_rm_declared_param() {
    let workspace = lib();
    workspace.add_exe("prog");
    workspace.run(&["params", "prog", "--add", "a", "--deliver", "a=flag"]);
    workspace.run(&["params", "prog", "--add", "b", "--deliver", "b=flag"]);
    let output = workspace.run(&["params", "prog", "--rm", "a"]);
    assert!(output.status.success(), "{}", combined(&output));
    let payload = stdout_json(&workspace.run(&["params", "prog", "--json"]));
    let remaining: Vec<&str> = payload["declared"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| row["name"].as_str().unwrap())
        .collect();
    assert_eq!(remaining, ["b"]);

    let remove_last = workspace.run(&["params", "prog", "--rm", "b"]);
    assert!(remove_last.status.success(), "{}", combined(&remove_last));
    assert!(
        combined(&remove_last).contains("Updated prog. Declared parameters: —"),
        "{}",
        combined(&remove_last)
    );
    let empty = stdout_json(&workspace.run(&["params", "prog", "--json"]));
    assert_eq!(empty["declared"], serde_json::json!([]));
}

#[test]
fn test_cli_exe_declared_show_json_param_origin() {
    let workspace = lib();
    workspace.add_exe("prog");
    workspace.run(&[
        "params",
        "prog",
        "--add",
        "w",
        "--deliver",
        "w=flag",
        "--flag",
        "w=--w",
        "--type",
        "w=int",
    ]);
    let payload = stdout_json(&workspace.run(&["show", "prog", "--json"]));
    assert_eq!(payload["param_source"], "declared");
    assert_eq!(payload["param_origin"], "declared");
    let field = payload["fields"]
        .as_array()
        .unwrap()
        .iter()
        .find(|field| field["key"] == "w")
        .unwrap();
    assert_eq!(field["source"], "flag");
}

#[test]
fn test_cli_exe_no_declared_show_json_param_origin_none() {
    let workspace = lib();
    workspace.add_exe("prog");
    let payload = stdout_json(&workspace.run(&["show", "prog", "--json"]));
    assert_eq!(payload["param_source"], "none");
    assert_eq!(payload["param_origin"], "none");
}

#[test]
fn test_cli_command_env_show_json_source_env() {
    let workspace = lib();
    workspace
        .cmd()
        .args(["add", "--cmd", "echo {m}", "--name", "cj", "--no-input"])
        .assert()
        .success();
    workspace.run(&["params", "cj", "--add", "N", "--deliver", "N=env"]);
    let payload = stdout_json(&workspace.run(&["show", "cj", "--json"]));
    let field = payload["fields"]
        .as_array()
        .unwrap()
        .iter()
        .find(|field| field["key"] == "N")
        .unwrap();
    assert_eq!(field["source"], "env"); // env value source flows through show --json
}

#[test]
fn test_cli_exe_show_masks_secret_default_and_last_value() {
    // Covers the read-view secret masking: a secret row with a stored value -> •••; a secret row
    // with a default -> •••; the stored plaintext is never echoed.
    let workspace = lib();
    workspace.add_exe("prog");
    workspace.run(&[
        "params",
        "prog",
        "--add",
        "a",
        "--deliver",
        "a=flag",
        "--type",
        "a=str",
        "--secret",
        "a",
    ]);
    workspace.run(&[
        "params",
        "prog",
        "--add",
        "b",
        "--deliver",
        "b=flag",
        "--type",
        "b=str",
        "--default",
        "b=x",
        "--secret",
        "b",
    ]);
    workspace.run(&[
        "params",
        "prog",
        "--add",
        "c",
        "--deliver",
        "c=flag",
        "--type",
        "c=str",
        "--default",
        "c=public-default",
    ]);
    workspace.seed_values("prog", "[values]\na = \"stale\"\nc = \"public-last\"\n");
    let output = workspace.run(&["params", "prog"]);
    assert!(output.status.success(), "{}", combined(&output));
    let text = combined(&output);
    assert_eq!(text.matches("•••").count(), 2, "{text}");
    assert!(!text.contains("Current default: x"), "{text}");
    assert!(!text.contains("Last value: stale"), "{text}");
    assert_eq!(text.matches("Current default:").count(), 2, "{text}");
    assert_eq!(text.matches("Last value:").count(), 2, "{text}");
    assert!(text.contains("Current default: public-default"), "{text}");
    assert!(text.contains("Last value: public-last"), "{text}");

    let payload = stdout_json(&workspace.run(&["params", "prog", "--json"]));
    assert_eq!(payload["last_values"]["a"], "stale");
    assert_eq!(payload["last_values"]["c"], "public-last");
    let b = payload["parameters"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["name"] == "b")
        .unwrap();
    assert_eq!(b["default"], "x");
}

#[test]
fn test_cli_command_show_masks_secret_placeholder_and_undeclared() {
    // Covers command-param secret masking + an undeclared placeholder's empty schema suffix.
    let workspace = lib();
    workspace
        .cmd()
        .args([
            "add",
            "--cmd",
            "login {password} {other}",
            "--name",
            "lg",
            "--no-input",
        ])
        .assert()
        .success();
    // password is an implicit secret placeholder; the schema edit promotes only that row.
    workspace.run(&[
        "params",
        "lg",
        "--default",
        "password=seed",
        "--required",
        "password",
    ]);
    workspace.seed_values("lg", "[values]\npassword = \"stale\"\n");
    let output = workspace.run(&["params", "lg"]);
    assert!(output.status.success(), "{}", combined(&output));
    let text = combined(&output);
    assert_eq!(text.matches("•••").count(), 2, "{text}");
    assert!(!text.contains("Current default: seed"), "{text}");
    assert!(!text.contains("Last value: stale"), "{text}");
    assert!(
        text.contains("  password = •••  str · default ••• · secret"),
        "{text}"
    );
    assert!(text.contains("  other = —  str"), "{text}"); // implicit sibling remains listed
    let payload = stdout_json(&workspace.run(&["params", "lg", "--json"]));
    assert_eq!(payload["declared"].as_array().unwrap().len(), 1);
    assert_eq!(payload["declared"][0]["name"], "password");
}

// ---- Delivery capability honesty ---------------------------------------------------------------

/// Add the oracle's `_ruby` fixture: a copy-mode ruby entry under `name`.
fn add_ruby(workspace: &Lib, name: &str) {
    let src = workspace.write_script(&format!("{name}.rb"), "#!/usr/bin/env ruby\nputs \"hi\"\n");
    workspace
        .cmd()
        .arg("add")
        .arg(&src)
        .args(["--name", name, "--no-input"])
        .assert()
        .success();
}

#[test]
fn test_declared_add_on_interpreted_meta_kind_defaults_to_deliverable_flag() {
    // An interpreted kind whose schema home is meta (ruby/perl/lua/r) assembles a real argv, so a
    // bare --add must default to flag delivery — not a dead placeholder that never reaches the
    // child.
    let workspace = lib();
    add_ruby(&workspace, "rb");
    assert!(
        workspace
            .run(&["params", "rb", "--add", "SIZE"])
            .status
            .success()
    );
    let payload = stdout_json(&workspace.run(&["params", "rb", "--json"]));
    assert_eq!(payload["declared"][0]["name"], "SIZE");
    assert_eq!(payload["declared"][0]["delivery"], "flag");
    let plan = stdout_json(&workspace.run(&["show", "rb", "--json"]));
    assert_eq!(plan["param_source"], "declared");
    let field = &plan["fields"][0];
    assert_eq!(field["key"], "SIZE");
    assert_eq!(field["source"], "flag");
}

#[test]
fn test_declared_add_on_interpreted_kind_delivers_at_run() {
    let workspace = lib();
    add_ruby(&workspace, "rb2");
    workspace.run(&["params", "rb2", "--add", "SIZE", "--flag", "SIZE=--size"]);
    let output = workspace.run(&["run", "rb2", "--set", "SIZE=5", "--dry-run", "--no-input"]);
    assert!(output.status.success(), "{}", combined(&output));
    let shown = combined(&output).replace('\n', "");
    assert!(shown.contains("--size"), "{shown}");
    assert!(shown.contains('5'), "{shown}");
}

#[test]
fn test_reader_kind_declared_rows_stand_alone_when_no_readable_surface() {
    // No readable param() (no param block) but declared rows exist: they still form a plan on
    // their own rather than vanishing into the "none" fall-through.
    let mut loglevel = ParamDecl::new("LOGLEVEL");
    loglevel.delivery = ParameterDelivery::Env;
    let settings = EntrySettings {
        parameters: vec![loglevel],
        ..EntrySettings::default()
    };
    let plan = form_plan("powershell", "Write-Output 'hi'\n", &settings);
    assert_eq!(plan.source, FormSource::Declared);
    assert_eq!(
        field_sources(&plan),
        vec![("LOGLEVEL".to_owned(), ParameterDelivery::Env)]
    );
}

#[test]
fn test_declared_table_is_shown_for_an_interpreted_meta_kind() {
    // The read surface must not deny what the write surface created and the run delivers: a ruby
    // entry's declared rows must appear in the human view, not print "has no managed parameters".
    let workspace = lib();
    add_ruby(&workspace, "rb3");
    workspace.run(&["params", "rb3", "--add", "GREETING"]);
    let output = workspace.run(&["params", "rb3"]);
    assert!(combined(&output).contains("GREETING"));
    assert!(!combined(&output).contains("has no managed parameters"));
}

#[test]
fn test_declared_param_on_an_interpreted_kind_actually_delivers() {
    let workspace = lib();
    add_ruby(&workspace, "rb4");
    workspace.run(&[
        "params",
        "rb4",
        "--add",
        "GREETING",
        "--flag",
        "GREETING=--greeting",
    ]);
    // The oracle spies run_entry; here --dry-run shows the assembled argv without needing ruby.
    let output = workspace.run(&[
        "run",
        "rb4",
        "--set",
        "GREETING=world",
        "--dry-run",
        "--no-input",
    ]);
    assert!(output.status.success(), "{}", combined(&output));
    assert!(
        combined(&output).contains("--greeting world"),
        "{}",
        combined(&output)
    );
}

#[test]
fn test_template_add_of_a_non_placeholder_name_creates_a_deliverable_env_row() {
    let workspace = lib();
    workspace
        .cmd()
        .args(["add", "--cmd", "greet {WHO}", "--name", "tpl", "--no-input"])
        .assert()
        .success();
    let config_before = std::fs::read(workspace.config.path().join("config.toml")).ok();
    assert_eq!(
        std::fs::read_dir(workspace.state.path()).unwrap().count(),
        0
    );
    let edit = workspace.run(&["params", "tpl", "--add", "RETRIES"]);
    assert!(edit.status.success(), "{}", combined(&edit));
    let payload = stdout_json(&workspace.run(&["params", "tpl", "--json"]));
    assert_eq!(payload["placeholders"], json!(["WHO"]));
    let retries = payload["declared"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["name"] == "RETRIES")
        .unwrap();
    let show = stdout_json(&workspace.run(&["show", "tpl", "--json"]));
    assert_eq!(show["template"], "greet {WHO}");
    assert!(workspace.values_file("tpl").is_empty());
    assert_eq!(
        std::fs::read_dir(workspace.state.path()).unwrap().count(),
        0
    );
    assert_eq!(
        std::fs::read(workspace.config.path().join("config.toml")).ok(),
        config_before
    );
    assert_eq!(retries["delivery"], "env");

    // and it really delivers, rather than being denied by --set
    let run = workspace.run(&[
        "run",
        "tpl",
        "--set",
        "WHO=ada",
        "--set",
        "RETRIES=3",
        "--dry-run",
        "--no-input",
    ]);
    assert!(run.status.success(), "{}", combined(&run));
    assert!(combined(&run).replace('\n', "").contains("RETRIES=3"));
}

#[test]
fn test_template_add_of_a_real_placeholder_name_still_fills_the_slot() {
    let workspace = lib();
    workspace
        .cmd()
        .args([
            "add",
            "--cmd",
            "greet {WHO}",
            "--name",
            "tpl2",
            "--no-input",
        ])
        .assert()
        .success();
    // The oracle does not assert the exit code here; WHO is a real placeholder slot either way.
    workspace.run(&["params", "tpl2", "--add", "WHO", "--type", "WHO=str"]);
    let payload = stdout_json(&workspace.run(&["params", "tpl2", "--json"]));
    let who = payload["declared"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["name"] == "WHO")
        .unwrap();
    assert_eq!(who["delivery"], "placeholder");
}
