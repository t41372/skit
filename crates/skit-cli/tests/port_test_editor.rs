//! Mechanical port of the Python oracle module `tests/test_editor.py`
//! (`origin/main@206f9ef`): the `editor` module (editor resolution + launch) plus the
//! `skit edit` / `skit add --edit` / `skit add --prompt` create-in-editor flows and the
//! `skit params --resync` source-management refusals. Each `#[test]` keeps its Python
//! `def test_*` name and its WHY comment so it traces back to its origin.
//!
//! HOW PYTHON MOCKS THE EDITOR, AND HOW THIS PORT DRIVES IT INSTEAD
//! The oracle never spawns a real editor: it `monkeypatch`es `cli.editor.open_in_editor`
//! (and, at the module level, `editor.subprocess.run`) with a Python callable that writes
//! the desired bytes to the target path. A Rust integration test cannot monkeypatch a
//! private function, so the CLI flows here drive the real `skit` binary through
//! `assert_cmd` inside a fresh three-directory sandbox and point `$EDITOR` at a tiny
//! executable shell script that (1) `touch`es a sentinel to prove it launched and
//! (2) `cat`s a fixed content file onto `$1` (the path skit appends). `/bin/true` is the
//! "leaves the file unchanged" editor; a missing path is the "cannot launch" editor. Two
//! interactive `skit edit <unknown>` flows are driven through a real PTY (portable-pty),
//! mirroring the oracle's `_is_interactive = True`.
//!
//! Concept mapping:
//! - Python `config.save_editor` / `config.load_editor` -> `skit_store::FileConfigStore`
//!   `.set("editor", …)` / `.get("editor")` (a normal dependency of skit-cli). `save_editor`'s
//!   strip-and-clear and non-string-is-blank semantics are reproduced exactly by
//!   `normalize_setting` (trim) + `write_key` (remove on empty) + `read_key` (string-only).
//! - Python `store.add_python(script, name=…)` -> `skit add <path> --name … --no-input`.
//! - Python `store.add_python(…, mode="reference")` -> `skit add <path> --name … --ref --no-input`.
//! - Python `store.add_command(tpl, name=…)` -> `skit add --cmd <tpl> --name … --no-input`.
//! - Python `runner.invoke(cli.app, ["edit", …])` -> `skit edit …`.
//! - Python `runner.invoke(cli.app, ["add", "-e", …])` -> `skit add -e …`.
//! - Python `_read_back` over `scripts/<slug>/script.py` -> read `scripts/<slug>/`'s stored source.
//!
//! Buckets (recorded per test in the porting ledger):
//! - REAL: an asserting test that drives the observable behavior (exit code, stored state,
//!   summary lines). For the CLI success flows the oracle's white-box "opened path" capture is
//!   replaced by its observable equivalent — the stored copy is updated / the entry lands with the
//!   right kind — because the path a mocked editor received is not observable end to end. The
//!   oracle's exact success verb ("Saved a") IS asserted verbatim; where the Rust CLI prints a
//!   different verb ("Edited") that is recorded as a FAILING CONTRACT, not a silent softening.
//!   Data/kind/exit assertions are kept verbatim.
//! - ABSENT (kind=absent): `open_in_editor` and `open_entry_in_editor` have no public equivalent on
//!   the Rust surface. Their remaining direct helper contracts stay as compiling `#[ignore]`
//!   trailheads. Editor resolution now has one private composition helper. Its nine exact frozen
//!   owners live beside that helper in `src/cli/tests.rs`; the public tests here keep the real
//!   config, environment, platform-default, raw-fallback, and launch contracts.
//! - DIVERGENCE (kind=divergence): the CLI API exists and the assertion compiles but the Rust
//!   behavior differs from the oracle (verified against the built binary). The full asserting body
//!   is kept intact behind `#[ignore = "FAILING CONTRACT (divergence): …"]`; deleting the ignore
//!   line after the impl is fixed is the whole fix workflow.
//! - CROSS-CRATE (kind=cross-crate): the interactive candidate-onboarding flow the oracle drives by
//!   mocking `cli.Prompt.ask` is reproducible here after all — it is driven through a real PTY (the
//!   same technique as the interactive edit-create flows), so it is now a live REAL test.

#![cfg(unix)]

use std::fs;
use std::io::{Read as _, Write as _};
use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};
use std::time::Duration;

use skit_store::FileConfigStore;
use tempfile::TempDir;

// --------------------------------------------------------------------------
// Sandbox + fake-editor harness
// --------------------------------------------------------------------------

/// A fresh three-directory skit sandbox plus a scratch dir for editor scripts and fixture files.
struct Sandbox {
    data: TempDir,
    state: TempDir,
    config: TempDir,
    scratch: TempDir,
}

impl Sandbox {
    fn new() -> Self {
        Self {
            data: TempDir::new().unwrap(),
            state: TempDir::new().unwrap(),
            config: TempDir::new().unwrap(),
            scratch: TempDir::new().unwrap(),
        }
    }

    /// An `assert_cmd` invocation with the sandbox env pinned. `$VISUAL`/`$EDITOR` are cleared so a
    /// dev machine's own editor never leaks in; a test that needs one sets `.env("EDITOR", …)`.
    fn command(&self) -> assert_cmd::Command {
        let mut command = assert_cmd::cargo::cargo_bin_cmd!("skit");
        command
            .env("SKIT_DATA_DIR", self.data.path())
            .env("SKIT_STATE_DIR", self.state.path())
            .env("SKIT_CONFIG_DIR", self.config.path())
            .env("SKIT_LANG", "en")
            .env_remove("VISUAL")
            .env_remove("EDITOR");
        command
    }

    fn config_store(&self) -> FileConfigStore {
        FileConfigStore::new(self.config.path())
    }

    /// Add a python copy entry through the real add lane (the oracle's `store.add_python`).
    fn add_python(&self, name: &str, body: &str) {
        let source = self.scratch.path().join(format!("{name}.py"));
        fs::write(&source, body).unwrap();
        self.command()
            .args([
                "add",
                source.to_str().unwrap(),
                "--name",
                name,
                "--no-input",
            ])
            .assert()
            .success();
    }

    /// Add a reference entry pointing at `source` (the oracle's `store.add_python(mode="reference")`).
    fn add_reference(&self, name: &str, source: &Path) {
        self.command()
            .args([
                "add",
                source.to_str().unwrap(),
                "--name",
                name,
                "--ref",
                "--no-input",
            ])
            .assert()
            .success();
    }

    /// Add a command-template entry (the oracle's `store.add_command`).
    fn add_command(&self, template: &str, name: &str) {
        self.command()
            .args(["add", "--cmd", template, "--name", name, "--no-input"])
            .assert()
            .success();
    }

    /// The stored source bytes of a copy entry (`scripts/<slug>/` minus `meta.toml`).
    fn stored_script(&self, slug: &str) -> String {
        let dir = self.data.path().join("scripts").join(slug);
        for entry in fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if path.file_name().and_then(|name| name.to_str()) != Some("meta.toml") {
                return fs::read_to_string(&path).unwrap();
            }
        }
        panic!("no stored source for {slug}");
    }

    /// The stored `meta.toml` text of an entry.
    fn stored_meta(&self, slug: &str) -> String {
        fs::read_to_string(
            self.data
                .path()
                .join("scripts")
                .join(slug)
                .join("meta.toml"),
        )
        .unwrap()
    }

    /// True when `skit show <name>` succeeds (the oracle's `store.resolve` not raising NotFound).
    fn resolvable(&self, name: &str) -> bool {
        self.command()
            .args(["show", name])
            .output()
            .unwrap()
            .status
            .success()
    }

    /// The draft files skit kept under `data_dir/drafts/`.
    fn draft_files(&self) -> Vec<PathBuf> {
        match fs::read_dir(self.data.path().join("drafts")) {
            Ok(reader) => reader
                .filter_map(|entry| entry.ok().map(|entry| entry.path()))
                .filter(|path| path.is_file())
                .collect(),
            Err(_) => Vec::new(),
        }
    }
}

/// Write an executable file with mode 0o755.
fn write_exec(path: &Path, body: &str) {
    fs::write(path, body).unwrap();
    let mut permissions = fs::metadata(path).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(path, permissions).unwrap();
}

