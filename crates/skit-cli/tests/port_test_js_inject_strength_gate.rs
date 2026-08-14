use std::path::Path;

#[test]
fn superseded_js_inject_fixture_is_not_allowed_to_count_as_parity() {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .unwrap();
    assert!(
        !repo.join("crates/skit-cli/tests/port_test_js_inject_cli.rs").exists(),
        "the superseded JS injection fixture contains a known TS module-flavor test bug; remove it before this module can be accounted"
    );
}
