"""editor module + the `skit edit` / `skit add --edit` (create-in-editor) flows.

Every test mocks the editor launch — no test ever spawns a real editor. The CLI flows patch
cli.editor.open_in_editor (and cli._is_interactive for the interactive gate); the module-level tests
patch editor.subprocess.run.
"""

from __future__ import annotations

from pathlib import Path

import pytest
from typer.testing import CliRunner

from skit import cli, config, editor, store

runner = CliRunner()


@pytest.fixture(autouse=True)
def _english(monkeypatch):
    # Assertions below check English copy; pin the locale so they don't depend on the dev machine's
    # LANG (this repo is developed on a zh-TW box).
    monkeypatch.setenv("SKIT_LANG", "en")


def _py(tmp_path: Path, text: str, name: str = "s.py") -> Path:
    p = tmp_path / name
    p.write_text(text, encoding="utf-8")
    return p


def _boom(*_a, **_k):
    raise AssertionError("the editor must not be launched here")


# --------------------------------------------------------------------------
# editor.resolve_editor — precedence: config > $VISUAL > $EDITOR > platform default
# --------------------------------------------------------------------------


def test_resolve_editor_config_wins_over_env(monkeypatch):
    monkeypatch.setenv("VISUAL", "vim")
    monkeypatch.setenv("EDITOR", "nano")
    config.save_editor("code --wait")
    assert editor.resolve_editor() == ["code", "--wait"]


def test_resolve_editor_visual_over_editor(monkeypatch):
    monkeypatch.setenv("VISUAL", "mvim -f")
    monkeypatch.setenv("EDITOR", "nano")
    assert editor.resolve_editor() == ["mvim", "-f"]


def test_resolve_editor_editor_env_when_no_visual(monkeypatch):
    monkeypatch.delenv("VISUAL", raising=False)
    monkeypatch.setenv("EDITOR", "nano")
    assert editor.resolve_editor() == ["nano"]


def test_resolve_editor_platform_default_unix(monkeypatch):
    monkeypatch.delenv("VISUAL", raising=False)
    monkeypatch.delenv("EDITOR", raising=False)
    monkeypatch.setattr(editor.sys, "platform", "linux")
    assert editor.resolve_editor() == ["vi"]


def test_resolve_editor_platform_default_windows(monkeypatch):
    # Kills the "notepad" / "win32" literals in _platform_default (both branches asserted).
    monkeypatch.delenv("VISUAL", raising=False)
    monkeypatch.delenv("EDITOR", raising=False)
    config.save_editor("")
    monkeypatch.setattr(editor.sys, "platform", "win32")
    assert editor.resolve_editor() == ["notepad"]


def test_resolve_editor_quoted_value_uses_posix_split_off_windows(monkeypatch):
    # A quoted editor path: on non-Windows, shlex uses posix mode and drops the quotes. Kills the
    # `posix=sys.platform != "win32"` -> `== "win32"` mutant (which would flip to non-posix).
    monkeypatch.delenv("VISUAL", raising=False)
    monkeypatch.delenv("EDITOR", raising=False)
    monkeypatch.setattr(editor.sys, "platform", "linux")
    config.save_editor('"/opt/my editor" --wait')
    assert editor.resolve_editor() == ["/opt/my editor", "--wait"]


def test_resolve_editor_quoted_value_non_posix_on_windows(monkeypatch):
    # On Windows the split is non-posix, so quotes/backslashes are kept literally. Kills the
    # win32 case/wrap variants of the posix comparison (they'd flip it back to posix on win32).
    monkeypatch.delenv("VISUAL", raising=False)
    monkeypatch.delenv("EDITOR", raising=False)
    monkeypatch.setattr(editor.sys, "platform", "win32")
    config.save_editor(r"C:\tools\edit.exe --wait")
    assert editor.resolve_editor() == [r"C:\tools\edit.exe", "--wait"]


def test_resolve_editor_quoted_spaced_path_on_windows(monkeypatch):
    # A Windows editor path with a space must be quoted the normal Windows way. The non-posix split
    # keeps the surrounding quotes on the token; those must be stripped so argv[0] is a real,
    # launchable executable path rather than one with literal quote characters in its name.
    monkeypatch.delenv("VISUAL", raising=False)
    monkeypatch.delenv("EDITOR", raising=False)
    monkeypatch.setattr(editor.sys, "platform", "win32")
    config.save_editor(r'"C:\Program Files\Microsoft VS Code\Code.exe" --wait')
    assert editor.resolve_editor() == [
        r"C:\Program Files\Microsoft VS Code\Code.exe",
        "--wait",
    ]


