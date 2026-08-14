//! Exact user-visible `$0` warning contracts from Python v0.4 `tests/test_shell_inject.py`.
#![cfg(unix)]

#[path = "support/shell_inject.rs"]
mod support;

use std::fs;
use support::{Sandbox, output_text};

fn install_quiet_bash(sandbox: &Sandbox) {
    use std::os::unix::fs::PermissionsExt as _;

    let path = sandbox.bin_path().join("bash");
    fs::write(
        &path,
        r#"#!/bin/sh
if [ "$1" = "-n" ]; then exit 0; fi
exit 0
"#,
    )
    .unwrap();
    let mut permissions = fs::metadata(&path).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(path, permissions).unwrap();
}

#[test]
fn test_self_location_warns_when_a_temp_copy_is_written() {
    let sandbox = Sandbox::new();
    install_quiet_bash(&sandbox);
    sandbox.create_managed_entry(
        "selfwarn",
        "#!/usr/bin/env bash\nHERE=$(dirname \"$0\")\nWIDTH=800\necho \"$HERE $WIDTH\"\n",
    );

    let output = sandbox.run_sets("selfwarn", &[("WIDTH", "1200")]);
    let text = output_text(&output);
    assert_eq!(output.status.code(), Some(0), "{text}");
    assert!(text.contains("$0"), "the temp-copy self-location warning was lost:\n{text}");
    assert!(
        text.contains("NAME=\"${NAME:-value}\""),
        "the warning must teach the manual env-default idiom:\n{text}"
    );
    assert!(
        text.contains("on the stored copy"),
        "the warning must scope --normalize to a stored copy:\n{text}"
    );
    assert!(
        !text.contains("`skit params <script> --normalize NAME` delivers"),
        "the old unconditional normalize claim must not return:\n{text}"
    );
    assert!(sandbox.staged_files("selfwarn").is_empty(), "{text}");
}

#[test]
fn test_self_location_does_not_warn_for_env_delivery() {
    let sandbox = Sandbox::new();
    install_quiet_bash(&sandbox);
    sandbox.create_managed_entry(
        "selfenv",
        "#!/usr/bin/env bash\nHERE=$(dirname \"$0\")\necho \"${MODE:-auto} $HERE\"\n",
    );

    let output = sandbox.run_sets("selfenv", &[("MODE", "manual")]);
    let text = output_text(&output);
    assert_eq!(output.status.code(), Some(0), "{text}");
    assert!(!text.contains("$0"), "env-only delivery must not emit a temp-copy warning:\n{text}");
    assert!(!text.contains("on the stored copy"), "env-only delivery must not offer normalize advice:\n{text}");
    assert!(sandbox.staged_files("selfenv").is_empty(), "env-only delivery materialized a source copy:\n{text}");
}

#[test]
fn test_no_self_location_no_warning() {
    let sandbox = Sandbox::new();
    install_quiet_bash(&sandbox);
    sandbox.create_managed_entry(
        "noself",
        "#!/usr/bin/env bash\nWIDTH=800\necho \"$WIDTH\"\n",
    );

    let output = sandbox.run_sets("noself", &[("WIDTH", "1200")]);
    let text = output_text(&output);
    assert_eq!(output.status.code(), Some(0), "{text}");
    assert!(!text.contains("$0"), "a non-self-locating script must not receive the warning:\n{text}");
    assert!(!text.contains("on the stored copy"), "normalize advice must not appear without the self-location risk:\n{text}");
    assert!(sandbox.staged_files("noself").is_empty(), "staged source was not cleaned:\n{text}");
}
