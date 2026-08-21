//! Final CLI/composition owners from Python v0.4 `tests/test_js_inject.py`.

use std::{fs, process::Command as ProcessCommand};

use skit_application::EntryRepository as _;
use skit_domain::EntrySettings;
use skit_form::{FormSource, form_plan};
use skit_runtime::{ProgramProbe as _, SystemProbe};

#[path = "support/js_inject_batch_c.rs"]
mod support;
use support::{Sandbox, exact_tree_keys, oracle_runtime, output_text, tagged};

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
        let sandbox = Sandbox::new();
        sandbox.install_inspector_node();
        let source = if kind == "js" {
            "const N = 5;\n"
        } else {
            "const N: number = 5;\n"
        };
        let name = format!("flavor{index}");
        sandbox.create_managed_entry(&name, kind, origin, source, "node", Vec::new());
        let output = sandbox.run(&name, "N", "7");
        let text = output_text(&output);
        assert_eq!(output.status.code(), Some(0), "origin={origin:?}: {text}");
        assert!(
            tagged(&text, "FAKE_PATH=").trim_end().ends_with(expected),
            "origin={origin:?}: {text}"
        );
        assert!(sandbox.staged_files(&name).is_empty());
    }
}

#[test]
fn test_execute_runs_a_js_entry_offline_plan() {
    let sandbox = Sandbox::new();
    sandbox.create_managed_entry(
        "offline",
        "js",
        "offline.js",
        "const WIDTH = 800;\nconsole.log(WIDTH);\n",
        "node",
        Vec::new(),
    );
    let entry = sandbox.store().resolve("offline").unwrap();
    let source = fs::read_to_string(sandbox.store().payload_path(&entry).unwrap()).unwrap();
    let plan = form_plan("js", &source, &EntrySettings::from_meta(&entry.meta));
    assert_eq!(plan.source, FormSource::Inject);
    assert_eq!(
        plan.fields
            .iter()
            .map(|field| field.declaration.name.as_str())
            .collect::<Vec<_>>(),
        ["WIDTH"]
    );
}

fn run_real_child(source: &str, key: &str, value: &str) -> Option<String> {
    let runtime = oracle_runtime()?;
    let sandbox = Sandbox::new();
    sandbox.create_managed_entry(
        "child",
        "js",
        "child.js",
        source,
        runtime.to_str().unwrap(),
        Vec::new(),
    );
    let output = sandbox.run("child", key, value);
    let text = output_text(&output);
    assert_eq!(
        output.status.code(),
        Some(0),
        "runtime={}: {text}",
        runtime.display()
    );
    assert!(sandbox.staged_files("child").is_empty());
    Some(String::from_utf8_lossy(&output.stdout).into_owned())
}

#[test]
fn test_injected_const_reaches_the_child() {
    let Some(stdout) = run_real_child(
        "const WIDTH = 800;\nconsole.log('w=' + WIDTH);\n",
        "WIDTH",
        "1200",
    ) else {
        return;
    };
    assert_eq!(stdout.lines().last(), Some("w=1200"));
}

#[test]
fn test_injected_string_reaches_the_child() {
    let Some(stdout) = run_real_child(
        "const CITY = 'here';\nconsole.log(CITY);\n",
        "CITY",
        "台北 🚀",
    ) else {
        return;
    };
    assert_eq!(stdout.lines().last(), Some("台北 🚀"));
}

#[test]
fn test_run_injects_and_executes_end_to_end() {
    if oracle_runtime().is_none() {
        return;
    }
    let sandbox = Sandbox::new();
    let source = sandbox.home.path().join("jsrun1.js");
    fs::write(&source, "const WIDTH = 800;\nconsole.log('w=' + WIDTH);\n").unwrap();
    let added = sandbox
        .command()
        .args([
            "add",
            source.to_str().unwrap(),
            "--kind",
            "js",
            "--name",
            "jsrun1",
            "--no-input",
        ])
        .output()
        .unwrap();
    assert_eq!(added.status.code(), Some(0), "{}", output_text(&added));
    let managed = sandbox
        .command()
        .args(["params", "jsrun1", "--manage", "WIDTH"])
        .output()
        .unwrap();
    assert_eq!(managed.status.code(), Some(0), "{}", output_text(&managed));
    let output = sandbox.run("jsrun1", "WIDTH", "1200");
    let text = output_text(&output);
    assert_eq!(output.status.code(), Some(0), "{text}");
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).lines().last(),
        Some("w=1200")
    );
}

#[test]
fn test_execute_maps_a_drifted_js_definition_to_drift() {
    for (locale, expected) in [
        (
            "en",
            "The script and its form definitions don't match anymore: WIDTH. Run `skit params mismatch --resync` to fix it.",
        ),
        (
            "zh-CN",
            "脚本内容和表单定义对不上了：WIDTH。运行 `skit params mismatch --resync` 即可修复。",
        ),
        (
            "zh-TW",
            "腳本內容和表單定義對不上了：WIDTH。執行 `skit params mismatch --resync` 即可修復。",
        ),
    ] {
        let sandbox = Sandbox::new();
        sandbox.install_inspector_node();
        sandbox.create_drifted_entry("mismatch", vec!["chalk".to_owned()]);
        let before = sandbox.snapshot();
        let launch = sandbox.home.path().join("launched");
        let output = sandbox
            .command_for_locale(locale)
            .env("SKIT_TEST_LAUNCH_MARKER", &launch)
            .args(["run", "mismatch", "--set", "WIDTH=1200", "--no-input"])
            .output()
            .unwrap();
        let text = output_text(&output);
        assert_eq!(output.status.code(), Some(125), "locale={locale}: {text}");
        assert!(text.contains(expected), "locale={locale}: {text}");
        assert!(!launch.exists());
        sandbox.assert_no_dependency_artifacts("mismatch");
        assert_eq!(sandbox.snapshot(), before);
    }
}

