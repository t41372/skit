//! Exact installer-diagnostic ports from Python v0.4 `tests/test_js_deps.py`.
#![cfg(unix)]

use std::{fs, path::Path};

use assert_cmd::Command;
use skit_application::{
    CreateEntry, EntryMutationRepository as _, EntryPayload, SourcePermissions, payload_stored_name,
};
use skit_domain::{EntryKind, EntrySettings, StorageMode};
use skit_store::FileStore;
use tempfile::TempDir;

struct Sandbox {
    data: TempDir,
    state: TempDir,
    config: TempDir,
    home: TempDir,
    bin: TempDir,
}

impl Sandbox {
    fn new() -> Self {
        let sandbox = Self {
            data: TempDir::new().unwrap(),
            state: TempDir::new().unwrap(),
            config: TempDir::new().unwrap(),
            home: TempDir::new().unwrap(),
            bin: TempDir::new().unwrap(),
        };
        sandbox.install("node", "#!/bin/sh\nexit 0\n");
        let store = FileStore::new(sandbox.data.path());
        let kind = EntryKind::parse("js").unwrap();
        store
            .create(CreateEntry {
                name: "t".to_owned(),
                kind: kind.clone(),
                mode: StorageMode::Copy,
                source: "t.js".to_owned(),
                workdir: "invoke".to_owned(),
                description: String::new(),
                payload: Some(EntryPayload {
                    bytes: b"console.log(1);\n".to_vec(),
                    stored_name: Some(payload_stored_name(&kind, Path::new("t.js"))),
                    permissions: SourcePermissions::default(),
                }),
                settings: EntrySettings {
                    dependencies: vec!["chalk".to_owned()],
                    interpreter: "node".to_owned(),
                    ..EntrySettings::default()
                },
            })
            .unwrap();
        sandbox
    }

    fn install(&self, name: &str, source: &str) {
        use std::os::unix::fs::PermissionsExt as _;
        let path = self.bin.path().join(name);
        fs::write(&path, source).unwrap();
        let mut permissions = fs::metadata(&path).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(path, permissions).unwrap();
    }

    fn command(&self) -> Command {
        let mut command = assert_cmd::cargo::cargo_bin_cmd!("skit");
        command
            .env("SKIT_DATA_DIR", self.data.path())
            .env("SKIT_STATE_DIR", self.state.path())
            .env("SKIT_CONFIG_DIR", self.config.path())
            .env("SKIT_LANG", "en")
            .env("HOME", self.home.path())
            .env("USERPROFILE", self.home.path())
            .env("XDG_CONFIG_HOME", self.home.path().join("xdg-config"))
            .env("XDG_DATA_HOME", self.home.path().join("xdg-data"))
            .env("XDG_STATE_HOME", self.home.path().join("xdg-state"))
            .env("PATH", self.bin.path())
            .current_dir(self.home.path());
        command
    }

    fn run(&self) -> std::process::Output {
        self.command().args(["run", "t", "--no-input"]).output().unwrap()
    }

    fn fail_with(&self, stderr: &str) -> std::process::Output {
        let escaped = stderr.replace('\\', "\\\\").replace('`', "\\`");
        self.install(
            "npm",
            &format!(
                "#!/bin/sh\nprintf '%s' \"{}\" >&2\nexit 1\n",
                escaped.replace('"', "\\\"").replace('\n', "\\n")
            ),
        );
        self.run()
    }
}

