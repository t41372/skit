//! Mechanical port of the Python oracle module `tests/test_params_edit.py`
//! (`origin/main@206f9ef`, version 0.4.1.dev0): "Pure edit ops on the declared
//! `[[parameters]]` schema (`params.edit_declared`) plus the default-coercion /
//! type-validation helpers." Each `#[test]` keeps its Python `def test_*` name and its
//! Python "WHY" comment so it traces back to its origin.
//!
//! Concept mapping:
//! - Python `params.coerce_default(value, type)` -> `skit_domain::parameters::coerce_default`
//!   (returns `Result<ParameterValue, DefaultCoercionError>`; the oracle raises `ValueError`).
//! - Python `params.ParamDecl` -> `skit_domain::parameters::ParamDecl`.
//! - Python `params.ParamType` string -> `skit_domain::parameters::ParameterType`.
//! - Python `params.as_param_type(value) -> ParamType | None` -> NO public Rust equivalent
//!   (see the ROOT GAP below).
//! - Python `params.edit_declared(...) -> DeclEditResult{decls, warnings}` -> NO public Rust
//!   equivalent (see the ROOT GAP below).
//!
//! ROOT GAP (why most of this module is `#[ignore]`d, kind="absent"):
//! The oracle's edit surface is a single PURE function, `edit_declared` (oracle
//! `src/skit/params.py:352-472`), that applies a fixed `rm -> add -> per-name tweak`
//! pipeline and ACCUMULATES a closed set of `code:name` WARNINGS (`already-declared:a`,
//! `not-declared:ghost`, `bad-delivery:a`, `bad-type:a`, `bad-default:a`,
//! `choice-without-choices:a`, `not-a-placeholder:a`, `env-source-not-secret:a`,
//! `bool-flag-on-by-default:v`), reverting any row whose invariants fail while KEEPING the
//! rest of the batch. That pure, warn-and-continue function is ABSENT from the Rust surface:
//! no `edit_declared`, no `DeclEditResult`, and none of the warning-code strings exist in any
//! crate. The Rust CLI instead inlines a FAIL-FAST, hard-error variant inside `params()`
//! (`crates/skit-cli/src/cli.rs:3762-3973`): e.g. an add on an existing name returns
//! `CliError::Usage("parameter already exists: {}")` and aborts the whole operation
//! (`cli.rs:3807-3815`), where the oracle would append `already-declared:a` and continue.
//! Asserting on the CLI's shape would transcribe Rust, not the oracle, and this module by its
//! own docstring targets the pure layer "the CLI/TUI wiring on top is covered in
//! test_declared_params.py and test_tui_settings_cov.py". So every `edit_declared` def is a
//! compiling `#[ignore]` stub that records the exact oracle behavior + the warning code the
//! fixer must implement. Restoring this is the tracked "params batch fault tolerance" work.
//!
//! Buckets:
//! - REAL (API exists): the three `coerce_default` defs drive the live
//!   `skit_domain::parameters::coerce_default`.
//! - ABSENT gaps: the 36 `edit_declared` defs (root gap above) and the 2 `as_param_type`
//!   defs (no public Option-returning type validator; `ParameterType::parse` at
//!   `parameters.rs:138` is private and takes a fallback, and the CLI's `parse_parameter_type`
//!   is private too).
//! - CROSS-CRATE: none. Every mapped API is either pure-domain or genuinely unimplemented.

use skit_domain::parameters::{ParameterType, ParameterValue, coerce_default};

// --------------------------------------------------------------------------- add / rm

#[test]
#[ignore = "MISSING API (absent): pure params.edit_declared pipeline (DeclEditResult{decls, warnings}) is absent from the Rust surface — oracle src/skit/params.py:352-472; CLI inlines a fail-fast hard-error variant at crates/skit-cli/src/cli.rs:3762-3973; tracked as 'params batch fault tolerance'"]
fn test_add_defaults_to_first_allowed_delivery_for_a_binary() {
    // A fresh add takes delivery = allowed_deliveries[0], binding="none", type="str",
    // required=False, and no warnings.
    // res = edit_declared([], add=["width"], allowed_deliveries=("flag", "env"))
    // res.warnings == []
    // res.decls[0] == (name="width", delivery="flag", binding="none", type="str", required=False)
}

