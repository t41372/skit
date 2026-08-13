//! Frozen JS/TS staged-copy contracts from Python v0.4 `tests/test_js_inject.py`.
#![cfg(unix)]

#[path = "support/js_inject.rs"]
mod support;
use support::{Sandbox, body, output_text, tagged};

#[test]
fn test_ts_temp_copy_has_ts_suffix() {
    let sandbox = Sandbox::new();
    sandbox.install_inspector_node();
    sandbox.create_managed_entry("tscopy", "ts", "plain.ts", "const N: number = 5;\n", "node");
    let output = sandbox.run("tscopy", "N", "7");
    let text = output_text(&output);
    assert_eq!(output.status.code(), Some(0), "{text}");
    assert!(tagged(&text, "FAKE_PATH=").ends_with(".ts"), "{text}");
    assert!(body(&text).contains("const N: number = 7;"), "{text}");
    assert!(sandbox.staged_files("tscopy").is_empty());
}

fn assert_module_flavor(origin: &str, kind: &str, expected: &str, name: &str) {
    let sandbox = Sandbox::new();
    sandbox.install_inspector_node();
    let (source, injected_line) = if kind == "js" {
        ("const N = 5;\n", "const N = 7;")
    } else {
        ("const N: number = 5;\n", "const N: number = 7;")
    };
    sandbox.create_managed_entry(name, kind, origin, source, "node");
    let output = sandbox.run(name, "N", "7");
    let text = output_text(&output);
    assert_eq!(output.status.code(), Some(0), "origin={origin:?}: {text}");
    assert!(tagged(&text, "FAKE_PATH=").ends_with(expected), "origin={origin:?}: {text}");
    assert!(body(&text).contains(injected_line), "origin={origin:?}: {text}");
    assert!(sandbox.staged_files(name).is_empty());
}

#[test]
fn test_injected_copy_carries_the_origins_module_flavor() {
    for (index, (origin, kind, expected)) in [
        ("tool.mjs", "js", ".mjs"),
        ("tool.cjs", "js", ".cjs"),
        ("plain.js", "js", ".js"),
        ("", "js", ".js"),
        ("tool.mts", "ts", ".mts"),
        ("tool.cts", "ts", ".cts"),
        ("plain.ts", "ts", ".ts"),
    ]
    .into_iter()
    .enumerate()
    {
        assert_module_flavor(origin, kind, expected, &format!("flavor{index}"));
    }
}

macro_rules! flavor_case {
    ($name:ident, $origin:literal, $kind:literal, $expected:literal) => {
        #[test]
        fn $name() { assert_module_flavor($origin, $kind, $expected, stringify!($name)); }
    };
}
flavor_case!(rust_additive_js_inject_flavor_mjs, "tool.mjs", "js", ".mjs");
flavor_case!(rust_additive_js_inject_flavor_cjs, "tool.cjs", "js", ".cjs");
flavor_case!(rust_additive_js_inject_flavor_js, "plain.js", "js", ".js");
flavor_case!(rust_additive_js_inject_flavor_no_origin, "", "js", ".js");
flavor_case!(rust_additive_js_inject_flavor_mts, "tool.mts", "ts", ".mts");
flavor_case!(rust_additive_js_inject_flavor_cts, "tool.cts", "ts", ".cts");
flavor_case!(rust_additive_js_inject_flavor_ts, "plain.ts", "ts", ".ts");

#[test]
fn test_injected_copy_is_0600() {
    let sandbox = Sandbox::new();
    sandbox.install_inspector_node();
    sandbox.create_managed_entry(
        "secret", "js", "secret.js", "const API_KEY = \"changeme\";\n", "node",
    );
    let output = sandbox.run("secret", "API_KEY", "s3cr3t");
    let text = output_text(&output);
    assert_eq!(output.status.code(), Some(0), "{text}");
    assert_eq!(tagged(&text, "FAKE_MODE="), "-rw-------", "{text}");
    assert!(body(&text).contains("const API_KEY = \"s3cr3t\";"), "{text}");
    assert!(sandbox.staged_files("secret").is_empty());
}
