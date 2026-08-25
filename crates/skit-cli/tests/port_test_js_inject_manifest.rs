//! Exact and unique accounting for Python v0.4 `tests/test_js_inject.py`.
//!
//! The frozen module has 37 owners. The deferred-owner audit classified 19 rows, then found that
//! one previously active byte assertion did not prove its frozen real-child claim. These 20 owners
//! keep their audited disposition here. A real gap can become executable without changing its
//! historical disposition; `GapState` records whether this batch closed it.

use std::{collections::BTreeSet, fs, path::Path};

use syn::{Attribute, Item};

const TARGETS: &[&str] = &[
    "crates/skit-language/tests/port_test_js_inject.rs",
    "crates/skit-runtime/tests/javascript_syntax_gate.rs",
    "crates/skit-cli/tests/port_test_js_inject_cli.rs",
    "crates/skit-cli/tests/port_test_js_inject_cli_batch_c.rs",
];

const EXECUTABLE: &[&str] = &[
    "test_int_injects_a_bare_number",
    "test_float_injects_a_bare_number",
    "test_string_injects_a_json_dumps_literal",
    "test_string_json_escapes_quote_backslash_newline",
    "test_cjk_and_emoji_escape_to_valid_js",
    "test_bool_injects_true_or_false_lowercase",
    "test_rewrites_every_same_name_occurrence",
    "test_same_name_nonliteral_declaration_is_not_a_target",
    "test_ts_temp_copy_has_ts_suffix",
    "test_missing_target_is_drift_not_value_error",
    "test_bad_int_value_raises_value_error",
    "test_bad_float_and_non_finite_are_refused",
    "test_bad_bool_value_raises_value_error",
    "test_no_values_writes_nothing",
    "test_value_for_unmanaged_name_is_ignored",
    "test_injected_copy_is_0600",
    "test_execute_refuses_a_bad_value_before_launch",
    "test_resolve_runner_respects_pinned_interpreter_and_normalizes",
    "test_gate_node_skips_ts_suffix",
    "test_mjs_origin_esm_copy_survives_gate2_before_any_package_json",
    "test_gate_node_skips_when_runner_is_not_node",
    "test_gate_node_passes_on_returncode_zero",
    "test_gate_node_raises_on_nonzero",
    "test_gate_node_raises_on_nonzero_with_empty_stderr",
    "test_gate_node_survives_a_spawn_failure",
    "test_gate2_failure_removes_the_temp_copy",
    "test_injected_copy_carries_the_origins_module_flavor",
    "test_execute_runs_a_js_entry_offline_plan",
    "test_injected_const_reaches_the_child",
    "test_injected_string_reaches_the_child",
    "test_run_injects_and_executes_end_to_end",
    "test_execute_maps_a_drifted_js_definition_to_drift",
    "test_execute_syntax_gate_failure_never_launches",
];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum GapState {
    Executable,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Disposition {
    StrongerOwnerClosure {
        reason: &'static str,
    },
    StaleExactRehome {
        target: &'static str,
        state: GapState,
    },
    RuntimeGatedRehome {
        target: &'static str,
        state: GapState,
    },
    RealProductGap {
        target: &'static str,
        state: GapState,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct AuditedOwner {
    name: &'static str,
    disposition: Disposition,
}

const AUDITED: &[AuditedOwner] = &[
    AuditedOwner {
        name: "test_offline_gate_refuses_a_corrupted_injection",
        disposition: Disposition::StrongerOwnerClosure {
            reason: "skit-language forces the post-edit reparse failure directly; staging starts only after that typed result succeeds",
        },
    },
    AuditedOwner {
        name: "test_resolve_runner_finds_first_installed",
        disposition: Disposition::StrongerOwnerClosure {
            reason: "the injected ProgramProbe runtime-plan owner pins deno, then bun, then node without a second gate-only lookup",
        },
    },
    AuditedOwner {
        name: "test_resolve_runner_none_when_nothing_installed",
        disposition: Disposition::StrongerOwnerClosure {
            reason: "the runtime-plan owner returns typed JsRuntimeMissing before any child can start",
        },
    },
    AuditedOwner {
        name: "test_gate_node_skips_when_no_runner_installed",
        disposition: Disposition::StrongerOwnerClosure {
            reason: "the resolved-runtime API passes None to the optional gate while the full launch keeps its typed missing-runtime refusal",
        },
    },
    AuditedOwner {
        name: "test_injected_copy_carries_the_origins_module_flavor",
        disposition: Disposition::StaleExactRehome {
            target: "Batch C: cross-platform staged-path owner for js/mjs/cjs/ts/mts/cts",
            state: GapState::Executable,
        },
    },
    AuditedOwner {
        name: "test_execute_runs_a_js_entry_offline_plan",
        disposition: Disposition::StaleExactRehome {
            target: "Batch C: skit-form FormSource::Inject owner",
            state: GapState::Executable,
        },
    },
    AuditedOwner {
        name: "test_injected_string_reaches_the_child",
        disposition: Disposition::RuntimeGatedRehome {
            target: "Batch C: real node, deno, or bun output owner",
            state: GapState::Executable,
        },
    },
    AuditedOwner {
        name: "test_run_injects_and_executes_end_to_end",
        disposition: Disposition::RuntimeGatedRehome {
            target: "Batch C: real CLI add, params --manage, and run owner",
            state: GapState::Executable,
        },
    },
    AuditedOwner {
        name: "test_injected_const_reaches_the_child",
        disposition: Disposition::RuntimeGatedRehome {
            target: "Batch C: real node, deno, or bun output owner; the byte-only assertion is Rust-additive",
            state: GapState::Executable,
        },
    },
    AuditedOwner {
        name: "test_resolve_runner_respects_pinned_interpreter_and_normalizes",
        disposition: Disposition::RealProductGap {
            target: "skit-runtime resolved JavaScript runtime identity",
            state: GapState::Executable,
        },
    },
    AuditedOwner {
        name: "test_gate_node_skips_ts_suffix",
        disposition: Disposition::RealProductGap {
            target: "skit-runtime JavaScript syntax gate",
            state: GapState::Executable,
        },
    },
    AuditedOwner {
        name: "test_mjs_origin_esm_copy_survives_gate2_before_any_package_json",
        disposition: Disposition::RealProductGap {
            target: "skit-runtime JavaScript syntax gate plus real Node",
            state: GapState::Executable,
        },
    },
    AuditedOwner {
        name: "test_gate_node_skips_when_runner_is_not_node",
        disposition: Disposition::RealProductGap {
            target: "skit-runtime JavaScript syntax gate",
            state: GapState::Executable,
        },
    },
    AuditedOwner {
        name: "test_gate_node_passes_on_returncode_zero",
        disposition: Disposition::RealProductGap {
            target: "skit-runtime JavaScript syntax gate",
            state: GapState::Executable,
        },
    },
    AuditedOwner {
        name: "test_gate_node_raises_on_nonzero",
        disposition: Disposition::RealProductGap {
            target: "skit-runtime JavaScript syntax gate",
            state: GapState::Executable,
        },
    },
    AuditedOwner {
        name: "test_gate_node_raises_on_nonzero_with_empty_stderr",
        disposition: Disposition::RealProductGap {
            target: "skit-runtime JavaScript syntax gate",
            state: GapState::Executable,
        },
    },
    AuditedOwner {
        name: "test_gate_node_survives_a_spawn_failure",
        disposition: Disposition::RealProductGap {
            target: "skit-runtime JavaScript syntax gate",
            state: GapState::Executable,
        },
    },
    AuditedOwner {
        name: "test_gate2_failure_removes_the_temp_copy",
        disposition: Disposition::RealProductGap {
            target: "skit-runtime owned-source gate helper",
            state: GapState::Executable,
        },
    },
    AuditedOwner {
        name: "test_execute_maps_a_drifted_js_definition_to_drift",
        disposition: Disposition::RealProductGap {
            target: "Batch C: CLI drift error projection with --resync guidance",
            state: GapState::Executable,
        },
    },
    AuditedOwner {
        name: "test_execute_syntax_gate_failure_never_launches",
        disposition: Disposition::RealProductGap {
            target: "Batch C: CLI injection-syntax error projection without --resync guidance",
            state: GapState::Executable,
        },
    },
];

fn has_test_attribute(attributes: &[Attribute]) -> bool {
    attributes
        .iter()
        .any(|attribute| attribute.path().is_ident("test"))
}

fn frozen_test_names(source: &str) -> Vec<String> {
    syn::parse_file(source)
        .unwrap()
        .items
        .into_iter()
        .filter_map(|item| match item {
            Item::Fn(function) if has_test_attribute(&function.attrs) => {
                let name = function.sig.ident.to_string();
                name.starts_with("test_").then_some(name)
            }
            _ => None,
        })
        .collect()
}

fn executable_names(repo: &Path) -> Vec<String> {
    TARGETS
        .iter()
        .flat_map(|target| frozen_test_names(&fs::read_to_string(repo.join(target)).unwrap()))
        .collect()
}

#[test]
fn test_js_inject_frozen_names_are_exactly_and_uniquely_accounted() {
    assert_eq!(AUDITED.len(), 20);
    assert_eq!(
        AUDITED
            .iter()
            .filter(|owner| matches!(owner.disposition, Disposition::StrongerOwnerClosure { .. }))
            .count(),
        4
    );
    assert_eq!(
        AUDITED
            .iter()
            .filter(|owner| matches!(owner.disposition, Disposition::StaleExactRehome { .. }))
            .count(),
        2
    );
    assert_eq!(
        AUDITED
            .iter()
            .filter(|owner| matches!(owner.disposition, Disposition::RuntimeGatedRehome { .. }))
            .count(),
        3
    );
    assert_eq!(
        AUDITED
            .iter()
            .filter(|owner| matches!(owner.disposition, Disposition::RealProductGap { .. }))
            .count(),
        11
    );

    let audited_names = AUDITED
        .iter()
        .map(|owner| owner.name)
        .collect::<BTreeSet<_>>();
    assert_eq!(audited_names.len(), AUDITED.len());

    let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .unwrap();
    let actual = executable_names(repo);
    let actual_set = actual.iter().map(String::as_str).collect::<BTreeSet<_>>();
    let expected = EXECUTABLE.iter().copied().collect::<BTreeSet<_>>();
    assert_eq!(
        actual.len(),
        actual_set.len(),
        "an exact JS injection owner is duplicated"
    );
    assert_eq!(
        actual_set, expected,
        "the executable JS injection owner set drifted"
    );

    let absent = AUDITED
        .iter()
        .filter(|owner| {
            !matches!(
                owner.disposition,
                Disposition::StaleExactRehome {
                    state: GapState::Executable,
                    ..
                } | Disposition::RuntimeGatedRehome {
                    state: GapState::Executable,
                    ..
                } | Disposition::RealProductGap {
                    state: GapState::Executable,
                    ..
                }
            )
        })
        .map(|owner| owner.name)
        .collect::<BTreeSet<_>>();
    assert!(actual_set.is_disjoint(&absent));
    assert_eq!(actual_set.len() + absent.len(), 37);

    for owner in AUDITED {
        match owner.disposition {
            Disposition::StrongerOwnerClosure { reason } => assert!(!reason.trim().is_empty()),
            Disposition::StaleExactRehome { target, .. }
            | Disposition::RuntimeGatedRehome { target, .. }
            | Disposition::RealProductGap { target, .. } => assert!(!target.trim().is_empty()),
        }
    }
}
