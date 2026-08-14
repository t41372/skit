//! Exact hosted-summary contracts from Python v0.4 `tests/test_add_no_source.py`.
//!
//! Python unit-tested `_hosted_add_summary` directly. Rust proves the same projection through the
//! public add command, the stored copy, and the user-visible summary; no replacement helper exists
//! in test code.

#[path = "support/add_no_source.rs"]
mod support;

use std::fs;

use skit_application::EntryRepository as _;
use skit_domain::parameters::ParamDecl;
use skit_language::{managed_params, read_uv_metadata, write_managed_params, write_uv_metadata};
use support::{Sandbox, combined};

fn two_decls() -> Vec<ParamDecl> {
    let mut api = ParamDecl::new("API_TOKEN");
    api.secret = false;
    let mut city = ParamDecl::new("city");
    city.secret = true;
    vec![api, city]
}

fn add_python_with_managed(s: &Sandbox, name: &str) -> (std::process::Output, String) {
    let with_uv = write_uv_metadata("print(1)\n", &["rich>=13".to_owned()], "").unwrap();
    let source = write_managed_params("python", &with_uv, &two_decls()).unwrap();
    let path = s.source(&format!("{name}.py"), source.as_bytes());
    let output = s.run(&[
        "add",
        path.to_str().unwrap(),
        "--name",
        name,
        "--no-input",
    ]);
    let shown = combined(&output);
    assert_eq!(output.status.code(), Some(0), "{shown}");
    (output, shown)
}

fn stored_text(s: &Sandbox, name: &str) -> String {
    let store = s.store();
    let entry = store.resolve(name).unwrap();
    fs::read_to_string(store.payload_path(&entry).unwrap()).unwrap()
}

#[test]
fn test_hosted_interpreted_branch_prints_managed_and_secret_lines() {
    let s = Sandbox::new();
    let source = write_managed_params(
        "shell",
        "#!/bin/sh\nAPI_TOKEN=x\ncity=Taipei\necho ok\n",
        &two_decls(),
    )
    .unwrap();
    let path = s.source("mystery.sh", source.as_bytes());
    let output = s.run(&[
        "add",
        path.to_str().unwrap(),
        "--name",
        "shmystery",
        "--no-input",
    ]);
    let shown = combined(&output);
    assert_eq!(output.status.code(), Some(0), "{shown}");

    let stored = stored_text(&s, "shmystery");
    let decls = managed_params("shell", &stored);
    assert_eq!(decls.iter().map(|decl| decl.name.as_str()).collect::<Vec<_>>(), ["API_TOKEN", "city"]);
    assert_eq!(decls.iter().filter(|decl| decl.secret).map(|decl| decl.name.as_str()).collect::<Vec<_>>(), ["city"]);
    assert!(shown.contains("Managed parameters"), "{shown}");
    assert!(shown.contains("city"), "{shown}");
    assert!(shown.contains("Secret parameter values are never saved"), "{shown}");
}

#[test]
fn test_hosted_python_branch_prints_managed_and_secret_lines() {
    let s = Sandbox::new();
    let (_output, shown) = add_python_with_managed(&s, "pyhosted");
    let stored = stored_text(&s, "pyhosted");
    let uv = read_uv_metadata(&stored).expect("PEP 723 metadata survived the add");
    assert_eq!(uv.dependencies, ["rich>=13"]);
    let decls = managed_params("python", &stored);
    assert_eq!(decls.iter().map(|decl| decl.name.as_str()).collect::<Vec<_>>(), ["API_TOKEN", "city"]);
    assert_eq!(decls.iter().filter(|decl| decl.secret).map(|decl| decl.name.as_str()).collect::<Vec<_>>(), ["city"]);
    assert!(shown.contains("Managed parameters"), "{shown}");
    assert!(shown.contains("Dependencies"), "{shown}");
    assert!(shown.contains("rich>=13"), "{shown}");
    assert!(shown.contains("Secret parameter values are never saved"), "{shown}");
}

#[test]
fn test_ans_tui_summary_receives_deps_params_and_secrets() {
    let s = Sandbox::new();
    let (_output, shown) = add_python_with_managed(&s, "thing");
    let stored = stored_text(&s, "thing");
    assert_eq!(read_uv_metadata(&stored).unwrap().dependencies, ["rich>=13"]);
    let decls = managed_params("python", &stored);
    assert_eq!(decls.iter().map(|decl| decl.name.as_str()).collect::<Vec<_>>(), ["API_TOKEN", "city"]);
    assert_eq!(decls.iter().filter(|decl| decl.secret).map(|decl| decl.name.as_str()).collect::<Vec<_>>(), ["city"]);
    assert!(shown.contains("thing"), "the created entry was not identified in the summary: {shown}");
}

#[test]
fn test_hosted_add_summary_script_reads_decls_and_honors_decl_secret() {
    let s = Sandbox::new();
    let (_output, _shown) = add_python_with_managed(&s, "summary-script");
    let stored = stored_text(&s, "summary-script");
    let decls = managed_params("python", &stored);
    let managed = decls.iter().map(|decl| decl.name.as_str()).collect::<Vec<_>>();
    let secrets = decls.iter().filter(|decl| decl.secret).map(|decl| decl.name.as_str()).collect::<Vec<_>>();
    assert_eq!(read_uv_metadata(&stored).unwrap().dependencies, ["rich>=13"]);
    assert_eq!(managed, ["API_TOKEN", "city"]);
    assert_eq!(secrets, ["city"], "name heuristic must not override the stored decl.secret bits");
}

#[test]
fn test_hosted_add_summary_prompt_falls_back_to_meta_and_name_heuristic() {
    let s = Sandbox::new();
    let path = s.source("greet.prompt.md", b"Say hi to {{name}} using {{API_KEY}}.\n");
    let output = s.run(&[
        "add",
        path.to_str().unwrap(),
        "--name",
        "greet",
        "--no-input",
    ]);
    let shown = combined(&output);
    assert_eq!(output.status.code(), Some(0), "{shown}");
    let entry = s.store().resolve("greet").unwrap();
    let params = skit_domain::EntrySettings::from_meta(&entry.meta).params;
    assert_eq!(params, ["name", "API_KEY"]);
    assert!(shown.contains("API_KEY"), "{shown}");
    assert!(shown.contains("Secret parameter values are never saved"), "{shown}");
    assert!(!shown.contains("Dependencies"), "prompt summary invented package dependencies: {shown}");
}

#[test]
fn test_hosted_add_summary_command_uses_meta_fallback() {
    let s = Sandbox::new();
    let output = s.run(&[
        "add",
        "--cmd",
        "echo {msg} {API_KEY}",
        "--name",
        "cmdsum",
    ]);
    let shown = combined(&output);
    assert_eq!(output.status.code(), Some(0), "{shown}");
    let entry = s.store().resolve("cmdsum").unwrap();
    let params = skit_domain::EntrySettings::from_meta(&entry.meta).params;
    assert_eq!(params, ["msg", "API_KEY"]);
    assert!(shown.contains("msg"), "{shown}");
    assert!(shown.contains("API_KEY"), "{shown}");
    assert!(shown.contains("Secret parameter values are never saved"), "{shown}");
    assert!(!shown.contains("Dependencies"), "command summary invented dependencies: {shown}");
}
