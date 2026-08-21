//! Mechanical port of the Python oracle module `tests/test_default_semantics_review_fixes.py`
//! (`origin/main@206f9ef`): the seven review-fix regressions on the source-default change set —
//! every case where "don't re-deliver a value equal to the default" dropped a value the form had
//! already shown. Each `#[test]` keeps its Python `def test_*` name; each Python "WHY" comment is
//! preserved above it.
//!
//! Concept mapping (Python -> Rust):
//! - `skit.analysis.reconcile(text, specs, analyze=py_analyze)` and
//!   `skit.langs.shell.analyzer.reconcile(text, specs)` -> parse then `ParsedDocument::reconcile`
//!   (the `reconcile` helper below; `ReconcileReport::from_syntax_error` on a parse failure).
//!   Report fields: `.ok` is `Vec<ReconcilePair>` (`pair.stored.name`); `.current_defaults` is a
//!   `BTreeMap<String, ParameterValue>`; `.empty_uses_default` is a `BTreeSet<String>`.
//! - `skit.langs.python.shim.inject(text, specs, values)` -> `ParsedDocument::plan_injection(specs,
//!   values).apply(text)` (the `inject_python` helper). The Python `_skit_i[K]` marker is the same
//!   byte string in both.
//! - `skit.langs.python.metawriter.write_params(body, specs)` -> `write_managed_params("python",
//!   body, &specs)`.
//! - live shell colon semantics (`field.empty_uses_default`) -> `source_parameter_semantics(...)`.
//! - `ParamDecl.type/default/secret/binding` -> the same-named `skit_domain` fields.
//!
//! Buckets:
//! - REAL (asserting `#[test]`): the language-owned slices — shell/python reconcile
//!   (`current_defaults`, secret exclusion, `empty_uses_default`), python shim injection, and shell
//!   `source_parameter_semantics`. Seven defs land here (3–6, 8, 17, 18); several of those defs
//!   ALSO carry cross-crate assertions (flows/CLI) that this crate cannot reach — each such slice is
//!   named in the test comment and recorded as a gap.
//! - PRIVATE UNIT: def 7. The const lane of the same coercibility gate is reachable only through a
//!   SYNTHETIC `analyze` returning an int/int candidate over a non-int literal. The exact oracle
//!   name lives beside private `reconcile_analysis` in `semantic.rs`, without exposing a test seam.
//! - CROSS-CRATE (`#[ignore]`, compiling stub): the flows / argstate / preset / `params --json` /
//!   `show --json` / `edit_specs` defs (1, 2, 9-16). Those use cases live in
//!   skit-application / skit-form / skit-store / skit-cli, which this integration test cannot depend
//!   on without a Cargo.toml edit (out of scope).

use std::collections::BTreeMap;

use skit_domain::parameters::{
    NamedEdit, ParamDecl, ParameterBinding, ParameterDelivery, ParameterType, ParameterValue,
    SourceEditRequest,
};
use skit_language::{
    ParseOutcome, ReconcileReport, edit_source_declarations, managed_params, parse_document,
    source_parameter_semantics, write_managed_params,
};

// A secret const whose source literal is the empty string — the shape that made the "skip a value
// equal to its default" shortcut lose an env-sourced secret entirely.
const SECRET_SCRIPT: &str = r#"# /// script
# dependencies = []
#
# [tool.skit]
# schema = 1
#
# [[tool.skit.params]]
# name = "API_KEY"
# kind = "const"
# type = "str"
# default = ""
# secret = true
# env_source = "MY_KEY"
# ///
API_KEY = ""
print(API_KEY)
"#;

// An input() binding with a stored default: the value must be intercepted, or a --no-input run
// hangs on stdin forever.
const INPUT_SCRIPT: &str = r#"# /// script
# dependencies = []
#
# [tool.skit]
# schema = 1
#
# [[tool.skit.params]]
# name = "input-1"
# kind = "input"
# type = "str"
# default = "Tim"
# order = 0
# prompt = "Your name? "
# ///
name = input("Your name? ")
print(name)
"#;

