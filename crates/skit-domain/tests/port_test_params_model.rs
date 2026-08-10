//! Mechanical port of the Python oracle module `tests/test_params_model.py`
//! (`origin/main@206f9ef`): "ParamDecl: the universal parameter model (contract tests)."
//! Each `#[test]` keeps its Python `def test_*` name so it traces back to its origin, and
//! each Python "WHY" comment is preserved above it.
//!
//! Crate choice: `skit-domain` (the `crate_hint` was `skit-cli`, but every symbol this
//! module drives — `to_block_map`, `from_block_map`, `to_meta_map`, `from_meta_map`,
//! `env_var`, `validate`, `normalized` — lives in `skit_domain::parameters`; the sibling
//! `port_test_params_edit.rs` sets the same domain-tier precedent).
//!
//! Concept mapping used throughout:
//! - Python `ParamDecl(name=, binding=, delivery=, type=, default=, ...)` -> the Rust struct
//!   literal `ParamDecl { binding: .., ..ParamDecl::new(name) }` (Python `type=` is the Rust
//!   `parameter_type` field; a scalar `default=` is `Some(ParameterValue::..)`).
//! - Python `d.to_block_dict()` -> `d.to_block_map()` (`BTreeMap<String, Value>`).
//! - Python `ParamDecl.from_block_dict(d)` -> `ParamDecl::from_block_map(&map)`.
//! - Python `d.to_meta_dict()` -> `d.to_meta_map()`.
//! - Python `ParamDecl.from_meta_dict(d)` -> `ParamDecl::from_meta_map(&map)`.
//! - Python `d.env_var` (property) -> `d.env_var()` (method).
//! - Python `validate_invariants(decl)` -> `decl.validate()` (`Option<ParameterInvariant>`).
//!   The Python reason-id STRINGS map to the Rust enum variants the oracle docstring calls a
//!   "symbolic reason id" (the UI owns the human wording): `"binding-delivery-mismatch"` ->
//!   `Some(ParameterInvariant::BindingDeliveryMismatch)`, `"choice-without-choices"` ->
//!   `Some(ParameterInvariant::ChoiceWithoutChoices)`, and `None` -> `None`.
//! - Python `normalize(decl)` -> `decl.normalized()` (consumes and returns `Self`). The
//!   Python `normalize(ok) is ok` identity check becomes value equality — identity cannot be
//!   observed across a Rust move, and the contract pinned is "nothing to repair -> unchanged".
//! - Python `field_replace(decl, **changes)` (a re-export of `dataclasses.replace`) -> the
//!   derived `Clone` plus a field assignment; the assertion pinned is that the copy is
//!   independent and the original is unchanged.
//!
//! Buckets:
//! - Bucket 1 (API EXISTS): the 18 asserting `#[test]`s below.
//! - Bucket 3 (cross-crate): `test_from_candidate_maps_fields_and_derives_delivery`. The Python
//!   `ParamDecl.from_candidate(Candidate(..))` field mapping (delivery derived from binding)
//!   is present in Rust — the `skit-language` analyzers build the field-aligned `ParamDecl`
//!   directly inside `SemanticCandidate.declaration` (asserted by `port_test_corpus.rs` via
//!   `detect_candidates`). It is unreachable from a `skit-domain` integration test without a
//!   reversed, forbidden dependency edge, so it is an `#[ignore]` stub here.

use std::collections::BTreeMap;

use serde_json::{Value, json};
use skit_domain::parameters::{
    ParamDecl, ParameterBinding, ParameterDelivery, ParameterInvariant, ParameterType,
    ParameterValue,
};

/// Build a `BTreeMap<String, Value>` from a JSON object literal, mirroring the Python dict the
/// oracle passes to `from_*_dict` / compares `to_*_dict` against. (Duplicated locally: each
/// integration-test file is its own crate, so the sibling `parameters.rs` helper is unreachable.)
fn map(value: Value) -> BTreeMap<String, Value> {
    serde_json::from_value(value).unwrap()
}

// ---- block (in-file [tool.skit]) shape: frozen -------------------------------------------------

#[test]
fn test_block_dict_const_shape_is_frozen() {
    let d = ParamDecl {
        binding: ParameterBinding::Const,
        delivery: ParameterDelivery::Inject,
        parameter_type: ParameterType::Str,
        default: Some(ParameterValue::String("xxx".to_owned())),
        secret: true,
        ..ParamDecl::new("API_KEY")
    };
    assert_eq!(
        d.to_block_map(),
        map(json!({
            "name": "API_KEY",
            "kind": "const", // the FROZEN key/value: existing user files carry exactly this
            "type": "str",
            "default": "xxx",
            "secret": true,
        }))
    );
}

#[test]
fn test_block_dict_input_shape_is_frozen() {
    let d = ParamDecl {
        binding: ParameterBinding::Input,
        delivery: ParameterDelivery::Inject,
        prompt: "Name: ".to_owned(),
        order: 0,
        env_source: "MY_NAME".to_owned(),
        ..ParamDecl::new("input-1")
    };
    assert_eq!(
        d.to_block_map(),
        map(json!({
            "name": "input-1",
            "kind": "input",
            "type": "str",
            "prompt": "Name: ",
            "order": 0,
            "env_source": "MY_NAME",
        }))
    );
}