#[test]
#[ignore = "MISSING API (absent): params.edit_declared absent — oracle src/skit/params.py:352-472 (add-on-placeholder branch :409-414)"]
fn test_add_on_a_template_placeholder_name_becomes_a_required_placeholder() {
    // An add whose name IS a template placeholder takes delivery="placeholder" and stays
    // required=True (a declared placeholder can never silently assemble an empty slot).
    // res = edit_declared([], add=["size"], allowed_deliveries=("placeholder","env"),
    //                     placeholder_names=["size"])
    // res.decls[0].delivery == "placeholder"; res.decls[0].required is True
}

#[test]
#[ignore = "MISSING API (absent): params.edit_declared absent — oracle src/skit/params.py:352-472 (add-non-placeholder fallback :415-422)"]
fn test_add_non_placeholder_name_on_a_template_uses_first_allowed_delivery() {
    // Only a name matching a placeholder overrides delivery; any other add takes
    // allowed_deliveries[0] and required stays False.
    // res = edit_declared([], add=["RETRIES"], allowed_deliveries=("placeholder","env"),
    //                     placeholder_names=["size"])
    // res.decls[0].delivery == "placeholder"  # allowed_deliveries[0]
    // res.decls[0].required is False
}

#[test]
#[ignore = "MISSING API (absent): params.edit_declared absent — oracle src/skit/params.py:405-408 emits WARNING 'already-declared:a'; Rust CLI hard-errors instead (cli.rs:3807-3815)"]
fn test_add_existing_name_warns_already_declared() {
    // An add on an existing name is a warning, not a mutation — the row stays.
    // res = edit_declared([ParamDecl(name="a")], add=["a"])
    // res.warnings == ["already-declared:a"]; [d.name for d in res.decls] == ["a"]
}

#[test]
#[ignore = "MISSING API (absent): params.edit_declared absent — oracle src/skit/params.py:398-403 (rm branch)"]
fn test_rm_drops_the_row() {
    // res = edit_declared([ParamDecl(name="a"), ParamDecl(name="b")], rm=["a"])
    // [d.name for d in res.decls] == ["b"]
}

#[test]
#[ignore = "MISSING API (absent): params.edit_declared absent — oracle src/skit/params.py:402-403 emits WARNING 'not-declared:ghost'"]
fn test_rm_unknown_name_warns_not_declared() {
    // res = edit_declared([ParamDecl(name="a")], rm=["ghost"])
    // res.warnings == ["not-declared:ghost"]; [d.name for d in res.decls] == ["a"]
}

#[test]
#[ignore = "MISSING API (absent): params.edit_declared absent — oracle src/skit/params.py:397-423 fixes apply order rm -> add -> tweak"]
fn test_apply_order_is_rm_then_add_then_tweak() {
    // rm a, add a fresh, then tweak the fresh one — the tweak must land on the re-added row.
    // res = edit_declared([ParamDecl(name="a", type="int")], rm=["a"], add=["a"],
    //                     types={"a": "float"})
    // res.decls named "a" has type == "float"
}

#[test]
#[ignore = "MISSING API (absent): params.edit_declared absent — oracle src/skit/params.py:395 shallow-copies each decl (field_replace), never mutates the caller's inputs"]
fn test_inputs_are_never_mutated() {
    // original = ParamDecl(name="a", type="str", prompt="orig")
    // edit_declared([original], prompts={"a": "changed"}, secret=["a"])
    // original.prompt == "orig"; original.secret is False
}

// --------------------------------------------------------------------------- tweaks

#[test]
#[ignore = "MISSING API (absent): params.edit_declared absent — oracle src/skit/params.py:519-526 (delivery tweak within allowed set)"]
fn test_delivery_tweak_within_allowed_set() {
    // res = edit_declared([ParamDecl(name="a", delivery="flag")], deliveries={"a": "env"},
    //                     allowed_deliveries=("flag", "env"))
    // res.decls named "a" delivery == "env"
}

#[test]
#[ignore = "MISSING API (absent): params.edit_declared absent — oracle src/skit/params.py:521-522 emits WARNING 'bad-delivery:a'; the delivery stays unchanged"]
fn test_delivery_outside_allowed_set_warns_bad_delivery() {
    // res = edit_declared([ParamDecl(name="a", delivery="flag")],
    //                     deliveries={"a": "placeholder"}, allowed_deliveries=("flag", "env"))
    // res.warnings == ["bad-delivery:a"]; res.decls named "a" delivery == "flag"  # unchanged
}