// The top-level constant and its main-guard override. Injecting "localhost" over "localhost" looks
// like a no-op — it is not: the guard body says 127.0.0.1.
const MAIN_GUARD_SCRIPT: &str = r#"# /// script
# dependencies = []
#
# [tool.skit]
# schema = 1
#
# [[tool.skit.params]]
# name = "HOST"
# kind = "const"
# type = "str"
# default = "localhost"
# ///
HOST = "localhost"

if __name__ == "__main__":
    HOST = "127.0.0.1"
    print(HOST)
"#;

const SHELL_ENVDEFAULT_BLOCK: &str = r#"#!/usr/bin/env bash
# /// script
# [tool.skit]
# schema = 1
#
# [[tool.skit.params]]
# name = "PORT"
# kind = "envdefault"
# type = "int"
# default = 8080
# ///
PORT=${PORT:-8080}
echo "$PORT"
"#;

const SECRET_LITERAL: &str = "sk-live-SUPERSECRET";

/// Python `analysis.reconcile(text, specs, analyze=...)` / `shell.reconcile(text, specs)`: parse,
/// reconcile against the current source, or return the conservative all-missing report on a syntax
/// error (the production composition across the parse boundary).
fn reconcile(kind: &str, source: &str, specs: &[ParamDecl]) -> ReconcileReport {
    match parse_document(kind, source) {
        ParseOutcome::Parsed(document) => document.reconcile(specs),
        _ => ReconcileReport::from_syntax_error(specs),
    }
}

fn const_decl(name: &str, parameter_type: ParameterType) -> ParamDecl {
    let mut declaration = ParamDecl::new(name);
    declaration.binding = ParameterBinding::Const;
    declaration.delivery = ParameterDelivery::Inject;
    declaration.parameter_type = parameter_type;
    declaration
}

/// Python `_envdefault(name, type_="str", default=None)`.
fn envdefault(
    name: &str,
    parameter_type: ParameterType,
    default: Option<ParameterValue>,
) -> ParamDecl {
    let mut declaration = ParamDecl::new(name);
    declaration.binding = ParameterBinding::EnvDefault;
    declaration.delivery = ParameterDelivery::Env;
    declaration.parameter_type = parameter_type;
    declaration.default = default;
    declaration
}

/// Python `shim.inject(text, specs, values)`: the stored `[tool.skit]` specs drive an in-source
/// rewrite of this exact source version.
fn inject_python(source: &str, values: &BTreeMap<String, String>) -> String {
    let ParseOutcome::Parsed(document) = parse_document("python", source) else {
        panic!("fixture parses");
    };
    let specs = managed_params("python", source);
    document
        .plan_injection(&specs, values)
        .expect("injection plan")
        .apply(source)
        .expect("apply to the exact source")
}

/// Names in `Report.ok`, in stored order (Python `[s.name for s in report.ok]`).
fn ok_names(report: &ReconcileReport) -> Vec<String> {
    report
        .ok
        .iter()
        .map(|pair| pair.stored.name.clone())
        .collect()
}

// --------------------------------------------------------------------------
// 1) a secret's empty source literal must not cancel its delivery
// --------------------------------------------------------------------------

#[test]
#[ignore = "CROSS-CRATE: flows.plan_for_entry/prefill/assemble + the masked display line live in \
            skit-form/skit-application (FormField, delivery::assemble); not reachable from \
            skit-language. The secret exclusion this rule relies on IS pinned by \
            test_secret_source_literal_is_absent_from_reconcile_and_json."]
fn test_secret_with_an_empty_source_literal_is_still_delivered() {
    // `API_KEY = ""` is the canonical secret placeholder: the literal is empty ON PURPOSE and the
    // real value arrives from $MY_KEY. A secret's field is never prefilled, so its raw text is
    // always "" — exactly equal to the recorded default. An earlier revision skipped delivery
    // whenever raw == default, which silently dropped every env-sourced secret.
    let _ = SECRET_SCRIPT; // the oracle fixture these two secret defs drive through flows
}

