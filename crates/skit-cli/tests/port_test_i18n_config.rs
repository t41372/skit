//! Environment-detection and language-config ports for Python `tests/test_i18n.py` at
//! `main@206f9ef`. Real CLI help/config invocations exercise the composition-root precedence and
//! recovery messages; direct `FileConfigStore` calls pin persistence semantics without UI noise.

use std::{
    fs,
    path::PathBuf,
    process::{Command, Output},
};

use skit_i18n::{Locale, negotiated_system_locale, requested_locale, system_locale, text};
use skit_store::FileConfigStore;
use tempfile::TempDir;

struct Sandbox {
    _root: TempDir,
    data: PathBuf,
    state: PathBuf,
    config: PathBuf,
    home: PathBuf,
}

impl Sandbox {
    fn new() -> Self {
        let root = TempDir::new().unwrap();
        let data = root.path().join("data");
        let state = root.path().join("state");
        let config = root.path().join("config");
        let home = root.path().join("home");
        for path in [&data, &state, &config, &home] {
            fs::create_dir_all(path).unwrap();
        }
        Self {
            _root: root,
            data,
            state,
            config,
            home,
        }
    }

    fn command(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_skit"));
        command
            .env("SKIT_DATA_DIR", &self.data)
            .env("SKIT_STATE_DIR", &self.state)
            .env("SKIT_CONFIG_DIR", &self.config)
            .env("HOME", &self.home)
            .env("USERPROFILE", &self.home)
            .env("XDG_CONFIG_HOME", self.home.join("xdg-config"))
            .env("XDG_DATA_HOME", self.home.join("xdg-data"))
            .env("XDG_STATE_HOME", self.home.join("xdg-state"))
            .current_dir(&self.home);
        command
    }

    fn config(&self) -> FileConfigStore {
        FileConfigStore::new(&self.config)
    }
}

fn combined(output: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

#[test]
fn test_env_override_wins() {
    let sandbox = Sandbox::new();
    let output = sandbox
        .command()
        .env("SKIT_LANG", "zh-TW")
        .env("LANG", "en_US.UTF-8")
        .arg("--help")
        .output()
        .unwrap();

    assert!(output.status.success(), "{}", combined(&output));
    let stdout = String::from_utf8(output.stdout).unwrap();
    let expected = text(
        Locale::ZhTw,
        "skit — a launcher and parameter manager for scripts, prompts, programs, and commands. Run it without a subcommand to open the main menu",
    );
    assert!(stdout.contains(expected.as_ref()), "{stdout}");
    assert!(!stdout.contains("scripts, prompts, programs, and commands"));
}

#[test]
fn test_lang_env() {
    let sandbox = Sandbox::new();
    let output = sandbox
        .command()
        .env_remove("SKIT_LANG")
        .env_remove("LC_ALL")
        .env_remove("LC_MESSAGES")
        .env("LANG", "zh_CN.UTF-8")
        .arg("--help")
        .output()
        .unwrap();

    assert!(output.status.success(), "{}", combined(&output));
    let stdout = String::from_utf8(output.stdout).unwrap();
    let expected = text(
        Locale::ZhCn,
        "skit — a launcher and parameter manager for scripts, prompts, programs, and commands. Run it without a subcommand to open the main menu",
    );
    assert!(stdout.contains(expected.as_ref()), "{stdout}");
}

#[test]
fn test_c_locale_ignored() {
    assert_eq!(requested_locale(Some("C")), None);
    let _ = system_locale();

    // The host answers with its own spelling, and the rule that reads the answer is the same
    // everywhere. A Windows desktop names BCP-47 tags, a unix one names locale values, and either
    // way the first supported preference wins. Version 0.4 asks Windows a different question and
    // gets a name no tag matches, which is why a Chinese desktop reads as Chinese here and as
    // English there (recorded in docs/design/rust-contract-matrix.md).
    assert_eq!(negotiated_system_locale(["zh-TW".to_owned()]), Locale::ZhTw);
    assert_eq!(
        negotiated_system_locale(["zh-Hans-CN".to_owned()]),
        Locale::ZhCn
    );
    assert_eq!(negotiated_system_locale(["en-US".to_owned()]), Locale::En);
    assert_eq!(
        negotiated_system_locale(["zh_TW.UTF-8".to_owned()]),
        Locale::ZhTw
    );
    // A preference the catalog cannot serve steps aside for the next one.
    assert_eq!(
        negotiated_system_locale(["C".to_owned(), "zh-CN".to_owned()]),
        Locale::ZhCn
    );
    // A host that names nothing, or nothing supported, reads as English.
    assert_eq!(negotiated_system_locale(Vec::<String>::new()), Locale::En);
    assert_eq!(negotiated_system_locale(["C".to_owned()]), Locale::En);

    let sandbox = Sandbox::new();
    let output = sandbox
        .command()
        .env_remove("SKIT_LANG")
        .env_remove("LC_ALL")
        .env_remove("LC_MESSAGES")
        .env_remove("LANG")
        .env_remove("LANGUAGE")
        .env("NO_COLOR", "1")
        .arg("--help")
        .output()
        .unwrap();

    assert!(output.status.success(), "{}", combined(&output));
    let stdout = String::from_utf8(output.stdout).unwrap();
    let source = "skit — a launcher and parameter manager for scripts, prompts, programs, and commands. Run it without a subcommand to open the main menu";
    assert!(
        [Locale::En, Locale::ZhCn, Locale::ZhTw]
            .into_iter()
            .map(|locale| text(locale, source))
            .any(|description| stdout.contains(description.as_ref())),
        "{stdout}"
    );
    assert_eq!(fs::read_dir(&sandbox.data).unwrap().count(), 0);
    assert_eq!(fs::read_dir(&sandbox.state).unwrap().count(), 0);
    assert_eq!(fs::read_dir(&sandbox.config).unwrap().count(), 0);
}

#[test]
fn test_set_language_persists() {
    let sandbox = Sandbox::new();
    let config = sandbox.config();

    config.set("lang", "zh-TW").unwrap();
    assert!(sandbox.config.join("config.toml").is_file());
    assert_eq!(config.get("lang").unwrap(), "zh-TW");

    // Python v0.4 accepts the empty spelling as "automatic" and removes the stored preference.
    // Keep that exact input contract; a Rust refusal is a deliberate parity finding.
    config.set("lang", "").unwrap();
    assert_eq!(config.get("lang").unwrap(), "");
    let raw = fs::read_to_string(sandbox.config.join("config.toml")).unwrap();
    assert!(
        !raw.contains("language"),
        "cleared language remained persisted: {raw}"
    );
}

#[test]
fn test_backs_up_corrupt_config_instead_of_wiping_it() {
    let sandbox = Sandbox::new();
    let corrupt = concat!(
        "language = \"zh-CN\"\n",
        "[mirror]\n",
        "enabled = true\n",
        "pypi = \"https://tsinghua\"\n",
        "this is = = not valid toml",
    );
    let config_path = sandbox.config.join("config.toml");
    fs::write(&config_path, corrupt).unwrap();

    let output = sandbox
        .command()
        .env("SKIT_LANG", "en")
        .args(["config", "lang", "en"])
        .output()
        .unwrap();

    assert!(output.status.success(), "{}", combined(&output));
    assert_eq!(sandbox.config().get("lang").unwrap(), "en");
    let backup = sandbox.config.join("config.toml.bak");
    assert_eq!(fs::read_to_string(&backup).unwrap(), corrupt);
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("config.toml"), "{stderr}");
    assert!(stderr.contains("config.toml.bak"), "{stderr}");
}

