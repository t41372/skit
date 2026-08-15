use std::{fs, path::Path};

use skit_application::{
    EntryMutationRepository as _, SourcePermissions,
    prompt_selection::PromptSelectionService,
};
use skit_store::{FileConfigStore, FilePromptSelectionStore, FileStore, PromptRunner};
use skit_ui::{KnownEntryKind, ReviewDefaults, ReviewState, SourceSnapshot};
use tempfile::TempDir;

#[path = "support/prompt_tui_pty.rs"]
mod prompt_tui_pty;
use prompt_tui_pty::TuiPty;

struct Sandbox {
    data: TempDir,
    state: TempDir,
    config: TempDir,
    home: TempDir,
}

impl Sandbox {
    fn new() -> Self {
        Self {
            data: TempDir::new().unwrap(),
            state: TempDir::new().unwrap(),
            config: TempDir::new().unwrap(),
            home: TempDir::new().unwrap(),
        }
    }

    fn config(&self) -> FileConfigStore {
        FileConfigStore::new(self.config.path())
    }

    fn create_prompt(&self, name: &str, body: &str, pin: &str) {
        let source = self.home.path().join(format!("{name}.prompt.md"));
        fs::write(&source, body).unwrap();
        let review = ReviewState::from_source(
            SourceSnapshot {
                path: source.clone(),
                source_record: source.display().to_string(),
                bytes: body.as_bytes().to_vec(),
                permissions: SourcePermissions::default(),
                is_regular: true,
                is_directory: false,
                is_draft: false,
            },
            KnownEntryKind::Prompt,
            ReviewDefaults {
                name: Some(name.to_owned()),
                ..ReviewDefaults::default()
            },
        );
        let mut request = review.create_entry().unwrap();
        request.settings.runner = pin.to_owned();
        FileStore::new(self.data.path()).create(request).unwrap();
    }

    fn clear_runners(&self) {
        let config = self.config();
        config.ensure_runners_seeded().unwrap();
        let names = config
            .runners()
            .unwrap()
            .into_iter()
            .map(|runner| runner.name)
            .collect::<Vec<_>>();
        for name in names {
            assert!(config.remove_runner(&name).unwrap(), "seed {name} could not be removed");
        }
        assert!(config.runners().unwrap().is_empty());
    }

    fn add_echo_runner(&self, name: &str, marker: &str) {
        self.config()
            .set_runner(
                PromptRunner {
                    name: name.to_owned(),
                    argv: echo_argv(marker),
                },
                false,
            )
            .unwrap();
    }

    fn remember_runner(&self, name: &str) {
        PromptSelectionService::new(FilePromptSelectionStore::new(self.state.path()))
            .remember_runner(name)
            .unwrap();
    }

    fn last_runner(&self) -> String {
        PromptSelectionService::new(FilePromptSelectionStore::new(self.state.path())).last_runner()
    }

    fn tui(&self) -> TuiPty {
        TuiPty::spawn(
            self.data.path(),
            self.state.path(),
            self.config.path(),
            self.home.path(),
        )
    }
}

#[cfg(windows)]
fn echo_argv(marker: &str) -> Vec<String> {
    vec![
        "cmd.exe".to_owned(),
        "/C".to_owned(),
        "echo".to_owned(),
        marker.to_owned(),
        "{{prompt}}".to_owned(),
    ]
}

#[cfg(not(windows))]
fn echo_argv(marker: &str) -> Vec<String> {
    vec![
        "/bin/echo".to_owned(),
        marker.to_owned(),
        "{{prompt}}".to_owned(),
    ]
}

#[cfg(windows)]
fn echo_command(marker: &str) -> String {
    format!("cmd.exe /C echo {marker} {{{{prompt}}}}")
}

#[cfg(not(windows))]
fn echo_command(marker: &str) -> String {
    format!("/bin/echo {marker} {{{{prompt}}}}")
}

fn open_selected(tui: &mut TuiPty) -> usize {
    tui.wait_for("Library");
    let checkpoint = tui.checkpoint();
    tui.send(b"\r");
    checkpoint
}

fn focus_runner(tui: &mut TuiPty) {
    // RunFormView boots on the first typeable parameter. Shift+Tab reaches the runner picker above.
    tui.send(b"\x1b[Z");
}

fn submit_one_value(tui: &mut TuiPty, value: &str) -> usize {
    tui.send(value.as_bytes());
    let checkpoint = tui.checkpoint();
    tui.send(&[0x12]); // Ctrl+R, the advertised Run chord.
    checkpoint
}

fn assert_runner(config: &FileConfigStore, name: &str, expected: &[String]) {
    let runner = config
        .runners()
        .unwrap()
        .into_iter()
        .find(|runner| runner.name == name)
        .unwrap_or_else(|| panic!("runner {name:?} was not persisted"));
    assert_eq!(runner.argv, expected);
}

#[test]
fn test_run_with_zero_runners_offers_the_new_agent_modal() {
    let sandbox = Sandbox::new();
    sandbox.create_prompt("p", "Do {{a}}\n", "");
    sandbox.clear_runners();
    let mut tui = sandbox.tui();

    let checkpoint = open_selected(&mut tui);
    tui.wait_for_after(checkpoint, "New agent (runner)");
    let checkpoint = tui.checkpoint();
    tui.send(b"\x1b");
    let visible = tui.wait_for_after(checkpoint, "needs a configured agent");
    assert!(visible.contains("Library"), "cancelling the zero-runner modal did not return to the Library: {visible}");
    assert!(sandbox.config().runners().unwrap().is_empty(), "cancelling the New agent modal wrote a runner");
}