#[test]
#[ignore = "CROSS-CRATE: FormField.delivers_empty is a skit-form/skit-application concept; \
            skit-language has no delivers-empty surface for a secret."]
fn test_secret_field_never_delivers_empty() {
    // The companion rule: a secret is NOT a delivers-empty field, so an unset env source is a named
    // error rather than an injected ''.
}

// --------------------------------------------------------------------------
// 2) an input() binding with a default must still be intercepted
// --------------------------------------------------------------------------

#[test]
fn test_input_binding_with_a_default_is_delivered() {
    // An input() binding's value is what REPLACES the interactive question. If a value equal to the
    // default were skipped, the injected copy would keep the real input() call and the script would
    // block on stdin — under `--no-input`, forever.
    //
    // Language slice (this crate): the shim actually rewrites the call site, so nothing is left to
    // read stdin. The flows.prefill/assemble/drift_lines assertions from the oracle def are
    // cross-crate (skit-application/skit-form) and are recorded as a gap.
    let injected = inject_python(
        INPUT_SCRIPT,
        &BTreeMap::from([("input-1".to_owned(), "Tim".to_owned())]),
    );
    assert!(injected.contains("_skit_i[0]("));
    assert!(!injected.contains("input(\"Your name? \")"));
}

// --------------------------------------------------------------------------
// 3) a main-guard override must receive the injected value too
// --------------------------------------------------------------------------

#[test]
fn test_main_guard_override_receives_the_unchanged_default() {
    // The form shows HOST = localhost. Submitting it unchanged still injects — and the point of
    // injecting is the SECOND occurrence: the main-guard body reassigns HOST to 127.0.0.1, so "skip
    // a value equal to the default" would have run the script on a host the form (and the
    // transparency line) denied.
    //
    // Language slice (this crate): the shim rewrites BOTH occurrences. The flows.prefill/assemble
    // assertions from the oracle def are cross-crate (skit-application) and are recorded as a gap.
    let injected = inject_python(
        MAIN_GUARD_SCRIPT,
        &BTreeMap::from([("HOST".to_owned(), "localhost".to_owned())]),
    );
    assert_eq!(injected.matches("HOST = 'localhost'").count(), 2); // top level AND the guard body
    assert!(!injected.contains("127.0.0.1")); // the override is gone; the run matches the form
}

// --------------------------------------------------------------------------
// 4) an unfit envdefault default is not published (the coercibility gate)
// --------------------------------------------------------------------------

#[test]
fn test_envdefault_default_that_no_longer_fits_the_type_is_not_published() {
    // An envdefault stays `ok` through a type change (the value arrives by environment either way),
    // so reconcile keeps delivering it. But its SOURCE default may now be text an int param cannot
    // hold: `${PORT:-$FALLBACK}` reads back as the str "$FALLBACK". Publishing that would prefill an
    // int field with "$FALLBACK" — the form opens in error and `--no-input` exits 125.
    //
    // Language slice (this crate): reconcile keeps PORT `ok` (env delivery survives the type change)
    // but the unfit default is withheld. The flows.plan_for_entry/validate assertions from the
    // oracle def are cross-crate (skit-form/skit-application) and are recorded as a gap.
    let text = SHELL_ENVDEFAULT_BLOCK.replace("PORT=${PORT:-8080}", "PORT=${PORT:-$FALLBACK}");
    let report = reconcile(
        "shell",
        &text,
        &[envdefault(
            "PORT",
            ParameterType::Int,
            Some(ParameterValue::Integer(8080)),
        )],
    );
    assert_eq!(ok_names(&report), ["PORT"]); // env delivery survives the type change
    assert!(
        report.current_defaults.is_empty(), // ... but the unfit default is withheld
        "oracle withholds the type-unfit default; Rust published {:?}",
        report.current_defaults
    );
}