#[test]
fn test_warns_when_corrupt_config_cannot_even_be_backed_up() {
    let sandbox = Sandbox::new();
    let config_path = sandbox.config.join("config.toml");
    fs::write(&config_path, "this is = = not valid toml").unwrap();
    let backup = sandbox.config.join("config.toml.bak");
    fs::create_dir(&backup).unwrap();
    let blocker = backup.join("config.toml");
    fs::create_dir(&blocker).unwrap();
    fs::write(blocker.join("owned"), "keep directory non-empty").unwrap();

    let output = sandbox
        .command()
        .env("SKIT_LANG", "en")
        .args(["config", "lang", "en"])
        .output()
        .unwrap();

    // Python v0.4 still applies the requested language when the safety copy itself fails, while
    // warning loudly. A Rust transaction that aborts here is a real parity difference, not a test
    // fixture error.
    assert!(output.status.success(), "{}", combined(&output));
    assert_eq!(sandbox.config().get("lang").unwrap(), "en");
    assert!(
        backup.is_dir(),
        "the failed backup path was unexpectedly replaced"
    );
    assert_eq!(
        fs::read_to_string(blocker.join("owned")).unwrap(),
        "keep directory non-empty"
    );
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("config.toml"), "{stderr}");
}

#[test]
fn test_valid_config_is_unaffected() {
    let sandbox = Sandbox::new();
    let config_path = sandbox.config.join("config.toml");
    fs::write(
        &config_path,
        concat!(
            "language = \"zh-CN\"\n",
            "[mirror]\n",
            "enabled = true\n",
            "pypi = \"https://tsinghua\"\n",
        ),
    )
    .unwrap();

    sandbox.config().set("lang", "en").unwrap();

    assert_eq!(sandbox.config().get("lang").unwrap(), "en");
    let raw = fs::read_to_string(&config_path).unwrap();
    let document: toml::Value = toml::from_str(&raw).unwrap();
    let mirror = document
        .get("mirror")
        .and_then(toml::Value::as_table)
        .expect("valid mirror section survived");
    assert_eq!(
        mirror.get("enabled").and_then(toml::Value::as_bool),
        Some(true)
    );
    assert_eq!(
        mirror.get("pypi").and_then(toml::Value::as_str),
        Some("https://tsinghua")
    );
    assert!(!sandbox.config.join("config.toml.bak").exists());
}