#[test]
fn test_run_with_zero_runners_define_agent_then_run() {
    const MARKER: &str = "PTY-RUNNER-LAUNCHED";
    let sandbox = Sandbox::new();
    sandbox.create_prompt("p", "Do {{a}}\n", "");
    sandbox.clear_runners();
    let mut tui = sandbox.tui();

    let checkpoint = open_selected(&mut tui);
    tui.wait_for_after(checkpoint, "New agent (runner)");
    tui.send(b"mycli");
    tui.send(b"\t");
    tui.send(echo_command(MARKER).as_bytes());
    let checkpoint = tui.checkpoint();
    tui.send(b"\r");
    let visible = tui.wait_for_after(checkpoint, "Run p");
    assert!(visible.contains("mycli"), "the newly saved agent was not selected in the reopened run form: {visible}");
    assert_runner(&sandbox.config(), "mycli", &echo_argv(MARKER));

    let checkpoint = submit_one_value(&mut tui, "x");
    let child = tui.wait_for_after(checkpoint, MARKER);
    assert!(child.contains("prompt.md"), "the configured runner did not receive the prepared prompt path: {child}");
}

#[test]
fn test_form_picker_defaults_to_the_pin_and_submits_it() {
    const CODEX: &str = "PTY-CODEX-PIN";
    let sandbox = Sandbox::new();
    sandbox.clear_runners();
    sandbox.add_echo_runner("opencode", "PTY-OPENCODE-LAST");
    sandbox.add_echo_runner("codex", CODEX);
    sandbox.create_prompt("p", "Do {{a}}\n", "codex");
    sandbox.remember_runner("opencode");
    let mut tui = sandbox.tui();

    let checkpoint = open_selected(&mut tui);
    let visible = tui.wait_for_after(checkpoint, "Run p");
    assert!(visible.contains("codex"), "the stored pin was not the visible runner default: {visible}");
    let checkpoint = submit_one_value(&mut tui, "hello");
    tui.wait_for_after(checkpoint, CODEX);
    assert_eq!(sandbox.last_runner(), "opencode", "an untouched pin default was incorrectly remembered as an active runner pick");
}

#[test]
fn test_form_picker_keyboard_pick_runs_and_remembers() {
    const CODEX: &str = "PTY-CODEX-KEYBOARD";
    let sandbox = Sandbox::new();
    sandbox.clear_runners();
    sandbox.add_echo_runner("claude", "PTY-CLAUDE-FIRST");
    sandbox.add_echo_runner("opencode", "PTY-OPENCODE-LAST");
    sandbox.add_echo_runner("codex", CODEX);
    sandbox.create_prompt("p", "Do {{a}}\n", "");
    sandbox.remember_runner("opencode");
    let mut tui = sandbox.tui();

    let checkpoint = open_selected(&mut tui);
    let visible = tui.wait_for_after(checkpoint, "Run p");
    assert!(visible.contains("opencode"), "the run form ignored the last actively picked runner: {visible}");

    focus_runner(&mut tui);
    tui.send(b"\r");
    tui.send(b"\x1b[B");
    tui.send(b"\r");
    tui.send(b"\t");
    let checkpoint = submit_one_value(&mut tui, "x");
    tui.wait_for_after(checkpoint, CODEX);
    assert_eq!(sandbox.last_runner(), "codex", "keyboard runner pick was not persisted as the next run-form default");
}

#[test]
fn test_form_picker_move_away_then_back_to_pin_is_still_remembered() {
    const CODEX: &str = "PTY-CODEX-RETURNED";
    let sandbox = Sandbox::new();
    sandbox.clear_runners();
    sandbox.add_echo_runner("claude", "PTY-CLAUDE-AWAY");
    sandbox.add_echo_runner("codex", CODEX);
    sandbox.add_echo_runner("amp", "PTY-AMP-LAST");
    sandbox.create_prompt("p", "Do {{a}}\n", "codex");
    sandbox.remember_runner("amp");
    let mut tui = sandbox.tui();

    let checkpoint = open_selected(&mut tui);
    let visible = tui.wait_for_after(checkpoint, "Run p");
    assert!(visible.contains("codex"), "pin did not prefill the runner picker: {visible}");

    focus_runner(&mut tui);
    tui.send(b"\r");
    tui.send(b"\x1b[A");
    tui.send(b"\r");
    tui.send(b"\r");
    tui.send(b"\x1b[B");
    tui.send(b"\r");
    tui.send(b"\t");
    let checkpoint = submit_one_value(&mut tui, "x");
    tui.wait_for_after(checkpoint, CODEX);
    assert_eq!(sandbox.last_runner(), "codex", "moving away and explicitly returning to the pin was not remembered as a user pick");
}

#[test]
fn rust_additive_prompt_tui_pty_echo_contract_is_platform_specific_but_prompt_safe() {
    let argv = echo_argv("MARK");
    assert_eq!(argv.last().map(String::as_str), Some("{{prompt}}"));
    assert_eq!(argv.iter().filter(|arg| arg.contains("{{prompt}}")).count(), 1);
    assert!(echo_command("MARK").contains("{{prompt}}"));
    assert!(Path::new(&argv[0]).is_absolute() || cfg!(windows));
}
