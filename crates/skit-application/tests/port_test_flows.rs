//! Mechanical port of the Python oracle module `tests/test_flows.py`
//! (`origin/main@206f9ef`): "Unified form layer: plans, prefill order, validation,
//! assembly, run recording." Each `#[test]` keeps its Python `def test_*` name so it
//! traces back to its origin, and each Python "WHY" comment is preserved above it.
//!
//! The Python `flows` module is one flat file. The Rust rewrite splits it across crates.
//! This port lives in `skit-application`, where the value pipeline is, and buckets the
//! rest honestly.
//!
//! Concept mapping used throughout:
//! - Python `flows.assemble(plan, values, extra, cwd=, env=, now=, expand_extra=)` ->
//!   `run_inputs::assemble_run_inputs(decls, raw_values, extra, expand_extra, ctx, glob)`.
//!   The one Python function is four Rust stages: `value_resolution::resolve_values`
//!   (tokens + secret source), `value_preparation::prepare_values` (validation + shlex
//!   split), `glob_expansion::expand_multi_values` (through a `GlobExpander` port), and
//!   `delivery::assemble` (routing). cwd/env/now/today arrive as an explicit `TokenContext`.
//! - Python `plan.fields` are `FormField`; Rust `assemble_run_inputs` takes `&[ParamDecl]`.
//!   `FormField` is a frontend projection with no Rust struct equivalent (`PreparedField`
//!   wraps a `ParamDecl`; label/kind collapsing lives in the frontends). The assembly
//!   tests hand-transcribe the two module fixtures (`ARGPARSE_SCRIPT`, `MANAGED_SCRIPT`)
//!   into `ParamDecl` lists whose shape the oracle itself pins in
//!   `test_plan_argparse_script` / `test_plan_managed_script_is_inject`. `action` is set
//!   explicitly because store_true defaulting is a `from_decl`/plan concern this port bypasses.
//! - Python `flows._assemble_flags(plan, final, cwd)` -> `delivery::assemble` on
//!   `PreparedValue` (same white-box altitude — no token/glob/validation).
//! - Python `flows.validate_value(f, v)` / `_type_error` -> `value_preparation::validate_form_value`
//!   (returns a TYPED `ValuePreparationError`; the exact user-facing wording lives in
//!   `skit-tui/src/session.rs`, so the exact-string test is cross-crate).
//! - Python `flows.prefill` / `argstate.save_last` / `save_preset` / `record_run` /
//!   `flows.save_after_run` -> `form_state::{prefill, remembered_values}` +
//!   `FormStateService` driven through an in-memory `FormStateRepository`.
//! - Python `flows.glob_feedback` -> `form_feedback::glob_count_request` (the reachable
//!   half: glob detection + shlex pieces; the actual match COUNT is `skit-store::path_glob`).
//! - Python `flows.transparency_lines(entry, asm, injected, ...)` -> partially
//!   `delivery::transparency_messages(asm, command)`. The exact inject line + the
//!   one-line flag form are reachable; building the command from an `Entry`/runner is
//!   `skit-runtime`/`skit-cli`.
//!
//! Buckets:
//! - REAL: the value pipeline (assembly, validation, resolution), prefill/state/save,
//!   glob detection, the two exact transparency lines, `truthy` via routing,
//!   `synthesized_placeholder` requiredness/secrecy, default rendering via prefill.
//! - cross-crate: `plan_for_entry` / `FormField.from_decl` (`skit-form::form_plan`),
//!   `execute` / `RunOutcome` / entry-based transparency (`skit-runtime`/`skit-cli`),
//!   `_expand_glob_piece` (`skit-store::path_glob`), the exact validation wording
//!   (`skit-tui`), and the os.environ default (composition root).
//! - divergence: `test_assemble_does_not_retypecheck_plain_values` — the Rust pipeline
//!   re-validates a plain saved value that the Python `assemble` passes straight through.

use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
    sync::Mutex,
};

use skit_application::{
    delivery::{self, Assembly, PreparedValue, transparency_messages},
    form_feedback::glob_count_request,
    form_state::{
        FormStateRepository, FormStateService, LastRunState, PersistedFormState, StateWriteError,
        prefill,
    },
    glob_expansion::GlobExpander,
    run_inputs::{RunInputError, assemble_run_inputs},
    tokens::TokenContext,
    value_preparation::{ValuePreparationError, validate_form_value},
    value_resolution::resolve_values,
};
use skit_domain::{
    Slug,
    parameters::{
        ParamDecl, ParameterDelivery, ParameterType, ParameterValue, synthesized_placeholder,
    },
};
use skit_i18n::{Locale, Localize};

// --------------------------------------------------------------------------
// shared test scaffolding
// --------------------------------------------------------------------------

/// A glob port whose matches are seeded per scenario. An unseeded piece returns itself,
/// which reproduces `_expand_glob_piece`'s "no match keeps the literal" behavior.
#[derive(Default)]
struct FakeGlob {
    matches: BTreeMap<String, Vec<String>>,
}

impl fmt::Debug for FakeGlob {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_struct("FakeGlob").finish_non_exhaustive()
    }
}

impl GlobExpander for FakeGlob {
    fn expand_piece(&self, piece: &str) -> Vec<String> {
        self.matches
            .get(piece)
            .cloned()
            .unwrap_or_else(|| vec![piece.to_owned()])
    }
}

fn glob_with(pairs: &[(&str, &[&str])]) -> FakeGlob {
    let mut glob = FakeGlob::default();
    for (pattern, matches) in pairs {
        glob.matches.insert(
            (*pattern).to_owned(),
            matches.iter().map(|item| (*item).to_owned()).collect(),
        );
    }
    glob
}

/// NOW = datetime(2026, 7, 9, 14, 30, 5): `{today}` -> "2026-07-09", `{now}` -> "14-30-05".
fn context(env: &[(&str, &str)]) -> TokenContext {
    TokenContext {
        cwd: "/run/dir".to_owned(),
        home: None,
        env: env
            .iter()
            .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
            .collect(),
        today: "2026-07-09".to_owned(),
        now: "14-30-05".to_owned(),
    }
}

