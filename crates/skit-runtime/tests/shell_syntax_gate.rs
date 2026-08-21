use std::{
    cell::RefCell,
    path::{Path, PathBuf},
};

use skit_i18n::{Locale, Localize as _};
use skit_runtime::{
    InjectedCommand, InjectedCommandOutput, InjectedCommandRunner, InjectedCommandUnavailable,
    ResolvedShellInterpreter, SHELL_SYNTAX_GATE_TIMEOUT, check_shell_syntax,
    retain_shell_source_if_valid, shell_self_location_warning,
};
use tempfile::NamedTempFile;

#[derive(Clone, Debug)]
enum Outcome {
    Output(InjectedCommandOutput),
    Unavailable(InjectedCommandUnavailable),
}

#[derive(Debug)]
struct Runner {
    outcome: Outcome,
    commands: RefCell<Vec<InjectedCommand>>,
}

impl Runner {
    fn output(success: bool, stderr: &[u8]) -> Self {
        Self {
            outcome: Outcome::Output(InjectedCommandOutput {
                success,
                stderr: stderr.to_vec(),
            }),
            commands: RefCell::new(Vec::new()),
        }
    }

    fn unavailable(error: InjectedCommandUnavailable) -> Self {
        Self {
            outcome: Outcome::Unavailable(error),
            commands: RefCell::new(Vec::new()),
        }
    }
}

impl InjectedCommandRunner for Runner {
    fn run(
        &self,
        command: &InjectedCommand,
    ) -> Result<InjectedCommandOutput, InjectedCommandUnavailable> {
        self.commands.borrow_mut().push(command.clone());
        match &self.outcome {
            Outcome::Output(output) => Ok(output.clone()),
            Outcome::Unavailable(error) => Err(error.clone()),
        }
    }
}

fn interpreter() -> ResolvedShellInterpreter {
    ResolvedShellInterpreter::new("bash", PathBuf::from("/resolved/bin/bash"))
}

#[test]
fn test_interpreter_gate_refuses_what_the_offline_gate_missed() {
    let staged = NamedTempFile::new().unwrap();
    let path = staged.path().to_path_buf();
    let runner = Runner::output(false, b"bash: staged source rejected\nsecond line\n");

    let error = retain_shell_source_if_valid(staged, Some(&interpreter()), &path, &runner)
        .unwrap_err();

    assert!(!path.exists(), "a rejected private source must be removed");
    assert_eq!(error.shell(), "bash");
    assert_eq!(error.detail(), "bash: staged source rejected");
    assert_eq!(
        error.message().localize(Locale::En),
        "bash rejected the injected copy: bash: staged source rejected"
    );
    let commands = runner.commands.borrow();
    assert_eq!(commands.len(), 1);
    assert_eq!(commands[0].program, Path::new("/resolved/bin/bash"));
    assert_eq!(
        commands[0].args,
        ["-n".into(), path.as_os_str().to_owned()]
    );
    assert_eq!(commands[0].timeout, SHELL_SYNTAX_GATE_TIMEOUT);
}

#[test]
fn test_interpreter_gate_is_skipped_when_the_shell_is_not_installed() {
    let runner = Runner::output(false, b"must not run");
    check_shell_syntax(None, Path::new("/tmp/injected.sh"), &runner).unwrap();
    assert!(runner.commands.borrow().is_empty());
}

#[test]
fn test_interpreter_gate_survives_a_spawn_failure() {
    let runner = Runner::unavailable(InjectedCommandUnavailable::Spawn {
        reason: "no fork for you".to_owned(),
    });
    check_shell_syntax(
        Some(&interpreter()),
        Path::new("/tmp/injected.sh"),
        &runner,
    )
    .unwrap();
    assert_eq!(runner.commands.borrow().len(), 1);
}

#[test]
fn test_interpreter_gate_reports_an_empty_stderr_without_crashing() {
    let runner = Runner::output(false, b"");
    let error = check_shell_syntax(
        Some(&interpreter()),
        Path::new("/tmp/injected.sh"),
        &runner,
    )
    .unwrap_err();
    assert_eq!(error.detail(), "");
    assert_eq!(
        error.message().localize(Locale::ZhCn),
        "bash 拒绝了注入后的副本:"
    );
    assert_eq!(
        error.message().localize(Locale::ZhTw),
        "bash 拒絕了注入後的副本:"
    );
}

#[test]
fn shell_gate_success_and_timeout_are_best_effort() {
    check_shell_syntax(
        Some(&interpreter()),
        Path::new("/tmp/injected.sh"),
        &Runner::output(true, b"ignored noise"),
    )
    .unwrap();
    check_shell_syntax(
        Some(&interpreter()),
        Path::new("/tmp/injected.sh"),
        &Runner::unavailable(InjectedCommandUnavailable::Timeout),
    )
    .unwrap();
}

#[test]
fn test_self_location_warns_when_a_temp_copy_is_written() {
    let warning = shell_self_location_warning(true).expect("a staged self-locating shell warns");
    assert_eq!(
        warning.localize(Locale::En),
        "⚠ This script reads its own location ($0 / $BASH_SOURCE), and the injected values run from a temporary copy — so it sees the copy's path, not the original's. Rewriting a constant as NAME=\"${NAME:-value}\" delivers the value through the environment instead, with no copy at all (`skit params <script> --normalize NAME` does the rewrite for you on a stored copy)."
    );
    assert_eq!(
        warning.localize(Locale::ZhCn),
        "⚠ 这个脚本会读自己的位置($0 / $BASH_SOURCE),而注入后的值是从临时副本运行的——所以它看到的是副本的路径,不是原文件的。把常量改写成 NAME=\"${NAME:-value}\" 就能改用环境变量传值,完全不写副本(`skit params <script> --normalize NAME` 会在已保存的副本上帮你完成这个改写)。"
    );
    assert_eq!(
        warning.localize(Locale::ZhTw),
        "⚠ 這個腳本會讀自己的位置($0 / $BASH_SOURCE),而注入後的值是從臨時副本執行的——所以它看到的是副本的路徑,不是原檔案的。把常數改寫成 NAME=\"${NAME:-value}\" 就能改用環境變數傳值,完全不寫副本(`skit params <script> --normalize NAME` 會在儲存的副本上幫你做這個改寫)。"
    );
    assert!(shell_self_location_warning(false).is_none());
}
