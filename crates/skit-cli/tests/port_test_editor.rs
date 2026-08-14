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
//! - ABSENT (kind=absent): the `editor` module's public functions `resolve_editor`,
//!   `open_in_editor`, `open_entry_in_editor` have NO public equivalent on the Rust surface. The
//!   resolution + launch logic is inlined (and duplicated) privately inside skit-cli's
//!   `edit_with_config` / `open_editor_in` (`crates/skit-cli/src/cli.rs:~3283`, `~3344`), reachable
//!   only through a whole CLI flow, never as a unit. A `resolve_editor` case whose divergence is
//!   observable end to end (no platform default; a blank `$VISUAL` used as-is instead of skipped;
//!   an unbalanced-quote value hard-errored instead of falling back to raw) is driven through the
//!   real `edit` lane with a fake editor on `PATH` and recorded as a full FAILING CONTRACT body
//!   below — not left an ABSENT stub. The purely unit-level cases (the win32 non-posix split and
//!   quote-strip; `open_in_editor` argv/returncode/message; `open_entry_in_editor` re-read) have no
//!   end-to-end seam and stay compiling `#[ignore]` stubs keeping the Python body as a comment plus
//!   a MUST-FIX trailhead.
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

/// stdout followed by stderr — the oracle's `CliRunner` merges both into `result.output`.
fn combined(output: &std::process::Output) -> String {
    let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&output.stderr));
    text
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

// ==========================================================================
// editor.resolve_editor — ABSENT (kind=absent): no public resolve_editor.
//
// The precedence chain (config > $VISUAL > $EDITOR > platform default) and the shlex split live
// inline and duplicated inside `edit_with_config` (cli.rs:~3283) and `open_editor_in` (cli.rs:~3344),
// private to the binary and reachable only by running a whole `edit`/`add -e` flow. There is no
// `resolve_editor(...) -> Vec<String>` to call, so these unit-level defs cannot compile against the
// Rust surface. MUST-FIX to make them assertable: expose one pure `resolve_editor` in a shared crate
// (Python `src/skit/editor.py:34-60`). The inline Rust logic also DIVERGES from the oracle on
// several of these — noted per stub — so the trailhead is exact.
// ==========================================================================

#[test]
#[ignore = "ABSENT (kind=absent): no public resolve_editor; logic inlined in cli.rs edit_with_config/open_editor_in. MUST-FIX: src/skit/editor.py:34-60 (precedence config>VISUAL>EDITOR)."]
fn test_resolve_editor_config_wins_over_env() {
    // The config.toml `editor` outranks $VISUAL and $EDITOR.
    //   setenv VISUAL=vim; setenv EDITOR=nano; config.save_editor("code --wait")
    //   assert resolve_editor() == ["code", "--wait"]
    // Rust: edit_with_config reads config editor first (matches), but as an inline flow only.
}

#[test]
#[ignore = "ABSENT (kind=absent): no public resolve_editor. MUST-FIX: src/skit/editor.py:45-48 ($VISUAL over $EDITOR + shlex split)."]
fn test_resolve_editor_visual_over_editor() {
    // $VISUAL outranks $EDITOR and is shlex-split.
    //   setenv VISUAL="mvim -f"; setenv EDITOR=nano
    //   assert resolve_editor() == ["mvim", "-f"]
    // Rust DIVERGES: env resolution is env::var("VISUAL").or_else(|_| env::var("EDITOR")), so the
    // precedence holds, but there is no unit surface to observe it.
}

#[test]
#[ignore = "ABSENT (kind=absent): no public resolve_editor. MUST-FIX: src/skit/editor.py:45 ($EDITOR when $VISUAL unset)."]
fn test_resolve_editor_editor_env_when_no_visual() {
    // With $VISUAL unset, $EDITOR is used.
    //   delenv VISUAL; setenv EDITOR=nano
    //   assert resolve_editor() == ["nano"]
}

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

#[test]
#[ignore = "ABSENT (kind=absent): no public resolve_editor; Rust has no platform default and no win32 branch. MUST-FIX: src/skit/editor.py:30-31 (notepad on win32)."]
fn test_resolve_editor_platform_default_windows() {
    // Nothing configured, Windows -> the platform default is "notepad".
    //   delenv VISUAL; delenv EDITOR; config.save_editor(""); sys.platform = "win32"
    //   assert resolve_editor() == ["notepad"]
}

