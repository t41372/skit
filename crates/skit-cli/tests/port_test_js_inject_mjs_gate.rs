//! Exact `.mjs` gate/runtime regression from Python v0.4 `tests/test_js_inject.py`.

use std::process::Command as ProcessCommand;

use skit_application::EntryRepository as _;

#[path = "support/js_inject.rs"]
mod support;
use support::{Sandbox, output_text, real_program};

#[test]
fn test_mjs_origin_esm_copy_survives_gate2_before_any_package_json() {
    let Some(node) = real_program("node") else {
        // Python oracle is skip-gated when Node is not installed.
        return;
    };
    let supports_old_module_mode = ProcessCommand::new(&node)
        .args(["--no-experimental-detect-module", "-e", ""])
        .status()
        .is_ok_and(|status| status.success());
    if !supports_old_module_mode {
        // Same skip boundary as the frozen Python test: old Node builds that predate the flag.
        return;
    }

    let sandbox = Sandbox::new();
    sandbox.create_managed_entry(
        "mjs-gate",
        "js",
        "orig.mjs",
        "import assert from \"node:assert\";\nconst N = 5;\nassert.ok(N);\nconsole.log('N=' + N);\n",
        node.to_str().expect("node path must be UTF-8 for the test fixture"),
    );
    let output = sandbox
        .command()
        .env("NODE_OPTIONS", "--no-experimental-detect-module")
        .args(["run", "mjs-gate", "--set", "N=7", "--no-input"])
        .output()
        .unwrap();
    let text = output_text(&output);
    assert_eq!(output.status.code(), Some(0), "{text}");
    assert!(text.contains("N=7"), "real Node did not execute the injected ESM copy:\n{text}");

    let entry = sandbox.store().resolve("mjs-gate").unwrap();
    let entry_dir = sandbox.data_path().join("scripts").join(entry.slug.as_str());
    assert!(
        !entry_dir.join("package.json").exists(),
        "the regression must be proven before any package.json can establish module type"
    );
    assert!(sandbox.staged_files("mjs-gate").is_empty());
}
