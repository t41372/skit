#[path = "support/shell_inject.rs"]
mod support;

use std::{fs, path::PathBuf};

use skit_runtime::{ProgramProbe as _, SystemProbe};
use support::{Sandbox, body, output_text, tagged};

#[cfg(unix)]
#[test]
fn test_env_delivery_writes_no_temp_file() {
    let sandbox = Sandbox::new();
    sandbox.install_inspector_bash();
    sandbox.create_managed_entry(
        "env_only",
        "#!/usr/bin/env bash\necho \"${GREETING:-hello}\"\n",
    );
    let payload = sandbox.payload_path("env_only");
    let stored = fs::read_to_string(&payload).unwrap();

    let output = sandbox.run_sets("env_only", &[("GREETING", "hi there")]);
    let text = output_text(&output);
    assert_eq!(output.status.code(), Some(0), "{text}");
    assert_eq!(PathBuf::from(tagged(&text, "SHELL_PATH=")), payload);
    assert_eq!(tagged(&text, "SHELL_GREETING="), "hi there");
    assert_eq!(body(&text), stored);
    assert!(sandbox.staged_files("env_only").is_empty(), "{text}");
    assert!(!text.contains("→ inject:"), "env-only delivery must not claim a source rewrite:\n{text}");
    assert!(!text.contains("$0"), "env-only delivery must not emit the temp-copy self-location warning:\n{text}");
}

#[cfg(unix)]
#[test]
fn test_env_delivery_actually_reaches_the_script() {
    if SystemProbe.find_program("bash").is_none() {
        return;
    }
    let sandbox = Sandbox::new();
    sandbox.create_managed_entry(
        "env_exec",
        concat!(
            "#!/usr/bin/env bash\n",
            "printf '%s\\n' \"${GREETING:-hello}\" > \"$PWD/env-result.txt\"\n",
        ),
    );
    let output = sandbox.run_sets("env_exec", &[("GREETING", "hi there")]);
    let text = output_text(&output);
    assert_eq!(output.status.code(), Some(0), "{text}");
    assert_eq!(
        fs::read_to_string(sandbox.home_path().join("env-result.txt")).unwrap(),
        "hi there\n"
    );
    assert!(sandbox.staged_files("env_exec").is_empty(), "{text}");
}

#[cfg(unix)]
#[test]
fn test_mixed_env_and_const_delivery() {
    let sandbox = Sandbox::new();
    sandbox.install_inspector_bash();
    sandbox.create_managed_entry(
        "mixed",
        "#!/usr/bin/env bash\nWIDTH=800\necho \"${MODE:-auto} $WIDTH\"\n",
    );
    let payload = sandbox.payload_path("mixed");

    let output = sandbox.run_sets("mixed", &[("WIDTH", "1200"), ("MODE", "manual")]);
    let text = output_text(&output);
    assert_eq!(output.status.code(), Some(0), "{text}");
    assert_eq!(tagged(&text, "SHELL_MODE="), "manual");
    let staged = PathBuf::from(tagged(&text, "SHELL_PATH="));
    assert_ne!(staged, payload, "const delivery must materialize an injected source copy");
    let staged_body = body(&text);
    assert!(staged_body.contains("WIDTH=1200"), "{staged_body}");
    assert!(staged_body.contains("${MODE:-auto}"), "envdefault must remain source-driven:\n{staged_body}");
    assert!(!staged_body.contains("MODE=manual"), "env delivery must not rewrite the source:\n{staged_body}");
    assert!(sandbox.staged_files("mixed").is_empty(), "staged copy must be cleaned after launch:\n{text}");
}
