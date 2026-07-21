"""CLI end-to-end (Typer CliRunner) + direct unit tests for interactive helper functions.

- Command layer uses CliRunner: the non-interactive path is the default (CliRunner's stdin is
  not a tty).
- Interactive branches (Prompt.ask / isatty True) are tested by calling the module functions
  directly with stubs, because CliRunner replaces sys.stdin during invoke and cannot reliably
  inject a tty.
- Assertions are behavioural (exit code, on-disk results, args passed to launcher), not tied
  to any locale copy.
"""

from __future__ import annotations

import dataclasses
import os
import sys
from pathlib import Path

import pytest
from rich.markup import escape
from typer.testing import CliRunner

from skit import (
    analysis,
    argstate,
    cli,
    config,
    flows,
    launcher,
    promptform,
    store,
)
from skit.langs.python import analyzer, metawriter, reconcile, shim
from skit.params import ParamDecl
from skit.paths import values_dir

runner = CliRunner()


@pytest.fixture(autouse=True)
def tmp_store(tmp_path: Path, monkeypatch: pytest.MonkeyPatch):
    monkeypatch.setenv("SKIT_DATA_DIR", str(tmp_path / "data"))
    monkeypatch.setenv("SKIT_STATE_DIR", str(tmp_path / "state"))
    monkeypatch.setenv("SKIT_CONFIG_DIR", str(tmp_path / "config"))
    monkeypatch.setenv("SKIT_LANG", "en")
    return tmp_path


def _py(tmp_path: Path, body: str, name: str = "job.py") -> Path:
    p = tmp_path / name
    p.write_text(body, encoding="utf-8")
    return p


# --------------------------------------------------------------------------
# main callback
# --------------------------------------------------------------------------


def test_version_flag_prints_and_exits():
    from skit import __version__

    result = runner.invoke(cli.app, ["--version"])
    assert result.exit_code == 0
    assert __version__ in result.output


def test_no_subcommand_dispatches_to_tui(monkeypatch):
    called = {}

    def fake_menu() -> int:
        called["ran"] = True
        return 0

    # cli imports run_menu inside the callback with `from .tui import run_menu`; patching the
    # module attribute is sufficient.
    monkeypatch.setattr("skit.tui.run_menu", fake_menu)
    result = runner.invoke(cli.app, [])
    assert result.exit_code == 0
    assert called.get("ran") is True


# --------------------------------------------------------------------------
# add
# --------------------------------------------------------------------------


def test_add_python_copy(tmp_path):
    p = _py(tmp_path, "print(1)\n")
    result = runner.invoke(cli.app, ["add", str(p), "--name", "hi"])
    assert result.exit_code == 0, result.output
    assert store.resolve("hi").meta.mode == "copy"


def test_add_interactive_tui_form_opens_review_panel(tmp_path, monkeypatch):
    """In a real terminal with form=tui, `skit add x.py` hosts the SAME review panel
    the TUI's `a` opens; the flags ride along as prefills and the panel's slug feeds
    the printed summary. (Lazy import — patch the attribute on skit.tui_add.)"""
    p = _py(tmp_path, "print(1)\n")
    seen: dict[str, object] = {}

    def fake_panel(path, **kwargs):
        seen["path"] = path
        seen.update(kwargs)
        return store.add_python(path, name="panelled").slug

    monkeypatch.setattr(cli, "_is_interactive", lambda: True)
    monkeypatch.setattr("skit.tui_add.run_add_review", fake_panel)
    result = runner.invoke(cli.app, ["add", str(p), "--name", "hint", "--ref"])
    assert result.exit_code == 0, result.output
    assert seen["name"] == "hint"
    assert seen["reference"] is True
    assert "panelled" in result.output  # the summary reflects the panel's entry


def test_add_interactive_panel_cancel_exits_130(tmp_path, monkeypatch):
    """Esc in the panel = the form-cancel contract: exit 130, nothing added."""
    p = _py(tmp_path, "print(1)\n")
    monkeypatch.setattr(cli, "_is_interactive", lambda: True)
    monkeypatch.setattr("skit.tui_add.run_add_review", lambda path, **kw: None)
    result = runner.invoke(cli.app, ["add", str(p)])
    assert result.exit_code == 130
    assert "Cancelled" in result.output
    assert store.list_entries() == []


def test_add_interactive_plain_form_keeps_line_prompts(tmp_path, monkeypatch):
    """form=plain opts out of the panel — the line-prompt path runs instead."""
    p = _py(tmp_path, "print(1)\n")
    config.save_form("plain")
    monkeypatch.setattr(cli, "_is_interactive", lambda: True)
    hit = {}
    monkeypatch.setattr("skit.tui_add.run_add_review", lambda *a, **kw: hit.setdefault("panel", 1))
    result = runner.invoke(cli.app, ["add", str(p), "--name", "plainly"], input="\n")
    assert result.exit_code == 0, result.output
    assert "Description (optional)" in result.output
    assert "panel" not in hit
    assert store.resolve("plainly").meta.mode == "copy"


def test_add_term_dumb_keeps_line_prompts(tmp_path, monkeypatch):
    """TERM=dumb can't host a Textual panel — same opt-out as the run form."""
    p = _py(tmp_path, "print(1)\n")
    monkeypatch.setenv("TERM", "dumb")
    monkeypatch.setattr(cli, "_is_interactive", lambda: True)
    hit = {}
    monkeypatch.setattr("skit.tui_add.run_add_review", lambda *a, **kw: hit.setdefault("panel", 1))
    result = runner.invoke(cli.app, ["add", str(p), "--name", "dumbly"], input="\n")
    assert result.exit_code == 0, result.output
    assert "Description (optional)" in result.output
    assert "panel" not in hit
    assert store.resolve("dumbly").meta.mode == "copy"


def test_add_python_reference_skips_onboarding(tmp_path):
    p = _py(tmp_path, 'CITY = "x"\nprint(CITY)\n')
    result = runner.invoke(cli.app, ["add", str(p), "--name", "ref", "--ref"])
    assert result.exit_code == 0, result.output
    assert store.resolve("ref").meta.mode == "reference"


def test_add_rejects_non_py(tmp_path):
    p = _py(tmp_path, "data", name="notes.txt")
    result = runner.invoke(cli.app, ["add", str(p)])
    assert result.exit_code == 2
    # Lead with the extensionless-script escape hatch: a file
    # skit can't classify might still be a real script that just lacks an extension.
    flat = " ".join(result.output.split())
    assert "pass --kind <language> for an extensionless script" in flat


def test_add_needs_path():
    result = runner.invoke(cli.app, ["add"])
    assert result.exit_code == 2


def test_add_exe_needs_path():
    result = runner.invoke(cli.app, ["add", "--exe"])
    assert result.exit_code == 2


def test_add_exe(tmp_path):
    exe = tmp_path / "tool"
    exe.write_text("#!/bin/sh\necho hi\n", encoding="utf-8")
    result = runner.invoke(cli.app, ["add", str(exe), "--exe", "--name", "tool"])
    assert result.exit_code == 0, result.output
    assert store.resolve("tool").meta.kind == "exe"


def test_add_exe_interactive_line_asks_name_and_description(tmp_path, monkeypatch):
    """The exe add lane no longer asks NOTHING while every sibling reviews identity: in a
    terminal it line-asks the name (default: the file stem) and a description."""
    exe = tmp_path / "backup"
    exe.write_text("#!/bin/sh\necho hi\n", encoding="utf-8")
    monkeypatch.setattr(cli, "_is_interactive", lambda: True)
    asked: list[str] = []

    def fake_ask(prompt, **kwargs):
        asked.append(str(prompt))
        return {"Name in skit": "nightly", "Description (optional)": "runs the backup"}.get(
            str(prompt), kwargs.get("default", "")
        )

    monkeypatch.setattr(cli.Prompt, "ask", fake_ask)
    result = runner.invoke(cli.app, ["add", str(exe), "--exe"])
    assert result.exit_code == 0, result.output
    assert any("Name in skit" in a for a in asked)
    assert any("Description (optional)" in a for a in asked)
    entry = store.resolve("nightly")
    assert entry.meta.kind == "exe"
    assert entry.meta.description == "runs the backup"


