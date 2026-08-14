#[test]
fn mirror_choice_oracle_must_not_be_mapped_to_display_only_assertions() {
    let source = include_str!("port_test_config_mirror_state.rs");
    assert!(
        !source.contains("fn test_axis_choice_readers()"),
        "test_axis_choice_readers includes custom/off semantics; the current preset-display-only mapping is too weak and must not count as Python parity"
    );
}
