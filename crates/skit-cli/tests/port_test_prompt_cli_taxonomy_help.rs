use std::{fs, path::PathBuf, process::Output};

use assert_cmd::Command;
use tempfile::TempDir;

struct Sandbox {
    data: TempDir,
    state: TempDir,
    config: TempDir,
    home: TempDir,
    tools: TempDir,
}

impl Sandbox {
    fn new() -> Self {
        Self {
            data: TempDir::new().unwrap(),
            state: TempDir::new().unwrap(),
            config: TempDir::new().unwrap(),
            home: TempDir::new().unwrap(),
            tools: TempDir::new().unwrap(),
        }
    }

    fn command(&self, lang: &str) -> Command {
        let mut command = assert_cmd::cargo::cargo_bin_cmd!("skit");
        command
            .env("SKIT_DATA_DIR", self.data.path())
            .env("SKIT_STATE_DIR", self.state.path())
            .env("SKIT_CONFIG_DIR", self.config.path())
            .env("SKIT_LANG", lang)
            .env("HOME", self.home.path())
            .env("USERPROFILE", self.home.path())
            .env("XDG_CONFIG_HOME", self.home.path().join("xdg-config"))
            .env("XDG_DATA_HOME", self.home.path().join("xdg-data"))
            .env("XDG_STATE_HOME", self.home.path().join("xdg-state"))
            .current_dir(self.home.path());
        command
    }

    fn run(&self, lang: &str, args: &[&str]) -> Output {
        self.command(lang).args(args).output().unwrap()
    }

    fn source(&self, name: &str, body: &str) -> PathBuf {
        let path = self.home.path().join(name);
        fs::write(&path, body).unwrap();
        path
    }

    fn add_prompt(&self) {
        let source = self.source("p.prompt.md", "Review this\n");
        let output = self.run("en", &["add", source.to_str().unwrap(), "--no-input"]);
        assert_eq!(output.status.code(), Some(0), "{}", combined(&output));
    }

    fn fake_uv_path(&self) -> PathBuf {
        let path = self.tools.path().join(if cfg!(windows) { "uv.exe" } else { "uv" });
        fs::write(&path, b"").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
        }
        path
    }
}

fn combined(output: &Output) -> String {
    format!("{}{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr))
}

fn flat(output: &Output) -> String {
    combined(output).split_whitespace().collect::<Vec<_>>().join(" ")
}

#[test]
fn test_umbrella_cli_help_uses_entry_taxonomy_in_the_requested_locale() {
    for (locale, expected) in [
        (
            "en",
            [
                (vec!["--help"], "scripts, prompts, programs, and commands"),
                (vec!["list", "--help"], "registered entry"),
                (vec!["show", "--help"], "one entry"),
                (vec!["remove", "--help"], "registered entry"),
                (vec!["rename", "--help"], "Rename an entry"),
                (vec!["describe", "--help"], "entry's description"),
                (vec!["params", "--help"], "an entry's managed or declared parameters"),
                (vec!["deps", "--help"], "an entry's package dependencies"),
                (vec!["doctor", "--help"], "entry library"),
            ],
        ),
        (
            "zh-TW",
            [
                (vec!["--help"], "腳本、提示詞、程式和命令"),
                (vec!["list", "--help"], "已登記的條目"),
                (vec!["show", "--help"], "一個條目"),
                (vec!["remove", "--help"], "已登記的條目"),
                (vec!["rename", "--help"], "重新命名條目"),
                (vec!["describe", "--help"], "條目的說明"),
                (vec!["params", "--help"], "條目的管理參數或宣告參數"),
                (vec!["deps", "--help"], "條目的套件依賴"),
                (vec!["doctor", "--help"], "工具庫"),
            ],
        ),
    ] {
        for (args, phrase) in expected {
            let output = Sandbox::new().run(locale, &args);
            assert_eq!(output.status.code(), Some(0), "locale={locale} args={args:?}\n{}", combined(&output));
            assert!(flat(&output).contains(phrase), "locale={locale} args={args:?} phrase={phrase:?}\n{}", flat(&output));
        }
    }
}

#[test]
fn test_prompt_only_library_uses_entry_taxonomy_on_dynamic_cli_surfaces() {
    let sandbox = Sandbox::new();
    sandbox.add_prompt();
    let _uv = sandbox.fake_uv_path();
    let output = sandbox
        .command("en")
        .env("PATH", sandbox.tools.path())
        .arg("doctor")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0), "{}", combined(&output));
    let shown = flat(&output);
    assert!(shown.contains("1 entry registered"), "{shown}");
    assert!(!shown.contains("script registered"), "{shown}");
}

#[test]
fn test_empty_library_does_not_claim_it_only_accepts_scripts() {
    let output = Sandbox::new().run("en", &["list"]);
    assert_eq!(output.status.code(), Some(0), "{}", combined(&output));
    let shown = combined(&output);
    assert!(shown.contains("No entries yet"), "{shown}");
    assert!(!shown.contains("No scripts yet"), "{shown}");
}