def test_add_exe_interactive_skips_asks_when_name_and_description_given(tmp_path, monkeypatch):
    """Interactive, but --name and --description already supplied: each ask is skipped (a
    flag already answered it), so no line prompt fires and both flags stand."""

    def _boom(*a, **k):
        raise AssertionError("no ask should fire when the flag already provided the value")

    monkeypatch.setattr(cli, "_is_interactive", lambda: True)
    monkeypatch.setattr(cli.Prompt, "ask", _boom)
    exe = tmp_path / "backup"
    exe.write_text("#!/bin/sh\necho hi\n", encoding="utf-8")
    result = runner.invoke(
        cli.app, ["add", str(exe), "--exe", "--name", "given", "--description", "prewritten"]
    )
    assert result.exit_code == 0, result.output
    entry = store.resolve("given")
    assert entry.meta.kind == "exe"
    assert entry.meta.description == "prewritten"


def test_add_exe_no_input_never_asks(tmp_path, monkeypatch):
    """--no-input keeps the deterministic contract: no line prompts at all (a pipe/CI run
    must never block on Prompt.ask), so the file stem becomes the name."""

    def _boom(*a, **k):
        raise AssertionError("Prompt.ask must not run under --no-input")

    monkeypatch.setattr(cli.Prompt, "ask", _boom)
    exe = tmp_path / "archiver"
    exe.write_text("#!/bin/sh\necho hi\n", encoding="utf-8")
    result = runner.invoke(cli.app, ["add", str(exe), "--exe", "--no-input"])
    assert result.exit_code == 0, result.output
    assert store.resolve("archiver").meta.kind == "exe"  # stem became the name, no ask


def test_add_exe_missing_path_errors_before_any_ask(tmp_path, monkeypatch):
    """The exe existence check is hoisted BEFORE the identity asks: adding a missing path
    with --exe interactively asks NOTHING (no name/description prompt lands, then a late
    "File not found") and errors exit 1 — the ordering the prompt lane's _require_file
    discipline forbids."""

    def _boom(*a, **k):
        raise AssertionError("the identity asks must not run before the existence check")

    monkeypatch.setattr(cli, "_is_interactive", lambda: True)
    monkeypatch.setattr(cli.Prompt, "ask", _boom)
    missing = tmp_path / "ghost.bin"  # never created
    result = runner.invoke(cli.app, ["add", str(missing), "--exe"])
    assert result.exit_code == 1, result.output
    assert "File not found" in result.output


def test_add_cmd_needs_name():
    result = runner.invoke(cli.app, ["add", "--cmd", "echo hi"])
    assert result.exit_code == 2


def test_add_cmd_with_params(tmp_path):
    result = runner.invoke(cli.app, ["add", "--cmd", "echo {msg}", "--name", "e"])
    assert result.exit_code == 0, result.output
    assert store.resolve("e").meta.params == ["msg"]


def test_add_with_explicit_deps_records(tmp_path):
    p = _py(tmp_path, "import requests\nprint(requests)\n")
    result = runner.invoke(
        cli.app,
        ["add", str(p), "--name", "r", "--dep", "requests", "--dep", "rich", "--no-input"],
    )
    assert result.exit_code == 0, result.output


def test_add_name_conflict_errors(tmp_path):
    p = _py(tmp_path, "print(1)\n")
    runner.invoke(cli.app, ["add", str(p), "--name", "dup"])
    result = runner.invoke(cli.app, ["add", str(p), "--name", "dup"])
    assert result.exit_code == 1


def test_add_missing_path_clean_error_not_traceback(tmp_path):
    # Regression: the read used to happen inside a try that only caught store.StoreError, so a
    # missing path's FileNotFoundError escaped as a bare traceback instead of a clean message.
    missing = tmp_path / "typo" / "path.py"
    result = runner.invoke(cli.app, ["add", str(missing)])
    assert result.exit_code == 1
    assert result.exception is None or isinstance(result.exception, SystemExit)
    assert "File not found" in result.output


def test_add_directory_path_clean_error_not_traceback(tmp_path):
    # A directory is present but is not an acceptable source file. Report that truthfully,
    # without letting read_text raise a traceback or claiming the path is missing.
    d = tmp_path / "adir.py"
    d.mkdir()
    result = runner.invoke(cli.app, ["add", str(d)])
    assert result.exit_code == 1
    assert result.exception is None or isinstance(result.exception, SystemExit)
    assert "Not a file" in result.output


@pytest.mark.skipif(sys.platform == "win32", reason="POSIX permission bits")
@pytest.mark.skipif(hasattr(os, "geteuid") and os.geteuid() == 0, reason="root bypasses file perms")
def test_add_unreadable_file_clean_error_not_traceback(tmp_path):
    # An existing-but-unreadable file raises PermissionError from read_text; also must be reported
    # cleanly (a distinct message from "File not found", since the path does exist).
    p = _py(tmp_path, "print(1)\n")
    p.chmod(0o000)
    try:
        result = runner.invoke(cli.app, ["add", str(p)])
    finally:
        p.chmod(0o644)  # restore so tmp_path cleanup can remove it
    assert result.exit_code == 1
    assert result.exception is None or isinstance(result.exception, SystemExit)
    assert "Can't read" in result.output


def test_add_read_error_reports_clean_message(tmp_path, monkeypatch):
    # The permission-based test above can only run as a non-root user (root reads through
    # chmod 0o000), so under a root euid — CI-in-Docker, this container — it is skipped and
    # cli.py's `except OSError` read guard goes uncovered, dropping the suite below the 100%
    # floor. Inject the OSError directly so the clean-error branch is exercised regardless of
    # euid: a mid-add read failure (a race unlinking the file, transient I/O) must still surface
    # as a localized "Can't read" message, never a traceback.
    p = _py(tmp_path, "print(1)\n")
    target = p.resolve()
    real_read_text = Path.read_text

    # Signature mirrors Path.read_text (every parameter is str | None across 3.12-3.13, incl.
    # 3.13's `newline`), so non-target reads delegate cleanly and the types check under `ty`.
    def failing_read_text(self: Path, *args: str | None, **kwargs: str | None) -> str:
        if self == target:
            raise PermissionError(13, "Permission denied")
        return real_read_text(self, *args, **kwargs)

    monkeypatch.setattr(Path, "read_text", failing_read_text)
    result = runner.invoke(cli.app, ["add", str(p)])
    assert result.exit_code == 1
    assert result.exception is None or isinstance(result.exception, SystemExit)
    assert "Can't read" in result.output


def test_add_onboards_params_non_interactive_skips(tmp_path):
    # --no-input: even when candidates are found, don't select any and don't write [tool.skit]
    p = _py(tmp_path, 'CITY = "Taipei"\nprint(CITY)\n')
    result = runner.invoke(cli.app, ["add", str(p), "--name", "j", "--no-input"])
    assert result.exit_code == 0, result.output
    entry = store.resolve("j")
    assert metawriter.read_params((entry.dir / "script.py").read_text(encoding="utf-8")) == []


# --------------------------------------------------------------------------
# list
# --------------------------------------------------------------------------


def test_list_empty():
    result = runner.invoke(cli.app, ["list"])
    assert result.exit_code == 0


def test_list_table(tmp_path):
    store.add_python(_py(tmp_path, "print(1)\n"), name="a")
    result = runner.invoke(cli.app, ["list"])
    assert result.exit_code == 0
    assert "a" in result.output


def test_list_json(tmp_path):
    store.add_python(_py(tmp_path, "print(1)\n"), name="a")
    result = runner.invoke(cli.app, ["list", "--json"])
    assert result.exit_code == 0
    assert '"slug"' in result.output


def test_list_table_marks_missing_target(tmp_path):
    p = _py(tmp_path, "print(1)\n")
    entry = store.add_python(p, name="gone")
    entry.script_path.unlink()
    result = runner.invoke(cli.app, ["list"])
    assert result.exit_code == 0
    assert "missing" in result.output  # the path itself may be truncated by Rich's column width