#[test]
#[ignore = "MISSING API (absent): params.edit_declared absent — oracle src/skit/params.py:523-524 emits WARNING 'not-a-placeholder:a'"]
fn test_placeholder_delivery_on_a_non_placeholder_name_warns() {
    // res = edit_declared([ParamDecl(name="a", delivery="env")],
    //                     deliveries={"a": "placeholder"},
    //                     allowed_deliveries=("placeholder","env"), placeholder_names=["other"])
    // res.warnings == ["not-a-placeholder:a"]; res.decls named "a" delivery == "env"
}

#[test]
#[ignore = "MISSING API (absent): params.edit_declared absent — oracle src/skit/params.py:523-526 (placeholder delivery allowed when name matches a placeholder)"]
fn test_placeholder_delivery_on_a_matching_placeholder_name_is_allowed() {
    // res = edit_declared([ParamDecl(name="size", delivery="env")],
    //                     deliveries={"size": "placeholder"},
    //                     allowed_deliveries=("placeholder","env"), placeholder_names=["size"])
    // res.decls named "size" delivery == "placeholder"
}

#[test]
#[ignore = "MISSING API (absent): params.edit_declared absent — oracle src/skit/params.py:527-536 (valid type tweak)"]
fn test_type_tweak_valid() {
    // res = edit_declared([ParamDecl(name="a")], types={"a": "int"})
    // res.decls named "a" type == "int"
}

#[test]
#[ignore = "MISSING API (absent): params.edit_declared absent — oracle src/skit/params.py:529-530 emits WARNING 'bad-type:a'; the type stays unchanged"]
fn test_type_tweak_invalid_warns_bad_type() {
    // res = edit_declared([ParamDecl(name="a", type="str")], types={"a": "integer"})
    // res.warnings == ["bad-type:a"]; res.decls named "a" type == "str"
}

#[test]
#[ignore = "MISSING API (absent): params.edit_declared absent — oracle src/skit/params.py:537-538 (choices tweak sets the tuple)"]
fn test_choices_tweak_sets_the_tuple() {
    // res = edit_declared([ParamDecl(name="a", type="choice")], choices={"a": ["x", "y"]})
    // res.decls named "a" choices == ("x", "y")
}

#[test]
#[ignore = "MISSING API (absent): params.edit_declared absent — oracle src/skit/params.py:539-543 (default coerced to the declared type)"]
fn test_default_coerced_to_the_declared_type() {
    // res = edit_declared([ParamDecl(name="a", type="int")], defaults={"a": "42"})
    // res.decls named "a" default == 42 and is an int
}

#[test]
#[ignore = "MISSING API (absent): params.edit_declared absent — oracle src/skit/params.py:527-543 (a same-call type tweak applies BEFORE the default is coerced)"]
fn test_default_type_set_in_same_call_applies_before_coercion() {
    // res = edit_declared([ParamDecl(name="a")], types={"a": "float"}, defaults={"a": "1.5"})
    // res.decls named "a" default == 1.5 and is a float
}

#[test]
#[ignore = "MISSING API (absent): params.edit_declared absent — oracle src/skit/params.py:541-543 emits WARNING 'bad-default:a' and keeps the old default"]
fn test_default_bad_value_warns_bad_default_and_keeps_old() {
    // res = edit_declared([ParamDecl(name="a", type="int", default=3)],
    //                     defaults={"a": "notanint"})
    // res.warnings == ["bad-default:a"]; res.decls named "a" default == 3
}

#[test]
#[ignore = "MISSING API (absent): params.edit_declared absent — oracle src/skit/params.py:544-545 (flag tweak is stripped; empty means positional)"]
fn test_flag_tweak_strips_and_sets_empty_for_positional() {
    // res = edit_declared([ParamDecl(name="a", delivery="flag")], flags={"a": "  --out "})
    // res.decls named "a" flag == "--out"
    // res2 = edit_declared([ParamDecl(name="a", delivery="flag", flag="--out")], flags={"a": ""})
    // res2.decls named "a" flag == ""  # empty => positional
}

#[test]
#[ignore = "MISSING API (absent): params.edit_declared absent — oracle src/skit/params.py:546-549 (required / optional tweaks)"]
fn test_required_and_optional_tweaks() {
    // res = edit_declared([ParamDecl(name="a")], required=["a"])
    // res.decls named "a" required is True
    // res2 = edit_declared([ParamDecl(name="a", required=True)], optional=["a"])
    // res2.decls named "a" required is False
}

#[test]
#[ignore = "MISSING API (absent): params.edit_declared absent — oracle src/skit/params.py:550-553 (help text and prompt tweaks)"]
fn test_help_text_and_prompt_tweaks() {
    // res = edit_declared([ParamDecl(name="a")], help_texts={"a": "what it does"},
    //                     prompts={"a": "A?"})
    // res.decls named "a" help == "what it does"; prompt == "A?"
}