#[test]
fn test_int_shaped_literal_still_refreshes_a_str_envdefault() {
    // The positive twin: fitness is COERCIBILITY, not type equality. The analyzers type a literal by
    // its shape, so a `str` param defaulting to 8080 reads back as an int candidate — and must still
    // refresh, because its value is text either way.
    let report = reconcile(
        "shell",
        "PORT=${PORT:-8080}\necho \"$PORT\"\n",
        &[envdefault("PORT", ParameterType::Str, None)],
    );
    assert_eq!(ok_names(&report), ["PORT"]);
    assert_eq!(
        report.current_defaults,
        BTreeMap::from([("PORT".to_owned(), ParameterValue::Integer(8080))])
    );
}

// --------------------------------------------------------------------------
// 5) C3: a secret's source literal never reaches a machine-facing surface
// --------------------------------------------------------------------------

#[test]
fn test_secret_source_literal_is_absent_from_reconcile_and_json() {
    // current_defaults feeds `params --json`, `show --json` and the settings pane — none of which
    // mask anything. Publishing a secret's literal there would take a hardcoded `TOKEN = "sk-live-…"`
    // out of the script's own text for the first time.
    //
    // Language slice (this crate): reconcile withholds the secret's literal from current_defaults.
    // The `params --json` / `show --json` assertions from the oracle def are cross-crate (skit-cli)
    // and are recorded as a gap.
    let body = format!("TOKEN = \"{SECRET_LITERAL}\"\nprint(TOKEN)\n");
    let mut spec = ParamDecl::new("TOKEN");
    spec.binding = ParameterBinding::Const;
    spec.delivery = ParameterDelivery::Inject;
    spec.parameter_type = ParameterType::Str;
    spec.secret = true;
    let specs = [spec];
    let managed = write_managed_params("python", &body, &specs).expect("managed source");
    let report = reconcile("python", &managed, &specs);
    assert!(report.current_defaults.is_empty());
    // The secret's literal must never appear in a machine-facing default record.
    assert!(!report.current_defaults.values().any(|value| matches!(
        value,
        ParameterValue::String(text) if text == SECRET_LITERAL
    )));
}

// --------------------------------------------------------------------------
// 6) preset save --from-last after a run that accepted every default
// --------------------------------------------------------------------------

#[test]
#[ignore = "CROSS-CRATE: `preset save --from-last`, flows.save_after_run and argstate.load_state \
            are skit-cli/skit-store use cases, not reachable from skit-language."]
fn test_preset_from_last_saves_effective_values_after_an_all_defaults_run() {
    // Last-used stores only what DIFFERED from the defaults, so a run that accepted everything leaves
    // the [values] table empty. Reading that table directly made --from-last refuse a good preset
    // right after a successful run. The gate now asks whether this entry has anything to remember at
    // all, and saves the EFFECTIVE values (definition default < last-used).
}

#[test]
#[ignore = "CROSS-CRATE: `preset save --from-last` refusal + message live in skit-cli."]
fn test_preset_from_last_still_refuses_an_entry_that_never_ran() {
    // No last_run AND no remembered values means there is genuinely nothing to save, and the message
    // says exactly that ("no remembered values yet").
}

#[test]
#[ignore = "CROSS-CRATE: flows.prefill/save_after_run + `preset save --from-last` + argstate are \
            skit-application/skit-store/skit-cli use cases."]
fn test_preset_from_last_pins_the_default_that_actually_ran() {
    // A preset from last pins the historical snapshot (the default that actually ran), not today's
    // edited source default.
}

#[test]
#[ignore = "CROSS-CRATE: `preset save --from-last` + argstate.record_run (legacy run without a \
            values snapshot) are skit-cli/skit-store use cases."]
fn test_preset_from_legacy_run_without_snapshot_refuses_to_guess() {
    // A legacy run with no recorded values snapshot must refuse ("run it once first") rather than
    // guess today's source defaults.
}

// --------------------------------------------------------------------------
// 7) presets pin what ran; last-used filters
// --------------------------------------------------------------------------