def test_list_table_does_not_mark_healthy_or_command_entries(tmp_path):
    store.add_python(_py(tmp_path, "print(1)\n"), name="healthy")
    store.add_command("echo hi", name="cmdok")
    result = runner.invoke(cli.app, ["list"])
    assert result.exit_code == 0
    assert "missing" not in result.output


def test_list_json_missing_field(tmp_path):
    p = _py(tmp_path, "print(1)\n")
    entry = store.add_python(p, name="gone")
    entry.script_path.unlink()
    result = runner.invoke(cli.app, ["list", "--json"])
    assert result.exit_code == 0
    assert '"missing": true' in result.output


def test_list_description_exact_marker_when_no_description(tmp_path):
    """No description: the cell is exactly the dim marker — never a stray "—" prefix.
    (Direct unit test: Rich's table truncates long paths, so the rendered output can't
    be asserted exactly.)"""
    p = _py(tmp_path, "print(1)\n")
    entry = store.add_python(p, name="gone")
    entry.script_path.unlink()
    assert cli._list_description(entry) == f"[dim]⚠ missing: {entry.script_path}[/dim]"


def test_list_and_show_human_faces_use_translated_kind_labels(tmp_path):
    """The human list/show faces show the kind's translated LABEL, not its raw registry id:
    under SKIT_LANG=en, python/prompt/exe render as Python/Prompt/Program. --json is the
    machine contract and keeps the raw ids untouched."""
    import json

    store.add_python(_py(tmp_path, "print(1)\n", name="pyjob.py"), name="pyjob")
    pr = tmp_path / "p.prompt.md"
    pr.write_text("Do {{a}}\n", encoding="utf-8")
    store.add_prompt(pr, name="pr")
    exe = tmp_path / "tool"
    exe.write_text("#!/bin/sh\necho hi\n", encoding="utf-8")
    store.add_exe(exe, name="prog")
    listed = runner.invoke(cli.app, ["list"]).output
    for label in ("Python", "Prompt", "Program"):
        assert label in listed  # the Kind column renders the label…
    assert "python" not in listed  # …and never the raw id (the label is capitalized)
    # show's (kind · mode) header uses the label too.
    assert "Python ·" in runner.invoke(cli.app, ["show", "pyjob"]).output
    assert "Prompt ·" in runner.invoke(cli.app, ["show", "pr"]).output
    assert "Program ·" in runner.invoke(cli.app, ["show", "prog"]).output
    # --json keeps the raw registry ids as a stable machine contract, never the labels.
    payload = json.loads(runner.invoke(cli.app, ["list", "--json"]).output)
    assert {row["name"]: row["kind"] for row in payload} == {
        "pyjob": "python",
        "pr": "prompt",
        "prog": "exe",
    }


def test_list_description_appends_marker_after_description(tmp_path):
    p = _py(tmp_path, '"""My job."""\nprint(1)\n')
    entry = store.add_python(p, name="gone2", description="My job.")
    entry.script_path.unlink()
    assert cli._list_description(entry) == f"My job.  [dim]⚠ missing: {entry.script_path}[/dim]"


def test_list_description_healthy_and_command_entries_untouched(tmp_path):
    healthy = store.add_python(_py(tmp_path, '"""Fine."""\nprint(1)\n'), description="Fine.")
    assert cli._list_description(healthy) == "Fine."
    bare = store.add_command("echo hi", name="cmdbare", description="")
    assert cli._list_description(bare) == "—"


def test_list_description_escapes_markup_in_description():
    """A description containing rich markup renders as literal text, never interpreted (would
    otherwise let a hostile description inject color/style into the list table)."""
    entry = store.add_command("echo hi", name="mkup", description="[red]DANGER[/red]")
    assert cli._list_description(entry) == r"\[red]DANGER\[/red]"


def test_list_description_escapes_markup_in_missing_path(tmp_path):
    """A script path containing literal rich markup (e.g. a hostile directory name) renders
    escaped in the missing-target marker, matching the description-escaping behavior above."""
    exe = tmp_path / "[red]boom[bold]" / "tool"
    exe.parent.mkdir()
    exe.touch()
    entry = store.add_exe(exe, name="mkup-path")
    exe.unlink()
    assert cli._list_description(entry) == f"[dim]{escape(f'⚠ missing: {exe}')}[/dim]"


def test_list_table_renders_markup_literally_end_to_end(tmp_path):
    """End-to-end: both a markup-bearing description and a markup-bearing missing path render as
    literal text in the actual `skit list` table output — no color/style is applied, proving Rich
    never interprets the injected markup as formatting."""
    exe = tmp_path / "[red]boom[bold]" / "tool"
    exe.parent.mkdir()
    exe.touch()
    store.add_exe(exe, name="mkup-path", description="[blue]hi[/blue]")
    exe.unlink()
    result = runner.invoke(cli.app, ["list"])
    assert result.exit_code == 0
    assert "[blue]hi[/blue]" in result.output
    assert "missing" in result.output  # the path itself may be truncated by Rich's column width


def test_list_table_name_column_escapes_markup(tmp_path):
    """A NAME containing rich markup (settable via --name) must render literally in the Name
    column too — not just the Description column."""
    store.add_command("echo hi", name="[blue]hi[/blue]")
    result = runner.invoke(cli.app, ["list"])
    assert result.exit_code == 0
    assert "[blue]hi[/blue]" in result.output


# --------------------------------------------------------------------------
# remove
# --------------------------------------------------------------------------


def test_remove_not_found():
    result = runner.invoke(cli.app, ["remove", "ghost"])
    assert result.exit_code == 1


def test_remove_with_yes(tmp_path):
    store.add_python(_py(tmp_path, "print(1)\n"), name="a")
    result = runner.invoke(cli.app, ["remove", "a", "--yes"])
    assert result.exit_code == 0
    with pytest.raises(store.NotFoundError):
        store.resolve("a")


def test_remove_confirm_abort(tmp_path):
    store.add_python(_py(tmp_path, "print(1)\n"), name="a")
    result = runner.invoke(cli.app, ["remove", "a"], input="n\n")
    assert result.exit_code != 0  # abort
    assert store.resolve("a")  # still there


# --------------------------------------------------------------------------
# run
# --------------------------------------------------------------------------


@pytest.fixture
def run_entry_spy(monkeypatch):
    calls = {}

    def fake(
        entry,
        extra_args=None,
        *,
        values=None,
        invoke_cwd=None,
        script_override=None,
        env_overlay=None,
        runner=None,
    ):
        calls["entry"] = entry
        calls["extra"] = list(extra_args or [])
        calls["values"] = dict(values or {})
        calls["override"] = script_override
        calls["runner"] = runner
        return calls.get("code", 0)

    monkeypatch.setattr(launcher, "run_entry", fake)
    return calls


def test_run_python_with_params_injects(tmp_path, run_entry_spy):
    text = metawriter.write_params(
        'CITY = "Taipei"\nprint(CITY)\n', [ParamDecl(name="CITY", binding="const", type="str")]
    )
    entry = store.add_python(_py(tmp_path, text), name="j")
    argstate.save_last(entry.slug, values={"CITY": "Kaohsiung"})
    result = runner.invoke(cli.app, ["run", "j", "--no-input"])
    assert result.exit_code == 0, result.output
    # A managed value exists → an injected artifact path is passed to the launcher
    assert run_entry_spy["override"] is not None


ARGPARSE_REQUIRED = (
    "import argparse\nap = argparse.ArgumentParser()\n"
    "ap.add_argument('-o', '--output', required=True)\nap.parse_args()\n"
)


def test_run_extra_args_bypass_required_field_validation(tmp_path, run_entry_spy):
    # Passthrough args are the legitimate manual escape (skit run x -- <args>): when the
    # user supplies them, the script's own parser is in charge and an unfilled required
    # FIELD must not block the run.
    store.add_python(_py(tmp_path, ARGPARSE_REQUIRED), name="ar")
    result = runner.invoke(cli.app, ["run", "ar", "--no-input", "--", "-o", "x.png"])
    assert result.exit_code == 0, result.output
    assert run_entry_spy["extra"] == ["-o", "x.png"]