def test_resolve_editor_windows_empty_quoted_token_strips_to_empty(monkeypatch):
    # Boundary for the `len(p) >= 2` guard in the win32 quote-strip: a degenerate empty-quoted
    # token `""` (len 2, first==last=='"') is a matching pair and strips to '' — it must NOT be
    # left as the literal `""`. Pins the len==2 side of the guard so a `>= 2` -> `> 2` mutation
    # (which would leave `""` unstripped) is caught.
    monkeypatch.delenv("VISUAL", raising=False)
    monkeypatch.delenv("EDITOR", raising=False)
    monkeypatch.setattr(editor.sys, "platform", "win32")
    config.save_editor('"" --wait')
    assert editor.resolve_editor() == ["", "--wait"]


def test_resolve_editor_unquoted_windows_path_untouched(monkeypatch):
    # An unquoted (no-space) Windows path has no surrounding quotes to strip; guards the quote-strip
    # logic against eating characters it shouldn't.
    monkeypatch.delenv("VISUAL", raising=False)
    monkeypatch.delenv("EDITOR", raising=False)
    monkeypatch.setattr(editor.sys, "platform", "win32")
    config.save_editor(r"C:\tools\edit.exe --wait")
    assert editor.resolve_editor() == [r"C:\tools\edit.exe", "--wait"]


def test_resolve_editor_whitespace_visual_falls_through_to_editor(monkeypatch):
    # A blank/whitespace-only VISUAL (e.g. a shell profile with `export VISUAL=" "`) must be treated
    # as unset, not as a valid-but-empty override that forces the platform default straight past a
    # perfectly good $EDITOR.
    monkeypatch.setenv("VISUAL", "   ")
    monkeypatch.setenv("EDITOR", "nano")
    assert editor.resolve_editor() == ["nano"]


def test_resolve_editor_whitespace_config_falls_through_to_visual(monkeypatch):
    # Same masking bug, but for a hand-edited config.toml `editor = "  "` instead of $VISUAL.
    config.save_editor("   ")
    monkeypatch.setenv("VISUAL", "mvim -f")
    monkeypatch.setenv("EDITOR", "nano")
    assert editor.resolve_editor() == ["mvim", "-f"]


def test_resolve_editor_all_whitespace_candidates_use_platform_default(monkeypatch):
    # When every candidate is blank/whitespace-only, fall all the way through to the platform
    # default rather than passing a whitespace string to shlex.
    config.save_editor("  ")
    monkeypatch.setenv("VISUAL", " ")
    monkeypatch.setenv("EDITOR", "")
    monkeypatch.setattr(editor.sys, "platform", "linux")
    assert editor.resolve_editor() == ["vi"]


def test_resolve_editor_unbalanced_quotes_falls_back_to_raw(monkeypatch):
    monkeypatch.delenv("VISUAL", raising=False)
    monkeypatch.setenv("EDITOR", 'weird "editor')  # unbalanced quote -> shlex ValueError
    assert editor.resolve_editor() == ['weird "editor']
    # (the `parts or [default]` fallback's `or->and` mutant is killed by the non-empty cases above,
    # which return `parts`, not the platform default.)


# --------------------------------------------------------------------------
# editor.open_in_editor
# --------------------------------------------------------------------------


def test_open_in_editor_appends_path_and_returns_code(monkeypatch, tmp_path):
    captured: dict[str, object] = {}

    class _Result:
        returncode = 0

    def fake_run(argv, check=False):
        captured["argv"] = argv
        captured["check"] = check
        return _Result()

    monkeypatch.setattr(editor.subprocess, "run", fake_run)
    monkeypatch.delenv("VISUAL", raising=False)
    monkeypatch.setenv("EDITOR", "nano")
    f = tmp_path / "x.py"
    assert editor.open_in_editor(f) == 0
    assert captured["argv"] == ["nano", str(f)]
    assert captured["check"] is False