#[test]
#[ignore = "MISSING API (absent): params.edit_declared absent — oracle src/skit/params.py:554-561 (secret + env_source: env_source is stripped)"]
fn test_secret_and_env_source_together() {
    // res = edit_declared([ParamDecl(name="tok")], secret=["tok"],
    //                     env_sources={"tok": " API_TOKEN "})
    // res.decls named "tok" secret is True; env_source == "API_TOKEN"  # stripped
}

#[test]
#[ignore = "MISSING API (absent): params.edit_declared absent — oracle src/skit/params.py:559-565 emits WARNING 'env-source-not-secret:a'; env_source stays \"\""]
fn test_env_source_on_a_non_secret_param_warns_and_leaves_it_unset() {
    // --env-source only applies to a secret param: on a non-secret one it does nothing to the
    // decl (env_source stays "") but must NOT vanish silently — the declared lane now warns
    // exactly like the in-file lane, so an explicit flag that no-ops is surfaced.
    // res = edit_declared([ParamDecl(name="a", secret=False)], env_sources={"a": "VAR"})
    // res.decls named "a" env_source == ""
    // "env-source-not-secret:a" in res.warnings
}

#[test]
#[ignore = "MISSING API (absent): params.edit_declared absent — oracle src/skit/params.py:556-558 (no_secret clears secret AND env_source)"]
fn test_no_secret_clears_the_env_source() {
    // res = edit_declared([ParamDecl(name="tok", secret=True, env_source="API_TOKEN")],
    //                     no_secret=["tok"])
    // res.decls named "tok" secret is False; env_source == ""
}

#[test]
#[ignore = "MISSING API (absent): params.edit_declared absent — oracle src/skit/params.py:436-437 emits WARNING 'not-declared:ghost' for a tweak on an unknown name"]
fn test_tweak_on_unknown_name_warns_not_declared() {
    // res = edit_declared([ParamDecl(name="a")], types={"ghost": "int"})
    // res.warnings == ["not-declared:ghost"]
}

#[test]
#[ignore = "MISSING API (absent): params.edit_declared absent — oracle src/skit/params.py:425-433 dedups a name touched by two ops; both tweaks apply"]
fn test_a_name_touched_by_two_ops_is_listed_once_and_both_apply() {
    // exercises the dedup in the tweak-name gather (dict + dict, and seq overlapping a dict)
    // res = edit_declared([ParamDecl(name="a")], types={"a": "int"}, defaults={"a": "5"},
    //                     secret=["a"], prompts={"a": "A?"})
    // res.decls named "a" (type, default, secret, prompt) == ("int", 5, True, "A?")
}

// --------------------------------------------------------------------------- revert on invalid

#[test]
#[ignore = "MISSING API (absent): params.edit_declared absent — oracle src/skit/params.py:465-471 emits WARNING 'choice-without-choices:a' and REVERTS the whole row"]
fn test_choice_type_without_choices_reverts_and_warns() {
    // res = edit_declared([ParamDecl(name="a", type="str", help="keep me")],
    //                     types={"a": "choice"}, help_texts={"a": "changed"})
    // res.warnings == ["choice-without-choices:a"]
    // res.decls named "a" type == "str"  # reverted to pre-tweak state
    // res.decls named "a" help == "keep me"  # the whole row reverted, so help edit dropped too
}

#[test]
#[ignore = "MISSING API (absent): params.edit_declared absent — oracle src/skit/params.py:465-470 (choice type WITH choices in the same call is valid, no warning)"]
fn test_choice_type_with_choices_in_the_same_call_is_valid() {
    // res = edit_declared([ParamDecl(name="a")], types={"a": "choice"}, choices={"a": ["r", "g"]})
    // res.warnings == []; res.decls named "a" type == "choice"; choices == ("r", "g")
}

// -------------------------------------------------------------- bool-flag action hygiene
// A `--type NAME=bool` on a flag-delivery decl used to leave action="" — a checkbox that
// fires no flag in EITHER state. The tweak pipeline now records store_true and sheds a stale
// action when a type moves OFF bool. Oracle helper: `_apply_bool_flag_action`
// (src/skit/params.py:475-494).

#[test]
#[ignore = "MISSING API (absent): params.edit_declared / _apply_bool_flag_action absent — oracle src/skit/params.py:488-491 stamps action='store_true' on an off-by-default bool flag"]
fn test_type_tweak_to_bool_on_a_flag_sets_store_true() {
    // res = edit_declared([ParamDecl(name="v", delivery="flag", flag="--v")], types={"v": "bool"})
    // res.decls named "v" type == "bool"; action == "store_true"
}