def test_run_required_field_missing_without_extra_args_exits_125(tmp_path, run_entry_spy):
    store.add_python(_py(tmp_path, ARGPARSE_REQUIRED), name="ar2")
    result = runner.invoke(cli.app, ["run", "ar2", "--no-input"])
    assert result.exit_code == 125
    assert "output" in result.output


def test_run_not_found_exits_127():
    result = runner.invoke(cli.app, ["run", "ghost"])
    assert result.exit_code == 127  # docker convention: target not found


def test_run_raw_skips_form(tmp_path, run_entry_spy):
    text = metawriter.write_params(
        'CITY = "Taipei"\nprint(CITY)\n', [ParamDecl(name="CITY", binding="const", type="str")]
    )
    store.add_python(_py(tmp_path, text), name="j")
    result = runner.invoke(cli.app, ["run", "j", "--raw", "--no-input"])
    assert result.exit_code == 0, result.output
    assert run_entry_spy["override"] is None  # --raw: no injection


def test_run_unknown_preset_rejected(tmp_path, run_entry_spy):
    store.add_python(_py(tmp_path, "print(1)\n"), name="j")
    result = runner.invoke(cli.app, ["run", "j", "--preset", "nope", "--no-input"])
    assert result.exit_code == 2


def test_run_passes_and_remembers_extra_args(tmp_path, run_entry_spy):
    store.add_python(_py(tmp_path, "print(1)\n"), name="j")
    result = runner.invoke(cli.app, ["run", "j", "--no-input", "--", "--flag", "v"])
    assert result.exit_code == 0, result.output
    assert run_entry_spy["extra"] == ["--flag", "v"]
    # Run again without args: last-used args are reused
    result2 = runner.invoke(cli.app, ["run", "j", "--no-input"])
    assert result2.exit_code == 0, result2.output
    assert run_entry_spy["extra"] == ["--flag", "v"]


def test_run_command_reuses_last_extra_args(tmp_path, run_entry_spy):
    """A command template remembers its appended tail too (docs/design/prompt.md v3.1):
    passing none on the next run replays it, matching the run form and the `r` rerun.
    Before v3.1 the command kind refused to replay because takes_argv=False."""
    store.add_command("echo ready", name="cmd")
    first = runner.invoke(cli.app, ["run", "cmd", "--no-input", "--", "--loud"])
    assert first.exit_code == 0, first.output
    assert run_entry_spy["extra"] == ["--loud"]
    second = runner.invoke(cli.app, ["run", "cmd", "--no-input"])
    assert second.exit_code == 0, second.output
    assert run_entry_spy["extra"] == ["--loud"]
    # An explicit tail still overrides the remembered one.
    third = runner.invoke(cli.app, ["run", "cmd", "--no-input", "--", "--quiet"])
    assert third.exit_code == 0, third.output
    assert run_entry_spy["extra"] == ["--quiet"]


def test_run_nonzero_exit_propagates(tmp_path, run_entry_spy):
    store.add_python(_py(tmp_path, "print(1)\n"), name="j")
    run_entry_spy["code"] = 3
    result = runner.invoke(cli.app, ["run", "j", "--no-input"])
    assert result.exit_code == 3


def test_run_shim_error(tmp_path, run_entry_spy, monkeypatch):
    text = metawriter.write_params(
        'CITY = "Taipei"\nprint(CITY)\n', [ParamDecl(name="CITY", binding="const", type="str")]
    )
    entry = store.add_python(_py(tmp_path, text), name="j")
    argstate.save_last(entry.slug, values={"CITY": "Kaohsiung"})

    def boom(*a, **k):
        raise shim.ShimError("nope")

    monkeypatch.setattr(shim, "inject", boom)
    result = runner.invoke(cli.app, ["run", "j", "--no-input"])
    assert result.exit_code == 125  # skit-side failure, not the script's own exit code


def test_run_bad_typed_value_caught_at_validation(tmp_path, run_entry_spy):
    # A value that can't coerce to its param's declared type (RETRIES is int) is a bad
    # input, not drift — v2 catches it at form validation, before shim ever runs, and
    # maps it to the skit-side exit code.
    text = metawriter.write_params(
        "RETRIES = 3\nprint(RETRIES)\n", [ParamDecl(name="RETRIES", binding="const", type="int")]
    )
    entry = store.add_python(_py(tmp_path, text), name="j")
    argstate.save_last(entry.slug, values={"RETRIES": "not-a-number"})
    result = runner.invoke(cli.app, ["run", "j", "--no-input"])
    assert result.exit_code == 125
    assert "not-a-number" in result.output
    assert "whole number" in result.output
    # The generic drift/re-add wording must NOT appear for a value failure.
    assert "resync" not in result.output.lower()


def test_run_launch_error(tmp_path, monkeypatch):
    store.add_python(_py(tmp_path, "print(1)\n"), name="j")

    def boom(*a, **k):
        raise launcher.LaunchError("bad")

    monkeypatch.setattr(launcher, "run_entry", boom)
    result = runner.invoke(cli.app, ["run", "j", "--no-input"])
    assert result.exit_code == 125


def test_run_command_entry_collects_values(tmp_path, run_entry_spy):
    ent = store.add_command("echo {msg}", name="e")
    argstate.save_last(ent.slug, values={"msg": "hi"})
    result = runner.invoke(cli.app, ["run", "e", "--no-input"])
    assert result.exit_code == 0, result.output
    assert run_entry_spy["values"] == {"msg": "hi"}


# --------------------------------------------------------------------------
# preset
# --------------------------------------------------------------------------


def test_preset_list_none(tmp_path):
    store.add_python(_py(tmp_path, "print(1)\n"), name="a")
    result = runner.invoke(cli.app, ["preset", "list", "a"])
    assert result.exit_code == 0


def test_preset_list_shows(tmp_path):
    ent = store.add_python(_py(tmp_path, "print(1)\n"), name="a")
    argstate.save_preset(ent.slug, "prod", {"CITY": "Taipei"})
    result = runner.invoke(cli.app, ["preset", "list", "a"])
    assert "prod" in result.output


def test_preset_list_not_found():
    result = runner.invoke(cli.app, ["preset", "list", "ghost"])
    assert result.exit_code == 1


def test_preset_delete(tmp_path):
    ent = store.add_python(_py(tmp_path, "print(1)\n"), name="a")
    argstate.save_preset(ent.slug, "prod", {"CITY": "Taipei"})
    result = runner.invoke(cli.app, ["preset", "delete", "a", "prod"])
    assert result.exit_code == 0
    assert argstate.load_state(ent.slug)["presets"] == {}


def test_preset_delete_unknown(tmp_path):
    store.add_python(_py(tmp_path, "print(1)\n"), name="a")
    result = runner.invoke(cli.app, ["preset", "delete", "a", "nope"])
    assert result.exit_code == 1


def test_preset_delete_not_found():
    result = runner.invoke(cli.app, ["preset", "delete", "ghost", "p"])
    assert result.exit_code == 1


def test_preset_save_not_found():
    result = runner.invoke(cli.app, ["preset", "save", "ghost", "p"])
    assert result.exit_code == 1


def test_preset_save_python_no_params(tmp_path):
    store.add_python(_py(tmp_path, "print(1)\n"), name="a")
    result = runner.invoke(cli.app, ["preset", "save", "a", "p"], input="\n")
    # A field-less entry has nothing to save: USAGE (2), the same code `run --save-preset`
    # now uses for this refusal — not 1, which docker-convention reserves for the script.
    assert result.exit_code == 2  # no managed parameters


def test_preset_save_command_no_params(tmp_path):
    store.add_command("echo hi", name="e")  # no placeholders
    result = runner.invoke(cli.app, ["preset", "save", "e", "p"])
    assert result.exit_code == 2


def test_preset_save_command_with_params(tmp_path, tty, monkeypatch):
    # Direct call: CliRunner swaps sys.stdin wholesale, so the interactive gate can't be
    # exercised through invoke() — the tty fixture + a direct call is the honest path.
    ent = store.add_command("echo {msg}", name="e")
    monkeypatch.setattr(cli.Prompt, "ask", lambda *a, **k: "hello")
    cli.preset_save("e", "prod", from_last=False)
    assert argstate.load_state(ent.slug)["presets"]["prod"] == {"msg": "hello"}