#[test]
#[ignore = "ABSENT (kind=absent): no public resolve_editor. MUST-FIX: src/skit/editor.py:48 (posix shlex off Windows drops quotes)."]
fn test_resolve_editor_quoted_value_uses_posix_split_off_windows() {
    // Off Windows, shlex uses posix mode and drops the surrounding quotes.
    //   sys.platform = "linux"; config.save_editor('"/opt/my editor" --wait')
    //   assert resolve_editor() == ["/opt/my editor", "--wait"]
    // Rust always splits posix (shlex::split), so this direction happens to agree — but no surface.
}

#[test]
#[ignore = "ABSENT (kind=absent): no public resolve_editor, and Rust DIVERGES — it always uses posix shlex::split, never the win32 non-posix split. MUST-FIX: src/skit/editor.py:48 (posix=sys.platform!='win32')."]
fn test_resolve_editor_quoted_value_non_posix_on_windows() {
    // On Windows the split is non-posix, so backslashes are kept literally.
    //   sys.platform = "win32"; config.save_editor(r"C:\\tools\\edit.exe --wait")
    //   assert resolve_editor() == [r"C:\\tools\\edit.exe", "--wait"]
}

#[test]
#[ignore = "ABSENT (kind=absent): no public resolve_editor; Rust has no win32 quote-strip. MUST-FIX: src/skit/editor.py:53-59 (strip one surrounding quote pair per token on win32)."]
fn test_resolve_editor_quoted_spaced_path_on_windows() {
    // A quoted spaced Windows path: the non-posix split keeps the quotes; skit strips one pair.
    //   sys.platform = "win32"; config.save_editor(r'"C:\\Program Files\\...\\Code.exe" --wait')
    //   assert resolve_editor() == [r"C:\\Program Files\\...\\Code.exe", "--wait"]
}

#[test]
#[ignore = "ABSENT (kind=absent): no public resolve_editor; Rust has no win32 quote-strip. MUST-FIX: src/skit/editor.py:59 (len(p) >= 2 empty-quoted-pair boundary)."]
fn test_resolve_editor_windows_empty_quoted_token_strips_to_empty() {
    // The degenerate `""` token (len 2, matching pair) strips to '' on win32, not left literal.
    //   sys.platform = "win32"; config.save_editor('"" --wait')
    //   assert resolve_editor() == ["", "--wait"]
}

#[test]
#[ignore = "ABSENT (kind=absent): no public resolve_editor. MUST-FIX: src/skit/editor.py:59 (unquoted token untouched)."]
fn test_resolve_editor_unquoted_windows_path_untouched() {
    // An unquoted (no-space) Windows path has no surrounding quotes to strip.
    //   sys.platform = "win32"; config.save_editor(r"C:\\tools\\edit.exe --wait")
    //   assert resolve_editor() == [r"C:\\tools\\edit.exe", "--wait"]
}

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
    let output = sandbox
        .command()
        .env("EDITOR", &editor)
        .args(["add", "-e", "--name", "fresh"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0), "{}", combined(&output));
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
    let output = sandbox
        .command()
        .env("EDITOR", &editor)
        .args(["add", "-e", "--name", "deploy"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0), "{}", combined(&output));
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
    let output = sandbox
        .command()
        .env("EDITOR", &editor)
        .args(["add", "-e", "--name", "colorized"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0), "{}", combined(&output));
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
    sandbox
        .command()
        .env("EDITOR", &editor)
        .args(["add", "-e", "--name", "zjob"])
        .assert()
        .success();
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
    let output = sandbox
        .command()
        .env("EDITOR", &editor)
        .args(["add", "-e", "--name", "d", "--dep", "rich"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2), "{}", combined(&output));
    assert!(combined(&output).contains("python flags")); // the refusal names the flags…
    assert!(combined(&output).contains("shell")); // …and the draft's actual kind
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
    let output = sandbox
        .command()
        .env("EDITOR", &editor)
        .args(["add", "-e", "--name", "keptpy"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1), "{}", combined(&output));
    assert!(combined(&output).contains("Your draft was kept at"));
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
#[ignore = "FAILING CONTRACT (divergence): Rust add -e has no interactivity gate; launches the editor and creates the entry non-interactively"]
fn test_add_edit_non_interactive_errors() {
    // A non-interactive `add -e` (piped, non-tty stdin/stdout) must refuse (exit 2) and NEVER launch
    // the editor — the oracle gates on `_is_interactive() == False`. `.output()` gives a non-tty
    // stdin and stdout (the Rust gate shape is `is_terminal() && is_terminal()`); no `--no-input` is
    // used, so this exercises the tty gate, not the flag. The editor writes a valid draft, so a Rust
    // that has NO gate runs the whole lane and exits 0 with the entry created (exit-2 fails first,
    // its message carrying the "Added" evidence); the editor sentinel then also proves the launch.
    let sandbox = Sandbox::new();
    let sentinel = sandbox.scratch.path().join("x.launched");
    let editor = writing_editor(sandbox.scratch.path(), "x", "print('x')\n", &sentinel);
    let output = sandbox
        .command()
        .env("EDITOR", &editor)
        .args(["add", "-e", "--name", "x"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2), "{}", combined(&output));
    assert!(
        !sentinel.exists(),
        "the editor must not be launched non-interactively"
    );
}