fn text(output: &std::process::Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

const NPM_E404: &str = concat!(
    "npm error code E404\n",
    "npm error 404 Not Found - GET https://registry.npmjs.org/skit-no-such-pkg-e2e-xyz - Not found\n",
    "npm error 404\n",
    "npm error 404  The requested resource 'skit-no-such-pkg-e2e-xyz@*' could not be found or you do not have permission to access it.\n",
    "npm error 404\n",
    "npm error 404 Note that you can also install from a\n",
    "npm error 404 tarball, folder, http url, or git url.\n",
    "npm error A complete log of this run can be found in: /Users/u/.npm/_logs/debug-0.log\n",
);
const DENO_MISSING: &str = concat!(
    "\u{1b}[0m\u{1b}[32mDownload\u{1b}[0m https://registry.npmjs.org/skit-no-such-pkg-e2e-xyz\n",
    "\u{1b}[0m\u{1b}[1m\u{1b}[31merror\u{1b}[0m: npm package 'skit-no-such-pkg-e2e-xyz' does not exist.\n",
);
const BUN_MISSING: &str = concat!(
    "Resolving dependencies\n",
    "Resolved, downloaded and extracted [1]\n",
    "error: GET https://registry.npmjs.org/skit-no-such-pkg-e2e-xyz - 404\n",
    "error: skit-no-such-pkg-e2e-xyz@* failed to resolve\n",
);
const NPM_ERESOLVE: &str = concat!(
    "npm error code ERESOLVE\n",
    "npm error ERESOLVE unable to resolve dependency tree\n",
    "npm error Could not resolve dependency:\n",
    "npm error peer react@\"17.0.2\" from react-dom@17.0.2\n",
    "npm error Fix the upstream dependency conflict, or retry this command with --force\n",
    "npm error For a full report see:\n",
    "npm error /Users/u/.npm/_logs/eresolve-report.txt\n",
);
const NPM_ECONNREFUSED: &str = concat!(
    "npm error code ECONNREFUSED\n",
    "npm error FetchError: request to http://127.0.0.1:9/chalk failed, reason: connect ECONNREFUSED 127.0.0.1:9\n",
    "npm error     at ClientRequest.<anonymous> (/opt/npm/index.js:130:14)\n",
    "npm error If you are behind a proxy, please make sure that the 'proxy' config is set properly.\n",
    "npm error A complete log of this run can be found in: /Users/u/.npm/_logs/debug-0.log\n",
);

#[test]
fn test_failure_detail_against_real_installer_output() {
    for (stderr, expected) in [
        (NPM_E404, "Not Found - GET"),
        (DENO_MISSING, "does not exist"),
        (BUN_MISSING, "failed to resolve"),
        (NPM_ERESOLVE, "dependency conflict"),
        (NPM_ECONNREFUSED, "connect ECONNREFUSED"),
    ] {
        let sandbox = Sandbox::new();
        let output = sandbox.fail_with(stderr);
        let rendered = text(&output);
        assert_ne!(output.status.code(), Some(0), "{rendered}");
        assert!(rendered.contains(expected), "expected {expected:?}:\n{rendered}");
        for forbidden in ["\u{1b}", "A complete log", "tarball, folder", "_logs/", "behind a proxy"] {
            assert!(!rendered.contains(forbidden), "installer noise {forbidden:?} leaked:\n{rendered}");
        }
    }
}

#[test]
fn test_failure_detail_names_the_missing_package() {
    for stderr in [NPM_E404, DENO_MISSING, BUN_MISSING] {
        let sandbox = Sandbox::new();
        let rendered = text(&sandbox.fail_with(stderr));
        assert!(rendered.contains("skit-no-such-pkg-e2e-xyz"), "{rendered}");
    }
}

#[test]
fn test_failure_detail_empty_stderr_degrades() {
    for stderr in ["", "npm error 404\n\n"] {
        let sandbox = Sandbox::new();
        let rendered = text(&sandbox.fail_with(stderr));
        assert!(
            rendered.contains("Installing dependencies failed (npm): ?"),
            "content-free installer stderr must degrade to '?':\n{rendered}"
        );
    }
}

#[test]
fn test_failure_detail_drops_bare_paths_even_without_a_cause_line() {
    let sandbox = Sandbox::new();
    let rendered = text(&sandbox.fail_with(concat!(
        "npm error something odd happened\n",
        "npm error /var/log/npm/report-123.txt\n",
        "npm error C:\\Users\\u\\AppData\\npm-report.txt\n",
    )));
    assert!(rendered.contains("npm error something odd happened"), "{rendered}");
    assert!(!rendered.contains("report-123.txt"), "{rendered}");
    assert!(!rendered.contains("npm-report.txt"), "{rendered}");
}

#[test]
fn test_failure_detail_filters_each_noise_marker() {
    for marker in [
        "npm error 404 failed: A complete log of this run can be found in: /x.log",
        "npm error 404 failed: Note that you can also install from a",
        "npm error 404 failed: tarball, folder, http url, or git url.",
        "npm error failed: For a full report see:",
        "npm error failed: If you are behind a proxy, check 'npm help config'",
    ] {
        let sandbox = Sandbox::new();
        let rendered = text(&sandbox.fail_with(&format!(
            "npm error install failed for pkg\n{marker}\n"
        )));
        assert!(rendered.contains("npm error install failed for pkg"), "{rendered}");
        assert!(!rendered.contains(marker), "noise marker won the diagnostic:\n{rendered}");
    }
}

#[test]
fn test_failure_detail_noise_before_the_cause_still_finds_the_cause() {
    let sandbox = Sandbox::new();
    let rendered = text(&sandbox.fail_with(concat!(
        "npm error A complete log of this run can be found in: /x.log\n",
        "npm error something odd happened\n",
    )));
    assert!(rendered.contains("npm error something odd happened"), "{rendered}");
    assert!(!rendered.contains("A complete log"), "{rendered}");
}

#[test]
fn test_failure_detail_drops_every_npm_prefix_noise_shape() {
    let sandbox = Sandbox::new();
    let rendered = text(&sandbox.fail_with(concat!(
        "npm error something odd happened\n",
        "npm error at Object.fn (/x/y.js:1:1)\n",
        "npm error {\n",
        "npm error }\n",
        "npm error c:\\Users\\u\\report.txt\n",
    )));
    assert!(rendered.contains("npm error something odd happened"), "{rendered}");
    for forbidden in ["Object.fn", "npm error {", "npm error }", "report.txt"] {
        assert!(!rendered.contains(forbidden), "{forbidden:?} leaked:\n{rendered}");
    }
}

#[test]
fn test_failure_detail_deno_line_is_reproduced_exactly() {
    let sandbox = Sandbox::new();
    let rendered = text(&sandbox.fail_with(DENO_MISSING));
    let expected = "Installing dependencies failed (npm): error: npm package 'skit-no-such-pkg-e2e-xyz' does not exist.";
    assert!(rendered.lines().any(|line| line == expected), "{rendered}");
    assert!(!rendered.contains('\u{1b}'), "ANSI escapes leaked:\n{rendered}");
}

#[test]
fn test_failure_detail_survives_invalid_utf8_bytes() {
    let sandbox = Sandbox::new();
    sandbox.install(
        "npm",
        "#!/bin/sh\nprintf 'npm error caf\\351 install failed\\n' >&2\nexit 1\n",
    );
    let output = sandbox.run();
    let rendered = text(&output);
    assert_ne!(output.status.code(), Some(0), "{rendered}");
    assert!(rendered.contains("install failed"), "{rendered}");
    assert!(rendered.contains('�'), "invalid installer bytes were not lossily replaced:\n{rendered}");
}

#[test]
fn test_install_announce_line_verbatim() {
    let sandbox = Sandbox::new();
    sandbox.install(
        "npm",
        "#!/bin/sh\n/bin/mkdir -p node_modules\nexit 0\n",
    );
    let output = sandbox.run();
    let rendered = text(&output);
    assert_eq!(output.status.code(), Some(0), "{rendered}");
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .lines()
            .any(|line| line == "Installing dependencies (npm)…"),
        "{rendered}"
    );
}