# --------------------------------------------------------------------------
# params
# --------------------------------------------------------------------------


def test_params_not_found():
    result = runner.invoke(cli.app, ["params", "ghost"])
    assert result.exit_code == 1


def test_params_empty(tmp_path):
    store.add_python(_py(tmp_path, "print(1)\n"), name="a")
    result = runner.invoke(cli.app, ["params", "a"])
    assert result.exit_code == 0


def test_params_command_entry(tmp_path):
    ent = store.add_command("echo {msg}", name="e")
    argstate.save_last(ent.slug, values={"msg": "hi"})
    result = runner.invoke(cli.app, ["params", "e"])
    assert result.exit_code == 0
    assert "msg" in result.output


def test_params_command_no_placeholders(tmp_path):
    store.add_command("echo hi", name="e")
    result = runner.invoke(cli.app, ["params", "e"])
    assert result.exit_code == 0


def test_params_python_table_with_secret(tmp_path):
    text = metawriter.write_params(
        'API = "x"\nprint(API)\n',
        [ParamDecl(name="API", binding="const", type="str", default="x", secret=True)],
    )
    ent = store.add_python(_py(tmp_path, text), name="a")
    argstate.save_last(ent.slug, values={"API": "shown"}, secret_names={"API"})
    result = runner.invoke(cli.app, ["params", "a"])
    assert result.exit_code == 0
    assert "API" in result.output


# --------------------------------------------------------------------------
# params --secret: marking a parameter secret must purge its already-persisted plaintext
# ("secrets aren't fully secret" — marking secret protects the future, not the past)
# --------------------------------------------------------------------------


def test_params_secret_purges_stored_last_value_and_presets(tmp_path):
    text = metawriter.write_params(
        'API_KEY = "x"\nprint(API_KEY)\n',
        [ParamDecl(name="API_KEY", binding="const", type="str", default="x")],
    )
    ent = store.add_python(_py(tmp_path, text), name="a")
    # Recorded while API_KEY was still a public parameter.
    argstate.save_last(ent.slug, values={"API_KEY": "plaintext-secret-123"})
    argstate.save_preset(ent.slug, "prod", {"API_KEY": "plaintext-secret-123"})
    result = runner.invoke(cli.app, ["params", "a", "--secret", "API_KEY"])
    assert result.exit_code == 0, result.output
    assert "plaintext-secret-123" not in result.output
    # Rich may line-wrap the long message at terminal width; collapse whitespace before matching.
    normalized_output = " ".join(result.output.split())
    expected_msg = (
        "Removed previously stored plaintext value(s) for now-secret parameter(s): API_KEY"
    )
    assert expected_msg in normalized_output
    state = argstate.load_state(ent.slug)
    assert "API_KEY" not in state["values"]
    # 'prod' held only API_KEY, so purging it leaves the preset empty and it is dropped entirely.
    assert "prod" not in state["presets"]
    # Scan the raw state file bytes: the plaintext must not merely be hidden from load_state, it
    # must not be on disk at all.
    for p in values_dir().glob("*.toml"):
        assert "plaintext-secret-123" not in p.read_text(encoding="utf-8")


def test_params_secret_does_not_purge_other_still_public_params(tmp_path):
    text = metawriter.write_params(
        "API_KEY = 'x'\nCITY = 'y'\nprint(API_KEY, CITY)\n",
        [
            ParamDecl(name="API_KEY", binding="const", type="str"),
            ParamDecl(name="CITY", binding="const", type="str"),
        ],
    )
    ent = store.add_python(_py(tmp_path, text), name="a")
    argstate.save_last(ent.slug, values={"API_KEY": "secretval", "CITY": "Taipei"})
    result = runner.invoke(cli.app, ["params", "a", "--secret", "API_KEY"])
    assert result.exit_code == 0, result.output
    state = argstate.load_state(ent.slug)
    assert "API_KEY" not in state["values"]
    assert state["values"]["CITY"] == "Taipei"  # untouched: CITY was never marked secret


def test_params_edit_without_stored_value_prints_no_purge_message(tmp_path):
    # Nothing was ever stored for CITY, so marking it secret has nothing to purge — the purge
    # message must not appear (regression: kills an unconditional-print mutant).
    text = metawriter.write_params(
        'CITY = "x"\nprint(CITY)\n', [ParamDecl(name="CITY", binding="const", type="str")]
    )
    store.add_python(_py(tmp_path, text), name="a")
    result = runner.invoke(cli.app, ["params", "a", "--secret", "CITY"])
    assert result.exit_code == 0, result.output
    assert "Removed previously stored plaintext" not in result.output


# --------------------------------------------------------------------------
# deps
# --------------------------------------------------------------------------


def test_deps_view(tmp_path):
    store.add_python(_py(tmp_path, "print(1)\n"), name="a")
    result = runner.invoke(cli.app, ["deps", "a"])
    assert result.exit_code == 0


def test_deps_not_found():
    result = runner.invoke(cli.app, ["deps", "ghost"])
    assert result.exit_code == 1


def test_deps_not_python(tmp_path):
    # The PEP 723 dependency flavor is python-only: --dep on a command entry is refused.
    # (The bare read view now works for every kind — it shows needs; see test_needs.py.)
    # A refused flag is a usage error (2), matching `skit add`.
    store.add_command("echo hi", name="e")
    result = runner.invoke(cli.app, ["deps", "e", "--dep", "requests"])
    assert result.exit_code == 2


def test_deps_set(tmp_path):
    store.add_python(_py(tmp_path, "print(1)\n"), name="a")
    result = runner.invoke(
        cli.app, ["deps", "a", "--dep", "requests", "--dep", "rich", "--python", ">=3.11"]
    )
    assert result.exit_code == 0, result.output
    assert store.resolve("a").meta.dependencies == ["requests", "rich"]


def test_deps_view_with_requires_python(tmp_path):
    store.add_python(_py(tmp_path, "print(1)\n"), name="a")
    runner.invoke(cli.app, ["deps", "a", "--dep", "requests", "--python", ">=3.12"])
    result = runner.invoke(cli.app, ["deps", "a"])
    assert result.exit_code == 0
    assert "3.12" in result.output


def test_deps_command_strips_a_whitespace_only_python_constraint(tmp_path):
    # A whitespace-only "   " is truthy but an unparseable version specifier that bricks every
    # run; the store strips it to "" (omitted) rather than recording it.
    store.add_python(_py(tmp_path, "print(1)\n"), name="a")
    result = runner.invoke(cli.app, ["deps", "a", "--python", "   "])
    assert result.exit_code == 0
    assert store.resolve("a").meta.requires_python == ""


# --------------------------------------------------------------------------
# doctor
# --------------------------------------------------------------------------


def test_doctor_uv_found(monkeypatch, tmp_path):
    monkeypatch.setattr("skit.langs.launch.find_uv", lambda: "/usr/bin/uv")
    result = runner.invoke(cli.app, ["doctor"])
    assert result.exit_code == 0


def test_doctor_uv_missing(monkeypatch):
    monkeypatch.setattr("skit.langs.launch.find_uv", lambda: None)
    result = runner.invoke(cli.app, ["doctor"])
    assert result.exit_code == 1


def test_doctor_rebuild(monkeypatch, tmp_path):
    monkeypatch.setattr("skit.langs.launch.find_uv", lambda: "/usr/bin/uv")
    store.add_python(_py(tmp_path, "print(1)\n"), name="a")
    result = runner.invoke(cli.app, ["doctor", "--rebuild"])
    assert result.exit_code == 0


def test_doctor_reports_missing_reference(monkeypatch, tmp_path):
    monkeypatch.setattr("skit.langs.launch.find_uv", lambda: "/usr/bin/uv")
    src = _py(tmp_path, "print(1)\n")
    store.add_python(src, name="ref", mode="reference")
    src.unlink()
    result = runner.invoke(cli.app, ["doctor"])
    assert result.exit_code == 0
    assert "ref" in result.output


# --------------------------------------------------------------------------
# lang
# --------------------------------------------------------------------------