/// A fake editor that records its launch (`touch <sentinel>`) and writes `content` onto the edited
/// path. Mirrors the oracle's `open_in_editor` mock that writes a fixed body to the target.
fn writing_editor(scratch: &Path, tag: &str, content: &str, sentinel: &Path) -> PathBuf {
    let content_file = scratch.join(format!("{tag}.content"));
    fs::write(&content_file, content).unwrap();
    let script = scratch.join(format!("{tag}.editor.sh"));
    write_exec(
        &script,
        &format!(
            "#!/bin/sh\ntouch '{}'\ncat '{}' > \"$1\"\n",
            sentinel.display(),
            content_file.display()
        ),
    );
    script
}

/// A fake editor that records its launch but leaves the target untouched (the oracle's
/// "untouched starter" / `_boom`-style launch probe, minus the abort).
fn touch_only_editor(scratch: &Path, tag: &str, sentinel: &Path) -> PathBuf {
    let script = scratch.join(format!("{tag}.touch.sh"));
    write_exec(
        &script,
        &format!("#!/bin/sh\ntouch '{}'\n", sentinel.display()),
    );
    script
}

fn capturing_touch_only_editor(
    scratch: &Path,
    tag: &str,
    sentinel: &Path,
    initial: &Path,
) -> PathBuf {
    let script = scratch.join(format!("{tag}.capture-touch.sh"));
    write_exec(
        &script,
        &format!(
            "#!/bin/sh\ntouch '{}'\ncp \"$1\" '{}'\n",
            sentinel.display(),
            initial.display()
        ),
    );
    script
}

/// stdout followed by stderr — the oracle's `CliRunner` merges both into `result.output`.
fn combined(output: &std::process::Output) -> String {
    let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&output.stderr));
    text
}

fn flat_text(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn snapshot_tree(root: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    fn visit(root: &Path, directory: &Path, output: &mut Vec<(PathBuf, Vec<u8>)>) {
        let Ok(entries) = fs::read_dir(directory) else {
            return;
        };
        for entry in entries {
            let path = entry.unwrap().path();
            if path.is_dir() {
                visit(root, &path, output);
            } else {
                output.push((
                    path.strip_prefix(root).unwrap().to_owned(),
                    fs::read(path).unwrap(),
                ));
            }
        }
    }

    let mut output = Vec::new();
    visit(root, root, &mut output);
    output.sort_by(|left, right| left.0.cmp(&right.0));
    output
}

/// Drive the real binary through a PTY (interactive `_is_interactive == True` flows).
fn run_pty(
    args: &[&str],
    data: &Path,
    state: &Path,
    config: &Path,
    editor: Option<&Path>,
    input: &[&[u8]],
) -> (u32, String) {
    use portable_pty::{CommandBuilder, PtySize, native_pty_system};

    let pair = native_pty_system()
        .openpty(PtySize {
            rows: 24,
            cols: 100,
            pixel_width: 0,
            pixel_height: 0,
        })
        .unwrap();
    let mut command = CommandBuilder::new(PathBuf::from(env!("CARGO_BIN_EXE_skit")));
    command.args(args);
    command.env("TERM", "xterm-256color");
    command.env("SKIT_LANG", "en");
    command.env("SKIT_DATA_DIR", data);
    command.env("SKIT_STATE_DIR", state);
    command.env("SKIT_CONFIG_DIR", config);
    if let Some(editor) = editor {
        command.env("EDITOR", editor);
    }
    let mut child = pair.slave.spawn_command(command).unwrap();
    drop(pair.slave);

    let mut reader = pair.master.try_clone_reader().unwrap();
    let drain = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = reader.read_to_end(&mut bytes);
        bytes
    });
    let mut writer = pair.master.take_writer().unwrap();
    std::thread::sleep(Duration::from_millis(100));
    for bytes in input {
        std::thread::sleep(Duration::from_millis(150));
        if writer.write_all(bytes).is_err() {
            break;
        }
        let _ = writer.flush();
    }
    let status = child.wait().unwrap();
    drop(writer);
    let output = String::from_utf8_lossy(&drain.join().unwrap()).into_owned();
    (status.exit_code(), output)
}

/// Drive one real line prompt, but do not send its answer before the prompt is visible.
///
/// `editor_marker` makes the pre-authoring order observable: if the editor opens before the name
/// prompt, fail immediately instead of waiting for a prompt that the broken implementation never
/// emits.
fn run_pty_after_prompt(
    sandbox: &Sandbox,
    args: &[&str],
    editor: &Path,
    editor_marker: &Path,
    prompt: &str,
    answer: &[u8],
) -> (u32, String) {
    use portable_pty::{CommandBuilder, PtySize, native_pty_system};
    use std::sync::{Arc, Mutex};
    use std::time::Instant;

    let pair = native_pty_system()
        .openpty(PtySize {
            rows: 24,
            cols: 120,
            pixel_width: 0,
            pixel_height: 0,
        })
        .unwrap();
    let mut command = CommandBuilder::new(PathBuf::from(env!("CARGO_BIN_EXE_skit")));
    command.args(args);
    command.env("TERM", "xterm-256color");
    command.env("SKIT_LANG", "en");
    command.env("SKIT_DATA_DIR", sandbox.data.path());
    command.env("SKIT_STATE_DIR", sandbox.state.path());
    command.env("SKIT_CONFIG_DIR", sandbox.config.path());
    command.env("VISUAL", editor);
    command.env("EDITOR", editor);
    let mut child = pair.slave.spawn_command(command).unwrap();
    drop(pair.slave);

    let captured = Arc::new(Mutex::new(Vec::new()));
    let drain_target = Arc::clone(&captured);
    let mut reader = pair.master.try_clone_reader().unwrap();
    let drain = std::thread::spawn(move || {
        let mut buffer = [0_u8; 1024];
        while let Ok(count) = reader.read(&mut buffer) {
            if count == 0 {
                break;
            }
            drain_target
                .lock()
                .unwrap()
                .extend_from_slice(&buffer[..count]);
        }
    });

    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let current = captured.lock().unwrap().clone();
        if String::from_utf8_lossy(&current).contains(prompt) {
            break;
        }
        assert!(
            !editor_marker.exists(),
            "the editor opened before prompt {prompt:?}: {}",
            String::from_utf8_lossy(&current)
        );
        assert!(
            Instant::now() < deadline,
            "PTY output never contained {prompt:?}: {}",
            String::from_utf8_lossy(&current)
        );
        std::thread::sleep(Duration::from_millis(10));
    }

    let mut writer = pair.master.take_writer().unwrap();
    writer.write_all(answer).unwrap();
    writer.flush().unwrap();
    let status = child.wait().unwrap();
    drop(writer);
    drop(pair.master);
    drain.join().unwrap();
    let output = String::from_utf8_lossy(&captured.lock().unwrap())
        .replace("\r\n", "\n")
        .replace('\r', "");
    (status.exit_code(), output)
}

// ==========================================================================
// editor.resolve_editor — the nine exact helper owners live beside the private CLI composition
// helper. The public launch contracts remain below.
// ==========================================================================

