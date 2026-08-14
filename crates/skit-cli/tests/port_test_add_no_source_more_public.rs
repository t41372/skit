//! More exact public-process ports from Python v0.4 `tests/test_add_no_source.py`.

#[path = "support/add_no_source.rs"]
mod support;

use skit_application::EntryRepository as _;
use support::{Sandbox, combined, flat};

fn assert_empty(s: &Sandbox) {
    assert!(s.store().scan().unwrap().entries.is_empty());
}

#[test]
fn test_add_unknown_directory_plain_confirm_yes_adds_program() {
    let s = Sandbox::new();
    let directory = s.home().join("bundle.dir");
    std::fs::create_dir(&directory).unwrap();
    let input = "y\ntoolname\na dir-shaped tool\n";
    let (code, output) = s.run_pty(&["add", directory.to_str().unwrap()], input);
    assert_eq!(code, 0, "{output}");
    let entry = s.store().resolve("toolname").unwrap();
    assert_eq!(entry.meta.kind.as_str(), "exe");
    assert_eq!(entry.meta.description, "a dir-shaped tool");
}

#[test]
fn test_add_unknown_directory_plain_confirm_no_cancels() {
    let s = Sandbox::new();
    let directory = s.home().join("bundle.dir");
    std::fs::create_dir(&directory).unwrap();
    let (code, output) = s.run_pty(&["add", directory.to_str().unwrap()], "n\n");
    assert_eq!(code, 130, "{output}");
    assert!(output.to_ascii_lowercase().contains("nothing was added"), "{output}");
    assert_empty(&s);
}

#[test]
fn test_add_unknown_directory_plain_confirm_call_contract() {
    let s = Sandbox::new();
    let directory = s.home().join("bundle.dir");
    std::fs::create_dir(&directory).unwrap();
    let (code, output) = s.run_pty(&["add", directory.to_str().unwrap()], "n\n");
    assert_eq!(code, 130, "{output}");
    let shown = flat(&output);
    assert!(
        shown.contains("bundle.dir is a directory. Add it as a program that runs directly?"),
        "{shown}"
    );
    assert!(
        shown.contains("[Y/n]"),
        "the frozen default-yes confirmation affordance disappeared: {shown}"
    );
}

#[test]
fn test_cmd_flag_secret_hole_gets_never_saved_note() {
    let s = Sandbox::new();
    let output = s.run(&["add", "--cmd", "curl -H {API_KEY} {url}", "-n", "curler"]);
    let shown = combined(&output);
    assert_eq!(output.status.code(), Some(0), "{shown}");
    assert!(shown.contains("Detected parameters"), "{shown}");
    assert!(shown.contains("Secret parameter values are never saved"), "{shown}");
    let entry = s.store().resolve("curler").unwrap();
    assert_eq!(entry.meta.kind.as_str(), "command");
}

#[test]
fn test_plain_menu_choice4_secret_hole_gets_never_saved_note() {
    let s = Sandbox::new();
    let (code, output) = s.run_pty(&["add"], "4\ndeploy {AUTH_TOKEN}\ndeployer\n\n");
    assert_eq!(code, 0, "{output}");
    assert!(output.contains("Detected parameters"), "{output}");
    assert!(output.contains("Secret parameter values are never saved"), "{output}");
    assert_eq!(s.store().resolve("deployer").unwrap().meta.kind.as_str(), "command");
}

#[test]
fn test_bare_add_refusal_names_only_lanes_that_honor_the_flag() {
    for (args, advice) in [
        (vec!["add", "--ref"], None),
        (vec!["add", "--exe"], None),
        (vec!["add", "--kind", "shell"], None),
        (vec!["add", "--dep", "rich"], Some("--edit")),
        (vec!["add", "--python", ">=3.11"], Some("--edit")),
        (vec!["add", "--runner", "claude"], Some("--prompt")),
        (vec!["add", "--no-interpolate"], Some("--prompt")),
        (vec!["add", "--name", "x"], Some("--edit, --prompt, --cmd")),
        (vec!["add", "--description", "d"], Some("--edit, --prompt, --cmd")),
    ] {
        let s = Sandbox::new();
        let (code, output) = s.run_pty(&args, "1\n\n");
        let shown = flat(&output);
        assert_eq!(code, 2, "argv={args:?}: {shown}");
        match advice {
            None => assert!(!shown.contains("pick a lane"), "argv={args:?}: {shown}"),
            Some(advice) => assert!(
                shown.contains(&format!("pick a lane outright with {advice} (nothing was added)")),
                "argv={args:?}: {shown}"
            ),
        }
        assert_empty(&s);
    }
}