# --------------------------------------------------------------------------
# Interactive helpers: called directly + stubbed (CliRunner cannot reliably inject a tty)
# --------------------------------------------------------------------------


@pytest.fixture
def tty(monkeypatch):
    monkeypatch.setattr("sys.stdin.isatty", lambda: True, raising=False)
    monkeypatch.setattr("sys.stdout.isatty", lambda: True, raising=False)
    monkeypatch.setattr(cli, "_is_interactive", lambda: True)


def test_parse_selection_variants():
    assert cli._parse_selection("all", 3) == [0, 1, 2]
    assert cli._parse_selection("none", 3) == []
    assert cli._parse_selection("", 3) == []
    assert cli._parse_selection("1,3", 3) == [0, 2]
    assert cli._parse_selection("1,1,9,x", 3) == [0]  # dedup + out-of-range / non-numeric ignored


def test_parse_selection_ignores_non_ascii_digit_like_chars():
    # Regression: str.isdigit() is True for '²' (superscript two) and '①' (circled one), but
    # int() rejects both -- the old `part.isdigit() and int(part)` guard let the ValueError
    # escape uncaught, crashing onboarding instead of ignoring the invalid part as documented.
    assert cli._parse_selection("1,²,3", 5) == [0, 2]
    assert cli._parse_selection("①", 5) == []
    # Non-ASCII characters that ARE genuine decimal digits (e.g. Arabic-indic) must still work,
    # since int() parses them fine -- the fix must not narrow to ASCII-only.
    arabic_indic_one = "\N{ARABIC-INDIC DIGIT ONE}"
    assert cli._parse_selection(arabic_indic_one, 5) == [0]


def test_parse_kv_opts():
    pairs, bad = cli._parse_kv_opts(["A=hello", "B=", "no-eq", "=novalue"], "--prompt")
    assert pairs == {"A": "hello", "B": ""}
    assert bad == ["--prompt: no-eq", "--prompt: =novalue"]


def test_resolve_metadata_existing_block_not_asked():
    text = '# /// script\n# dependencies = ["requests"]\n# ///\nprint(1)\n'
    deps, py = cli._resolve_python_metadata(text, None, None, no_input=False)
    assert deps == []
    assert py == ""


def test_resolve_metadata_explicit_opts():
    deps, py = cli._resolve_python_metadata("print(1)\n", ["requests", "rich"], ">=3.11", False)
    assert deps == ["requests", "rich"]
    assert py == ">=3.11"


def test_resolve_metadata_explicit_opts_strips_and_drops_empties():
    # Empty/whitespace explicit values would brick the entry: "" makes PEP 508 refuse the whole
    # block ("Empty field is not allowed"), and a whitespace-only --python is an unparseable
    # version constraint. Strip and drop them, matching the interactive and npm paths.
    deps, py = cli._resolve_python_metadata("print(1)\n", ["", "  requests  ", "   "], "   ", False)
    assert deps == ["requests"]
    assert py == ""


def test_resolve_metadata_no_suggestions():
    deps, py = cli._resolve_python_metadata("print(1)\n", None, None, no_input=False)
    assert deps == []
    assert py == ""


def test_resolve_metadata_non_interactive_uses_suggestions():
    deps, _py = cli._resolve_python_metadata(
        "import requests\nprint(requests)\n", None, None, no_input=True
    )
    assert deps == ["requests"]


def test_resolve_metadata_interactive(monkeypatch, tty):
    answers = iter(["requests, rich", ">=3.12"])
    monkeypatch.setattr(cli.Prompt, "ask", lambda *a, **k: next(answers))
    deps, py = cli._resolve_python_metadata(
        "import requests\nprint(requests)\n", None, None, no_input=False
    )
    assert deps == ["requests", "rich"]
    assert py == ">=3.12"


def test_resolve_metadata_interactive_dash_clears_deps(monkeypatch, tty):
    # "-" (or "none") at the deps prompt means "install nothing", overriding the suggested default.
    answers = iter(["-", ""])
    monkeypatch.setattr(cli.Prompt, "ask", lambda *a, **k: next(answers))
    deps, py = cli._resolve_python_metadata(
        "import requests\nprint(requests)\n", None, None, no_input=False
    )
    assert deps == []
    assert py == ""


def test_resolve_metadata_interactive_none_word_clears_deps(monkeypatch, tty):
    answers = iter(["None", ""])
    monkeypatch.setattr(cli.Prompt, "ask", lambda *a, **k: next(answers))
    deps, _py = cli._resolve_python_metadata(
        "import requests\nprint(requests)\n", None, None, no_input=False
    )
    assert deps == []


def test_prompt_identity_non_interactive_passes_through(tmp_path):
    p = tmp_path / "s.py"
    name, desc = cli._prompt_identity(p, "print(1)\n", None, None, no_input=True)
    assert name is None
    assert desc is None


def test_prompt_identity_prompts_name_and_description(monkeypatch, tty, tmp_path):
    p = tmp_path / "image_stitch.py"
    answers = iter(["stitch", "Stack images vertically"])
    monkeypatch.setattr(cli.Prompt, "ask", lambda *a, **k: next(answers))
    name, desc = cli._prompt_identity(p, '"""doc first line."""\n', None, None, no_input=False)
    assert name == "stitch"
    assert desc == "Stack images vertically"


def test_prompt_identity_explicit_values_skip_prompts(monkeypatch, tty, tmp_path):
    # Both already supplied via flags: Prompt.ask must never be called.
    def _boom(*_a, **_k):
        raise AssertionError("should not prompt when name and description are given")

    monkeypatch.setattr(cli.Prompt, "ask", _boom)
    name, desc = cli._prompt_identity(
        tmp_path / "s.py", "print(1)\n", "given", "a desc", no_input=False
    )
    assert (name, desc) == ("given", "a desc")


def test_prompt_identity_blank_name_falls_back_to_stem(monkeypatch, tty, tmp_path):
    # An all-whitespace name answer collapses to None so the store derives the stem.
    p = tmp_path / "worker.py"
    answers = iter(["   ", ""])
    monkeypatch.setattr(cli.Prompt, "ask", lambda *a, **k: next(answers))
    name, desc = cli._prompt_identity(p, "print(1)\n", None, None, no_input=False)
    assert name is None
    assert desc == ""


def test_onboard_params_framework_detected(monkeypatch, tty):
    text = "import argparse\np = argparse.ArgumentParser()\n"
    specs = cli._onboard_params(text, "cli-tool", no_input=False)
    assert specs == []


def test_onboard_params_no_candidates(tty):
    assert cli._onboard_params("print(1)\n", "x", no_input=False) == []


def test_onboard_params_non_interactive_returns_empty():
    text = 'CITY = "Taipei"\nprint(CITY)\n'
    assert cli._onboard_params(text, "x", no_input=True) == []


def test_onboard_params_interactive_selection(monkeypatch, tty):
    text = 'CITY = "Taipei"\nRETRIES = 3\nwho = input("Name: ")\nprint(CITY, RETRIES, who)\n'
    monkeypatch.setattr(cli.Prompt, "ask", lambda *a, **k: "all")
    specs = cli._onboard_params(text, "x", no_input=False)
    assert len(specs) >= 2
    assert any(s.name == "CITY" for s in specs)


def test_paramspec_from_candidate_roundtrip():
    result = analyzer.analyze('CITY = "Taipei"\nprint(CITY)\n')
    spec = ParamDecl.from_candidate(result.candidates[0])
    assert spec.name == "CITY"


def test_command_placeholders_collect_interactively(monkeypatch, tty, tmp_path):
    ent = store.add_command("echo {msg}", name="e")
    plan = flows.plan_for_entry(ent)
    monkeypatch.setattr(cli.Prompt, "ask", lambda *a, **k: "typed")
    values = promptform.collect(plan, {}, console=cli.console)
    assert values == {"msg": "typed"}


def test_command_placeholders_prefill_from_last(tmp_path):
    ent = store.add_command("echo {msg}", name="e")
    argstate.save_last(ent.slug, values={"msg": "remembered"})
    plan = flows.plan_for_entry(ent)
    assert flows.prefill(plan, ent.slug) == {"msg": "remembered"}