#[test]
#[ignore = "FAILING CONTRACT (divergence): an untouched draft makes Rust error 'the draft is empty and was kept at …' with exit 2 (cli.rs:1416-1419); the oracle exits 0 with 'Nothing was written, so nothing was added.' and adds nothing. Verified against the built binary."]
fn test_add_edit_empty_content_adds_nothing() {
    // Leaving the starter unchanged -> exit 0, "Nothing was written", nothing added.
    let sandbox = Sandbox::new();
    let output = sandbox
        .command()
        .env("EDITOR", "/bin/true")
        .args(["add", "-e", "--name", "ghost"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0));
    assert!(combined(&output).contains("Nothing was written"));
    assert!(!sandbox.resolvable("ghost"));
}

#[test]
#[ignore = "FAILING CONTRACT (divergence): an unregistered shebang (awk) is inferred as python (infer_kind fallback, cli.rs:1425-1429) and ADDED as a copy with exit 0; the oracle refuses (exit 2, 'names no interpreter skit knows', --kind escape), keeps the draft under data_dir/drafts/, and fabricates nothing. Verified against the built binary."]
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
    let output = sandbox
        .command()
        .env("EDITOR", &editor)
        .args(["add", "-e", "--name", "aw"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    let text = combined(&output);
    assert!(text.contains("names no interpreter skit knows"));
    assert!(text.contains("--kind")); // the escape hatch
    assert!(!sandbox.draft_files().is_empty()); // the draft survived
    assert!(!sandbox.resolvable("aw")); // nothing fabricated
}

#[test]
#[ignore = "FAILING CONTRACT (divergence): the untouched starter is KEPT (Rust errors 'the draft is empty and was kept at …', exit 2) rather than unlinked and reported as 'Nothing was written' with exit 0. Verified against the built binary."]
fn test_add_edit_untouched_starter_unlinks_the_draft() {
    // The untouched-starter cancel is pure litter -> unlink it; the temp is gone after "Nothing was
    // written".
    let sandbox = Sandbox::new();
    let output = sandbox
        .command()
        .env("EDITOR", "/bin/true")
        .args(["add", "-e", "--name", "ghost"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0));
    assert!(combined(&output).contains("Nothing was written"));
    assert!(
        sandbox.draft_files().is_empty(),
        "the litter was cleaned up"
    );
}

#[test]
#[ignore = "FAILING CONTRACT (divergence): this non-tty harness takes the prompt stdin lane and now correctly refuses its empty pipe with exit 1 ('Nothing arrived on stdin'); the oracle forces the interactive editor lane, unlinks its untouched starter, reports 'Nothing was written', and exits 0. A black-box non-tty test cannot drive that editor-only observable."]
fn test_add_prompt_editor_untouched_starter_unlinks_the_draft() {
    // Same for the prompt editor lane: an untouched starter is unlinked, not left behind.
    let sandbox = Sandbox::new();
    let output = sandbox
        .command()
        .env("EDITOR", "/bin/true")
        .args(["add", "--prompt", "--name", "ghostp"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0));
    assert!(combined(&output).contains("Nothing was written"));
    assert!(!sandbox.resolvable("ghostp"));
}

#[test]
#[ignore = "FAILING CONTRACT (divergence): the -e lane never prompts for a name — with --name omitted Rust auto-derives a machine name from the draft filename (skit-<id>) and exits 0 (add() name derivation), whereas the oracle prompts and uses the answer. Verified against the built binary (name 'skit-<uuid>')."]
fn test_add_edit_prompts_for_name_when_omitted() {
    // Omitting --name -> the user is prompted and the answer becomes the name.
    let sandbox = Sandbox::new();
    let sentinel = sandbox.scratch.path().join("noname.launched");
    let editor = writing_editor(sandbox.scratch.path(), "noname", "print('x')\n", &sentinel);
    sandbox
        .command()
        .env("EDITOR", &editor)
        .args(["add", "-e"])
        .assert()
        .success();
    // The created entry's name must come from a prompt, not be auto-derived from the draft file.
    let scripts = fs::read_dir(sandbox.data.path().join("scripts")).unwrap();
    let names: Vec<String> = scripts
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(names.len(), 1, "{names:?}");
    assert!(
        !names[0].starts_with("skit-"),
        "the name must be prompted, not auto-derived: {names:?}"
    );
}

#[test]
#[ignore = "FAILING CONTRACT (divergence): the -e lane has no interactive name prompt, so a missing/blank name is never refused — Rust auto-derives a name and exits 0, whereas the oracle exits 2 on a whitespace-only prompted name. Verified against the built binary."]
fn test_add_edit_blank_name_errors() {
    // A whitespace-only prompted name -> no name -> exit 2.
    let sandbox = Sandbox::new();
    let sentinel = sandbox.scratch.path().join("blank.launched");
    let editor = writing_editor(sandbox.scratch.path(), "blank", "print('x')\n", &sentinel);
    let output = sandbox
        .command()
        .env("EDITOR", &editor)
        .args(["add", "-e"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
}

#[test]
fn test_add_edit_editor_error_exits_one() {
    // An editor that cannot launch -> exit 1.
    let sandbox = Sandbox::new();
    let output = sandbox
        .command()
        .env("EDITOR", "/nonexistent/skit-no-such-editor")
        .args(["add", "-e", "--name", "x"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1), "{}", combined(&output));
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
    let output = sandbox
        .command()
        .env("EDITOR", &editor)
        .args(["add", "-e", "--name", "fresh"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0), "{}", combined(&output));
    let text = combined(&output);
    assert!(text.contains("Managed parameters: API"), "{text}");
    assert!(
        text.contains("Secret parameter values are never saved by skit: API"),
        "{text}"
    );
}

// ==========================================================================
// skit params — source-management refusals.
// ==========================================================================

#[test]
#[ignore = "FAILING CONTRACT (divergence): `params <command> --resync` returns CliError::Usage 'source management applies only to a stored copy' -> exit 2; the oracle treats it as a failed operation, exit 1 (src/skit/cli.py params). Verified against the built binary. Same exit-1-vs-2 family as the completed source-managed-params fix (task #13)."]
fn test_params_edit_command_entry_refused() {
    // A command entry has no editable source -> params source-edit is refused (exit 1).
    let sandbox = Sandbox::new();
    sandbox.add_command("echo {x}", "ec");
    let output = sandbox
        .command()
        .args(["params", "ec", "--resync"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1), "{}", combined(&output));
}

#[test]
#[ignore = "FAILING CONTRACT (divergence): with the stored copy deleted, Rust reads an empty source, resyncs it (a no-op) and exits 0; the oracle refuses with exit 1 and 'no stored copy'. Verified against the built binary."]
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
    let output = sandbox
        .command()
        .args(["params", "a", "--resync"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1), "{}", combined(&output));
    assert!(combined(&output).contains("no stored copy"));
}