#[test]
#[ignore = "CROSS-CRATE: flows.remembered_values + FormPlan/FormField (delivers_empty) live in \
            skit-application (form_state)."]
fn test_last_used_filters_the_default_but_keeps_a_delivered_empty() {
    // Accepting a default is not a choice: remembering it would freeze today's default and hide
    // tomorrow's edit. A cleared delivers-empty field, by contrast, WAS delivered as '' and replays.
}

#[test]
#[ignore = "CROSS-CRATE: `run --save-preset` (verbatim preset write vs filtered last-used) is a \
            skit-cli use case over argstate."]
fn test_run_save_preset_stores_a_default_equal_value_verbatim() {
    // A preset is the named way to PIN a value, so it stores the run's values verbatim — including
    // one that happens to equal today's default — while last-used still filters it out.
}

// --------------------------------------------------------------------------
// 8) public -> secret never caches the source literal
// --------------------------------------------------------------------------

#[test]
fn test_resync_and_secret_in_one_edit_drops_the_refreshed_literal() {
    // A single edit that both resyncs the public default AND marks the field secret must not cache
    // the refreshed source literal into the secret block (edited.default is None, no "default" key).
    let mut declaration = const_decl("CITY", ParameterType::Str);
    declaration.default = Some(ParameterValue::String("old".to_owned()));
    let result = edit_source_declarations(
        "python",
        "CITY = \"sk-live-source\"\n",
        &[declaration],
        &SourceEditRequest {
            resync: true,
            secret: vec!["CITY".to_owned()],
            ..SourceEditRequest::default()
        },
    )
    .unwrap();
    let edited = &result.declarations[0];
    assert!(edited.secret);
    assert!(edited.default.is_none());
    let block = edited.to_block_map();
    assert!(!block.contains_key("default"));
    assert!(!format!("{block:?}").contains("sk-live-source"));
}

#[test]
fn test_final_no_secret_in_same_edit_keeps_the_public_default() {
    // `--secret X --no-secret X` ends public and must keep its public default ("new").
    let mut declaration = const_decl("CITY", ParameterType::Str);
    declaration.default = Some(ParameterValue::String("old".to_owned()));
    let result = edit_source_declarations(
        "python",
        "CITY = \"new\"\n",
        &[declaration],
        &SourceEditRequest {
            resync: true,
            secret: vec!["CITY".to_owned()],
            no_secret: vec!["CITY".to_owned()],
            ..SourceEditRequest::default()
        },
    )
    .unwrap();
    let edited = &result.declarations[0];
    assert!(!edited.secret);
    assert_eq!(
        edited.default,
        Some(ParameterValue::String("new".to_owned()))
    );
}

#[test]
fn unrelated_source_tweaks_keep_an_existing_secret_fallback_exact() {
    let mut token = const_decl("API_KEY", ParameterType::Str);
    token.secret = true;
    token.default = Some(ParameterValue::String("stored-fallback".to_owned()));
    token.env_source = "API_KEY_ENV".to_owned();

    let result = edit_source_declarations(
        "python",
        "API_KEY = \"live-source\"\nprint(API_KEY)\n",
        &[token],
        &SourceEditRequest {
            prompts: vec![NamedEdit::new("API_KEY", "Token: ")],
            ..SourceEditRequest::default()
        },
    )
    .unwrap();

    let edited = &result.declarations[0];
    assert!(edited.secret);
    assert_eq!(edited.prompt, "Token: ");
    assert_eq!(edited.env_source, "API_KEY_ENV");
    assert_eq!(
        edited.default,
        Some(ParameterValue::String("stored-fallback".to_owned())),
        "v0.4 clears a cached fallback only for an explicit final transition to secret"
    );
}

