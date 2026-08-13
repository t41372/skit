#![cfg(unix)]
#[path = "support/js_inject.rs"] mod support;
use support::{Sandbox, output_text};

#[test]
fn test_execute_refuses_a_bad_value_before_launch() {
    let sandbox = Sandbox::new();
    sandbox.install_inspector_node();
    sandbox.create_managed_entry(
        "badvalue", "js", "badvalue.js", "const WIDTH = 800;\n", "node",
    );
    let marker = sandbox.home_path().join("launched");
    let output = sandbox.run_with_marker("badvalue", "WIDTH", "abc", &marker);
    let text = output_text(&output);
    assert_eq!(output.status.code(), Some(125), "{text}");
    assert!(!marker.exists(), "bad JS value reached the child");
    assert!(sandbox.staged_files("badvalue").is_empty());
}
