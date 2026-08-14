#[path = "support/js_inject.rs"] mod support;
use support::{Sandbox, output_text, python_order_runtime};

#[test]
fn test_run_injects_and_executes_end_to_end() {
    let Some(runtime) = python_order_runtime() else { return; };
    let sandbox = Sandbox::new();
    sandbox.create_managed_entry(
        "jsrun1", "js", "jsrun1.js",
        "const WIDTH = 800;\nconsole.log(\"w=\" + WIDTH);\n", &runtime,
    );
    let output = sandbox.run("jsrun1", "WIDTH", "1200");
    let text = output_text(&output);
    assert_eq!(output.status.code(), Some(0), "runtime={runtime}: {text}");
    assert_eq!(String::from_utf8_lossy(&output.stdout).lines().last(), Some("w=1200"));
    assert!(sandbox.staged_files("jsrun1").is_empty());
}