#[test]
#[ignore = "MISSING API (absent): params.edit_declared / _apply_bool_flag_action absent — oracle src/skit/params.py:488 (the `decl.flag` half of the guard: no flag => empty action)"]
fn test_type_tweak_to_bool_on_a_positional_keeps_empty_action() {
    // No flag -> nothing to fire -> no store_true default (the `decl.flag` half of the guard).
    // res = edit_declared([ParamDecl(name="b", delivery="flag", flag="")], types={"b": "bool"})
    // res.decls named "b" type == "bool"; action == ""
}

#[test]
#[ignore = "MISSING API (absent): params.edit_declared / _apply_bool_flag_action absent — oracle src/skit/params.py:488 (the `delivery == 'flag'` half of the guard: env bool gets no store_true)"]
fn test_type_tweak_to_bool_on_env_delivery_keeps_empty_action() {
    // store_true is a flag-assembly concept: an env-delivered bool must not gain one.
    // res = edit_declared([ParamDecl(name="v", delivery="env", flag="--v")],
    //                     types={"v": "bool"}, allowed_deliveries=("flag", "env"))
    // res.decls named "v" type == "bool"; action == ""
}

#[test]
#[ignore = "MISSING API (absent): params.edit_declared / _apply_bool_flag_action absent — oracle src/skit/params.py:492-493 sheds a stale action when a type moves OFF bool"]
fn test_type_tweak_off_bool_sheds_stale_action() {
    // res = edit_declared([ParamDecl(name="v", delivery="flag", flag="--v", type="bool",
    //                     action="store_true")], types={"v": "str"})
    // res.decls named "v" type == "str"; action == ""
}

#[test]
#[ignore = "MISSING API (absent): params.edit_declared / _apply_bool_flag_action absent — oracle src/skit/params.py:488 (the `not decl.action` half: an existing action is not clobbered)"]
fn test_non_type_tweak_on_a_bool_leaves_its_action_alone() {
    // An unrelated tweak on a bool decl that already carries an action must NOT clobber it to
    // store_true.
    // res = edit_declared([ParamDecl(name="c", delivery="flag", flag="--c", type="bool",
    //                     action="store_false")], defaults={"c": "true"})
    // res.decls named "c" action == "store_false"
}

#[test]
#[ignore = "MISSING API (absent): params.edit_declared / _apply_bool_flag_action absent — oracle src/skit/params.py:492-493 (the `type != 'bool'` clean runs for EVERY tweaked decl)"]
fn test_non_type_tweak_on_a_str_with_stale_action_clears_it() {
    // A non-bool row carrying a stale action (a hand-edited meta.toml) is cleaned even by an
    // unrelated tweak.
    // res = edit_declared([ParamDecl(name="a", delivery="flag", flag="--a", type="str",
    //                     action="store_true")], help_texts={"a": "x"})
    // res.decls named "a" action == ""
}

// --------------------------------------------------------------------------- coerce_default

#[test]
fn test_coerce_default_success() {
    // Coerce a default STRING to the parameter's declared scalar type; str/choice keep the raw
    // string, the bool spellings match langs/python/shim._coerce_bool. Oracle parametrize table
    // (test_params_edit.py:326-342).
    let cases: [(&str, ParameterType, ParameterValue); 10] = [
        ("42", ParameterType::Int, ParameterValue::Integer(42)),
        ("3.5", ParameterType::Float, ParameterValue::Float(3.5)),
        ("true", ParameterType::Bool, ParameterValue::Bool(true)),
        ("YES", ParameterType::Bool, ParameterValue::Bool(true)),
        ("on", ParameterType::Bool, ParameterValue::Bool(true)),
        ("false", ParameterType::Bool, ParameterValue::Bool(false)),
        ("0", ParameterType::Bool, ParameterValue::Bool(false)),
        ("off", ParameterType::Bool, ParameterValue::Bool(false)),
        (
            "anything",
            ParameterType::Str,
            ParameterValue::String("anything".to_owned()),
        ),
        (
            "anything",
            ParameterType::Choice,
            ParameterValue::String("anything".to_owned()),
        ),
    ];
    for (value, parameter_type, expected) in cases {
        assert_eq!(
            coerce_default(value, parameter_type).unwrap(),
            expected,
            "coerce_default({value:?}, {parameter_type:?})"
        );
    }
}

