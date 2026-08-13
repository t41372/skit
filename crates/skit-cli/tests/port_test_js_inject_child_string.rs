#[path = "support/js_inject.rs"] mod support;
use support::{Sandbox, output_text, python_order_runtime};

#[test]
fn test_injected_string_reaches_the_child() {
    let Some(runtime) = python_order_runtime() else { return; };
    let sandbox = Sandbox::new();
    sandbox.create_managed_entry(
        "childstring", "js", "childstring.js",
        "const CITY = \"here\";\nconsole.log(CITY);\n", &runtime,
    );
    let output = sandbox.run("childstring", "CITY", "台北 🚀");
    let text = output_text(&output);
    assert_eq!(output.status.code(), Some(0), "runtime={runtime}: {text}");
    assert_eq!(String::from_utf8_lossy(&output.stdout).lines().last(), Some("台北 🚀"));
    assert!(sandbox.staged_files("childstring").is_empty());
}