#[test]
fn test_execute_syntax_gate_failure_never_launches() {
    for (locale, expected) in [
        (
            "en",
            "skit refused to run its own injected copy: node rejected the injected copy: SyntaxError: boom",
        ),
        (
            "zh-CN",
            "skit 拒绝运行自己注入出来的副本:node 拒绝了注入副本：SyntaxError: boom",
        ),
        (
            "zh-TW",
            "skit 拒絕執行自己注入出來的副本:node 拒絕了注入副本：SyntaxError: boom",
        ),
    ] {
        let sandbox = Sandbox::new();
        sandbox.install_inspector_node();
        sandbox.create_managed_entry(
            "syntax",
            "js",
            "syntax.js",
            "const TITLE = 'hello';\n",
            "node",
            vec!["chalk".to_owned()],
        );
        let before = sandbox.snapshot();
        let gate = sandbox.home.path().join("gated");
        let launch = sandbox.home.path().join("launched");
        let output = sandbox
            .command_for_locale(locale)
            .env("SKIT_TEST_GATE_MARKER", &gate)
            .env("SKIT_TEST_LAUNCH_MARKER", &launch)
            .env("SKIT_TEST_REJECT_CHECK", "1")
            .args(["run", "syntax", "--set", "TITLE=x", "--no-input"])
            .output()
            .unwrap();
        let text = output_text(&output);
        assert_eq!(output.status.code(), Some(125), "locale={locale}: {text}");
        assert!(gate.exists(), "node --check was not called: {text}");
        assert!(!launch.exists(), "rejected source launched: {text}");
        assert!(text.contains(expected), "locale={locale}: {text}");
        assert!(!text.contains("--resync"));
        sandbox.assert_no_dependency_artifacts("syntax");
        assert_eq!(sandbox.snapshot(), before);
    }
}

#[test]
fn rust_additive_bad_value_precedes_all_dependency_writes() {
    let sandbox = Sandbox::new();
    sandbox.install_inspector_node();
    sandbox.create_managed_entry(
        "bad",
        "js",
        "bad.js",
        "const WIDTH = 800;\n",
        "node",
        vec!["chalk".to_owned()],
    );
    let before = sandbox.snapshot();
    let launch = sandbox.home.path().join("launched");
    let output = sandbox
        .command()
        .env("SKIT_TEST_LAUNCH_MARKER", &launch)
        .args(["run", "bad", "--set", "WIDTH=abc", "--no-input"])
        .output()
        .unwrap();
    let text = output_text(&output);
    assert_eq!(output.status.code(), Some(125), "{text}");
    let after = sandbox.snapshot();
    assert_eq!(exact_tree_keys(&after), exact_tree_keys(&before));
    assert_eq!(after, before);
    assert!(!launch.exists());
    sandbox.assert_no_dependency_artifacts("bad");
}

#[test]
fn test_mjs_origin_esm_copy_survives_gate2_before_any_package_json() {
    let Some(node) = SystemProbe.find_program("node") else {
        return;
    };
    let supports_old_module_mode = ProcessCommand::new(&node)
        .args(["--no-experimental-detect-module", "-e", ""])
        .status()
        .is_ok_and(|status| status.success());
    if !supports_old_module_mode {
        return;
    }

    let sandbox = Sandbox::new();
    sandbox.install_node_order_wrapper();
    sandbox.create_managed_entry(
        "mjs-gate",
        "js",
        "orig.mjs",
        "import assert from 'node:assert';\nconst N = 5;\nassert.ok(N);\nconsole.log('N=' + N);\n",
        "node",
        Vec::new(),
    );
    let entry_dir = sandbox.entry_dir("mjs-gate");
    let gate = sandbox.home.path().join("gated");
    assert!(!entry_dir.join("package.json").exists());
    let output = sandbox
        .command()
        .env("SKIT_REAL_NODE", &node)
        .env("SKIT_TEST_ENTRY_DIR", &entry_dir)
        .env("SKIT_TEST_GATE_MARKER", &gate)
        .env("NODE_OPTIONS", "--no-experimental-detect-module")
        .args(["run", "mjs-gate", "--set", "N=7", "--no-input"])
        .output()
        .unwrap();
    let text = output_text(&output);
    assert_eq!(output.status.code(), Some(0), "{text}");
    assert!(gate.exists(), "node gate did not run: {text}");
    assert!(text.contains("N=7"), "{text}");
    assert!(
        entry_dir.join("package.json").exists(),
        "successful launch did not materialize the module manifest"
    );
    assert!(sandbox.staged_files("mjs-gate").is_empty());
}