#[test]
fn test_coerce_default_rejects_bad_values() {
    // A value that doesn't fit int/float/bool raises ValueError whose text mentions the value.
    // inf/nan are refused like shim does. Oracle parametrize table
    // (test_params_edit.py:345-352, match=re.escape(value)).
    let cases: [(&str, ParameterType); 5] = [
        ("x", ParameterType::Int),
        ("x", ParameterType::Float),
        ("maybe", ParameterType::Bool),
        ("inf", ParameterType::Float),
        ("nan", ParameterType::Float),
    ];
    for (value, parameter_type) in cases {
        let error = coerce_default(value, parameter_type).expect_err(&format!(
            "coerce_default({value:?}, {parameter_type:?}) must reject"
        ));
        // Python `pytest.raises(ValueError, match=re.escape(value))`: the offending value appears
        // in the error text (Rust Display: "{value:?} is not a valid {type} default").
        assert!(
            error.to_string().contains(value),
            "error {error} must mention {value:?}"
        );
    }
}

#[test]
fn test_coerce_default_rejects_infinity_specifically() {
    // A magnitude that overflows to infinity is refused just like the literal "inf" (repr(inf)
    // is not a valid literal). Oracle test_params_edit.py:354-357 (match="1e999").
    assert!("1e999".parse::<f64>().unwrap().is_infinite()); // sanity: parses to +inf
    let error = coerce_default("1e999", ParameterType::Float)
        .expect_err("coerce_default(\"1e999\", Float) must reject an infinity");
    assert!(
        error.to_string().contains("1e999"),
        "error {error} must mention 1e999"
    );
}

// --------------------------------------------------------------------------- as_param_type

#[test]
#[ignore = "MISSING API (absent): no public Option-returning type validator — oracle params.as_param_type src/skit/params.py:311-317; private near-miss ParameterType::parse (crates/skit-domain/src/parameters.rs:138) takes a fallback, and the CLI's parse_parameter_type is private too"]
fn test_as_param_type_accepts_the_five() {
    // as_param_type returns the value unchanged for a valid ParamType. Oracle parametrize over
    // ["str","int","float","bool","choice"]. NOTE: the oracle also accepts "path" (it iterates
    // the full _TYPES tuple), even though this test names only the five.
    // for value in ["str","int","float","bool","choice"]:
    //     params.as_param_type(value) == value
}

#[test]
#[ignore = "MISSING API (absent): no public Option-returning type validator — oracle params.as_param_type src/skit/params.py:311-317 returns None for a non-type; Rust has no public equivalent"]
fn test_as_param_type_rejects_others() {
    // for value in ["integer", "", "STR", "number"]:
    //     params.as_param_type(value) is None
}

// -------------------------------------------------- bool-flag refusal (on-by-default) + control

#[test]
#[ignore = "MISSING API (absent): params.edit_declared / _apply_bool_flag_action absent — oracle src/skit/params.py:488-490 & 460-464 emit WARNING 'bool-flag-on-by-default:verbose', stamp nothing, and REVERT the whole row"]
fn test_bool_flag_that_is_on_by_default_is_refused_not_stamped() {
    // A flag already ON can only be turned off by a DIFFERENT spelling skit cannot invent, so
    // store_true there ships a checkbox whose unticked state delivers nothing. The reader side
    // refuses the same shape (argspec._typer_finish_bool); the hand-declared path must not be the
    // way around it, so the row is kept unchanged and the caller gets a warning code.
    // pre = ParamDecl(name="verbose", delivery="flag", flag="--verbose")
    // res = edit_declared([pre], types={"verbose": "bool"}, defaults={"verbose": "true"})
    // res.warnings == ["bool-flag-on-by-default:verbose"]
    // res.decls named "verbose": action == "" (nothing stamped); type == "str" (row rolled back)
}

#[test]
#[ignore = "MISSING API (absent): params.edit_declared / _apply_bool_flag_action absent — oracle src/skit/params.py:488-491 (control: an off-by-default bool flag DOES get store_true, no warning)"]
fn test_bool_flag_that_is_off_by_default_still_gets_store_true() {
    // The control: the refusal must key off the default, not fire for every bool flag.
    // res = edit_declared([ParamDecl(name="verbose", delivery="flag", flag="--verbose")],
    //                     types={"verbose": "bool"}, defaults={"verbose": "false"})
    // res.warnings == []; res.decls named "verbose" (type, default, action) ==
    //                     ("bool", False, "store_true")
}