#[test]
fn resync_keeps_an_unchanged_existing_secret_fallback() {
    let mut token = const_decl("API_KEY", ParameterType::Str);
    token.secret = true;
    token.default = Some(ParameterValue::String("stored-fallback".to_owned()));

    let result = edit_source_declarations(
        "python",
        "API_KEY = \"live-source\"\nprint(API_KEY)\n",
        &[token],
        &SourceEditRequest {
            resync: true,
            ..SourceEditRequest::default()
        },
    )
    .unwrap();

    assert_eq!(
        result.declarations[0].default,
        Some(ParameterValue::String("stored-fallback".to_owned()))
    );
}

#[test]
fn adding_a_detected_secret_candidate_keeps_the_v04_candidate_default() {
    let result = edit_source_declarations(
        "python",
        "API_KEY = \"sk-live-candidate\"\nprint(API_KEY)\n",
        &[],
        &SourceEditRequest {
            add: vec!["API_KEY".to_owned()],
            ..SourceEditRequest::default()
        },
    )
    .unwrap();
    let declaration = &result.declarations[0];
    assert!(declaration.secret);
    assert_eq!(
        declaration.default,
        Some(ParameterValue::String("sk-live-candidate".to_owned()))
    );

    let written = write_managed_params(
        "python",
        "API_KEY = \"sk-live-candidate\"\nprint(API_KEY)\n",
        &result.declarations,
    )
    .unwrap();
    let close = written.rfind("# ///").unwrap();
    let block = &written[..close];
    assert!(block.contains("default = \"sk-live-candidate\""), "{block}");
}

// --------------------------------------------------------------------------
// 10) shell colon operators treat empty as unset
// --------------------------------------------------------------------------

/// Python `_shell_envdefault_text(operator)`: a shell script whose CITY envdefault uses `operator`.
fn shell_envdefault_text(operator: &str) -> String {
    format!(
        "#!/usr/bin/env bash\n# /// script\n# [tool.skit]\n# schema = 1\n#\n# [[tool.skit.params]]\n# name = \"CITY\"\n# kind = \"envdefault\"\n# type = \"str\"\n# default = \"Taipei\"\n# ///\necho \"${{CITY{operator}Taipei}}\"\n"
    )
}

#[test]
fn test_shell_colon_envdefaults_do_not_claim_to_deliver_empty() {
    // Language slice (this crate): the colon operators (`:-`, `:=`) test unset OR null, so an empty
    // environment value still activates the source fallback — empty_uses_default is true. The
    // FormField.delivers_empty / assemble.env_values assertions from the oracle def are cross-crate
    // (skit-form/skit-application) and are recorded as a gap.
    for operator in [":-", ":="] {
        let text = shell_envdefault_text(operator);
        let spec = envdefault("CITY", ParameterType::Str, None);
        assert!(
            source_parameter_semantics("shell", &text, &spec).empty_uses_default,
            "operator {operator:?} must set empty_uses_default"
        );
        // Addition (superset rule): the same fact reaches reconcile's empty_uses_default set.
        let report = reconcile(
            "shell",
            &text,
            &[envdefault("CITY", ParameterType::Str, None)],
        );
        assert!(report.empty_uses_default.contains("CITY"));
    }
}

#[test]
fn test_shell_noncolon_envdefaults_genuinely_deliver_empty() {
    // Language slice (this crate): the non-colon operators (`-`, `=`) test only unset, so they
    // genuinely accept an empty value — empty_uses_default is false. The FormField.delivers_empty /
    // assemble.env_values assertions from the oracle def are cross-crate and are recorded as a gap.
    for operator in ["-", "="] {
        let text = shell_envdefault_text(operator);
        let spec = envdefault("CITY", ParameterType::Str, None);
        assert!(
            !source_parameter_semantics("shell", &text, &spec).empty_uses_default,
            "operator {operator:?} must leave empty_uses_default false"
        );
        // Addition (superset rule): reconcile does not add CITY to empty_uses_default either.
        let report = reconcile(
            "shell",
            &text,
            &[envdefault("CITY", ParameterType::Str, None)],
        );
        assert!(!report.empty_uses_default.contains("CITY"));
    }
}