#[test]
fn test_resolve_editor_platform_default_unix() {
    // Nothing configured, VISUAL/EDITOR unset -> the oracle falls all the way through to the unix
    // platform default ["vi"] and launches it (`resolve_editor() == ["vi"]`). Observable: a fake
    // `vi` placed first on PATH is the program the edit lane must invoke. (`sandbox.command()`
    // already clears VISUAL/EDITOR, and no config editor is set.)
    let sandbox = Sandbox::new();
    sandbox.add_python("a", "print(1)\n");
    let sentinel = sandbox.scratch.path().join("vi.launched");
    let bin = sandbox.scratch.path().join("pathbin");
    fs::create_dir_all(&bin).unwrap();
    let content = sandbox.scratch.path().join("vi.content");
    fs::write(&content, "import rich\nprint('x')\n").unwrap();
    write_exec(
        &bin.join("vi"),
        &format!(
            "#!/bin/sh\ntouch '{}'\ncat '{}' > \"$1\"\n",
            sentinel.display(),
            content.display()
        ),
    );
    let path = format!(
        "{}:{}",
        bin.display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let output = sandbox
        .command()
        .env("PATH", path)
        .args(["edit", "a"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0), "{}", combined(&output));
    assert!(
        sentinel.exists(),
        "the platform default `vi` must be launched"
    );
    assert!(sandbox.stored_script("a").contains("import rich"));
}

// The nine precedence and platform-tokenization owners live beside the private composition helper in
// `src/cli/tests.rs`. Public tests below keep the real precedence and launch contracts.

#[test]
fn test_resolve_editor_whitespace_visual_falls_through_to_editor() {
    // A blank/whitespace-only $VISUAL is treated as unset, so a good $EDITOR still wins and is
    // launched (`resolve_editor() == ["nano"]`). Observable: the $EDITOR program is the one the edit
    // lane must invoke.
    let sandbox = Sandbox::new();
    sandbox.add_python("a", "print(1)\n");
    let sentinel = sandbox.scratch.path().join("nano.launched");
    let editor = writing_editor(
        sandbox.scratch.path(),
        "nano",
        "import rich\nprint('x')\n",
        &sentinel,
    );
    let output = sandbox
        .command()
        .env("VISUAL", "   ")
        .env("EDITOR", &editor)
        .args(["edit", "a"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0), "{}", combined(&output));
    assert!(
        sentinel.exists(),
        "a blank VISUAL must fall through to EDITOR"
    );
    assert!(sandbox.stored_script("a").contains("import rich"));
}

#[test]
fn test_resolve_editor_whitespace_config_falls_through_to_visual() {
    // A whitespace-only config editor falls through to $VISUAL, which beats $EDITOR
    // (`resolve_editor() == ["mvim", "-f"]` shape). Observable: the $VISUAL program is
    // the one the edit lane must invoke while $EDITOR stays untouched.
    let sandbox = Sandbox::new();
    sandbox.add_python("a", "print(1)\n");
    sandbox
        .command()
        .args(["config", "editor", "   "])
        .output()
        .unwrap();
    let visual_sentinel = sandbox.scratch.path().join("visual.launched");
    let visual = writing_editor(
        sandbox.scratch.path(),
        "visual",
        "import rich\nprint('x')\n",
        &visual_sentinel,
    );
    let editor_sentinel = sandbox.scratch.path().join("editor.launched");
    let editor = writing_editor(
        sandbox.scratch.path(),
        "editor",
        "print('editor')\n",
        &editor_sentinel,
    );
    let output = sandbox
        .command()
        .env("VISUAL", &visual)
        .env("EDITOR", &editor)
        .args(["edit", "a"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0), "{}", combined(&output));
    assert!(
        visual_sentinel.exists(),
        "a whitespace config editor must fall through to VISUAL"
    );
    assert!(!editor_sentinel.exists(), "VISUAL wins over EDITOR");
    assert!(sandbox.stored_script("a").contains("import rich"));
}

#[test]
fn test_resolve_editor_all_whitespace_candidates_use_platform_default() {
    // Every candidate blank -> the platform default `vi`, not a whitespace string
    // handed to shlex (`resolve_editor() == ["vi"]`). Observable: a fake `vi` placed
    // first on PATH is the program the edit lane must invoke.
    let sandbox = Sandbox::new();
    sandbox.add_python("a", "print(1)\n");
    sandbox
        .command()
        .args(["config", "editor", "  "])
        .output()
        .unwrap();
    let sentinel = sandbox.scratch.path().join("vi-default.launched");
    let bin = sandbox.scratch.path().join("pathbin-default");
    fs::create_dir_all(&bin).unwrap();
    let content = sandbox.scratch.path().join("vi-default.content");
    fs::write(&content, "import rich\nprint('x')\n").unwrap();
    write_exec(
        &bin.join("vi"),
        &format!(
            "#!/bin/sh\ntouch '{}'\ncat '{}' > \"$1\"\n",
            sentinel.display(),
            content.display()
        ),
    );
    let path = format!(
        "{}:{}",
        bin.display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let output = sandbox
        .command()
        .env("VISUAL", " ")
        .env("EDITOR", "")
        .env("PATH", path)
        .args(["edit", "a"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0), "{}", combined(&output));
    assert!(
        sentinel.exists(),
        "all-blank candidates must resolve the platform default vi"
    );
    assert!(sandbox.stored_script("a").contains("import rich"));

    // A configured comment-only command wins precedence, but shlex parses it to no argv. The same
    // platform fallback applies after parsing. Nonempty environment editors must not win.
    let comment = Sandbox::new();
    comment.add_python("comment", "print(1)\n");
    comment
        .command()
        .args(["config", "editor", "# no editor command"])
        .assert()
        .success();
    let config_before = fs::read(comment.config.path().join("config.toml")).unwrap();
    let comment_sentinel = comment.scratch.path().join("comment-vi.launched");
    let comment_bin = comment.scratch.path().join("comment-pathbin");
    fs::create_dir_all(&comment_bin).unwrap();
    let comment_content = comment.scratch.path().join("comment-vi.content");
    fs::write(&comment_content, "print('comment fallback')\n").unwrap();
    write_exec(
        &comment_bin.join("vi"),
        &format!(
            "#!/bin/sh\ntouch '{}'\ncat '{}' > \"$1\"\n",
            comment_sentinel.display(),
            comment_content.display()
        ),
    );
    let comment_path = format!(
        "{}:{}",
        comment_bin.display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let output = comment
        .command()
        .env("VISUAL", "/bin/false")
        .env("EDITOR", "/bin/false")
        .env("PATH", comment_path)
        .args(["edit", "comment"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0), "{}", combined(&output));
    assert!(
        comment_sentinel.exists(),
        "a parsed empty argv must launch the platform default vi"
    );
    assert_eq!(
        comment.stored_script("comment"),
        "print('comment fallback')\n"
    );
    assert_eq!(
        fs::read(comment.config.path().join("config.toml")).unwrap(),
        config_before
    );
}

#[test]
fn test_resolve_editor_unbalanced_quotes_falls_back_to_raw() {
    // An unbalanced-quote value is unusable as a parsed command; the oracle treats the whole raw
    // string as argv[0] (`resolve_editor() == ['weird "editor']`). Observable: a fake editor whose
    // filename is literally `weird "editor`, placed first on PATH, is the program the edit lane must
    // invoke — a fixed Rust would `Command::new` the raw string, which has no slash, so PATH is
    // searched and this file is found.
    let sandbox = Sandbox::new();
    sandbox.add_python("a", "print(1)\n");
    let sentinel = sandbox.scratch.path().join("raw.launched");
    let bin = sandbox.scratch.path().join("pathbin");
    fs::create_dir_all(&bin).unwrap();
    let content = sandbox.scratch.path().join("raw.content");
    fs::write(&content, "import rich\nprint('x')\n").unwrap();
    write_exec(
        &bin.join("weird \"editor"),
        &format!(
            "#!/bin/sh\ntouch '{}'\ncat '{}' > \"$1\"\n",
            sentinel.display(),
            content.display()
        ),
    );
    let path = format!(
        "{}:{}",
        bin.display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let output = sandbox
        .command()
        .env("EDITOR", "weird \"editor")
        .env("PATH", path)
        .args(["edit", "a"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0), "{}", combined(&output));
    assert!(
        sentinel.exists(),
        "the raw unbalanced-quote command must be launched as argv[0]"
    );
    assert!(sandbox.stored_script("a").contains("import rich"));
}

// ==========================================================================
// editor.open_in_editor — ABSENT (kind=absent): no public open_in_editor.
//
// The launch (argv = [*resolve_editor(), path]; subprocess.run; OSError -> EditorError) is inlined
// in cli.rs via ProcessCommand::new(argv[0]).args(...).arg(target).status() and the OSError branch
// becomes source_error("start editor for", …) (cli.rs:3362-3373). No public function returns the
// editor's exit code or raises a typed EditorError. MUST-FIX: expose open_in_editor
// (src/skit/editor.py:63-81). Message DIVERGES: "could not start editor for <path>: <err>" vs the
// oracle's "Could not launch the editor (<cmd>): <err>. Set one with: skit config editor <cmd>".
// ==========================================================================

#[test]
#[ignore = "ABSENT (kind=absent): no public open_in_editor. MUST-FIX: src/skit/editor.py:69-81 (argv appends path; returns returncode)."]
fn test_open_in_editor_appends_path_and_returns_code() {
    // The resolved argv gets the file path appended; the editor's exit code is returned.
    //   fake_run captures argv; delenv VISUAL; setenv EDITOR=nano
    //   assert open_in_editor(f) == 0
    //   assert captured["argv"] == ["nano", str(f)]; captured["check"] is False
}

#[test]
#[ignore = "ABSENT (kind=absent): no public open_in_editor. MUST-FIX: src/skit/editor.py:81 (non-zero returned, not raised)."]
fn test_open_in_editor_returns_nonzero_without_raising() {
    // A non-zero editor exit is returned, never raised (some editors exit non-zero on close).
    //   returncode = 3; setenv EDITOR=nano
    //   assert open_in_editor(...) == 3
}

#[test]
#[ignore = "ABSENT (kind=absent): no public open_in_editor, and the Rust launch-failure message DIVERGES (source_error 'could not start editor for …' vs the oracle string). MUST-FIX: src/skit/editor.py:74-81 (exact EditorError message)."]
fn test_open_in_editor_launch_failure_message_exact() {
    // A launch failure raises EditorError naming the command (argv minus path) and the OS error.
    //   config.save_editor("code --wait"); subprocess.run raises FileNotFoundError("boom-err")
    //   with pytest.raises(EditorError) as e: open_in_editor(...)
    //   assert "Could not launch the editor (code --wait): boom-err." in str(e.value)
    //   assert "skit config editor <cmd>" in str(e.value); "XX" not in str(e.value)
}

// ==========================================================================
// editor.open_entry_in_editor — ABSENT (kind=absent): no public open_entry_in_editor.
//
// The post-edit prompt-encoding revalidation (open the editor, then re-read a prompt body and wrap
// an OSError as EditedSourceError "Can't read <path>: <err>") is not exposed on the Rust surface;
// the `edit` command handles prompt entries inline. MUST-FIX: expose open_entry_in_editor
// (src/skit/editor.py:84-108).
// ==========================================================================

#[test]
#[ignore = "ABSENT (kind=absent): no public open_entry_in_editor / EditedSourceError. MUST-FIX: src/skit/editor.py:94-108 (post-edit prompt re-read; OSError -> EditedSourceError)."]
fn test_open_entry_prompt_removed_by_editor_is_a_clean_edited_source_error() {
    // An editor that deletes its target -> post-edit validation reports it cleanly.
    //   open_in_editor := remove_target (unlinks path, returns 0)
    //   with pytest.raises(EditedSourceError) as e: open_entry_in_editor(path, kind="prompt")
    //   assert "Can't read" in str(e.value); str(path) in str(e.value)
    //   assert isinstance(e.value.__cause__, FileNotFoundError)
}

// ==========================================================================
// config editor read/write — REAL (via skit_store::FileConfigStore).
// ==========================================================================

#[test]
fn test_config_editor_roundtrip_and_clear() {
    // load/save/clear round-trips; an empty save clears the key.
    let sandbox = Sandbox::new();
    let config = sandbox.config_store();
    assert_eq!(config.get("editor").unwrap(), "");
    config.set("editor", "code --wait").unwrap();
    assert_eq!(config.get("editor").unwrap(), "code --wait");
    config.set("editor", "").unwrap(); // empty clears the key
    assert_eq!(config.get("editor").unwrap(), "");
}

#[test]
fn test_save_editor_preserves_other_keys() {
    // Saving the editor preserves every other key (the language). (Python uses config.save_config;
    // the Rust equivalent is another typed set — both are key-preserving config transactions.)
    let sandbox = Sandbox::new();
    let config = sandbox.config_store();
    config.set("lang", "zh-TW").unwrap();
    config.set("editor", "nano").unwrap();
    assert_eq!(config.get("lang").unwrap(), "zh-TW");
    assert_eq!(config.get("editor").unwrap(), "nano");
}

#[test]
fn test_load_editor_non_string_value_is_blank() {
    // A hand-edited non-string editor value is treated as unset, not str()-coerced.
    let sandbox = Sandbox::new();
    fs::write(sandbox.config.path().join("config.toml"), "editor = 123\n").unwrap();
    assert_eq!(sandbox.config_store().get("editor").unwrap(), "");
}

#[test]
fn test_save_editor_clear_when_absent_does_not_raise() {
    // Clearing with no editor key present must be a no-op, not an error.
    let sandbox = Sandbox::new();
    let config = sandbox.config_store();
    assert_eq!(config.get("editor").unwrap(), "");
    config.set("editor", "").unwrap(); // must not raise
    assert_eq!(config.get("editor").unwrap(), "");
}

// ==========================================================================
// skit edit — open an existing script's source.
// ==========================================================================

#[test]
fn test_edit_opens_copy_source() {
    // edit opens the copy's stored source; a change is saved back and reported with "Saved a". (The
    // oracle's white-box opened-path capture == scripts/a/script.py is observed here as the stored
    // copy being updated.)
    let sandbox = Sandbox::new();
    sandbox.add_python("a", "print(1)\n");
    let sentinel = sandbox.scratch.path().join("a.launched");
    let editor = writing_editor(
        sandbox.scratch.path(),
        "a",
        "import rich\nprint('x')\n",
        &sentinel,
    );
    let output = sandbox
        .command()
        .env("EDITOR", &editor)
        .args(["edit", "a"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0), "{}", combined(&output));
    assert!(sandbox.stored_script("a").contains("import rich"));
    assert!(combined(&output).contains("Saved a")); // the oracle's success verb
}

#[test]
fn test_edit_repository_error_precedes_editor_resolution_without_a_write() {
    let sandbox = Sandbox::new();
    sandbox.add_python("Faulted Entry", "print('keep')\n");
    let sentinel = sandbox.scratch.path().join("repository-error.launched");
    let editor = touch_only_editor(sandbox.scratch.path(), "repository-error", &sentinel);
    let meta = sandbox.data.path().join("scripts/faulted-entry/meta.toml");
    fs::remove_file(&meta).unwrap();
    fs::create_dir(&meta).unwrap();
    let data_before = snapshot_tree(sandbox.data.path());
    let registry_before = fs::read(sandbox.data.path().join("registry.toml")).unwrap();
    let config_before = snapshot_tree(sandbox.config.path());
    let state_before = snapshot_tree(sandbox.state.path());
    let scratch_before = snapshot_tree(sandbox.scratch.path());

    let output = sandbox
        .command()
        .env("EDITOR", &editor)
        .args(["edit", "Faulted Entry"])
        .output()
        .unwrap();

    let text = combined(&output);
    assert_eq!(output.status.code(), Some(1), "{text}");
    assert!(text.contains("could not read"), "{text}");
    assert!(text.contains("scripts/faulted-entry/meta.toml"), "{text}");
    assert!(!text.contains("no editable entry is named"), "{text}");
    assert!(!sentinel.exists(), "repository lookup precedes the editor");
    assert!(meta.is_dir());
    assert_eq!(snapshot_tree(sandbox.data.path()), data_before);
    assert_eq!(
        fs::read(sandbox.data.path().join("registry.toml")).unwrap(),
        registry_before
    );
    assert_eq!(snapshot_tree(sandbox.config.path()), config_before);
    assert_eq!(snapshot_tree(sandbox.state.path()), state_before);
    assert_eq!(snapshot_tree(sandbox.scratch.path()), scratch_before);
}

#[test]
fn test_edit_opens_reference_original() {
    // A reference entry edits its original file in place (the oracle: opened path == src.resolve()).
    let sandbox = Sandbox::new();
    let original = sandbox.scratch.path().join("orig.py");
    fs::write(&original, "print(1)\n").unwrap();
    sandbox.add_reference("r", &original);
    let sentinel = sandbox.scratch.path().join("r.launched");
    let editor = writing_editor(
        sandbox.scratch.path(),
        "r",
        "import rich\nprint('x')\n",
        &sentinel,
    );
    let output = sandbox
        .command()
        .env("EDITOR", &editor)
        .args(["edit", "r"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0), "{}", combined(&output));
    // the original, not a copy, carries the edit
    assert!(
        fs::read_to_string(&original)
            .unwrap()
            .contains("import rich")
    );
}

#[test]
fn test_edit_reference_source_gone() {
    // A reference whose original was deleted -> exit 1, "gone", editor never launched.
    let sandbox = Sandbox::new();
    let original = sandbox.scratch.path().join("orig.py");
    fs::write(&original, "print(1)\n").unwrap();
    sandbox.add_reference("r", &original);
    fs::remove_file(&original).unwrap();
    let sentinel = sandbox.scratch.path().join("gone.launched");
    let editor = touch_only_editor(sandbox.scratch.path(), "gone", &sentinel);
    let output = sandbox
        .command()
        .env("EDITOR", &editor)
        .args(["edit", "r"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(combined(&output).contains("gone"));
    assert!(!sentinel.exists(), "the editor must not be launched");
}

#[test]
fn test_edit_reports_editor_launch_failure() {
    // A launch failure surfaces as a failed operation (exit 1). (Python raises EditorError "could
    // not launch"; Rust's source_error message differs and is not asserted verbatim.)
    let sandbox = Sandbox::new();
    sandbox.add_python("a", "print(1)\n");
    let output = sandbox
        .command()
        .env("EDITOR", "/nonexistent/skit-no-such-editor")
        .args(["edit", "a"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1), "{}", combined(&output));
}

// ==========================================================================
// skit edit <unknown> — offer to create.
// ==========================================================================

#[test]
fn test_edit_unknown_confirmed_creates() {
    // Confirming the offer creates the script from the editor's bytes (python, "requests").
    let sandbox = Sandbox::new();
    let sentinel = sandbox.scratch.path().join("new.launched");
    let editor = writing_editor(
        sandbox.scratch.path(),
        "new",
        "import requests\nprint('hi')\n",
        &sentinel,
    );
    let (code, out) = run_pty(
        &["edit", "newscript"],
        sandbox.data.path(),
        sandbox.state.path(),
        sandbox.config.path(),
        Some(&editor),
        &[b"y\n"],
    );
    assert_eq!(code, 0, "{out}");
    assert!(sandbox.resolvable("newscript"));
    assert!(
        sandbox
            .stored_meta("newscript")
            .contains("kind = \"python\"")
    );
    assert!(sandbox.stored_script("newscript").contains("requests"));
}

#[test]
fn test_edit_unknown_declined_creates_nothing() {
    // Declining -> clean exit 0 and nothing created.
    let sandbox = Sandbox::new();
    let sentinel = sandbox.scratch.path().join("declined.launched");
    let editor = touch_only_editor(sandbox.scratch.path(), "declined", &sentinel);
    let (code, _out) = run_pty(
        &["edit", "nope"],
        sandbox.data.path(),
        sandbox.state.path(),
        sandbox.config.path(),
        Some(&editor),
        &[b"n\n"],
    );
    assert_eq!(code, 0);
    assert!(!sentinel.exists());
    assert!(!sandbox.data.path().join("scripts/nope").exists());
    assert!(!sandbox.resolvable("nope"));
}

#[test]
fn test_edit_unknown_non_interactive_errors() {
    // A non-interactive shell can't be offered the create -> exit 1.
    let sandbox = Sandbox::new();
    let output = sandbox
        .command()
        .env("EDITOR", "/bin/true")
        .args(["edit", "ghost"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1), "{}", combined(&output));
}

// ==========================================================================
// skit add --edit / -e — create a brand-new script in the editor.
// ==========================================================================

#[test]
fn test_add_edit_creates_in_editor() {
    // A python draft lands as a python copy carrying its authored source ("rich").
    let sandbox = Sandbox::new();
    let sentinel = sandbox.scratch.path().join("fresh.launched");
    let editor = writing_editor(
        sandbox.scratch.path(),
        "fresh",
        "import rich\nprint('x')\n",
        &sentinel,
    );
    let (code, output) = run_pty(
        &["add", "-e", "--name", "fresh"],
        sandbox.data.path(),
        sandbox.state.path(),
        sandbox.config.path(),
        Some(&editor),
        &[],
    );
    assert_eq!(code, 0, "{output}");
    assert!(sandbox.stored_meta("fresh").contains("kind = \"python\""));
    assert!(sandbox.stored_script("fresh").contains("rich"));
}

#[test]
fn test_add_edit_bash_shebang_draft_becomes_a_shell_entry() {
    // A #!/usr/bin/env bash body makes the entry SHELL (re-inferred from the shebang), not python.
    let sandbox = Sandbox::new();
    let sentinel = sandbox.scratch.path().join("deploy.launched");
    let editor = writing_editor(
        sandbox.scratch.path(),
        "deploy",
        "#!/usr/bin/env bash\n# Ship it\necho drafted\n",
        &sentinel,
    );
    let (code, output) = run_pty(
        &["add", "-e", "--name", "deploy"],
        sandbox.data.path(),
        sandbox.state.path(),
        sandbox.config.path(),
        Some(&editor),
        &[],
    );
    assert_eq!(code, 0, "{output}");
    assert!(sandbox.stored_meta("deploy").contains("kind = \"shell\""));
    assert!(sandbox.stored_script("deploy").contains("echo drafted"));
}

#[test]
fn test_add_edit_js_shebang_draft_scans_npm_deps() {
    // A node-shebang draft lands as js and its declared npm imports are scanned into dependencies.
    let sandbox = Sandbox::new();
    let sentinel = sandbox.scratch.path().join("color.launched");
    let editor = writing_editor(
        sandbox.scratch.path(),
        "color",
        "#!/usr/bin/env node\nimport chalk from 'chalk'\nconsole.log(chalk)\n",
        &sentinel,
    );
    let (code, output) = run_pty(
        &["add", "-e", "--name", "colorized"],
        sandbox.data.path(),
        sandbox.state.path(),
        sandbox.config.path(),
        Some(&editor),
        &[],
    );
    assert_eq!(code, 0, "{output}");
    let meta = sandbox.stored_meta("colorized");
    assert!(meta.contains("kind = \"js\""));
    assert!(
        meta.contains("chalk"),
        "npm scan materialized the dep: {meta}"
    );
}

#[test]
fn test_add_edit_zsh_draft_records_interpreter_and_dry_run_names_zsh() {
    // A #!/usr/bin/env zsh draft lands as shell with interpreter=zsh; the dry-run names zsh.
    let sandbox = Sandbox::new();
    let sentinel = sandbox.scratch.path().join("zjob.launched");
    let editor = writing_editor(
        sandbox.scratch.path(),
        "zjob",
        "#!/usr/bin/env zsh\necho hi\n",
        &sentinel,
    );
    let (code, output) = run_pty(
        &["add", "-e", "--name", "zjob"],
        sandbox.data.path(),
        sandbox.state.path(),
        sandbox.config.path(),
        Some(&editor),
        &[],
    );
    assert_eq!(code, 0, "{output}");
    let meta = sandbox.stored_meta("zjob");
    assert!(meta.contains("kind = \"shell\""));
    assert!(meta.contains("interpreter = \"zsh\""));
    let dry = sandbox
        .command()
        .args(["run", "zjob", "--dry-run", "--no-input"])
        .output()
        .unwrap();
    assert_eq!(dry.status.code(), Some(0), "{}", combined(&dry));
    assert!(combined(&dry).contains("zsh"));
}

#[test]
fn test_add_edit_shell_draft_onboards_picked_constants() {
    // The shell analyzer's constants are offered and accepted. The oracle mocks Prompt.ask -> "all";
    // the Rust equivalent is a MultiSelect where BOTH candidates start checked (demotion.is_none(),
    // so selected_by_default() == true, skit-form/src/lib.rs:205), so a single Enter accepts all of
    // them — the load-bearing equivalence to the "all" answer. Both CITY and API_KEY then land as
    // managed params in the stored copy.
    let sandbox = Sandbox::new();
    let sentinel = sandbox.scratch.path().join("deploy.launched");
    let editor = writing_editor(
        sandbox.scratch.path(),
        "deploy",
        "#!/usr/bin/env bash\nCITY=Taipei\nAPI_KEY=secret\necho $CITY\n",
        &sentinel,
    );
    let (code, out) = run_pty(
        &["add", "-e", "--name", "deploy"],
        sandbox.data.path(),
        sandbox.state.path(),
        sandbox.config.path(),
        Some(&editor),
        &[b"\r"], // Enter accepts the default-checked candidates = "all"
    );
    assert_eq!(code, 0, "{out}");
    assert!(sandbox.stored_meta("deploy").contains("kind = \"shell\""));
    // managed == {"CITY", "API_KEY"}: both appear as [[tool.skit.params]] declarations in the copy
    // (never bare — the raw source line `CITY=Taipei` would make a bare substring vacuous).
    let script = sandbox.stored_script("deploy");
    assert!(script.contains("name = \"CITY\""), "CITY managed: {script}");
    assert!(
        script.contains("name = \"API_KEY\""),
        "API_KEY managed: {script}"
    );
}

#[test]
fn test_add_edit_dep_flag_on_non_python_draft_is_refused() {
    // --dep is python-only: riding it on a draft whose shebang names another kind is REFUSED
    // (exit 2), the refusal names the python flags AND the draft's actual kind, and nothing is added.
    let sandbox = Sandbox::new();
    let sentinel = sandbox.scratch.path().join("d.launched");
    let editor = writing_editor(
        sandbox.scratch.path(),
        "d",
        "#!/usr/bin/env bash\necho drafted\n",
        &sentinel,
    );
    let (code, output) = run_pty(
        &["add", "-e", "--name", "d", "--dep", "rich"],
        sandbox.data.path(),
        sandbox.state.path(),
        sandbox.config.path(),
        Some(&editor),
        &[],
    );
    assert_eq!(code, 2, "{output}");
    assert!(output.contains("python flags")); // the refusal names the flags…
    assert!(output.contains("shell")); // …and the draft's actual kind
    assert!(!sandbox.resolvable("d")); // nothing was added
}

// --------------------------------------------------------------------------
// Draft preservation: refuse a taken name BEFORE the editor; keep the draft on a
// post-edit failure.
// --------------------------------------------------------------------------

#[test]
fn test_add_edit_python_name_taken_refuses_before_the_editor() {
    // A taken name is caught BEFORE $EDITOR opens; the editor is never launched.
    let sandbox = Sandbox::new();
    sandbox.add_python("taken", "print(1)\n");
    let sentinel = sandbox.scratch.path().join("taken.launched");
    let editor = touch_only_editor(sandbox.scratch.path(), "taken", &sentinel);
    let (code, output) = run_pty(
        &["add", "-e", "--name", "taken"],
        sandbox.data.path(),
        sandbox.state.path(),
        sandbox.config.path(),
        Some(&editor),
        &[],
    );
    assert_eq!(code, 1, "{output}");
    assert!(output.contains("already taken"));
    assert!(!sentinel.exists(), "the editor must not be launched");
}

#[test]
fn test_add_edit_python_post_edit_failure_keeps_the_draft() {
    // A failure AFTER the edit never deletes the temp draft (the user's only copy); the error names
    // where the draft lives and NO new entry is added — with zero pre-existing entries that is
    // exactly the oracle's `store.list_entries() == []`. The oracle injects a StoreError in
    // `_onboard_python`; the black-box equivalent is a read-only `scripts/` dir, so the post-edit
    // commit fails after the editor has already written the draft.
    let sandbox = Sandbox::new();
    let scripts = sandbox.data.path().join("scripts");
    fs::create_dir_all(&scripts).unwrap();
    let mut perms = fs::metadata(&scripts).unwrap().permissions();
    perms.set_mode(0o500); // read + execute, no write: the store commit will fail
    fs::set_permissions(&scripts, perms).unwrap();
    let sentinel = sandbox.scratch.path().join("kept.launched");
    let editor = writing_editor(
        sandbox.scratch.path(),
        "kept",
        "import sys\nprint('drafted')\n",
        &sentinel,
    );
    let (code, output) = run_pty(
        &["add", "-e", "--name", "keptpy"],
        sandbox.data.path(),
        sandbox.state.path(),
        sandbox.config.path(),
        Some(&editor),
        &[],
    );
    assert_eq!(code, 1, "{output}");
    assert!(output.contains("Your draft was kept at"));
    assert!(sentinel.exists(), "the editor ran (post-edit failure)");
    assert!(!sandbox.draft_files().is_empty(), "the draft survived");
    // Nothing was added: the store is still empty (scripts/ stays readable at 0o500, no entry).
    assert!(!sandbox.resolvable("keptpy"));
    assert_eq!(
        fs::read_dir(&scripts).unwrap().count(),
        0,
        "no entry was committed"
    );
}

#[test]
fn test_add_edit_rejects_path() {
    // `add -e <path>` mixes a create-in-editor with a source path -> exit 2.
    let sandbox = Sandbox::new();
    let path = sandbox.scratch.path().join("s.py");
    fs::write(&path, "print(1)\n").unwrap();
    let output = sandbox
        .command()
        .args(["add", "-e", path.to_str().unwrap()])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2), "{}", combined(&output));
}

#[test]
fn test_add_edit_non_interactive_errors() {
    // A non-interactive `add -e` (piped, non-tty stdin/stdout) must refuse (exit 2) and NEVER launch
    // the editor — the oracle gates on `_is_interactive() == False`. `.output()` gives a non-tty
    // stdin and stdout (the Rust gate shape is `is_terminal() && is_terminal()`); no `--no-input` is
    // used, so this exercises the tty gate, not the flag. The editor writes a valid draft, so a Rust
    // that has NO gate runs the whole lane and exits 0 with the entry created (exit-2 fails first,
    // its message carrying the "Added" evidence); the editor sentinel then also proves the launch.
    // `--no-input` is the first refusal. It keeps its more specific pipe guidance even though this
    // process also has non-terminal streams.
    let no_input = Sandbox::new();
    let no_input_sentinel = no_input.scratch.path().join("no-input.launched");
    let no_input_editor = writing_editor(
        no_input.scratch.path(),
        "no-input",
        "print('must not run')\n",
        &no_input_sentinel,
    );
    let no_input_data = snapshot_tree(no_input.data.path());
    let no_input_state = snapshot_tree(no_input.state.path());
    let no_input_config = snapshot_tree(no_input.config.path());
    let no_input_output = no_input
        .command()
        .env("VISUAL", &no_input_editor)
        .env("EDITOR", &no_input_editor)
        .args(["add", "-e", "--name", "x", "--no-input"])
        .output()
        .unwrap();
    assert_eq!(no_input_output.status.code(), Some(2));
    assert_eq!(
        flat_text(&combined(&no_input_output)),
        "--edit opens your editor, which --no-input forbids — pipe the script in instead: skit add - -n NAME"
    );
    assert!(!no_input_sentinel.exists());
    assert_eq!(snapshot_tree(no_input.data.path()), no_input_data);
    assert_eq!(snapshot_tree(no_input.state.path()), no_input_state);
    assert_eq!(snapshot_tree(no_input.config.path()), no_input_config);

    // Without `--no-input`, the terminal refusal is exact and happens before the editor or store.
    let sandbox = Sandbox::new();
    let sentinel = sandbox.scratch.path().join("x.launched");
    let editor = writing_editor(sandbox.scratch.path(), "x", "print('x')\n", &sentinel);
    let data_before = snapshot_tree(sandbox.data.path());
    let state_before = snapshot_tree(sandbox.state.path());
    let config_before = snapshot_tree(sandbox.config.path());
    let output = sandbox
        .command()
        .env("VISUAL", &editor)
        .env("EDITOR", &editor)
        .args(["add", "-e", "--name", "x"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2), "{}", combined(&output));
    assert_eq!(
        flat_text(&combined(&output)),
        "Writing a new script in an editor needs an interactive terminal."
    );
    assert!(
        !sentinel.exists(),
        "the editor must not be launched non-interactively"
    );
    assert_eq!(snapshot_tree(sandbox.data.path()), data_before);
    assert_eq!(snapshot_tree(sandbox.state.path()), state_before);
    assert_eq!(snapshot_tree(sandbox.config.path()), config_before);
    assert!(sandbox.draft_files().is_empty());

    // The terminal gate also precedes flag validation and conflict lookup.
    let ordered = Sandbox::new();
    ordered.add_python("taken", "print('old')\n");
    let ordered_sentinel = ordered.scratch.path().join("ordered.launched");
    let ordered_editor = writing_editor(
        ordered.scratch.path(),
        "ordered",
        "print('must not run')\n",
        &ordered_sentinel,
    );
    let ordered_data = snapshot_tree(ordered.data.path());
    let ordered_state = snapshot_tree(ordered.state.path());
    let ordered_config = snapshot_tree(ordered.config.path());
    let ordered_output = ordered
        .command()
        .env("VISUAL", &ordered_editor)
        .env("EDITOR", &ordered_editor)
        .args(["add", "-e", "--name", "taken", "--python", "not-a-version"])
        .output()
        .unwrap();
    assert_eq!(ordered_output.status.code(), Some(2));
    assert_eq!(
        flat_text(&combined(&ordered_output)),
        "Writing a new script in an editor needs an interactive terminal."
    );
    assert!(!ordered_sentinel.exists());
    assert_eq!(snapshot_tree(ordered.data.path()), ordered_data);
    assert_eq!(snapshot_tree(ordered.state.path()), ordered_state);
    assert_eq!(snapshot_tree(ordered.config.path()), ordered_config);
}

#[test]
fn test_add_edit_empty_content_adds_nothing() {
    // Leaving the starter unchanged -> exit 0, "Nothing was written", nothing added.
    let sandbox = Sandbox::new();
    let sentinel = sandbox.scratch.path().join("empty.launched");
    let editor = touch_only_editor(sandbox.scratch.path(), "empty", &sentinel);
    let (code, output) = run_pty(
        &["add", "-e", "--name", "ghost"],
        sandbox.data.path(),
        sandbox.state.path(),
        sandbox.config.path(),
        Some(&editor),
        &[],
    );
    assert_eq!(code, 0, "{output}");
    assert!(
        output.contains("Nothing was written, so no script was added."),
        "{output}"
    );
    assert!(sentinel.exists(), "the editor must open");
    assert!(!sandbox.resolvable("ghost"));
    assert!(sandbox.draft_files().is_empty(), "{output}");
}

#[test]
fn test_add_edit_unregistered_shebang_refused_keeps_draft() {
    // An unregistered-interpreter shebang can't be honored: refuse (exit 2), keep the draft, add
    // nothing — never fabricate a python entry.
    let sandbox = Sandbox::new();
    let sentinel = sandbox.scratch.path().join("aw.launched");
    let editor = writing_editor(
        sandbox.scratch.path(),
        "aw",
        "#!/usr/bin/awk -f\nBEGIN { print 1 }\n",
        &sentinel,
    );
    let (code, text) = run_pty(
        &["add", "-e", "--name", "aw"],
        sandbox.data.path(),
        sandbox.state.path(),
        sandbox.config.path(),
        Some(&editor),
        &[],
    );
    assert_eq!(code, 2, "{text}");
    let usage = text
        .find("The draft's #! names no interpreter skit knows")
        .expect("the editor-draft usage voice");
    let kept = text
        .find("Your draft was kept at")
        .expect("the short kept-draft notice");
    assert!(usage < kept, "usage must precede the kept notice: {text}");
    assert!(text.contains("skit add ") && text.contains(" --kind <language>"));
    assert!(!text.contains("--exe")); // editor drafts never use the regular-path escape
    assert_eq!(text.matches("Your draft was kept at").count(), 1, "{text}");
    assert!(sentinel.exists(), "the editor authored the rejected draft");
    let drafts = sandbox.draft_files();
    assert_eq!(drafts.len(), 1, "the draft survived: {text}");
    assert_eq!(
        drafts[0].parent(),
        Some(sandbox.data.path().join("drafts").as_path())
    );
    assert!(!sandbox.resolvable("aw")); // nothing fabricated
}

#[test]
fn test_add_edit_untouched_starter_unlinks_the_draft() {
    // The untouched-starter cancel is pure litter -> unlink it; the temp is gone after "Nothing was
    // written".
    let sandbox = Sandbox::new();
    let sentinel = sandbox.scratch.path().join("untouched.launched");
    let editor = touch_only_editor(sandbox.scratch.path(), "untouched", &sentinel);
    let (code, output) = run_pty(
        &["add", "-e", "--name", "ghost"],
        sandbox.data.path(),
        sandbox.state.path(),
        sandbox.config.path(),
        Some(&editor),
        &[],
    );
    assert_eq!(code, 0, "{output}");
    assert!(
        output.contains("Nothing was written, so no script was added."),
        "{output}"
    );
    assert!(sentinel.exists(), "the editor must open");
    assert!(!sandbox.resolvable("ghost"));
    assert!(
        sandbox.draft_files().is_empty(),
        "the litter was cleaned up"
    );
}

#[test]
fn test_add_prompt_editor_untouched_starter_unlinks_the_draft() {
    // Same for the prompt editor lane: an untouched starter is unlinked, not left behind.
    let sandbox = Sandbox::new();
    let sentinel = sandbox.scratch.path().join("prompt-untouched.launched");
    let initial = sandbox.scratch.path().join("prompt-untouched.initial");
    let editor = capturing_touch_only_editor(
        sandbox.scratch.path(),
        "prompt-untouched",
        &sentinel,
        &initial,
    );
    let (code, output) = run_pty(
        &["add", "--prompt", "--name", "ghostp"],
        sandbox.data.path(),
        sandbox.state.path(),
        sandbox.config.path(),
        Some(&editor),
        &[],
    );
    assert_eq!(code, 0, "{output}");
    assert!(
        output.contains("Nothing was written, so no prompt was added."),
        "{output}"
    );
    assert!(sentinel.exists(), "the editor must open");
    assert_eq!(fs::read(initial).unwrap(), b"# New prompt\n\n");
    assert!(!sandbox.resolvable("ghostp"));
    assert!(sandbox.draft_files().is_empty(), "{output}");
}

#[test]
fn test_add_edit_prompts_for_name_when_omitted() {
    // Omitting --name -> the user is prompted and the answer becomes the name.
    // Explicit flag validation is earlier than that prompt and must not cost an editor session.
    let invalid = Sandbox::new();
    let invalid_sentinel = invalid.scratch.path().join("invalid.launched");
    let invalid_editor = writing_editor(
        invalid.scratch.path(),
        "invalid",
        "print('must not run')\n",
        &invalid_sentinel,
    );
    let invalid_data = snapshot_tree(invalid.data.path());
    let invalid_state = snapshot_tree(invalid.state.path());
    let invalid_config = snapshot_tree(invalid.config.path());
    let (invalid_code, invalid_output) = run_pty(
        &["add", "-e", "--python", "not-a-version"],
        invalid.data.path(),
        invalid.state.path(),
        invalid.config.path(),
        Some(&invalid_editor),
        &[],
    );
    assert_eq!(invalid_code, 2, "{invalid_output}");
    assert!(!invalid_output.contains("Name in skit"), "{invalid_output}");
    assert!(!invalid_sentinel.exists());
    assert_eq!(snapshot_tree(invalid.data.path()), invalid_data);
    assert_eq!(snapshot_tree(invalid.state.path()), invalid_state);
    assert_eq!(snapshot_tree(invalid.config.path()), invalid_config);

    let sandbox = Sandbox::new();
    let sentinel = sandbox.scratch.path().join("noname.launched");
    let editor = writing_editor(sandbox.scratch.path(), "noname", "print('x')\n", &sentinel);
    let state_before = snapshot_tree(sandbox.state.path());
    let config_before = snapshot_tree(sandbox.config.path());
    let (code, output) = run_pty_after_prompt(
        &sandbox,
        &["add", "-e"],
        &editor,
        &sentinel,
        "Name in skit",
        b"prompted\r",
    );
    assert_eq!(code, 0, "{output}");
    assert!(output.contains("Name in skit"), "{output}");
    assert!(output.contains("Added: prompted"), "{output}");
    assert!(sentinel.exists(), "the editor must open after the answer");
    assert_eq!(sandbox.stored_script("prompted"), "print('x')\n");
    let meta = sandbox.stored_meta("prompted");
    assert!(meta.contains("name = \"prompted\""), "{meta}");
    assert!(meta.contains("kind = \"python\""), "{meta}");
    let names: Vec<String> = fs::read_dir(sandbox.data.path().join("scripts"))
        .unwrap()
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(names, ["prompted"]);
    assert!(sandbox.draft_files().is_empty(), "{output}");
    assert_eq!(snapshot_tree(sandbox.state.path()), state_before);
    assert_eq!(snapshot_tree(sandbox.config.path()), config_before);
}

#[test]
fn test_add_edit_blank_name_errors() {
    // A whitespace-only prompted name -> no name -> exit 2.
    let sandbox = Sandbox::new();
    let sentinel = sandbox.scratch.path().join("blank.launched");
    let editor = writing_editor(sandbox.scratch.path(), "blank", "print('x')\n", &sentinel);
    let data_before = snapshot_tree(sandbox.data.path());
    let state_before = snapshot_tree(sandbox.state.path());
    let config_before = snapshot_tree(sandbox.config.path());
    let (code, output) = run_pty_after_prompt(
        &sandbox,
        &["add", "-e"],
        &editor,
        &sentinel,
        "Name in skit",
        b"   \r",
    );
    assert_eq!(code, 2, "{output}");
    assert!(output.contains("Name in skit"), "{output}");
    assert!(output.contains("A name is required."), "{output}");
    assert!(!sentinel.exists(), "a blank name must precede the editor");
    assert_eq!(snapshot_tree(sandbox.data.path()), data_before);
    assert_eq!(snapshot_tree(sandbox.state.path()), state_before);
    assert_eq!(snapshot_tree(sandbox.config.path()), config_before);
    assert!(sandbox.draft_files().is_empty());

    // A non-blank prompted name is resolved only after the prompt and before the editor.
    let conflict = Sandbox::new();
    conflict.add_python("taken", "print('old')\n");
    let conflict_sentinel = conflict.scratch.path().join("conflict.launched");
    let conflict_editor = writing_editor(
        conflict.scratch.path(),
        "conflict",
        "print('must not run')\n",
        &conflict_sentinel,
    );
    let conflict_data = snapshot_tree(conflict.data.path());
    let conflict_state = snapshot_tree(conflict.state.path());
    let conflict_config = snapshot_tree(conflict.config.path());
    let (conflict_code, conflict_output) = run_pty_after_prompt(
        &conflict,
        &["add", "-e"],
        &conflict_editor,
        &conflict_sentinel,
        "Name in skit",
        b"taken\r",
    );
    assert_eq!(conflict_code, 1, "{conflict_output}");
    assert!(
        flat_text(&conflict_output)
            .contains("The name taken is already taken — pick another name."),
        "{conflict_output}"
    );
    assert!(!conflict_sentinel.exists());
    assert_eq!(snapshot_tree(conflict.data.path()), conflict_data);
    assert_eq!(snapshot_tree(conflict.state.path()), conflict_state);
    assert_eq!(snapshot_tree(conflict.config.path()), conflict_config);
}

#[test]
fn test_add_edit_editor_error_exits_one() {
    // An editor that cannot launch -> exit 1.
    let sandbox = Sandbox::new();
    let missing = Path::new("/nonexistent/skit-no-such-editor");
    let (code, output) = run_pty(
        &["add", "-e", "--name", "x"],
        sandbox.data.path(),
        sandbox.state.path(),
        sandbox.config.path(),
        Some(missing),
        &[],
    );
    assert_eq!(code, 1, "{output}");
}

#[test]
fn test_add_edit_name_conflict_exits_one() {
    // A store name conflict is a failed operation (exit 1) that names the entry and says "taken".
    let sandbox = Sandbox::new();
    sandbox.add_python("dup", "print(1)\n");
    let sentinel = sandbox.scratch.path().join("dup.launched");
    let editor = writing_editor(sandbox.scratch.path(), "dup", "print('x')\n", &sentinel);
    let (code, output) = run_pty(
        &["add", "-e", "--name", "dup"],
        sandbox.data.path(),
        sandbox.state.path(),
        sandbox.config.path(),
        Some(&editor),
        &[],
    );
    assert_eq!(code, 1, "{output}");
    assert!(output.contains("dup")); // the name is surfaced
    assert!(output.contains("taken")); // the StoreError is surfaced
    assert!(!sentinel.exists(), "the editor must not be launched");
}

#[test]
fn test_add_edit_writes_and_reports_managed_and_secret() {
    // The add summary reports the managed and secret parameters. (The oracle mocks _onboard_params
    // to return one secret const; here the draft carries the same decl in its [tool.skit] block, so
    // the create call's _print_add_summary reports it — the same observable summary contract.)
    let sandbox = Sandbox::new();
    let secret_draft = concat!(
        "# /// script\n",
        "# [tool.skit]\n",
        "# schema = 1\n",
        "#\n",
        "# [[tool.skit.params]]\n",
        "# name = \"API\"\n",
        "# binding = \"const\"\n",
        "# type = \"str\"\n",
        "# default = \"x\"\n",
        "# secret = true\n",
        "# ///\n",
        "API = \"x\"\n",
        "print(API)\n",
    );
    let sentinel = sandbox.scratch.path().join("sec.launched");
    let editor = writing_editor(sandbox.scratch.path(), "sec", secret_draft, &sentinel);
    let (code, output) = run_pty(
        &["add", "-e", "--name", "fresh"],
        sandbox.data.path(),
        sandbox.state.path(),
        sandbox.config.path(),
        Some(&editor),
        &[b" \r"], // Keep the authored declaration instead of replacing it from detection.
    );
    assert_eq!(code, 0, "{output}");
    assert!(output.contains("Managed parameters: API"), "{output}");
    assert!(
        output.contains("Secret parameter values are never saved by skit: API"),
        "{output}"
    );
}

// ==========================================================================
// skit params — source-management refusals.
// ==========================================================================

#[test]
fn test_params_edit_command_entry_refused() {
    // A command entry has no editable source -> params source-edit is refused (exit 1).
    let sandbox = Sandbox::new();
    sandbox.add_command("echo {x}", "ec");
    let meta = sandbox
        .data
        .path()
        .join("scripts")
        .join("ec")
        .join("meta.toml");
    let meta_before = fs::read(&meta).unwrap();
    assert_eq!(fs::read_dir(sandbox.state.path()).unwrap().count(), 0);
    let output = sandbox
        .command()
        .args(["params", "ec", "--resync"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1), "{}", combined(&output));
    assert_eq!(
        fs::read(meta).unwrap(),
        meta_before,
        "metadata is unchanged"
    );
    assert_eq!(
        fs::read_dir(sandbox.state.path()).unwrap().count(),
        0,
        "no state was written"
    );
}

#[test]
fn test_params_edit_missing_copy_refused() {
    // A copy whose stored source is gone -> params --resync refuses (exit 1, "no stored copy").
    let sandbox = Sandbox::new();
    sandbox.add_python("a", "CITY = \"x\"\nprint(CITY)\n");
    // delete the stored copy
    let dir = sandbox.data.path().join("scripts").join("a");
    for entry in fs::read_dir(&dir).unwrap() {
        let path = entry.unwrap().path();
        if path.file_name().and_then(|name| name.to_str()) != Some("meta.toml") {
            fs::remove_file(&path).unwrap();
        }
    }
    let meta_before = sandbox.stored_meta("a");
    assert_eq!(fs::read_dir(sandbox.state.path()).unwrap().count(), 0);
    let view = sandbox.command().args(["params", "a"]).output().unwrap();
    assert_eq!(view.status.code(), Some(0), "{}", combined(&view));
    let output = sandbox
        .command()
        .args(["params", "a", "--resync"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1), "{}", combined(&output));
    assert!(
        combined(&output).contains("a has no stored copy to edit."),
        "{}",
        combined(&output)
    );
    assert_eq!(
        sandbox.stored_meta("a"),
        meta_before,
        "metadata is unchanged"
    );
    assert_eq!(
        fs::read_dir(sandbox.state.path()).unwrap().count(),
        0,
        "no state was written"
    );
    let remaining = fs::read_dir(&dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect::<Vec<_>>();
    assert_eq!(
        remaining,
        ["meta.toml"],
        "the missing payload was not recreated"
    );
}
