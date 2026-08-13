#![cfg(unix)]
#[path = "support/js_inject.rs"] mod support;
use support::{Sandbox, output_text};

#[test]
fn test_execute_maps_a_drifted_js_definition_to_drift() {
    let sandbox = Sandbox::new();
    sandbox.install_inspector_node();
    sandbox.create_drifted_entry("mismatch", "node");
    let marker = sandbox.home_path().join("ran");
    let output = sandbox.run_with_marker("mismatch", "WIDTH", "1200", &marker);
    let text = output_text(&output);
    assert_eq!(output.status.code(), Some(125), "{text}");
    let guidance = ["--", "resync"].concat();
    assert!(text.contains(&guidance), "missing Python repair guidance: {text}");
    assert!(!marker.exists(), "source/form mismatch reached the child");
    assert!(sandbox.staged_files("mismatch").is_empty());
}