def test_command_without_placeholders_has_no_fields(tmp_path):
    ent = store.add_command("echo hi", name="e")
    assert flows.plan_for_entry(ent).fields == []


def test_collect_param_form_interactive_secret(monkeypatch, tty, tmp_path):
    text = metawriter.write_params(
        'API = "x"\nprint(API)\n',
        [ParamDecl(name="API", binding="const", type="str", secret=True)],
    )
    ent = store.add_python(_py(tmp_path, text), name="a")
    plan = flows.plan_for_entry(ent)
    monkeypatch.setattr(cli.Prompt, "ask", lambda *a, **k: "secretval")
    values = promptform.collect(plan, flows.prefill(plan, ent.slug), console=cli.console)
    assert values == {"API": "secretval"}


def test_param_form_prefill_uses_definition_default(tmp_path):
    text = metawriter.write_params(
        'CITY = "Osaka"\nprint(CITY)\n',
        [ParamDecl(name="CITY", binding="const", type="str", default="Osaka")],
    )
    ent = store.add_python(_py(tmp_path, text), name="a")
    plan = flows.plan_for_entry(ent)
    assert flows.prefill(plan, ent.slug) == {"CITY": "Osaka"}


# --------------------------------------------------------------------------
# Markup escaping: every remaining render site that carries user-controlled data
# (entry names, param names/values/prompts, presets, deps, file paths, error messages) must
# never let rich markup embedded in that data be interpreted instead of shown literally.
# --------------------------------------------------------------------------


def test_add_summary_escapes_markup_in_name_and_description(tmp_path):
    store.add_command("echo hi", name="[blue]hi[/blue]")
    result = runner.invoke(
        cli.app,
        ["add", "--cmd", "echo {x}", "--name", "[red]evil[/red]", "--description", "[b]d[/b]"],
    )
    assert result.exit_code == 0, result.output
    assert "[red]evil[/red]" in result.output
    assert "[b]d[/b]" in result.output


def test_add_deps_summary_escapes_markup(tmp_path):
    # A dep string that is BOTH a valid PEP 508 requirement (extras syntax) and rich markup:
    # `demo[bold]` parses (package `demo`, extra `bold`) yet `[bold]` reads as a style tag, so
    # if the summary failed to escape it the literal `[bold]` would vanish from the output.
    result = runner.invoke(
        cli.app,
        ["add", str(_py(tmp_path, "print(1)\n")), "--dep", "demo[bold]", "--no-input"],
    )
    assert result.exit_code == 0, result.output
    assert "demo[bold]" in result.output


def test_add_not_py_file_warning_escapes_markup_in_filename(tmp_path):
    p = tmp_path / "[red]evil[bold].txt"
    p.write_text("hi", encoding="utf-8")
    result = runner.invoke(cli.app, ["add", str(p)])
    assert result.exit_code == 2
    assert "[red]evil[bold].txt" in result.output


def test_remove_escapes_markup_in_name():
    store.add_command("echo hi", name="[blue]hi[/blue]")
    result = runner.invoke(cli.app, ["remove", "[blue]hi[/blue]", "--yes"])
    assert result.exit_code == 0
    assert "[blue]hi[/blue]" in result.output


def test_not_found_error_escapes_markup_in_argument():
    """store.NotFoundError embeds the raw name_or_slug the user typed; a markup-bearing CLI
    argument must render literally in the error, not be interpreted."""
    result = runner.invoke(cli.app, ["deps", "[red]ghost[/red]"])
    assert result.exit_code == 1
    assert "[red]ghost[/red]" in result.output


def test_params_table_escapes_markup_in_name_and_default(tmp_path):
    """A managed parameter's name/default can carry markup when the script's [tool.skit] block was
    hand-edited (names there aren't constrained to valid identifiers) — it must render literally
    in the `skit params` table."""
    text = metawriter.write_params(
        "print(1)\n",
        [ParamDecl(name="[red]NAME[/red]", binding="const", type="str", default="[blue]hi[/blue]")],
    )
    store.add_python(_py(tmp_path, text), name="a")
    result = runner.invoke(cli.app, ["params", "a"])
    assert result.exit_code == 0, result.output
    assert "[red]NAME[/red]" in result.output
    assert "[blue]hi[/blue]" in result.output


def test_params_command_placeholder_line_escapes_markup(tmp_path):
    ent = store.add_command("echo {msg}", name="e")
    argstate.save_last(ent.slug, values={"msg": "[green]hello[/green]"})
    result = runner.invoke(cli.app, ["params", "e"])
    assert result.exit_code == 0
    assert "[green]hello[/green]" in result.output


def test_params_candidates_line_escapes_markup_in_name(tmp_path, monkeypatch):
    """The "Detected but not yet managed" line interpolates candidate names raw; even though
    analyzer-derived names are normally valid identifiers, this is defense in depth against any
    future candidate source that isn't so constrained."""
    hostile = analysis.Candidate(binding="const", name="[red]NEW[/red]", type="str", default="x")
    monkeypatch.setattr(reconcile, "analyze", lambda text: analysis.Analysis(candidates=[hostile]))
    store.add_python(_py(tmp_path, "print(1)\n"), name="a")
    result = runner.invoke(cli.app, ["params", "a"])
    assert result.exit_code == 0, result.output
    assert "[red]NEW[/red]" in result.output


def test_preset_list_escapes_markup_in_name_and_values(tmp_path):
    ent = store.add_python(_py(tmp_path, "print(1)\n"), name="a")
    argstate.save_preset(ent.slug, "[blue]prod[/blue]", {"CITY": "[red]Taipei[/red]"})
    result = runner.invoke(cli.app, ["preset", "list", "a"])
    assert result.exit_code == 0
    assert "[blue]prod[/blue]" in result.output
    assert "[red]Taipei[/red]" in result.output


def test_preset_save_command_escapes_markup_in_preset_name_and_entry_name(tmp_path):
    store.add_command("echo {msg}", name="[blue]e[/blue]")
    result = runner.invoke(
        cli.app, ["preset", "save", "[blue]e[/blue]", "[green]p[/green]"], input="hi\n"
    )
    assert result.exit_code == 0, result.output
    assert "[green]p[/green]" in result.output
    assert "[blue]e[/blue]" in result.output


def test_preset_delete_unknown_escapes_markup_in_preset_name():
    store.add_command("echo hi", name="a")
    result = runner.invoke(cli.app, ["preset", "delete", "a", "[red]nope[/red]"])
    assert result.exit_code == 1
    assert "[red]nope[/red]" in result.output


def test_validate_preset_unknown_escapes_markup(tmp_path):
    store.add_command("echo hi", name="a")
    result = runner.invoke(cli.app, ["run", "a", "--preset", "[red]nope[/red]"])
    assert result.exit_code == 2
    assert "[red]nope[/red]" in result.output


def test_deps_view_escapes_markup(tmp_path):
    # A dependency that is BOTH a valid PEP 508 requirement (extras syntax) and rich
    # markup (`[bold]` is a style tag): the store now validates deps, so the fake must
    # parse — but `demo[bold]` still exercises the escape (unescaped, rich would eat the
    # `[bold]` tag and the literal brackets would vanish from the view).
    store.add_python(_py(tmp_path, "print(1)\n"), name="a")
    result = runner.invoke(cli.app, ["deps", "a", "--dep", "demo[bold]"])
    assert result.exit_code == 0, result.output
    result = runner.invoke(cli.app, ["deps", "a"])
    assert result.exit_code == 0
    assert "demo[bold]" in result.output  # brackets survive → the view escaped the markup


def test_deps_set_summary_escapes_markup(tmp_path):
    store.add_python(_py(tmp_path, "print(1)\n"), name="a")
    result = runner.invoke(cli.app, ["deps", "a", "--dep", "demo[bold]"])
    assert result.exit_code == 0
    assert "demo[bold]" in result.output


