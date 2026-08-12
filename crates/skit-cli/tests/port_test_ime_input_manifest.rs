//! Architecture-accounting guard for Python `tests/test_ime_input.py` at `main@206f9ef`.
//!
//! Python needs three tests because the Textual implementation has three ways to lose its
//! `TEXTUAL_DISABLE_KITTY_KEY` opt-out: missing package-import default, overwriting an explicit user
//! override, or importing Textual before the flag lands. Rust has no Textual import and no such
//! environment knob. Recreating those seams would be invented behavior. All three collapse onto the
//! stronger observable contract in `port_test_ime_input.rs`: a real initialized TUI PTY emits no
//! kitty keyboard push/set sequence at all.

use std::{fs, path::Path};

use syn::{Attribute, Item};

const PYTHON_CONTRACTS: &[&str] = &[
    "test_kitty_protocol_opt_out_is_set_at_package_import",
    "test_kitty_protocol_opt_out_respects_an_explicit_user_override",
    "test_kitty_protocol_opt_out_lands_before_textual_reads_it",
];
const RUST_ORACLE: &str = "test_kitty_protocol_opt_out_is_effective_before_tui_input_starts";

fn is_test(attributes: &[Attribute]) -> bool {
    attributes.iter().any(|attribute| attribute.path().is_ident("test"))
}

#[test]
fn all_3_python_ime_import_guards_collapse_to_the_real_terminal_protocol_oracle() {
    assert_eq!(PYTHON_CONTRACTS.len(), 3, "frozen Python IME oracle count changed");
    assert_eq!(
        PYTHON_CONTRACTS.iter().copied().collect::<std::collections::BTreeSet<_>>().len(),
        3,
        "duplicate Python names make IME architecture accounting dishonest"
    );

    let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("skit-cli lives under <repo>/crates/skit-cli");
    let path = repo.join("crates/skit-cli/tests/port_test_ime_input.rs");
    let source = fs::read_to_string(&path).unwrap();
    let file = syn::parse_file(&source).expect("IME parity target must parse as Rust");
    let actual = file
        .items
        .iter()
        .filter_map(|item| match item {
            Item::Fn(function) if is_test(&function.attrs) => Some(function.sig.ident.to_string()),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        actual,
        [RUST_ORACLE],
        "the IME architecture port must remain the single real-terminal no-kitty-protocol oracle"
    );

    // The observable test must still pin both the exact incident sequence and the generalized
    // numeric push/set scanner. This is a manifest-strength check, not a substitute for executing
    // the PTY test itself.
    assert!(source.contains(r#"b\"\\x1b[>25u\""#));
    assert!(source.contains("kitty_keyboard_enable_sequences(&output)"));
}
