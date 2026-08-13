#[path = "js_inject.rs"] mod base;
use base::{Sandbox, output_text};

pub fn run_case() {
    let sandbox = Sandbox::new();
    sandbox.install_rejecting_check_node();
    sandbox.create_managed_entry("gatecase", "js", "gatecase.js", "const T = \"hi\";\n", "node");
    let marker = sandbox.home_path().join("ran");
    let output = sandbox.run_with_marker("gatecase", "T", "x", &marker);
    let text = output_text(&output);
    assert_eq!(output.status.code(), Some(125), "{text}");
    assert!(!marker.exists());
    assert!(sandbox.staged_files("gatecase").is_empty());
}
