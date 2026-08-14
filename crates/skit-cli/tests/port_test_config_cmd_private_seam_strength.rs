//! Strength gate for the private Rust dependency-injection seams behind frozen config wizard tests.
//!
//! These internal tests are not counted as Python parity: the public executable set lives in
//! `port_test_config_cmd_*`.  They are required because the only deterministic way Rust can inject
//! network state and scripted wizard answers is private to `cli.rs`; replacing them with a live-
//! network PTY test would make the oracle weaker and host-dependent.

use std::collections::BTreeSet;
use syn::{Attribute, Item};

fn has_test(attributes: &[Attribute]) -> bool {
    attributes.iter().any(|attribute| attribute.path().is_ident("test"))
}

#[test]
fn rust_private_first_run_seams_remain_deterministically_tested() {
    let source = include_str!("../src/cli/tests.rs");
    let file = syn::parse_file(source).expect("cli internal tests must remain valid Rust");
    let tests = file
        .items
        .into_iter()
        .filter_map(|item| match item {
            Item::Fn(function) if has_test(&function.attrs) => Some(function.sig.ident.to_string()),
            _ => None,
        })
        .collect::<BTreeSet<_>>();

    for required in [
        "the_first_run_mirror_offer_writes_its_marker_once_and_never_probes_again",
        "a_blocked_network_offers_the_wizard_and_a_decline_only_writes_the_marker",
        "a_blocked_network_configures_each_axis_from_its_own_answer",
        "the_mirror_wizard_answers_reach_the_store_on_every_axis",
        "a_custom_mirror_url_is_one_token_and_github_demands_https",
    ] {
        assert!(
            tests.contains(required),
            "private deterministic first-run/wizard coverage disappeared: {required}"
        );
    }

    assert!(
        source.contains("a non-interactive run probed the network")
            && source.contains("first_run_mirror_offer(&store, &Forbidden, &ScriptedFirstRun::default(), false)"),
        "the private non-interactive no-probe contract disappeared"
    );
}