fn map(items: &[(&str, &str)]) -> BTreeMap<String, String> {
    items
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn strings(items: &[&str]) -> Vec<String> {
    items.iter().map(|item| (*item).to_owned()).collect()
}

/// Drive the full run-input pipeline (the Python `flows.assemble`).
fn run(
    decls: &[ParamDecl],
    values: &[(&str, &str)],
    extra: &[&str],
    expand_extra: bool,
    env: &[(&str, &str)],
    glob: &FakeGlob,
) -> Result<Assembly, RunInputError> {
    assemble_run_inputs(
        decls,
        &map(values),
        &strings(extra),
        expand_extra,
        &context(env),
        glob,
    )
}

/// The ARGPARSE_SCRIPT plan, transcribed from `test_plan_argparse_script`'s pins:
/// inputs (positional, multiple, repeat=false, required), --output (required),
/// --gap (int, default 0), --mode (choice a|b, default a), --fast (store_true bool),
/// --bg (degraded free text).
fn argparse_decls() -> Vec<ParamDecl> {
    let mut inputs = ParamDecl::new("inputs");
    inputs.multiple = true;
    inputs.required = true;

    let mut output = ParamDecl::new("output");
    output.flag = "--output".to_owned();
    output.required = true;

    let mut gap = ParamDecl::new("gap");
    gap.flag = "--gap".to_owned();
    gap.parameter_type = ParameterType::Int;
    gap.default = Some(ParameterValue::Integer(0));

    let mut mode = ParamDecl::new("mode");
    mode.flag = "--mode".to_owned();
    mode.parameter_type = ParameterType::Choice;
    mode.choices = strings(&["a", "b"]);
    mode.default = Some(ParameterValue::String("a".to_owned()));

    let mut fast = ParamDecl::new("fast");
    fast.flag = "--fast".to_owned();
    fast.parameter_type = ParameterType::Bool;
    fast.action = "store_true".to_owned();

    let mut bg = ParamDecl::new("bg");
    bg.flag = "--bg".to_owned();
    bg.degraded = true;

    vec![inputs, output, gap, mode, fast, bg]
}

/// The MANAGED_SCRIPT plan, transcribed from `test_plan_managed_script_is_inject`:
/// OUTPUT (inject str, default "out.jpg"), WIDTH (inject int, default 800),
/// API_KEY (inject str, default "xxx", secret, env_source MY_API_KEY).
fn managed_decls() -> Vec<ParamDecl> {
    let mut output = ParamDecl::new("OUTPUT");
    output.delivery = ParameterDelivery::Inject;
    output.default = Some(ParameterValue::String("out.jpg".to_owned()));

    let mut width = ParamDecl::new("WIDTH");
    width.delivery = ParameterDelivery::Inject;
    width.parameter_type = ParameterType::Int;
    width.default = Some(ParameterValue::Integer(800));

    let mut api_key = ParamDecl::new("API_KEY");
    api_key.delivery = ParameterDelivery::Inject;
    api_key.default = Some(ParameterValue::String("xxx".to_owned()));
    api_key.secret = true;
    api_key.env_source = "MY_API_KEY".to_owned();

    vec![output, width, api_key]
}

/// _values_ok(): a full, valid argparse submission.
fn values_ok() -> Vec<(&'static str, &'static str)> {
    vec![
        ("inputs", "a.png"),
        ("output", "o.png"),
        ("gap", "0"),
        ("mode", "a"),
        ("fast", "false"),
    ]
}

/// Python `flows.validate(plan, values)` keys: the field names whose value fails a check.
fn validate_keys(decls: &[ParamDecl], values: &BTreeMap<String, String>) -> BTreeSet<String> {
    decls
        .iter()
        .filter_map(|decl| {
            let value = values.get(&decl.name).map(String::as_str).unwrap_or("");
            validate_form_value(decl, value)
                .err()
                .map(|_| decl.name.clone())
        })
        .collect()
}

fn prepared(items: &[(&str, PreparedValue)]) -> BTreeMap<String, PreparedValue> {
    items
        .iter()
        .map(|(name, value)| ((*name).to_owned(), value.clone()))
        .collect()
}

fn scalar(value: &str) -> PreparedValue {
    PreparedValue::Scalar(value.to_owned())
}

/// An in-memory `FormStateRepository` for the prefill/save use-cases (argstate stand-in).
#[derive(Debug, Default)]
struct MemoryState {
    states: Mutex<BTreeMap<String, PersistedFormState>>,
}

impl FormStateRepository for MemoryState {
    fn load(&self, slug: &Slug) -> PersistedFormState {
        self.states
            .lock()
            .unwrap()
            .get(slug.as_str())
            .cloned()
            .unwrap_or_default()
    }

    fn last_run(&self, slug: &Slug) -> LastRunState {
        self.states
            .lock()
            .unwrap()
            .get(slug.as_str())
            .map(|state| state.last_run.clone())
            .unwrap_or_default()
    }

    fn update<T, F>(&self, slug: &Slug, update: F) -> Result<T, StateWriteError>
    where
        F: FnOnce(&mut PersistedFormState) -> T,
    {
        let mut states = self.states.lock().unwrap();
        let state = states.entry(slug.as_str().to_owned()).or_default();
        Ok(update(state))
    }

    fn try_update<T, E, F>(&self, slug: &Slug, update: F) -> Result<Result<T, E>, StateWriteError>
    where
        F: FnOnce(&mut PersistedFormState) -> Result<T, E>,
    {
        let mut states = self.states.lock().unwrap();
        let state = states.entry(slug.as_str().to_owned()).or_default();
        let before = state.clone();
        match update(state) {
            Ok(result) => Ok(Ok(result)),
            Err(error) => {
                *state = before;
                Ok(Err(error))
            }
        }
    }

    fn forget(&self, slug: &Slug) -> Result<(), StateWriteError> {
        self.states.lock().unwrap().remove(slug.as_str());
        Ok(())
    }
}

fn service() -> FormStateService<MemoryState> {
    FormStateService::new(MemoryState::default())
}

fn slug(name: &str) -> Slug {
    Slug::parse(name).unwrap()
}

/// Python `flows.save_after_run`: retroactive secret purge, then last-used save, then run stamp.
fn save_after_run(
    service: &FormStateService<MemoryState>,
    slug: &Slug,
    decls: &[ParamDecl],
    values: &BTreeMap<String, String>,
    extra_args: Vec<String>,
    exit_code: i64,
    at: &str,
) {
    if decls.iter().any(|decl| decl.secret) {
        service.purge_secrets(slug, decls).unwrap();
    }
    service
        .save_last(slug, decls, Some(values), Some(extra_args), false)
        .unwrap();
    service
        .record_run(slug, exit_code, at, decls, Some(values))
        .unwrap();
}

// --------------------------------------------------------------------------
// plans
// --------------------------------------------------------------------------

#[test]
#[ignore = "cross-crate (skit-form::form_plan): plan_for_entry projecting a managed script to an inject FormPlan, plus FormField.key/kind/secret/env_source, lives in skit-form; skit-application has no FormField projection to observe."]
fn test_plan_managed_script_is_inject() {}

#[test]
#[ignore = "cross-crate (skit-form::form_plan): plan_for_entry over a CLI-reader script (argparse source, multiple/repeat/degraded, FormPlan.text) is skit-form + skit-language; the reader is not reachable from skit-application."]
fn test_plan_argparse_script() {}

#[test]
#[ignore = "cross-crate (skit-form::form_plan): the 'none' plan for a plain readable-but-fieldless script, including FormPlan.text, is skit-form."]
fn test_plan_plain_script_is_none() {}

#[test]
#[ignore = "cross-crate (skit-form::form_plan): command-template placeholder plan (source=command, field key/label) is skit-form; label==name collapsing lives in the frontends."]
fn test_plan_command_entry_placeholders() {}

#[test]
#[ignore = "cross-crate (skit-form::form_plan): the managed-wins-over-argparse precedence is decided inside skit-form::form_plan."]
fn test_plan_managed_wins_over_argparse() {}

#[test]
#[ignore = "cross-crate (skit-form::form_plan): a missing script yielding the 'none' plan is skit-form + a real filesystem Entry."]
fn test_plan_missing_script_is_none() {}

// --------------------------------------------------------------------------
// prefill
// --------------------------------------------------------------------------

/// Definition default < last-used < preset (this run's input wins in the UI).
#[test]
fn test_prefill_default_then_last_then_preset() {
    let decls = managed_decls();
    let service = service();
    let slug = slug("s");

    // definition default
    assert_eq!(prefill(&decls, &BTreeMap::new(), None)["OUTPUT"], "out.jpg");

    // last wins over default
    service
        .save_last(
            &slug,
            &decls,
            Some(&map(&[("OUTPUT", "last.jpg")])),
            None,
            false,
        )
        .unwrap();
    let state = service.load(&slug);
    assert_eq!(prefill(&decls, &state.values, None)["OUTPUT"], "last.jpg");

    // preset wins
    service
        .save_preset(&slug, "web", &decls, &map(&[("OUTPUT", "web.jpg")]))
        .unwrap();
    let state = service.load(&slug);
    assert_eq!(
        prefill(&decls, &state.values, state.presets.get("web"))["OUTPUT"],
        "web.jpg"
    );

    // no preset asked -> last
    assert_eq!(prefill(&decls, &state.values, None)["OUTPUT"], "last.jpg");
}

/// even though the definition has a default, a secret is never prefilled.
#[test]
fn test_prefill_never_offers_secrets() {
    let values = prefill(&managed_decls(), &BTreeMap::new(), None);
    assert!(!values.contains_key("API_KEY"));
}

// --------------------------------------------------------------------------
// validation
// --------------------------------------------------------------------------

#[test]
fn test_validate_required_empty() {
    let decls = argparse_decls();
    let errors = validate_keys(&decls, &BTreeMap::new());
    assert_eq!(
        errors,
        BTreeSet::from(["inputs".to_owned(), "output".to_owned()])
    );
    // The required message DOES match the oracle wording verbatim in skit-application.
    assert_eq!(
        validate_form_value(&decls[1], "")
            .unwrap_err()
            .message()
            .localize(Locale::En),
        "output is required."
    );
}

#[test]
fn test_validate_int_error_names_field_and_value() {
    let decls = argparse_decls();
    let error = validate_form_value(&decls[2], "abc").unwrap_err();
    assert_eq!(
        error,
        ValuePreparationError::InvalidType {
            name: "gap".to_owned(),
            value: "abc".to_owned(),
            parameter_type: ParameterType::Int,
        }
    );
}

#[test]
fn test_validate_choice() {
    let decls = argparse_decls();
    let error = validate_form_value(&decls[3], "zzz").unwrap_err();
    assert_eq!(
        error,
        ValuePreparationError::InvalidChoice {
            name: "mode".to_owned(),
            value: "zzz".to_owned(),
            choices: strings(&["a", "b"]),
        }
    );
}

/// "{env:N}" cannot be type-checked before expansion; validate defers to assembly.
#[test]
fn test_validate_token_values_deferred() {
    let decls = argparse_decls();
    assert!(validate_form_value(&decls[2], "{env:GAP}").is_ok());
}

// --------------------------------------------------------------------------
// assembly
// --------------------------------------------------------------------------

#[test]
fn test_assemble_argparse_positionals_then_flags() {
    let asm = run(
        &argparse_decls(),
        &[
            ("inputs", "a.png b.png"),
            ("output", "o.png"),
            ("gap", "4"),
            ("mode", "b"),
            ("fast", "true"),
        ],
        &[],
        true,
        &[],
        &FakeGlob::default(),
    )
    .unwrap();
    assert_eq!(
        asm.args,
        [
            "a.png", "b.png", "--output", "o.png", "--gap", "4", "--mode", "b", "--fast",
        ]
    );
}

#[test]
fn test_assemble_unchecked_store_true_omits_flag() {
    let asm = run(
        &argparse_decls(),
        &values_ok(),
        &[],
        true,
        &[],
        &FakeGlob::default(),
    )
    .unwrap();
    assert!(!asm.args.contains(&"--fast".to_owned()));
}

#[test]
fn test_assemble_degraded_empty_omitted_filled_passed() {
    let asm = run(
        &argparse_decls(),
        &values_ok(),
        &[],
        true,
        &[],
        &FakeGlob::default(),
    )
    .unwrap();
    assert!(!asm.args.contains(&"--bg".to_owned()));

    let mut filled = values_ok();
    filled.push(("bg", "#fff"));
    let asm2 = run(
        &argparse_decls(),
        &filled,
        &[],
        true,
        &[],
        &FakeGlob::default(),
    )
    .unwrap();
    assert_eq!(&asm2.args[asm2.args.len() - 2..], ["--bg", "#fff"]);
}

/// glob returns sorted matches expanded against cwd; the fake supplies the sorted pair.
#[test]
fn test_assemble_glob_expands_multiple_fields_against_cwd() {
    let glob = glob_with(&[("shots/*.png", &["shots/1.png", "shots/2.png"])]);
    let mut values = values_ok();
    values[0] = ("inputs", "shots/*.png");
    let asm = run(&argparse_decls(), &values, &[], true, &[], &glob).unwrap();
    assert_eq!(&asm.args[..2], ["shots/1.png", "shots/2.png"]);
}

#[test]
fn test_assemble_glob_without_match_keeps_literal() {
    let mut values = values_ok();
    values[0] = ("inputs", "none/*.xyz");
    let asm = run(
        &argparse_decls(),
        &values,
        &[],
        true,
        &[],
        &FakeGlob::default(),
    )
    .unwrap();
    assert_eq!(asm.args[0], "none/*.xyz");
}

#[test]
fn test_assemble_tokens_expand_and_type_check_after_expansion() {
    let mut values = values_ok();
    values[1] = ("output", "out_{today}.png");
    values[2] = ("gap", "{env:GAP}");
    let asm = run(
        &argparse_decls(),
        &values,
        &[],
        true,
        &[("GAP", "8")],
        &FakeGlob::default(),
    )
    .unwrap();
    assert!(asm.args.contains(&"out_2026-07-09.png".to_owned()));
    assert!(asm.args.contains(&"8".to_owned()));

    let error = run(
        &argparse_decls(),
        &values,
        &[],
        true,
        &[("GAP", "not-a-number")],
        &FakeGlob::default(),
    )
    .unwrap_err();
    assert!(error.to_string().contains("not-a-number"));
}

#[test]
fn test_assemble_missing_env_token_is_named_error() {
    let mut values = values_ok();
    values[1] = ("output", "{env:NOPE}");
    let error = run(
        &argparse_decls(),
        &values,
        &[],
        true,
        &[],
        &FakeGlob::default(),
    )
    .unwrap_err();
    assert!(error.to_string().contains("NOPE"));
}

#[test]
fn test_assemble_inject_values_expanded_and_masked_display() {
    let asm = run(
        &managed_decls(),
        &[
            ("OUTPUT", "long_{today}.jpg"),
            ("WIDTH", "800"),
            ("API_KEY", "typed-secret"),
        ],
        &[],
        true,
        &[],
        &FakeGlob::default(),
    )
    .unwrap();
    assert_eq!(asm.inject_values["OUTPUT"], "long_2026-07-09.jpg");
    assert!(
        asm.display
            .contains(&("API_KEY".to_owned(), "•••".to_owned()))
    );
    assert!(asm.display.iter().all(|(_, value)| value != "typed-secret"));
}

#[test]
fn test_assemble_secret_env_source_reads_environment() {
    let asm = run(
        &managed_decls(),
        &[("OUTPUT", "o.jpg"), ("WIDTH", "1"), ("API_KEY", "")],
        &[],
        true,
        &[("MY_API_KEY", "from-env")],
        &FakeGlob::default(),
    )
    .unwrap();
    assert_eq!(asm.inject_values["API_KEY"], "from-env");
}

/// Pin the exact sentence (field label + env-var name), so a corrupted message cannot
/// survive behind a bare substring check.
#[test]
fn test_assemble_secret_env_source_missing_is_named_error() {
    let error = run(
        &managed_decls(),
        &[("OUTPUT", "o"), ("WIDTH", "1"), ("API_KEY", "")],
        &[],
        true,
        &[],
        &FakeGlob::default(),
    )
    .unwrap_err();
    assert_eq!(
        error.message().localize(Locale::En),
        "API_KEY reads from the environment variable MY_API_KEY, but it isn't set."
    );
}

#[test]
fn test_assemble_typed_secret_beats_env_source() {
    let asm = run(
        &managed_decls(),
        &[("OUTPUT", "o"), ("WIDTH", "1"), ("API_KEY", "typed")],
        &[],
        true,
        &[("MY_API_KEY", "env")],
        &FakeGlob::default(),
    )
    .unwrap();
    assert_eq!(asm.inject_values["API_KEY"], "typed");
}

#[test]
fn test_assemble_command_values_and_extra_args() {
    let decls = vec![synthesized_placeholder("msg")];
    let asm = run(
        &decls,
        &[("msg", "hi {today}")],
        &["--verbose"],
        true,
        &[],
        &FakeGlob::default(),
    )
    .unwrap();
    assert_eq!(
        asm.command_values,
        BTreeMap::from([("msg".to_owned(), "hi 2026-07-09".to_owned())])
    );
    assert_eq!(asm.args, ["--verbose"]);
    assert_eq!(asm.masked_args, ["--verbose"]);
}

/// Every token forwarded into the extra-args expansion is pinned: glob, {today}, {now},
/// {cwd} (not "None"), and {env:...} (from the passed env, not os.environ).
#[test]
fn test_assemble_extra_args_expand_tokens_and_globs() {
    let glob = glob_with(&[("x*.txt", &["x1.txt", "x2.txt"])]);
    let asm = run(
        &[],
        &[],
        &["x*.txt", "{today}", "{now}", "{cwd}", "{env:XV}"],
        true,
        &[("XV", "envval")],
        &glob,
    )
    .unwrap();
    assert_eq!(
        asm.args,
        [
            "x1.txt",
            "x2.txt",
            "2026-07-09",
            "14-30-05",
            "/run/dir",
            "envval"
        ]
    );
}

/// A failed token in an extra arg surfaces the token error's own message.
#[test]
fn test_assemble_extra_arg_token_error_forwards_the_token_message() {
    let error = run(
        &[],
        &[],
        &["{env:NOPE_EXTRA}"],
        true,
        &[],
        &FakeGlob::default(),
    )
    .unwrap_err();
    assert!(error.to_string().contains("NOPE_EXTRA"));
}

/// The inject delivery still carries the extra-args escape hatch through to argv.
#[test]
fn test_assemble_inject_source_forwards_extra_args() {
    let asm = run(
        &managed_decls(),
        &[("OUTPUT", "o"), ("WIDTH", "1"), ("API_KEY", "k")],
        &["--flag", "v"],
        true,
        &[],
        &FakeGlob::default(),
    )
    .unwrap();
    assert_eq!(asm.args, ["--flag", "v"]);
}

/// A field value's {cwd}/{now} tokens must expand against the run's cwd/now.
#[test]
fn test_assemble_field_expands_cwd_and_now_tokens() {
    let mut values = values_ok();
    values[1] = ("output", "{cwd}/{now}.png");
    let asm = run(
        &argparse_decls(),
        &values,
        &[],
        true,
        &[],
        &FakeGlob::default(),
    )
    .unwrap();
    assert!(asm.args.contains(&"/run/dir/14-30-05.png".to_owned()));
}

/// Plain (token-free) values were already checked by pre-submit validate(); Python
/// `assemble` only re-checks token-bearing values, so a plain "abc" for an int passes
/// straight through. The Rust pipeline re-validates every value in `prepare_values`, so
/// `assemble_run_inputs` errors instead. Genuine oracle divergence (stale saved values).
#[test]
#[ignore = "ARCHITECTURE-CLOSED / STAGE-FUSION: Python exposes flows.assemble after a separate validation pass, so this frozen direct-stage test supplies an invalid plain value under an already-validated precondition and proves assembly does not recheck it. Rust intentionally exposes one public assemble_run_inputs pipeline that owns resolution, validation, preparation, glob expansion, and delivery; it has no opaque already-validated-values seam. Active application and CLI owners pin the public v0.4 outcome: stale invalid typed state is rejected before launch. delivery::assemble is only the prepared routing stage and is not an equivalent owner. Keep this body ignored at the honest fused seam; do not weaken validation or count it as REAL."]
fn test_assemble_does_not_retypecheck_plain_values() {
    let mut values = values_ok();
    values[2] = ("gap", "abc");
    let asm = run(
        &argparse_decls(),
        &values,
        &[],
        true,
        &[],
        &FakeGlob::default(),
    )
    .unwrap();
    let index = asm.args.iter().position(|arg| arg == "--gap").unwrap();
    assert_eq!(asm.args[index + 1], "abc");
}

/// With no env kwarg, Python `assemble` reads the process environment. skit-application
/// takes an explicit TokenContext by design; the os.environ default lives in the
/// composition root.
#[test]
#[ignore = "cross-crate (composition root): the process-environment default for a missing env arg is applied where TokenContext is built (skit-cli), not inside skit-application, which receives ambient state explicitly."]
fn test_assemble_defaults_env_to_os_environ() {}

/// _assemble_flags defends against a `final` lacking keys: a missing value field is
/// omitted (not injected as a sentinel).
#[test]
fn test_assemble_flags_tolerates_missing_keys() {
    let asm = run(
        &argparse_decls(),
        &[("inputs", "a"), ("output", "o")],
        &[],
        true,
        &[],
        &FakeGlob::default(),
    )
    .unwrap();
    assert_eq!(asm.args, ["a", "--output", "o"]);
}

/// An empty optional field is skipped, not a hard stop: a later filled field must still
/// be assembled (kills continue->break).
#[test]
fn test_assemble_empty_field_does_not_stop_later_flags() {
    let mut values = values_ok();
    values[2] = ("gap", "");
    let asm = run(
        &argparse_decls(),
        &values,
        &[],
        true,
        &[],
        &FakeGlob::default(),
    )
    .unwrap();
    assert!(asm.args.contains(&"--mode".to_owned()));
}

/// A multi-value field whose text cannot be shlex-split (unbalanced quote) falls back to
/// the whole raw value instead of crashing.
#[test]
fn test_split_multi_falls_back_on_unbalanced_quote() {
    let mut values = values_ok();
    values[0] = ("inputs", "a\"b");
    let asm = run(
        &argparse_decls(),
        &values,
        &[],
        true,
        &[],
        &FakeGlob::default(),
    )
    .unwrap();
    assert_eq!(asm.args[0], "a\"b");
}

/// A secret with neither a typed value nor an env_source resolves to "" (nothing
/// delivered), never a placeholder string.
#[test]
fn test_resolve_secret_empty_when_no_input_and_no_env_source() {
    let mut secret = ParamDecl::new("k");
    secret.secret = true;
    let resolved = resolve_values(&[secret], &map(&[("k", "")]), &context(&[])).unwrap();
    assert_eq!(resolved["k"], "");
}

/// A value that IS among the choices validates clean.
#[test]
fn test_validate_value_accepts_a_valid_choice() {
    let mut choice = ParamDecl::new("m");
    choice.parameter_type = ParameterType::Choice;
    choice.choices = strings(&["a", "b"]);
    assert!(validate_form_value(&choice, "a").is_ok());
    assert!(validate_form_value(&choice, "b").is_ok());
}

/// Defense in depth: even if a secret key sits in saved values, prefill must not surface it.
#[test]
fn test_prefill_drops_a_secret_that_leaked_into_saved_values() {
    let decls = managed_decls();
    let leaked = map(&[("OUTPUT", "o.jpg"), ("API_KEY", "leaked")]);
    let values = prefill(&decls, &leaked, None);
    assert_eq!(values["OUTPUT"], "o.jpg");
    assert!(!values.contains_key("API_KEY"));
}

/// Same guard, preset branch.
#[test]
fn test_prefill_preset_drops_leaked_secret() {
    let decls = managed_decls();
    let leaked = map(&[("OUTPUT", "web.jpg"), ("API_KEY", "leaked")]);
    let values = prefill(&decls, &BTreeMap::new(), Some(&leaked));
    assert_eq!(values["OUTPUT"], "web.jpg");
    assert!(!values.contains_key("API_KEY"));
}

/// A never-saved preset yields an empty overlay, not a crash.
#[test]
fn test_prefill_unknown_preset_is_no_op_not_a_crash() {
    let decls = managed_decls();
    let service = service();
    let slug = slug("up");
    let state = service.load(&slug);
    let values = prefill(&decls, &state.values, state.presets.get("ghost"));
    assert_eq!(values["OUTPUT"], "out.jpg");
}

// --------------------------------------------------------------------------
// glob feedback + run recording
// --------------------------------------------------------------------------

/// The reachable half of glob_feedback: a plain piece reports None; glob pieces are
/// shlex-split into the count request. The actual match COUNT (2, 3, and the 2+2=4
/// accumulate) is computed by the GlobCountPort adapter in skit-store::path_glob.
#[test]
fn test_glob_feedback_counts() {
    assert!(glob_count_request("plain.txt", "/cwd").is_none());
    assert_eq!(
        glob_count_request("*.png", "/cwd").unwrap().pieces,
        ["*.png"]
    );
    assert_eq!(
        glob_count_request("*.png extra.txt", "/cwd")
            .unwrap()
            .pieces,
        ["*.png", "extra.txt"]
    );
    assert_eq!(
        glob_count_request("*.png ?.png", "/cwd").unwrap().pieces,
        ["*.png", "?.png"]
    );
}

/// Persist intent (raw token text, not expansion); secrets stripped; stamp the run.
#[test]
fn test_save_after_run_persists_intent_and_stamps_run() {
    let decls = managed_decls();
    let service = service();
    let slug = slug("s");
    let values = map(&[
        ("OUTPUT", "long_{today}.jpg"),
        ("WIDTH", "800"),
        ("API_KEY", "secret!"),
    ]);
    save_after_run(
        &service,
        &slug,
        &decls,
        &values,
        strings(&["--fast"]),
        0,
        "2026-07-09T14:30:05+00:00",
    );
    let state = service.load(&slug);
    // raw token text, not expansion; WIDTH equals its default and is not remembered; C3.
    assert_eq!(state.values, map(&[("OUTPUT", "long_{today}.jpg")]));
    assert!(!state.values.contains_key("API_KEY"));
    assert_eq!(state.extra_args, ["--fast"]);
    assert_eq!(
        state.last_run.at.as_deref(),
        Some("2026-07-09T14:30:05+00:00")
    );
    assert_eq!(state.last_run.exit, Some(0));
    assert_eq!(
        state.last_run.values,
        Some(map(&[("OUTPUT", "long_{today}.jpg"), ("WIDTH", "800")]))
    );
}

#[test]
fn test_record_run_zero_exit_survives_save() {
    let service = service();
    let slug = slug("z");
    service
        .record_run(&slug, 0, "2026-07-09T00:00:00+00:00", &[], None)
        .unwrap();
    assert_eq!(service.load(&slug).last_run.exit, Some(0));
}

// --------------------------------------------------------------------------
// mutation hardening: the small helpers
// --------------------------------------------------------------------------

/// flows.truthy is THE single public bool-spelling rule every renderer shares — the same
/// spellings assembly fires the flag on. There is no public `truthy` in Rust, so this
/// observes the rule through delivery routing: a store_true flag fires for exactly the
/// truthy spellings. The `_coerce_bool_lenient is truthy` identity assertion has no
/// translation — Rust `delivery::truthy` is the one private routing helper.
#[test]
fn test_truthy_accepts_every_truthy_spelling() {
    let mut flag = ParamDecl::new("v");
    flag.parameter_type = ParameterType::Bool;
    flag.flag = "--v".to_owned();
    flag.action = "store_true".to_owned();

    for spelling in ["true", "1", "yes", "y", "on", " TRUE ", "On"] {
        let asm = delivery::assemble(
            std::slice::from_ref(&flag),
            &prepared(&[("v", scalar(spelling))]),
            &[],
        )
        .unwrap();
        assert_eq!(asm.args, ["--v"], "{spelling:?}");
    }
    for spelling in ["false", "0", "no", "n", "off", "", "garbage"] {
        let asm = delivery::assemble(
            std::slice::from_ref(&flag),
            &prepared(&[("v", scalar(spelling))]),
            &[],
        )
        .unwrap();
        assert!(asm.args.is_empty(), "{spelling:?}");
    }
}

/// A values dict that never mentions the checkbox must behave as unchecked, not crash.
#[test]
fn test_assemble_tolerates_a_bool_field_missing_from_values() {
    let mut values = values_ok();
    values.retain(|(name, _)| *name != "fast");
    let asm = run(
        &argparse_decls(),
        &values,
        &[],
        true,
        &[],
        &FakeGlob::default(),
    )
    .unwrap();
    assert!(!asm.args.contains(&"--fast".to_owned()));
}

#[test]
fn test_assemble_store_false_fires_flag_when_unchecked() {
    let mut color = ParamDecl::new("color");
    color.parameter_type = ParameterType::Bool;
    color.flag = "--color".to_owned();
    color.action = "store_false".to_owned();

    let checked = delivery::assemble(
        std::slice::from_ref(&color),
        &prepared(&[("color", scalar("true"))]),
        &[],
    )
    .unwrap();
    assert!(checked.args.is_empty()); // matches the script default: no flag

    let unchecked = delivery::assemble(
        std::slice::from_ref(&color),
        &prepared(&[("color", scalar("false"))]),
        &[],
    )
    .unwrap();
    assert_eq!(unchecked.args, ["--color"]);
}

// --------------------------------------------------------------------------
// assembly: repeated-option (click/parseArgs) delivery
// --------------------------------------------------------------------------

fn repeat_field(flag: &str, repeat: bool) -> ParamDecl {
    let mut field = ParamDecl::new("tag");
    field.flag = flag.to_owned();
    field.multiple = true;
    field.repeat = repeat;
    field
}

/// click multiple / parseArgs multiple: each value travels behind its OWN flag occurrence.
#[test]
fn test_assemble_repeat_emits_flag_before_each_piece() {
    let asm = run(
        &[repeat_field("--tag", true)],
        &[("tag", "a b")],
        &[],
        true,
        &[],
        &FakeGlob::default(),
    )
    .unwrap();
    assert_eq!(asm.args, ["--tag", "a", "--tag", "b"]);
}

/// multiple but NOT repeat is argparse nargs grammar — one flag, then every value.
#[test]
fn test_assemble_non_repeat_multi_keeps_one_flag_then_values() {
    let asm = run(
        &[repeat_field("--tag", false)],
        &[("tag", "a b")],
        &[],
        true,
        &[],
        &FakeGlob::default(),
    )
    .unwrap();
    assert_eq!(asm.args, ["--tag", "a", "b"]);
}

/// One value still goes through the per-piece path: flag then the lone value.
#[test]
fn test_assemble_repeat_single_piece() {
    let asm = run(
        &[repeat_field("--tag", true)],
        &[("tag", "a")],
        &[],
        true,
        &[],
        &FakeGlob::default(),
    )
    .unwrap();
    assert_eq!(asm.args, ["--tag", "a"]);
}

/// repeat runs the SAME shlex + glob split as non-repeat; only the emission shape differs.
#[test]
fn test_assemble_repeat_shares_shlex_and_glob_split_with_non_repeat() {
    let glob = glob_with(&[("*.png", &["1.png", "2.png"])]);
    let value = &[("tag", "'a b' *.png")];

    let rep = run(&[repeat_field("--src", true)], value, &[], true, &[], &glob).unwrap();
    assert_eq!(
        rep.args,
        ["--src", "a b", "--src", "1.png", "--src", "2.png"]
    );

    let plain = run(
        &[repeat_field("--src", false)],
        value,
        &[],
        true,
        &[],
        &glob,
    )
    .unwrap();
    assert_eq!(plain.args, ["--src", "a b", "1.png", "2.png"]);
}

// --------------------------------------------------------------------------
// assembly: bool guard — a flag must be present to fire, never an empty argv element
// --------------------------------------------------------------------------

fn bool_field(flag: &str, action: &str) -> ParamDecl {
    let mut field = ParamDecl::new("v");
    field.parameter_type = ParameterType::Bool;
    field.flag = flag.to_owned();
    field.action = action.to_owned();
    field
}

#[test]
fn test_assemble_bool_store_true_fires_only_when_checked() {
    let field = bool_field("--v", "store_true");
    assert_eq!(
        delivery::assemble(
            std::slice::from_ref(&field),
            &prepared(&[("v", scalar("true"))]),
            &[]
        )
        .unwrap()
        .args,
        ["--v"]
    );
    assert!(
        delivery::assemble(
            std::slice::from_ref(&field),
            &prepared(&[("v", scalar("false"))]),
            &[]
        )
        .unwrap()
        .args
        .is_empty()
    );
}

/// A flagless bool that WOULD fire must append nothing — never argv "".
#[test]
fn test_assemble_bool_flagless_never_appends_empty_string() {
    let st = delivery::assemble(
        &[bool_field("", "store_true")],
        &prepared(&[("v", scalar("true"))]),
        &[],
    )
    .unwrap();
    assert!(st.args.is_empty());
    assert!(!st.args.contains(&String::new()));

    let sf = delivery::assemble(
        &[bool_field("", "store_false")],
        &prepared(&[("v", scalar("false"))]),
        &[],
    )
    .unwrap();
    assert!(sf.args.is_empty());
    assert!(!sf.args.contains(&String::new()));
}

/// A raw field with an empty action fires in NO state (the guard requires a concrete
/// store_true/store_false).
#[test]
fn test_assemble_bool_empty_action_fires_in_neither_state() {
    let field = bool_field("--v", "");
    assert!(
        delivery::assemble(
            std::slice::from_ref(&field),
            &prepared(&[("v", scalar("true"))]),
            &[]
        )
        .unwrap()
        .args
        .is_empty()
    );
    assert!(
        delivery::assemble(
            std::slice::from_ref(&field),
            &prepared(&[("v", scalar("false"))]),
            &[]
        )
        .unwrap()
        .args
        .is_empty()
    );
}

// --------------------------------------------------------------------------
// FormField.from_decl (cross-crate: skit-form has no FormField projection)
// --------------------------------------------------------------------------

#[test]
#[ignore = "cross-crate (skit-form): FormField.from_decl projecting every ParamDecl field (key/label/kind/source/choices/default/has_default/help/required/secret/multiple/degraded/flag/action) onto a render-only model has no skit-application equivalent; PreparedField wraps the ParamDecl and frontends collapse label/kind."]
fn test_field_from_arg_maps_every_field() {}

#[test]
#[ignore = "cross-crate (skit-form): the degraded-renders-as-text projection (kind='str', empty default, has_default=false) is FormField.from_decl in the frontend layer."]
fn test_field_from_arg_degraded_renders_as_text() {}

#[test]
#[ignore = "cross-crate (skit-form): FormField.from_decl copying multiple/repeat onto the flag form field."]
fn test_field_from_arg_copies_repeat() {}

#[test]
#[ignore = "cross-crate (skit-form): store_true defaulting for a bool flag with an empty action happens in FormField.from_decl / plan projection, not in assembly."]
fn test_field_from_arg_bool_flag_empty_action_defaults_store_true() {}

#[test]
#[ignore = "cross-crate (skit-form): the degraded gate on the store_true default is a FormField.from_decl concern."]
fn test_field_from_arg_bool_flag_degraded_stays_text_and_keeps_empty_action() {}

#[test]
#[ignore = "cross-crate (skit-form): a flagless bool keeping its empty action is FormField.from_decl."]
fn test_field_from_arg_bool_positional_no_flag_keeps_empty_action() {}

#[test]
#[ignore = "cross-crate (skit-form): preserving an explicit action against the store_true default is FormField.from_decl."]
fn test_field_from_arg_bool_flag_explicit_action_preserved() {}

/// _render_default spells booleans lowercase / ints and strings verbatim. Rust renders
/// declaration defaults through the same rule inside prefill, so this observes it publicly.
#[test]
fn test_render_default_spells_booleans_lowercase() {
    let mut yes = ParamDecl::new("YES");
    yes.parameter_type = ParameterType::Bool;
    yes.default = Some(ParameterValue::Bool(true));
    let mut no = ParamDecl::new("NO");
    no.parameter_type = ParameterType::Bool;
    no.default = Some(ParameterValue::Bool(false));
    let mut num = ParamDecl::new("NUM");
    num.parameter_type = ParameterType::Int;
    num.default = Some(ParameterValue::Integer(8));
    let mut text = ParamDecl::new("TEXT");
    text.default = Some(ParameterValue::String("x".to_owned()));

    let rendered = prefill(&[yes, no, num, text], &BTreeMap::new(), None);
    assert_eq!(rendered["YES"], "true");
    assert_eq!(rendered["NO"], "false");
    assert_eq!(rendered["NUM"], "8");
    assert_eq!(rendered["TEXT"], "x");
}

#[test]
#[ignore = "cross-crate (skit-form::form_plan): per-field FormField.source ('placeholder') is a projection fact in skit-form."]
fn test_plan_sources_are_exact_per_field() {}

#[test]
#[ignore = "cross-crate (skit-form::form_plan): per-field FormField.source ('inject' vs 'flag') is decided in skit-form."]
fn test_plan_field_sources_inject_and_flag() {}

#[test]
#[ignore = "cross-crate (skit-form::form_plan): drift banners naming the entry, and dropping missing definitions from FormPlan.specs/fields/text, are skit-form reconciliation (FormDrift is typed there, without the localized banner string)."]
fn test_plan_drift_names_entry_and_keeps_usable_specs() {}

#[test]
#[ignore = "cross-crate (skit-form::form_plan): whole-parser subparser degradation (degraded_reason, empty fields, FormPlan.text) is skit-form + skit-language."]
fn test_plan_subparsers_degrades_with_reason() {}

/// The exact user-facing wording ("gap needs a whole number — you typed 'abc'." etc.) is
/// rendered in skit-tui/src/session.rs from the ParameterType, not by skit-application's
/// typed ValuePreparationError. The typed variants themselves are asserted in
/// test_validate_int_error_names_field_and_value / test_validate_choice / the required
/// line of test_validate_required_empty.
#[test]
#[ignore = "cross-crate (skit-tui/src/session.rs): the exact int/float/bool/choice validation sentences are formatted in the TUI session renderer; skit-application carries only the typed error."]
fn test_type_error_messages_exact() {}

/// WIDTH "900" differs from the source default 800 on purpose (equal values still deliver
/// at the assemble level; the injector, not assemble, skips source-equal ones).
#[test]
fn test_assemble_display_order_and_masking() {
    let asm = run(
        &managed_decls(),
        &[
            ("OUTPUT", "long_{today}.jpg"),
            ("WIDTH", "900"),
            ("API_KEY", "sekret"),
        ],
        &[],
        true,
        &[],
        &FakeGlob::default(),
    )
    .unwrap();
    assert_eq!(
        asm.display,
        [
            ("OUTPUT".to_owned(), "long_2026-07-09.jpg".to_owned()),
            ("WIDTH".to_owned(), "900".to_owned()),
            ("API_KEY".to_owned(), "•••".to_owned()),
        ]
    );
    assert_eq!(
        asm.inject_values,
        map(&[
            ("OUTPUT", "long_2026-07-09.jpg"),
            ("WIDTH", "900"),
            ("API_KEY", "sekret"),
        ])
    );
    assert_eq!(asm.masked_args, asm.args); // inject: values are not in argv, nothing to mask
}

#[test]
fn test_assemble_none_plan_only_carries_extras() {
    let asm = run(&[], &[], &["-v"], true, &[], &FakeGlob::default()).unwrap();
    assert_eq!(asm.args, ["-v"]);
    assert_eq!(asm.masked_args, ["-v"]);
    assert!(asm.inject_values.is_empty());
    assert!(asm.command_values.is_empty());
    assert!(asm.display.is_empty());
}

// --------------------------------------------------------------------------
// Placeholder requiredness, persisted clearing, and transparency masking
// --------------------------------------------------------------------------

/// C3 applies to every source: a credential-named placeholder is secret; every
/// placeholder is required so empty values must not assemble silently.
#[test]
fn test_command_placeholders_are_required_and_secret_prechecked() {
    let api_key = synthesized_placeholder("api_key");
    let url = synthesized_placeholder("url");
    assert!(api_key.secret);
    assert!(!url.secret);
    assert!(api_key.required && url.required);

    let errors = validate_keys(&[api_key, url], &map(&[("api_key", ""), ("url", "")]));
    assert_eq!(
        errors,
        BTreeSet::from(["api_key".to_owned(), "url".to_owned()])
    );
}

/// The user emptied the extra-args field: the cleared state must PERSIST.
#[test]
fn test_save_after_run_clears_cleared_extra_args() {
    let decls = managed_decls();
    let service = service();
    let slug = slug("clr");
    let values = map(&[("OUTPUT", "a")]);

    save_after_run(
        &service,
        &slug,
        &decls,
        &values,
        strings(&["--fast"]),
        0,
        "2026-01-01T00:00:00+00:00",
    );
    assert_eq!(service.load(&slug).extra_args, ["--fast"]);

    save_after_run(
        &service,
        &slug,
        &decls,
        &values,
        Vec::new(),
        0,
        "2026-01-01T00:00:01+00:00",
    );
    assert!(service.load(&slug).extra_args.is_empty());
}

/// A placeholder that is secret NOW must not keep old plaintext in values or presets.
#[test]
fn test_save_after_run_purges_secret_placeholder_from_presets() {
    let decls = vec![synthesized_placeholder("api_key")];
    let service = service();
    let slug = slug("c3");
    // Plaintext saved back when the placeholder was not treated as secret yet.
    service
        .repository()
        .update(&slug, |state| {
            state
                .presets
                .insert("old".to_owned(), map(&[("api_key", "sk-123")]));
            state.values = map(&[("api_key", "sk-123")]);
        })
        .unwrap();

    save_after_run(
        &service,
        &slug,
        &decls,
        &map(&[("api_key", "sk-456")]),
        Vec::new(),
        0,
        "2026-01-01T00:00:00+00:00",
    );
    let state = service.load(&slug);
    assert!(!state.values.contains_key("api_key"));
    assert!(
        state
            .presets
            .values()
            .all(|preset| !preset.contains_key("api_key"))
    );
}

/// The CLI's argv already went through the user's shell: no re-glob, no token pass, and
/// an unset {env:...} is NOT an error — it is just text the script will receive.
#[test]
fn test_assemble_expand_extra_false_passes_argv_untouched() {
    let asm = run(
        &[],
        &[],
        &["x*.txt", "{env:UNSET_VAR}"],
        false,
        &[],
        &FakeGlob::default(),
    )
    .unwrap();
    assert_eq!(asm.args, ["x*.txt", "{env:UNSET_VAR}"]);
}

#[test]
fn test_masked_args_hide_flag_source_secret_values() {
    let mut api_key = ParamDecl::new("api_key");
    api_key.flag = "--api-key".to_owned();
    api_key.secret = true;
    let mut name = ParamDecl::new("name");
    name.flag = "--name".to_owned();

    let asm = run(
        &[api_key, name],
        &[("api_key", "sk-secret"), ("name", "ada")],
        &[],
        true,
        &[],
        &FakeGlob::default(),
    )
    .unwrap();
    assert_eq!(asm.args, ["--api-key", "sk-secret", "--name", "ada"]);
    assert_eq!(asm.masked_args, ["--api-key", "•••", "--name", "ada"]);
}

/// The masked mirror runs through the same assembly: a multiple field must glob-expand
/// identically on both sides while the secret stays masked.
#[test]
fn test_masked_args_still_glob_expand_multiple_fields() {
    let mut inputs = ParamDecl::new("inputs");
    inputs.multiple = true;
    let mut api_key = ParamDecl::new("api_key");
    api_key.flag = "--api-key".to_owned();
    api_key.secret = true;

    let glob = glob_with(&[("*.png", &["a.png"])]);
    let asm = run(
        &[inputs, api_key],
        &[("inputs", "*.png"), ("api_key", "sk-1")],
        &[],
        true,
        &[],
        &glob,
    )
    .unwrap();
    assert_eq!(asm.args, ["a.png", "--api-key", "sk-1"]);
    assert_eq!(asm.masked_args, ["a.png", "--api-key", "•••"]);
}

// --------------------------------------------------------------------------
// execute — the unified delivery pipeline (cross-crate: skit-runtime/skit-cli)
// --------------------------------------------------------------------------

#[test]
#[ignore = "cross-crate (skit-runtime/skit-cli): flows.execute runs launcher.run_entry and returns the script's exit code; the launch pipeline is skit-runtime, wired in skit-cli."]
fn test_execute_runs_and_returns_the_scripts_exit_code() {}

#[test]
#[ignore = "cross-crate (skit-runtime/skit-cli): the prompt-boundary secret warning inside execute is emitted from the launch orchestration, not skit-application."]
fn test_command_template_secret_does_not_get_prompt_agent_warning() {}

#[test]
#[ignore = "cross-crate (skit-runtime/skit-cli): the amp one-shot warning on a runner=None prompt rerun is resolved inside PromptLaunch during execute."]
fn test_pinned_amp_prompt_warns_on_runner_none_shared_execution_path() {}

#[test]
#[ignore = "cross-crate (skit-runtime/skit-cli): execute injecting a temp copy, passing entry.dir fallback, and cleaning up afterward is the launcher/injector path."]
fn test_execute_injects_then_cleans_up_the_temp_copy() {}

#[test]
#[ignore = "cross-crate (skit-runtime/skit-cli): classifying TargetMissingError -> FAIL_MISSING is execute's mapping of the launcher exception hierarchy."]
fn test_execute_classifies_missing_target() {}

#[test]
#[ignore = "cross-crate (skit-runtime/skit-cli): classifying NotExecutableError -> FAIL_NOT_EXECUTABLE is execute over the launcher."]
fn test_execute_classifies_not_executable() {}

#[test]
#[ignore = "cross-crate (skit-runtime/skit-cli): prompt-body existence preflight before transparency is the launch boundary."]
fn test_prompt_validation_classifies_missing_body_before_transparency() {}

#[test]
#[ignore = "cross-crate (skit-runtime/skit-cli): an empty runner-config preflight before transparency is the launch boundary + config store."]
fn test_prompt_validation_classifies_empty_runner_config_before_transparency() {}

#[test]
#[ignore = "cross-crate (skit-runtime/skit-cli): mapping a shim/inject failure to FAIL_DRIFT with a resync hint is execute over the injector."]
fn test_execute_classifies_injection_drift() {}

#[test]
#[ignore = "cross-crate (skit-runtime/skit-cli): the injector's bad-value error (FAIL_BAD_VALUE, distinct from drift) is raised in the launch/inject path, not skit-application."]
fn test_execute_bad_value_reports_value_not_drift() {}

// --------------------------------------------------------------------------
// transparency
// --------------------------------------------------------------------------

/// The inject line shows masked `k = v` pairs (no repr quotes — the old CLI/TUI drift)
/// and the temporary-copy note; the secret value never appears. The described command is
/// built over an Entry in skit-cli, so a neutral command stand-in stands in for it here.
#[test]
fn test_transparency_lines_inject_source_shows_masked_and_temp_note() {
    let asm = run(
        &managed_decls(),
        &[
            ("OUTPUT", "new.jpg"),
            ("WIDTH", "900"),
            ("API_KEY", "sekret"),
        ],
        &[],
        true,
        &[],
        &FakeGlob::default(),
    )
    .unwrap();
    let joined = transparency_messages(&asm, "python script.py")
        .into_iter()
        .map(|message| message.localize(Locale::En))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(joined.contains("→ inject:"));
    assert!(joined.contains("OUTPUT = new.jpg")); // plain `k = v`, no repr quotes
    assert!(joined.contains("temporary copy"));
    assert!(!joined.contains("sekret")); // the secret value never appears
    assert!(joined.contains("•••"));
}

/// The "→ inject:" display lists ONLY inject-delivered values: env values render as a
/// VAR=value prefix and flag values appear in the command line itself. One mixed plan,
/// three deliveries, one honest inject line — every assertion is on `asm`.
#[test]
fn test_assemble_display_lists_only_inject_delivered_values() {
    let mut out = ParamDecl::new("OUT");
    out.delivery = ParameterDelivery::Inject;
    let mut city = ParamDecl::new("CITY");
    city.delivery = ParameterDelivery::Env;
    let mut name = ParamDecl::new("name");
    name.flag = "--name".to_owned();

    let asm = run(
        &[out, city, name],
        &[("OUT", "out.jpg"), ("CITY", "Taipei"), ("name", "ada")],
        &[],
        true,
        &[],
        &FakeGlob::default(),
    )
    .unwrap();
    // Only the inject field appears in the inject display; env/flag deliver elsewhere.
    assert_eq!(asm.display, [("OUT".to_owned(), "out.jpg".to_owned())]);
    assert_eq!(asm.masked_env, map(&[("CITY", "Taipei")])); // env renders its own prefix
    assert!(asm.args.contains(&"--name".to_owned())); // the flag rides the command line…
    assert!(asm.args.contains(&"ada".to_owned())); // …with its value
}

/// A flag-source run has no inject note: transparency is a single command line.
#[test]
fn test_transparency_lines_flag_source_is_single_command_line() {
    let asm = run(
        &argparse_decls(),
        &values_ok(),
        &[],
        true,
        &[],
        &FakeGlob::default(),
    )
    .unwrap();
    let lines = transparency_messages(&asm, "python script.py a.png --output o.png")
        .into_iter()
        .map(|message| message.localize(Locale::En))
        .collect::<Vec<_>>();
    assert_eq!(lines.len(), 1);
    assert!(lines[0].starts_with("→ "));
}

/// Exact first line (kills the ", " separator and the "→ inject: " string mutants).
#[test]
fn test_transparency_inject_lines_are_exact() {
    let asm = run(
        &managed_decls(),
        &[("OUTPUT", "new.jpg"), ("WIDTH", "900"), ("API_KEY", "s")],
        &[],
        true,
        &[],
        &FakeGlob::default(),
    )
    .unwrap();
    let lines = transparency_messages(&asm, "python script.py")
        .into_iter()
        .map(|message| message.localize(Locale::En))
        .collect::<Vec<_>>();
    assert_eq!(
        lines[0],
        "→ inject: OUTPUT = new.jpg, WIDTH = 900, API_KEY = •••"
    );
    assert!(lines[1].starts_with("  (written to a temporary copy"));
}

#[test]
#[ignore = "cross-crate (skit-runtime/skit-cli): the command line naming the .injected- temp copy is launcher.describe_command."]
fn test_transparency_shows_the_injected_temp_path() {}

#[test]
#[ignore = "cross-crate (skit-runtime/skit-cli): masking a secret flag inside the described command line is launcher.describe_command over masked_args; transparency_messages only wraps a prebuilt command string."]
fn test_transparency_flag_source_masks_secret_in_command() {}

#[test]
#[ignore = "cross-crate (skit-runtime/skit-cli): filling a command template into the shown line is launcher.describe_command over masked_command_values."]
fn test_transparency_command_source_shows_filled_template() {}

#[test]
#[ignore = "cross-crate (skit-runtime/skit-cli): masking a secret command placeholder in the shown line is launcher.describe_command."]
fn test_transparency_command_source_masks_secret_placeholder() {}

#[test]
#[ignore = "cross-crate (skit-runtime/skit-cli): compact prompt transparency (PromptLaunch.describe_compact, 'rendered prompt omitted', never reading the body) is the prompt launch strategy."]
fn test_normal_prompt_transparency_is_compact_and_never_reads_the_body() {}

#[test]
#[ignore = "cross-crate (skit-runtime/skit-cli): execute forwarding NotExecutableError's message is the launch path."]
fn test_execute_not_executable_message_carries_the_error() {}

#[test]
#[ignore = "cross-crate (skit-runtime/skit-cli): execute mapping LaunchError -> FAIL_LAUNCH with its message is the launch path."]
fn test_execute_launch_error_message_carries_the_error() {}

#[test]
#[ignore = "cross-crate (skit-runtime/skit-cli): execute forwarding invoke_cwd into run_entry is the launcher call seam."]
fn test_execute_forwards_invoke_cwd() {}

#[test]
#[ignore = "cross-crate (skit-runtime/skit-cli): the OS-temp-dir fallback to entry.dir for the injected copy is write_injected inside the injector."]
fn test_execute_inject_falls_back_to_entry_dir() {}

// --------------------------------------------------------------------------
// typed multi-value validation
// --------------------------------------------------------------------------

/// A multi-value field applies its type to each PIECE, not the whole box. The Ok cases
/// port cleanly. For the failing case the oracle quotes the WHOLE input ('1 x'); the Rust
/// typed error carries the failing PIECE ("x") and the exact sentence is skit-tui — so
/// this asserts skit-application's contract: each piece is validated and the typed error names the
/// bad PIECE. The oracle's user-facing message quotes the WHOLE input ("'1 x'"); that is NOT a
/// divergence -- both frontends reproduce it (skit-tui session.rs:2293 renders the whole control
/// value; skit-ui run.rs uses a value-less InvalidType), so the whole-value message is a faithful
/// cross-crate concern, asserted where it lives, not here.
#[test]
fn test_typed_multi_value_field_validates_each_piece_not_the_whole_box() {
    let mut field = ParamDecl::new("point");
    field.parameter_type = ParameterType::Int;
    field.multiple = true;
    assert!(validate_form_value(&field, "1 2").is_ok());
    assert!(validate_form_value(&field, "1 -2 30").is_ok());
    let error = validate_form_value(&field, "1 x").unwrap_err();
    // skit-application reports the failing piece "x"; the whole-value message is the frontends' job.
    assert_eq!(
        error,
        ValuePreparationError::InvalidType {
            name: "point".to_owned(),
            value: "x".to_owned(),
            parameter_type: ParameterType::Int,
        }
    );
}

/// Without multiple, a space-separated pair is exactly the wrong input.
#[test]
fn test_single_value_field_still_validates_the_whole_string() {
    let mut field = ParamDecl::new("n");
    field.parameter_type = ParameterType::Int;
    assert!(validate_form_value(&field, "1 2").is_err());
    assert!(validate_form_value(&field, "12").is_ok());
}
