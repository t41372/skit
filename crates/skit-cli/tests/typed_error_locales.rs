//! Typed errors must reach the user in every supported locale.
//!
//! The template is catalog text. The values stay exactly as the user wrote them.

use predicates::prelude::*;
use tempfile::TempDir;

struct Sandbox {
    data: TempDir,
    state: TempDir,
    config: TempDir,
}

impl Sandbox {
    fn new() -> Self {
        Self {
            data: TempDir::new().unwrap(),
            state: TempDir::new().unwrap(),
            config: TempDir::new().unwrap(),
        }
    }

    fn command(&self, locale: &str) -> assert_cmd::Command {
        let mut command = assert_cmd::cargo::cargo_bin_cmd!("skit");
        command
            .env("SKIT_DATA_DIR", self.data.path())
            .env("SKIT_STATE_DIR", self.state.path())
            .env("SKIT_CONFIG_DIR", self.config.path())
            .env("SKIT_LANG", locale);
        command
    }

    fn add_python_entry(&self, name: &str) {
        let source = self.data.path().join(format!("{name}.py"));
        std::fs::write(&source, "#!/usr/bin/env python3\nprint(1)\n").unwrap();
        self.command("en")
            .args(["add"])
            .arg(&source)
            .args(["--name", name])
            .assert()
            .success();
    }
}

#[test]
fn package_requirement_errors_localize_the_template_and_keep_the_value_verbatim() {
    let sandbox = Sandbox::new();
    let source = sandbox.data.path().join("sample.py");
    std::fs::write(&source, "#!/usr/bin/env python3\nprint(1)\n").unwrap();
    sandbox
        .command("en")
        .args(["add"])
        .arg(&source)
        .args(["--name", "Sample"])
        .assert()
        .success();

    // `not` is catalog text. Substring translation would rewrite it inside the value.
    let value = "!!!not valid!!!";
    for (locale, template) in [
        ("zh-CN", "不是有效的 PEP 508 依赖描述"),
        ("zh-TW", "不是有效的 PEP 508 相依描述"),
    ] {
        sandbox
            .command(locale)
            .args(["deps", "sample", "--dep", value])
            .assert()
            .failure()
            .stderr(predicate::str::contains(template))
            .stderr(predicate::str::contains(value));
    }

    sandbox
        .command("en")
        .args(["deps", "sample", "--dep", value])
        .assert()
        .failure()
        .stderr(predicate::str::contains("invalid PEP 508 requirement"))
        .stderr(predicate::str::contains(value));
}

#[test]
fn version_constraint_errors_localize_the_template_and_keep_the_value_verbatim() {
    let sandbox = Sandbox::new();
    let source = sandbox.data.path().join("pinned.py");
    std::fs::write(&source, "#!/usr/bin/env python3\nprint(1)\n").unwrap();
    sandbox
        .command("en")
        .args(["add"])
        .arg(&source)
        .args(["--name", "Pinned"])
        .assert()
        .success();

    let value = "not a constraint";
    sandbox
        .command("zh-CN")
        .args(["deps", "pinned", "--python", value])
        .assert()
        .failure()
        .stderr(predicate::str::contains("不是有效的 PEP 440 版本约束"))
        .stderr(predicate::str::contains(value));
}

#[test]
fn usage_errors_localize_their_template_in_every_supported_locale() {
    let sandbox = Sandbox::new();
    sandbox.add_python_entry("Command");

    for (locale, template) in [
        ("en", "use --dep or --clear, not both"),
        ("zh-CN", "请使用 --dep 或 --clear，不能同时使用"),
        ("zh-TW", "請使用 --dep 或 --clear，不能同時使用"),
    ] {
        sandbox
            .command(locale)
            .args(["deps", "command", "--dep", "rich", "--clear"])
            .assert()
            .code(2)
            .stderr(predicate::str::contains(template));
    }
}

#[test]
fn repository_errors_localize_their_template_and_keep_the_query_verbatim() {
    let sandbox = Sandbox::new();
    // `list` is catalog text, so substring translation would rewrite this query.
    let query = "list-me";

    for (locale, template) in [
        ("zh-CN", "找不到条目"),
        ("zh-TW", "找不到項目"),
        ("en", "entry not found"),
    ] {
        sandbox
            .command(locale)
            .args(["show", query])
            .assert()
            .code(127)
            .stderr(predicate::str::contains(template))
            .stderr(predicate::str::contains(query));
    }
}

#[test]
fn launch_errors_localize_their_template_in_every_supported_locale() {
    let sandbox = Sandbox::new();
    let needed = "skit-no-such-program";
    sandbox
        .command("en")
        .args(["add", "--cmd", "printf ok", "--name", "Missing"])
        .assert()
        .success();
    sandbox
        .command("en")
        .args(["deps", "missing", "--need", needed])
        .assert()
        .success();

    for (locale, template) in [
        ("zh-CN", "找不到所需命令"),
        ("zh-TW", "找不到必要命令"),
        ("en", "required command was not found"),
    ] {
        sandbox
            .command(locale)
            .args(["run", "missing", "--no-input"])
            .assert()
            .failure()
            .stderr(predicate::str::contains(template))
            .stderr(predicate::str::contains(needed));
    }
}