def test_open_in_editor_returns_nonzero_without_raising(monkeypatch, tmp_path):
    class _Result:
        returncode = 3

    monkeypatch.setattr(editor.subprocess, "run", lambda *a, **k: _Result())
    monkeypatch.setenv("EDITOR", "nano")
    assert editor.open_in_editor(tmp_path / "x.py") == 3


def test_open_in_editor_launch_failure_message_exact(monkeypatch, tmp_path):
    def raise_oserror(*_a, **_k):
        raise FileNotFoundError("boom-err")

    monkeypatch.setattr(editor.subprocess, "run", raise_oserror)
    monkeypatch.delenv("VISUAL", raising=False)
    monkeypatch.delenv("EDITOR", raising=False)
    config.save_editor("code --wait")  # two words -> argv[:-1] join separator is exercised
    with pytest.raises(editor.EditorError) as exc_info:
        editor.open_in_editor(tmp_path / "x.py")
    msg = str(exc_info.value)
    # The command (argv minus the path) and the OS error are both reported.
    assert "Could not launch the editor (code --wait): boom-err." in msg
    assert "skit config editor <cmd>" in msg
    assert "XX" not in msg


# --------------------------------------------------------------------------
# config editor read/write
# --------------------------------------------------------------------------


def test_config_editor_roundtrip_and_clear():
    assert config.load_editor() == ""
    config.save_editor("code --wait")
    assert config.load_editor() == "code --wait"
    config.save_editor("")  # empty clears the key
    assert config.load_editor() == ""


def test_save_editor_preserves_other_keys():
    config.save_config({"language": "zh-TW"})
    config.save_editor("nano")
    doc = config.load_config()
    assert doc["language"] == "zh-TW"
    assert doc["editor"] == "nano"


def test_load_editor_non_string_value_is_blank():
    # A hand-edited non-string editor value is treated as unset, not str()-coerced (kills the
    # `else "XXXX"` return mutant).
    config.save_config({"editor": 123})
    assert config.load_editor() == ""


def test_save_editor_clear_when_absent_does_not_raise():
    # Clearing with no editor key present must not raise (kills doc.pop("editor") without a default).
    assert "editor" not in config.load_config()
    config.save_editor("")  # must be a no-op, not a KeyError
    assert config.load_editor() == ""


# --------------------------------------------------------------------------
# skit edit — open an existing script's source
# --------------------------------------------------------------------------


def test_edit_opens_copy_source(monkeypatch, tmp_path):
    opened: dict[str, Path] = {}

    def fake(p):
        opened["path"] = p
        return 0

    monkeypatch.setattr(cli.editor, "open_in_editor", fake)
    store.add_python(_py(tmp_path, "print(1)\n"), name="a")
    result = runner.invoke(cli.app, ["edit", "a"])
    assert result.exit_code == 0, result.output
    assert opened["path"] == store.resolve("a").dir / "script.py"
    assert "Saved a" in result.output


def test_edit_opens_reference_original(monkeypatch, tmp_path):
    src = _py(tmp_path, "print(1)\n", "orig.py")
    store.add_python(src, name="r", mode="reference")
    opened: dict[str, Path] = {}

    def fake(p):
        opened["path"] = p
        return 0

    monkeypatch.setattr(cli.editor, "open_in_editor", fake)
    result = runner.invoke(cli.app, ["edit", "r"])
    assert result.exit_code == 0, result.output
    assert opened["path"] == src.resolve()


def test_edit_reference_source_gone(monkeypatch, tmp_path):
    src = _py(tmp_path, "print(1)\n", "orig.py")
    store.add_python(src, name="r", mode="reference")
    src.unlink()
    monkeypatch.setattr(cli.editor, "open_in_editor", _boom)
    result = runner.invoke(cli.app, ["edit", "r"])
    assert result.exit_code == 1
    assert "gone" in result.output


def test_edit_reports_editor_launch_failure(monkeypatch, tmp_path):
    def fail(_p):
        raise editor.EditorError("could not launch")

    monkeypatch.setattr(cli.editor, "open_in_editor", fail)
    store.add_python(_py(tmp_path, "print(1)\n"), name="a")
    result = runner.invoke(cli.app, ["edit", "a"])
    assert result.exit_code == 1
    assert "could not launch" in result.output


# --------------------------------------------------------------------------
# skit edit <unknown> — offer to create
# --------------------------------------------------------------------------