#[test]
fn test_install_announces_itself_but_a_fresh_marker_stays_silent() {
    let sandbox = Sandbox::new();
    sandbox.install(
        "npm",
        "#!/bin/sh\n/bin/mkdir -p node_modules\nexit 0\n",
    );
    let first = sandbox.run();
    let first_err = String::from_utf8_lossy(&first.stderr);
    assert_eq!(first.status.code(), Some(0), "{}", text(&first));
    assert!(first_err.lines().any(|line| line == "Installing dependencies (npm)…"));
    assert!(first.stdout.is_empty(), "installer status polluted script stdout");

    let second = sandbox.run();
    let second_err = String::from_utf8_lossy(&second.stderr);
    assert_eq!(second.status.code(), Some(0), "{}", text(&second));
    assert!(
        !second_err.lines().any(|line| line == "Installing dependencies (npm)…"),
        "fresh dependency state announced a nonexistent reinstall:\n{}",
        text(&second)
    );
}

#[test]
fn test_install_subprocess_contract_and_marker_dir_reuse() {
    let sandbox = Sandbox::new();
    sandbox.install(
        "npm",
        concat!(
            "#!/bin/sh\n",
            "printf '%s\\n' 'INSTALLER-STDOUT-MUST-BE-CAPTURED'\n",
            "printf '%s\\n' 'INSTALLER-STDERR-MUST-BE-CAPTURED' >&2\n",
            "/bin/mkdir -p node_modules/chalk\n",
            "exit 0\n",
        ),
    );
    let output = sandbox.run();
    let rendered = text(&output);
    assert_eq!(output.status.code(), Some(0), "{rendered}");
    assert!(!rendered.contains("INSTALLER-STDOUT-MUST-BE-CAPTURED"), "{rendered}");
    assert!(!rendered.contains("INSTALLER-STDERR-MUST-BE-CAPTURED"), "{rendered}");
    let entry_dir = sandbox.data.path().join("scripts").join("t");
    assert!(
        entry_dir.join("node_modules/.skit-deps-ok").is_file(),
        "freshness marker must reuse the installer-created node_modules directory"
    );
}