#[test]
fn test_block_roundtrip_derives_delivery_from_binding() {
    let src = ParamDecl {
        binding: ParameterBinding::Const,
        delivery: ParameterDelivery::Inject,
        parameter_type: ParameterType::Int,
        default: Some(ParameterValue::Integer(3)),
        ..ParamDecl::new("N")
    };
    let back = ParamDecl::from_block_map(&src.to_block_map());
    assert_eq!(back, src);
    let envd = ParamDecl::from_block_map(&map(json!({
        "name": "V",
        "kind": "envdefault",
        "default": "x",
    })));
    assert_eq!(envd.binding, ParameterBinding::EnvDefault);
    assert_eq!(envd.delivery, ParameterDelivery::Env); // implied, never stored in the block
}

#[test]
fn test_from_block_dict_is_total_on_garbage() {
    let d = ParamDecl::from_block_map(&map(json!({
        "name": 5,
        "kind": "martian",
        "type": [],
        "order": "NaN",
        "default": {"t": 1},
    })));
    assert_eq!(d.name, "5");
    assert_eq!(d.binding, ParameterBinding::Const); // unknown binding degrades to the default
    assert_eq!(d.parameter_type, ParameterType::Str);
    assert_eq!(d.order, -1);
    assert_eq!(d.default, None); // a table is not an injectable scalar
}

// ---- from a source candidate --------------------------------------------------------------------

#[test]
#[ignore = "CROSS-CRATE (skit-language): the Python ParamDecl.from_candidate(Candidate(..)) \
    field mapping (delivery derived from binding) is present in Rust as the analyzers building \
    the field-aligned ParamDecl directly inside SemanticCandidate.declaration; asserted by \
    port_test_corpus.rs via detect_candidates. Unreachable from a skit-domain integration test \
    without a reversed, forbidden dependency edge. Oracle: src/skit/params.py::from_candidate."]
fn test_from_candidate_maps_fields_and_derives_delivery() {
    // Python: from a Candidate(binding="const", name="CITY", type="str", default="Taipei",
    // secret=True) the decl keeps name/binding/type/default/secret and DERIVES delivery="inject"
    // from the binding (a Candidate has no delivery of its own); order/prompt default to -1/"".
    // A Candidate(binding="input", name="input-1", prompt="Name: ", order=0) derives
    // delivery="inject" and keeps prompt/order.
}

// ---- meta [[parameters]] shape ------------------------------------------------------------------

#[test]
fn test_meta_roundtrip_full_model() {
    let src = ParamDecl {
        binding: ParameterBinding::None,
        delivery: ParameterDelivery::Flag,
        parameter_type: ParameterType::Choice,
        default: Some(ParameterValue::String("800".to_owned())),
        required: true,
        multiple: true,
        choices: vec!["400".to_owned(), "800".to_owned()],
        prompt: "Width".to_owned(),
        help: "output width".to_owned(),
        secret: false,
        flag: "--width".to_owned(),
        action: String::new(),
        env_target: String::new(),
        ..ParamDecl::new("width")
    };
    let back = ParamDecl::from_meta_map(&src.to_meta_map());
    assert_eq!(back, src);
}

#[test]
fn test_meta_dict_omits_defaults() {
    let d = ParamDecl::new("x").to_meta_map();
    assert_eq!(
        d,
        map(json!({"name": "x", "delivery": "flag", "type": "str"}))
    );
}

#[test]
fn test_meta_dict_omits_repeat_when_false() {
    // repeat rides the truthiness-gated tail: at its False default it is absent entirely,
    // never serialized as `repeat = false` (additive-only forward contract).
    let d = ParamDecl {
        delivery: ParameterDelivery::Flag,
        ..ParamDecl::new("x")
    }
    .to_meta_map();
    assert!(!d.contains_key("repeat"));
}

#[test]
fn test_meta_dict_repeat_emitted_and_roundtrips_only_when_set() {
    let src = ParamDecl {
        delivery: ParameterDelivery::Flag,
        flag: "--tag".to_owned(),
        multiple: true,
        repeat: true,
        ..ParamDecl::new("tag")
    };
    let d = src.to_meta_map();
    assert_eq!(d.get("repeat"), Some(&Value::Bool(true))); // emitted only because it is truthy
    let back = ParamDecl::from_meta_map(&d);
    assert_eq!(back, src);
    assert!(back.repeat);
}

#[test]
fn test_from_meta_dict_repeat_defaults_false_when_absent() {
    assert!(!ParamDecl::from_meta_map(&map(json!({"name": "x", "delivery": "flag"}))).repeat);
}

#[test]
fn test_from_meta_dict_repeat_coerces_truthy_to_bool() {
    // A hand-edited meta.toml may carry a non-bool truthy scalar; from_meta_dict normalizes it to
    // a real bool (kills the bool()-wrapper drop mutant, which would leave repeat as the raw int).
    assert!(
        ParamDecl::from_meta_map(&map(json!({"name": "x", "delivery": "flag", "repeat": 1})))
            .repeat
    );
}