def test_edit_unknown_confirmed_creates(monkeypatch):
    monkeypatch.setattr(cli, "_is_interactive", lambda: True)
    monkeypatch.setattr(cli.Confirm, "ask", lambda *a, **k: True)

    def write_script(p):
        p.write_text("import requests\nprint('hi')\n", encoding="utf-8")
        return 0

    monkeypatch.setattr(cli.editor, "open_in_editor", write_script)
    result = runner.invoke(cli.app, ["edit", "newscript"])
    assert result.exit_code == 0, result.output
    ent = store.resolve("newscript")
    assert ent.meta.kind == "python"
    assert "requests" in (ent.dir / "script.py").read_text(encoding="utf-8")


def test_edit_unknown_declined_creates_nothing(monkeypatch):
    monkeypatch.setattr(cli, "_is_interactive", lambda: True)
    monkeypatch.setattr(cli.Confirm, "ask", lambda *a, **k: False)
    monkeypatch.setattr(cli.editor, "open_in_editor", _boom)
    result = runner.invoke(cli.app, ["edit", "nope"])
    assert result.exit_code == 0
    with pytest.raises(store.NotFoundError):
        store.resolve("nope")


def test_edit_unknown_non_interactive_errors(monkeypatch):
    monkeypatch.setattr(cli, "_is_interactive", lambda: False)
    monkeypatch.setattr(cli.editor, "open_in_editor", _boom)
    result = runner.invoke(cli.app, ["edit", "ghost"])
    assert result.exit_code == 1


# --------------------------------------------------------------------------
# skit add --edit / -e — create a brand-new script in the editor
# --------------------------------------------------------------------------


def test_add_edit_creates_in_editor(monkeypatch):
    monkeypatch.setattr(cli, "_is_interactive", lambda: True)

    def write_script(p):
        p.write_text("import rich\nprint('x')\n", encoding="utf-8")
        return 0

    monkeypatch.setattr(cli.editor, "open_in_editor", write_script)
    result = runner.invoke(cli.app, ["add", "-e", "--name", "fresh"])
    assert result.exit_code == 0, result.output
    ent = store.resolve("fresh")
    assert ent.meta.kind == "python"
    assert "rich" in (ent.dir / "script.py").read_text(encoding="utf-8")


def test_add_edit_bash_shebang_draft_becomes_a_shell_entry(monkeypatch):
    """A changed shebang in the draft is honored exactly like the TUI draft lane: writing a
    #!/usr/bin/env bash body makes the entry SHELL via store.add_script, never a broken
    python entry with a bash body (finding 7)."""
    monkeypatch.setattr(cli, "_is_interactive", lambda: True)

    def write_script(p):
        p.write_text("#!/usr/bin/env bash\n# Ship it\necho drafted\n", encoding="utf-8")
        return 0

    monkeypatch.setattr(cli.editor, "open_in_editor", write_script)
    result = runner.invoke(cli.app, ["add", "-e", "--name", "deploy"])
    assert result.exit_code == 0, result.output
    entry = store.resolve("deploy")
    assert entry.meta.kind == "shell"  # re-inferred from the shebang, not the .py temp suffix
    assert "echo drafted" in entry.script_path.read_text(encoding="utf-8")


def test_add_edit_js_shebang_draft_scans_npm_deps(monkeypatch):
    """A node-shebang draft lands as a js entry and its declared npm imports are scanned
    into the entry's dependencies (the deps branch of the changed-shebang lane, finding 7)."""
    monkeypatch.setattr(cli, "_is_interactive", lambda: True)

    def write_script(p):
        p.write_text(
            "#!/usr/bin/env node\nimport chalk from 'chalk'\nconsole.log(chalk)\n", encoding="utf-8"
        )
        return 0

    monkeypatch.setattr(cli.editor, "open_in_editor", write_script)
    result = runner.invoke(cli.app, ["add", "-e", "--name", "colorized"])
    assert result.exit_code == 0, result.output
    entry = store.resolve("colorized")
    assert entry.meta.kind == "js"
    assert "chalk" in (entry.meta.dependencies or [])  # npm scan materialized the dep


def test_add_edit_rejects_path(tmp_path):
    p = _py(tmp_path, "print(1)\n")
    result = runner.invoke(cli.app, ["add", "-e", str(p)])
    assert result.exit_code == 2