def test_doctor_rebuild_problem_line_escapes_markup(monkeypatch, tmp_path):
    monkeypatch.setattr("skit.langs.launch.find_uv", lambda: "/usr/bin/uv")
    monkeypatch.setattr(store, "doctor_rebuild", lambda: (0, ["[red]broken[/red]"]))
    result = runner.invoke(cli.app, ["doctor", "--rebuild"])
    assert result.exit_code == 0
    assert "[red]broken[/red]" in result.output


def test_doctor_missing_reference_escapes_markup_in_name(tmp_path):
    exe = tmp_path / "tool"
    exe.touch()
    store.add_exe(exe, name="[red]gone[/red]")
    exe.unlink()
    result = runner.invoke(cli.app, ["doctor"])
    assert "[red]gone[/red]" in result.output


def test_doctor_uv_path_escapes_markup(monkeypatch):
    monkeypatch.setattr("skit.langs.launch.find_uv", lambda: "/usr/[red]bin[/red]/uv")
    result = runner.invoke(cli.app, ["doctor"])
    assert result.exit_code == 0
    assert "[red]bin[/red]" in result.output


def test_config_set_unknown_language_escapes_markup():
    result = runner.invoke(cli.app, ["config", "lang", "[red]xx-YY[/red]"])
    assert result.exit_code == 2
    assert "[red]xx-YY[/red]" in result.output


def test_config_set_unknown_mirror_escapes_markup():
    result = runner.invoke(cli.app, ["config", "mirror", "[red]nope[/red]"])
    assert result.exit_code == 2
    assert "[red]nope[/red]" in result.output


def test_edit_reports_escape_markup_in_name(tmp_path, monkeypatch):
    store.add_python(_py(tmp_path, "print(1)\n"), name="[blue]a[/blue]")
    monkeypatch.setattr(cli.editor, "open_in_editor", lambda p: None)
    result = runner.invoke(cli.app, ["edit", "[blue]a[/blue]"])
    assert result.exit_code == 0, result.output
    assert "[blue]a[/blue]" in result.output


def test_edit_reference_mode_escapes_markup_in_name_and_path(tmp_path, monkeypatch):
    script = tmp_path / "[red]weird[bold]" / "job.py"
    script.parent.mkdir()
    script.write_text("print(1)\n", encoding="utf-8")
    store.add_python(script, mode="reference", name="refjob")
    monkeypatch.setattr(cli.editor, "open_in_editor", lambda p: None)
    result = runner.invoke(cli.app, ["edit", "refjob"])
    assert result.exit_code == 0, result.output
    assert "[red]weird[bold]" in result.output


def test_edit_missing_reference_source_escapes_markup_in_path(tmp_path):
    script = tmp_path / "[red]weird[bold]" / "job.py"
    script.parent.mkdir()
    script.write_text("print(1)\n", encoding="utf-8")
    store.add_python(script, mode="reference", name="refjob")
    script.unlink()
    result = runner.invoke(cli.app, ["edit", "refjob"])
    assert result.exit_code == 1
    assert "[red]weird[bold]" in result.output


def test_edit_params_updated_summary_escapes_markup_in_name(tmp_path):
    text = metawriter.write_params(
        "X = 1\nprint(X)\n", [ParamDecl(name="X", binding="const", type="int", default=1)]
    )
    store.add_python(_py(tmp_path, text), name="[blue]a[/blue]")
    result = runner.invoke(cli.app, ["params", "[blue]a[/blue]", "--resync"])
    assert result.exit_code == 0, result.output
    assert "[blue]a[/blue]" in result.output


def test_edit_params_malformed_prompt_escapes_markup(tmp_path):
    text = metawriter.write_params(
        "X = 1\nprint(X)\n", [ParamDecl(name="X", binding="const", type="int", default=1)]
    )
    store.add_python(_py(tmp_path, text), name="a")
    result = runner.invoke(cli.app, ["params", "a", "--prompt", "[red]bad[/red]"])
    assert result.exit_code == 0, result.output
    assert "[red]bad[/red]" in result.output


def test_run_reusing_last_arguments_escapes_markup(tmp_path, run_entry_spy):
    ent = store.add_python(_py(tmp_path, "print(1)\n"), name="j")
    argstate.save_last(ent.slug, extra_args=["[red]arg[/red]"])
    result = runner.invoke(cli.app, ["run", "j", "--no-input"])
    assert result.exit_code == 0, result.output
    assert "[red]arg[/red]" in result.output


def test_collect_command_values_prompt_escapes_markup_in_placeholder_name(monkeypatch, tty):
    """The prompt TEXT (not just its default) is parsed as Rich markup by Prompt.ask, so a
    placeholder name must be escaped there too — checked directly since placeholder names are
    normally identifier-constrained and can't carry markup through the CLI."""
    ent = store.add_command("echo {msg}", name="e")
    ent = dataclasses.replace(ent, meta=dataclasses.replace(ent.meta, params=["[red]msg[/red]"]))
    captured: dict[str, str] = {}

    def fake_ask(prompt, **k):
        captured["prompt"] = prompt
        return "x"

    monkeypatch.setattr(cli.Prompt, "ask", fake_ask)
    promptform.collect(flows.plan_for_entry(ent), {}, console=cli.console)
    assert captured["prompt"] == r"  \[red]msg\[/red]"


def test_collect_param_form_prompt_escapes_markup_in_param_prompt_text(monkeypatch, tty, tmp_path):
    """Same as above for the Python-entry param form: `s.prompt` comes from `--prompt NAME=text`
    and can freely carry markup."""
    text = metawriter.write_params(
        'CITY = "x"\nprint(CITY)\n',
        [ParamDecl(name="CITY", binding="const", type="str", prompt="[red]Where[/red]?")],
    )
    ent = store.add_python(_py(tmp_path, text), name="a")
    captured: dict[str, str] = {}

    def fake_ask(prompt, **k):
        captured["prompt"] = prompt
        return "x"

    monkeypatch.setattr(cli.Prompt, "ask", fake_ask)
    promptform.collect(flows.plan_for_entry(ent), {}, console=cli.console)
    assert captured["prompt"] == r"  \[red]Where\[/red]?"


def test_preset_save_prompt_escapes_markup_in_placeholder_name(monkeypatch, tty):
    ent = store.add_command("echo {msg}", name="e")
    hostile = dataclasses.replace(
        ent, meta=dataclasses.replace(ent.meta, params=["[red]msg[/red]"])
    )
    monkeypatch.setattr(cli.store, "resolve", lambda name: hostile)
    captured: dict[str, str] = {}

    def fake_ask(prompt, **k):
        captured["prompt"] = prompt
        return "x"

    monkeypatch.setattr(cli.Prompt, "ask", fake_ask)
    cli.preset_save("e", "p", from_last=False)
    assert captured["prompt"] == r"  \[red]msg\[/red]"


def test_run_raw_passes_argv_genuinely_raw(tmp_path, run_entry_spy):
    # --raw is the escape hatch: no token pass, no glob pass — even weird argv survives.
    (tmp_path / "match.txt").touch()
    store.add_python(_py(tmp_path, "print(1)\n"), name="rawr")
    result = runner.invoke(
        cli.app, ["run", "rawr", "--raw", "--no-input", "--", "{env:UNSET}", "*.txt"]
    )
    assert result.exit_code == 0, result.output
    assert run_entry_spy["extra"] == ["{env:UNSET}", "*.txt"]


def test_run_cli_argv_not_reexpanded(tmp_path, run_entry_spy, monkeypatch):
    # `-- '*.txt'` already survived the user's shell (they quoted it on purpose);
    # skit must not glob/token it a second time — run() must call assemble with
    # expand_extra=False. (No chdir: it breaks mutmut's stats collection.)
    captured: dict[str, object] = {}
    orig = flows.assemble

    def spy(plan, values, extra, **kw):
        captured.update(kw)
        return orig(plan, values, extra, **kw)

    monkeypatch.setattr(cli.flows, "assemble", spy)
    store.add_python(_py(tmp_path, "print(1)\n"), name="noglob")
    result = runner.invoke(cli.app, ["run", "noglob", "--no-input", "--", "*.txt"])
    assert result.exit_code == 0, result.output
    assert captured["expand_extra"] is False
    assert run_entry_spy["extra"] == ["*.txt"]