#[test]
fn test_meta_dict_includes_binding_and_order_when_set() {
    // The two truthiness-gated head fields of to_meta_dict: a source-anchored binding and a
    // call-order key are emitted only when present, and round-trip back unchanged.
    let src = ParamDecl {
        binding: ParameterBinding::Input,
        delivery: ParameterDelivery::Inject,
        order: 2,
        ..ParamDecl::new("input-1")
    };
    let d = src.to_meta_map();
    assert_eq!(d.get("binding"), Some(&Value::String("input".to_owned())));
    assert_eq!(d.get("order"), Some(&json!(2)));
    assert_eq!(ParamDecl::from_meta_map(&d), src);
}

#[test]
fn test_meta_roundtrip_env_delivery_and_target() {
    let src = ParamDecl {
        delivery: ParameterDelivery::Env,
        env_target: "WIDTH_PX".to_owned(),
        secret: true,
        ..ParamDecl::new("width")
    };
    let back = ParamDecl::from_meta_map(&src.to_meta_map());
    assert_eq!(back, src);
    assert_eq!(back.env_var(), "WIDTH_PX");
}

#[test]
fn test_from_meta_dict_is_total_on_garbage() {
    let d = ParamDecl::from_meta_map(&map(json!({
        "name": "x",
        "delivery": "carrier-pigeon",
        "choices": "abc",
        "order": null,
    })));
    assert_eq!(d.delivery, ParameterDelivery::Flag);
    assert_eq!(d.choices, Vec::<String>::new());
    assert_eq!(d.order, -1);
}

// ---- env_var / invariants / normalize -----------------------------------------------------------

#[test]
fn test_env_var_defaults_to_name() {
    assert_eq!(
        ParamDecl {
            delivery: ParameterDelivery::Env,
            ..ParamDecl::new("WIDTH")
        }
        .env_var(),
        "WIDTH"
    );
    assert_eq!(
        ParamDecl {
            delivery: ParameterDelivery::Env,
            env_target: "WIDTH".to_owned(),
            ..ParamDecl::new("w")
        }
        .env_var(),
        "WIDTH"
    );
}

#[test]
fn test_invariants_binding_implies_delivery() {
    let ok = ParamDecl {
        binding: ParameterBinding::Const,
        delivery: ParameterDelivery::Inject,
        ..ParamDecl::new("a")
    };
    assert_eq!(ok.validate(), None);
    let bad = ParamDecl {
        binding: ParameterBinding::Const,
        delivery: ParameterDelivery::Env,
        ..ParamDecl::new("a")
    };
    assert_eq!(
        bad.validate(),
        Some(ParameterInvariant::BindingDeliveryMismatch)
    );
    let envd = ParamDecl {
        binding: ParameterBinding::EnvDefault,
        delivery: ParameterDelivery::Flag,
        ..ParamDecl::new("a")
    };
    assert_eq!(
        envd.validate(),
        Some(ParameterInvariant::BindingDeliveryMismatch)
    );
    let free = ParamDecl {
        binding: ParameterBinding::None,
        delivery: ParameterDelivery::Env,
        ..ParamDecl::new("a")
    };
    assert_eq!(free.validate(), None);
}

#[test]
fn test_invariants_choice_needs_choices() {
    assert_eq!(
        ParamDecl {
            parameter_type: ParameterType::Choice,
            ..ParamDecl::new("a")
        }
        .validate(),
        Some(ParameterInvariant::ChoiceWithoutChoices)
    );
    assert_eq!(
        ParamDecl {
            parameter_type: ParameterType::Choice,
            choices: vec!["x".to_owned()],
            ..ParamDecl::new("a")
        }
        .validate(),
        None
    );
}

#[test]
fn test_normalize_repairs_delivery_from_binding() {
    let bad = ParamDecl {
        binding: ParameterBinding::EnvDefault,
        delivery: ParameterDelivery::Flag,
        ..ParamDecl::new("a")
    };
    let fixed = bad.normalized();
    assert_eq!(fixed.delivery, ParameterDelivery::Env);
    let ok = ParamDecl {
        binding: ParameterBinding::None,
        delivery: ParameterDelivery::Env,
        ..ParamDecl::new("b")
    };
    // Python asserts `normalize(ok) is ok` (same object back); identity cannot be observed across
    // a Rust move, so the pinned contract is the behavioral one: nothing to repair -> unchanged.
    assert_eq!(ok.clone().normalized(), ok);
}

#[test]
fn test_field_replace_returns_modified_copy() {
    // Python `field_replace` re-exports `dataclasses.replace`; the Rust equivalent is the derived
    // `Clone` plus a field assignment. The contract pinned is independence: the copy carries the
    // new type and the original is untouched.
    let a = ParamDecl {
        parameter_type: ParameterType::Int,
        ..ParamDecl::new("a")
    };
    let mut b = a.clone();
    b.parameter_type = ParameterType::Float;
    assert_eq!(b.parameter_type, ParameterType::Float);
    assert_eq!(a.parameter_type, ParameterType::Int);
}