def test_add_edit_non_interactive_errors(monkeypatch):
    monkeypatch.setattr(cli, "_is_interactive", lambda: False)
    monkeypatch.setattr(cli.editor, "open_in_editor", _boom)
    result = runner.invoke(cli.app, ["add", "-e", "--name", "x"])
    assert result.exit_code == 2


def test_add_edit_empty_content_adds_nothing(monkeypatch):
    monkeypatch.setattr(cli, "_is_interactive", lambda: True)
    monkeypatch.setattr(cli.editor, "open_in_editor", lambda p: 0)  # leaves the starter unchanged
    result = runner.invoke(cli.app, ["add", "-e", "--name", "ghost"])
    assert result.exit_code == 0, result.output
    assert "Nothing was written" in result.output
    with pytest.raises(store.NotFoundError):
        store.resolve("ghost")


def test_add_edit_prompts_for_name_when_omitted(monkeypatch):
    monkeypatch.setattr(cli, "_is_interactive", lambda: True)
    monkeypatch.setattr(cli.Prompt, "ask", lambda *a, **k: "prompted")

    def write_script(p):
        p.write_text("print('x')\n", encoding="utf-8")
        return 0

    monkeypatch.setattr(cli.editor, "open_in_editor", write_script)
    result = runner.invoke(cli.app, ["add", "-e"])
    assert result.exit_code == 0, result.output
    assert store.resolve("prompted").meta.kind == "python"


def test_add_edit_blank_name_errors(monkeypatch):
    monkeypatch.setattr(cli, "_is_interactive", lambda: True)
    monkeypatch.setattr(cli.Prompt, "ask", lambda *a, **k: "   ")  # whitespace -> no name
    monkeypatch.setattr(cli.editor, "open_in_editor", _boom)
    result = runner.invoke(cli.app, ["add", "-e"])
    assert result.exit_code == 2


def test_add_edit_editor_error_exits_one(monkeypatch):
    monkeypatch.setattr(cli, "_is_interactive", lambda: True)

    def fail(_p):
        raise editor.EditorError("cannot launch")

    monkeypatch.setattr(cli.editor, "open_in_editor", fail)
    result = runner.invoke(cli.app, ["add", "-e", "--name", "x"])
    assert result.exit_code == 1
    assert "cannot launch" in result.output


def test_add_edit_name_conflict_exits_one(monkeypatch, tmp_path):
    store.add_python(_py(tmp_path, "print(1)\n"), name="dup")
    monkeypatch.setattr(cli, "_is_interactive", lambda: True)

    def write_script(p):
        p.write_text("print('x')\n", encoding="utf-8")
        return 0

    monkeypatch.setattr(cli.editor, "open_in_editor", write_script)
    result = runner.invoke(cli.app, ["add", "-e", "--name", "dup"])
    assert result.exit_code == 1  # store.NameConflictError is a StoreError
    assert "dup" in result.output  # the name
    assert "taken" in result.output  # the StoreError is surfaced


def test_add_edit_writes_and_reports_managed_and_secret(monkeypatch, tmp_path):
    from skit.params import ParamDecl

    monkeypatch.setattr(cli, "_is_interactive", lambda: True)
    monkeypatch.setattr(
        cli,
        "_onboard_params",
        lambda text, name, no_input: [
            ParamDecl(name="API", binding="const", type="str", default="x", secret=True)
        ],
    )

    def write_script(p):
        p.write_text("API = 'x'\nprint(API)\n", encoding="utf-8")
        return 0

    monkeypatch.setattr(cli.editor, "open_in_editor", write_script)
    result = runner.invoke(cli.app, ["add", "-e", "--name", "fresh"])
    assert result.exit_code == 0, result.output
    # kills _print_add_summary(entry, deps, None, secrets) / (.., None) at the create call site
    assert "Managed parameters: API" in result.output
    assert "Secret parameter values are never saved to disk: API" in result.output


def test_params_edit_command_entry_refused():
    store.add_command("echo {x}", name="ec")
    result = runner.invoke(cli.app, ["params", "ec", "--resync"])
    assert result.exit_code == 1


def test_params_edit_missing_copy_refused(tmp_path):
    ent = store.add_python(_py(tmp_path, 'CITY = "x"\nprint(CITY)\n'), name="a")
    ent.script_path.unlink()
    result = runner.invoke(cli.app, ["params", "a", "--resync"])
    assert result.exit_code == 1
    assert "no stored copy" in result.output
