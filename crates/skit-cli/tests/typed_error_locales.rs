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
    for (locale, template) in [("zh-CN", "不是软件包依赖"), ("zh-TW", "不是套件需求")]
    {
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
        .stderr(predicate::str::contains("isn't a package requirement"))
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
        .stderr(predicate::str::contains("不是 Python 版本约束"))
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
            // `show` is a management command, so a missing name exits 1
            // (src/skit/cli.py:2483). 127 belongs to the run path (cli.py:3008).
            .assert()
            .code(1)
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
        ("zh-TW", "找不到所需命令"),
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

const REFUSE_EMPTY: [&str; 3] = [
    "A runner needs a command — e.g. skit runner add mycli mycli run {{prompt}}",
    "执行器需要一条命令——例如 skit runner add mycli mycli run {{prompt}}",
    "執行器需要一條命令——例如 skit runner add mycli mycli run {{prompt}}",
];
const REFUSE_SLOT: [&str; 3] = [
    "A runner command must contain the {{prompt}} slot exactly once — that's where the rendered prompt lands.",
    "执行器命令必须恰好包含一个 {{prompt}} 槽位——渲染后的提示词就放在那里。",
    "執行器命令必須恰好包含一個 {{prompt}} 槽位——渲染後的提示詞就放在那裡。",
];
const REFUSE_BIN: [&str; 3] = [
    "{{prompt}} can't be the command itself — the first word must be the program to run.",
    "{{prompt}} 不能是命令本身——第一个词必须是要运行的程序。",
    "{{prompt}} 不能是命令本身——第一個詞必須是要執行的程式。",
];
const REFUSE_STRAY: [&str; 3] = [
    "Runner commands take only the {{prompt}} slot — single-brace text is literal, and other {{holes}} aren't supported.",
    "执行器命令只接受 {{prompt}} 槽位——单大括号是字面文字,其他 {{占位符}} 不支持。",
    "執行器命令只接受 {{prompt}} 槽位——單大括號是字面文字,其他 {{佔位符}} 不支援。",
];
const STATUS_EMPTY: [&str; 3] = [
    "Type the agent's command, e.g. mycli run {{prompt}}",
    "请输入 agent 的命令,例如 mycli run {{prompt}}",
    "請輸入 agent 的命令,例如 mycli run {{prompt}}",
];
const STATUS_SLOT: [&str; 3] = [
    "The command needs the {{prompt}} slot exactly once — that's where the rendered prompt lands.",
    "命令必须恰好包含一个 {{prompt}} 槽位——渲染后的提示词会放在那里。",
    "命令必須恰好包含一個 {{prompt}} 槽位——渲染後的提示詞會放在那裡。",
];

/// `skit runner add` refuses with its own sentences, never with the status of a stored row.
///
/// Version 0.4 keeps two closed sets: `_runner_reason` for a `runner add` refusal
/// (`src/skit/cli.py:3358-3378`) and `prompt_runner_row_reason` for the status of a row that is
/// already in the file (`src/skit/config.py:592-624`). A refusal tells a person what to type on
/// the command line, so it names `skit runner add`; a row status describes the row.
#[test]
fn runner_add_refuses_with_its_own_sentences_in_every_locale() {
    let sandbox = Sandbox::new();
    let cases: [(&[&str], [&str; 3], [&str; 3]); 4] = [
        (&["bare"], REFUSE_EMPTY, STATUS_EMPTY),
        (&["noslot", "agent"], REFUSE_SLOT, STATUS_SLOT),
        (&["bin", "{{prompt}}"], REFUSE_BIN, REFUSE_BIN),
        (&["stray", "agent", "{{other}}"], REFUSE_STRAY, REFUSE_STRAY),
    ];
    for (arguments, refusals, statuses) in cases {
        for ((locale, refusal), status) in ["en", "zh-CN", "zh-TW"]
            .into_iter()
            .zip(refusals)
            .zip(statuses)
        {
            let output = sandbox
                .command(locale)
                .args(["runner", "add"])
                .args(arguments)
                .output()
                .unwrap();
            let stderr = String::from_utf8(output.stderr).unwrap();
            assert_eq!(
                output.status.code(),
                Some(2),
                "{arguments:?} {locale}: {stderr}"
            );
            assert!(stderr.contains(refusal), "{arguments:?} {locale}: {stderr}");
            if refusal != status {
                assert!(!stderr.contains(status), "{arguments:?} {locale}: {stderr}");
            }
        }
    }
    let config =
        std::fs::read_to_string(sandbox.config.path().join("config.toml")).unwrap_or_default();
    assert!(
        !config.contains("runners"),
        "a refusal wrote the configuration: {config}"
    );
}
