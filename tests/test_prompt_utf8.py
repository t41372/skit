"""Prompt payloads have one strict UTF-8 boundary across every product surface."""

from __future__ import annotations

import contextlib
import hashlib
import json
import os
import stat
import subprocess
import sys
from collections.abc import Callable
from pathlib import Path

import pytest
from textual.widgets import Static
from typer.testing import CliRunner

from skit import cli, config, flows, healthcheck, launcher, paths, store, tui
from skit.langs.prompt import text as prompt_text
from skit.models import Entry, Mode, ScriptMeta
from skit.tui_add import PromptReviewScreen
from skit.tui_settings import ScriptSettingsScreen

runner = CliRunner()


def _invalid_prompt(tmp_path: Path) -> tuple[Path, int]:
    path = tmp_path / "bad.prompt.md"
    data = b"Review {{target}}\r\ninvalid:\xc3(\n"
    path.write_bytes(data)
    return path, data.index(b"\xc3")


@pytest.mark.parametrize("mode", ["copy", "reference"])
def test_store_rejects_invalid_prompt_before_any_entry_write(tmp_path: Path, mode: Mode):
    path, offset = _invalid_prompt(tmp_path)

    with pytest.raises(store.StoreError) as caught:
        store.add_prompt(path, mode=mode)

    message = str(caught.value)
    assert str(path.resolve()) in message
    assert f"offset {offset}" in message
    assert not (tmp_path / "data" / "scripts").exists()
    assert store.list_entries() == []


def test_valid_utf8_crlf_cjk_and_emoji_stays_byte_exact_in_store_and_argv(tmp_path, monkeypatch):
    body = "審查 {{目標}} 👩🏽‍💻\r\n第二行\r\n"
    raw = body.encode("utf-8")
    path = tmp_path / "exact.prompt.md"
    path.write_bytes(raw)
    entry = store.add_prompt(path, mode="copy", managed=[])
    chosen = config.PromptRunner("agent", ("agent", "{{prompt}}"))
    monkeypatch.setattr("skit.langs.launch._which", lambda _name: "/bin/agent")

    assert entry.script_path.read_bytes() == raw
    assert prompt_text.read(entry.script_path) == body
    assert launcher.build_command(entry, runner=chosen) == ["/bin/agent", body]


def _replace_source_before_store(
    monkeypatch: pytest.MonkeyPatch, source: Path, replacement: bytes
) -> None:
    real_add_entry = store._add_entry

    def replace_then_add(
        meta: ScriptMeta,
        *,
        payload: Path | None,
        payload_bytes: bytes | None = None,
        payload_mode: int | None = None,
        after_copy: Callable[[Path], None] | None = None,
    ) -> Entry:
        source.write_bytes(replacement)
        return real_add_entry(
            meta,
            payload=payload,
            payload_bytes=payload_bytes,
            payload_mode=payload_mode,
            after_copy=after_copy,
        )

    monkeypatch.setattr(store, "_add_entry", replace_then_add)


def test_copy_add_stores_the_same_snapshot_it_analyzed_and_hashed(tmp_path, monkeypatch):
    source = tmp_path / "racy.prompt.md"
    original = "# Original\r\nHello {{first}} 🚀\r\n".encode()
    replacement = b"# Replacement\nHello {{second}}\n"
    source.write_bytes(original)
    _replace_source_before_store(monkeypatch, source, replacement)

    entry = store.add_prompt(source, mode="copy")

    assert source.read_bytes() == replacement
    assert entry.script_path.read_bytes() == original
    assert entry.meta.description == "Original"
    assert entry.meta.params == ["first"]
    assert entry.meta.source_hash == f"sha256:{hashlib.sha256(original).hexdigest()}"


def test_reference_add_records_one_snapshot_then_preflight_reads_the_live_body(
    tmp_path, monkeypatch
):
    source = tmp_path / "live.prompt.md"
    original = b"# Original\nHello {{first}}\n"
    replacement = b"invalid live body: \xff\n"
    source.write_bytes(original)
    _replace_source_before_store(monkeypatch, source, replacement)
    chosen = config.PromptRunner("agent", ("agent", "{{prompt}}"))
    monkeypatch.setattr("skit.langs.launch._which", lambda _name: "/bin/agent")

    entry = store.add_prompt(source, mode="reference")

    assert entry.script_path.read_bytes() == replacement
    assert entry.meta.description == "Original"
    assert entry.meta.params == ["first"]
    assert entry.meta.source_hash == f"sha256:{hashlib.sha256(original).hexdigest()}"
    with pytest.raises(launcher.LaunchError, match="offset 19"):
        launcher.preflight(entry, runner=chosen)


def test_prompt_read_error_and_ambiguous_payload_leave_no_store_writes(tmp_path, monkeypatch):
    source = tmp_path / "unreadable.prompt.md"
    source.write_text("Hello", encoding="utf-8")
    real_open = Path.open

    def fail_for_source(path: Path, *args, **kwargs):
        if path == source and args and args[0] == "rb":
            raise PermissionError("blocked by test")
        return real_open(path, *args, **kwargs)

    monkeypatch.setattr(Path, "open", fail_for_source)
    with pytest.raises(store.StoreError, match="blocked by test"):
        store.add_prompt(source)

    with pytest.raises(ValueError, match="mutually exclusive"):
        store._add_entry(
            ScriptMeta(name="ambiguous", kind="prompt"),
            payload=source,
            payload_bytes=b"other",
        )
    with pytest.raises(ValueError, match="payload_mode requires payload_bytes"):
        store._add_entry(
            ScriptMeta(name="mode-only", kind="prompt"),
            payload=None,
            payload_mode=0o600,
        )
    assert store.list_entries() == []
    assert not (tmp_path / "data" / "scripts").exists()


@pytest.mark.skipif(os.name == "nt", reason="POSIX permission bits")
def test_prompt_copy_preserves_private_source_permissions(tmp_path):
    source = tmp_path / "private.prompt.md"
    source.write_text("Private {{topic}}\n", encoding="utf-8")
    source.chmod(0o600)

    entry = store.add_prompt(source, mode="copy")

    assert stat.S_IMODE(entry.script_path.stat().st_mode) == 0o600
    assert entry.script_path.read_bytes() == source.read_bytes()


def test_store_generic_script_api_refuses_prompt_onboarding_bypass(tmp_path):
    source = tmp_path / "bypass.prompt.md"
    source.write_text("Review {{target}}\n", encoding="utf-8")

    with pytest.raises(store.StoreUsageError, match="add_prompt"):
        store.add_script(source, kind="prompt")

    assert store.list_entries() == []


def test_invalid_utf8_prompt_stdin_fails_before_allocating_a_draft(tmp_path):
    bad = b"Review \xff now\n"
    executable = Path(sys.executable).with_name("skit")

    completed = subprocess.run(
        [str(executable), "add", "-", "--kind", "prompt", "--name", "bad-pipe"],
        input=bad,
        capture_output=True,
        check=False,
    )

    output = completed.stdout + completed.stderr
    assert completed.returncode == 1
    assert b"<stdin>" in output
    assert b"offset 7" in output
    assert b"Traceback" not in output
    assert store.list_entries() == []
    assert not paths.drafts_dir().exists() or list(paths.drafts_dir().iterdir()) == []


def test_invalid_utf8_prompt_stdin_cli_boundary_maps_decode_error_to_clean_exit(tmp_path):
    """Exercise the in-process Click stdin wrapper too: subprocess coverage cannot
    attribute the strict-decode exception handler to this pytest process."""
    result = runner.invoke(
        cli.app,
        ["add", "-", "--kind", "prompt", "--name", "bad-in-process"],
        input=b"Review \xff now\n",
    )

    assert result.exit_code == 1
    assert "<stdin>" in result.output
    assert "offset 7" in result.output
    assert isinstance(result.exception, SystemExit)
    assert "Traceback" not in result.output
    assert store.list_entries() == []
    assert not paths.drafts_dir().exists()


def test_add_entry_raw_byte_payload_without_explicit_mode_remains_supported(tmp_path):
    """The generic raw-snapshot seam still has a default-mode path for callers whose
    payload is not permission-derived.  It must persist the bytes and registry row as
    one ordinary entry, rather than silently dropping the payload."""
    raw = b"Review {{target}}\r\n"

    entry = store._add_entry(
        ScriptMeta(name="raw-snapshot", kind="prompt"),
        payload=None,
        payload_bytes=raw,
    )

    assert entry.script_path.read_bytes() == raw
    assert store.resolve("raw-snapshot").script_path == entry.script_path


def test_changed_prompt_is_launch_blocked_and_health_reports_the_same_error(tmp_path, monkeypatch):
    path = tmp_path / "changed.prompt.md"
    path.write_text("Review {{target}}\n", encoding="utf-8")
    entry = store.add_prompt(path, mode="copy", runner="agent")
    bad = b"Review \xff now\n"
    entry.script_path.write_bytes(bad)
    chosen = config.PromptRunner("agent", ("agent", "{{prompt}}"))
    monkeypatch.setattr("skit.langs.launch._which", lambda _name: "/bin/agent")

    with pytest.raises(launcher.LaunchError, match="offset 7"):
        launcher.preflight(entry, runner=chosen)
    with pytest.raises(launcher.LaunchError, match="offset 7"):
        launcher.build_command(entry, runner=chosen)

    plan = flows.plan_for_entry(entry)
    assert plan.source == "none"
    assert plan.text == ""
    assert "�" not in plan.text
    report = healthcheck.collect([entry])
    assert healthcheck.entry_drifted(entry) is False
    assert "offset 7" in report.launch_blocked[entry.meta.name]


@pytest.mark.parametrize("mode", ["copy", "reference"])
def test_cli_edit_refuses_invalid_prompt_bytes_and_the_next_edit_can_repair_them(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, mode: Mode
):
    source = tmp_path / f"cli-{mode}.prompt.md"
    source.write_text("Review {{target}}\n", encoding="utf-8")
    entry = store.add_prompt(source, name=f"cli-{mode}", mode=mode)
    target = entry.script_path
    invalid = b"edited:\xff\n"
    monkeypatch.setattr(cli.editor, "open_in_editor", lambda opened: opened.write_bytes(invalid))

    refused = runner.invoke(cli.app, ["edit", entry.meta.name])

    assert refused.exit_code == 1
    assert "offset 7" in refused.output
    assert "Saved" not in refused.output
    assert target.read_bytes() == invalid  # authored bytes are kept for a corrective edit
    if mode == "copy":
        assert source.read_text(encoding="utf-8") == "Review {{target}}\n"

    repaired = b"Repaired {{target}}\n"
    monkeypatch.setattr(cli.editor, "open_in_editor", lambda opened: opened.write_bytes(repaired))
    accepted = runner.invoke(cli.app, ["edit", entry.meta.name])

    assert accepted.exit_code == 0, accepted.output
    assert "Saved" in accepted.output
    assert target.read_bytes() == repaired


@pytest.mark.parametrize("mode", ["copy", "reference"])
async def test_library_edit_refuses_invalid_prompt_bytes_and_recovers_on_reedit(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, mode: Mode
):
    source = tmp_path / f"tui-{mode}.prompt.md"
    source.write_text("Review {{target}}\n", encoding="utf-8")
    entry = store.add_prompt(source, name=f"tui-{mode}", mode=mode)
    target = entry.script_path
    invalid = b"edited:\xff\n"
    monkeypatch.setattr(tui.MenuApp, "suspend", lambda self: _noop_suspend())
    monkeypatch.setattr(tui.editor, "open_in_editor", lambda opened: opened.write_bytes(invalid))

    app = tui.MenuApp()
    async with app.run_test(size=(100, 32)) as pilot:
        await pilot.pause()
        app.action_edit()
        await pilot.pause()

        status = str(app.query_one("#status", Static).render())
        assert status.startswith("Error:")
        assert "offset 7" in status
        assert "Edited" not in status
        assert target.read_bytes() == invalid
        if mode == "copy":
            assert source.read_text(encoding="utf-8") == "Review {{target}}\n"

        repaired = b"Repaired {{target}}\n"
        monkeypatch.setattr(
            tui.editor, "open_in_editor", lambda opened: opened.write_bytes(repaired)
        )
        app.action_edit()
        await pilot.pause()

        assert str(app.query_one("#status", Static).render()) == f"Edited {entry.meta.name}."
        assert target.read_bytes() == repaired


def test_cli_add_params_run_and_doctor_refuse_corrupt_prompt_cleanly(tmp_path, monkeypatch):
    path, offset = _invalid_prompt(tmp_path)
    added = runner.invoke(cli.app, ["add", str(path), "--prompt", "--no-input"])
    assert added.exit_code == 1
    assert str(path.resolve()) in added.output.replace("\n", "")
    # rich wraps the long (absolute) path, so the "offset N" phrase can straddle a line
    # break on a narrow/Windows path — collapse whitespace runs before the substring check.
    assert f"offset {offset}" in " ".join(added.output.split())
    assert "�" not in added.output
    assert store.list_entries() == []

    path.write_text("Review {{target}}\n", encoding="utf-8")
    entry = store.add_prompt(path, runner="codex")
    entry.script_path.write_bytes(b"broken:\xff\n")

    for args in (["show", entry.meta.name], ["show", entry.meta.name, "--json"]):
        shown = runner.invoke(cli.app, args)
        assert shown.exit_code == 1
        assert "offset 7" in shown.output
        assert "fields" not in shown.output
        assert "No form fields" not in shown.output
        assert "�" not in shown.output

    shown = runner.invoke(cli.app, ["params", entry.meta.name, "--json"])
    assert shown.exit_code == 1
    assert "offset 7" in shown.output
    assert "�" not in shown.output

    run = runner.invoke(
        cli.app,
        ["run", entry.meta.name, "--runner", "codex", "--no-input"],
    )
    assert run.exit_code == 125
    assert "offset 7" in run.output
    assert "�" not in run.output

    monkeypatch.setattr(cli.launcher, "find_uv", lambda: "/bin/uv")
    doctor = runner.invoke(cli.app, ["doctor", "--json"])
    payload = json.loads(doctor.output)
    assert "offset 7" in payload["launch_blocked"][entry.meta.name]


async def test_tui_review_refuses_invalid_initial_body_without_replacement(tmp_path):
    path, offset = _invalid_prompt(tmp_path)
    app = tui.MenuApp()
    async with app.run_test(size=(100, 32)) as pilot:
        await pilot.pause()
        app.push_screen(PromptReviewScreen(path))
        await pilot.pause()
        review = app.screen
        assert isinstance(review, PromptReviewScreen)
        message = str(review.query_one("#pv-text-error", Static).render())
        assert f"offset {offset}" in message
        assert "�" not in message

        review.action_accept()
        await pilot.pause()
        assert app.screen is review
        review.action_cancel()
        await pilot.pause()
    assert store.list_entries() == []


@contextlib.contextmanager
def _noop_suspend():
    yield


async def test_tui_review_rescan_and_settings_handle_new_invalid_bytes(tmp_path, monkeypatch):
    path = tmp_path / "edit.prompt.md"
    path.write_text("Review {{target}}\n", encoding="utf-8")
    monkeypatch.setattr(tui.MenuApp, "suspend", lambda self: _noop_suspend())
    monkeypatch.setattr(
        "skit.tui_add.editor.open_in_editor",
        lambda edited: edited.write_bytes(b"changed:\xff\n"),
    )
    app = tui.MenuApp()
    async with app.run_test(size=(100, 32)) as pilot:
        await pilot.pause()
        app.push_screen(PromptReviewScreen(path))
        await pilot.pause()
        review = app.screen
        assert isinstance(review, PromptReviewScreen)
        review.action_edit_source()
        await pilot.pause()
        assert review._detected == ["target"]  # no schema was derived from U+FFFD
        error = str(review.query_one("#pv-text-error", Static).render())
        assert "offset 8" in error
        review.action_cancel()
        await pilot.pause()

    path.write_text("Review {{target}}\n", encoding="utf-8")
    entry = store.add_prompt(path)
    entry.script_path.write_bytes(b"settings:\xff\n")
    app = tui.MenuApp()
    async with app.run_test(size=(100, 32)) as pilot:
        await pilot.pause()
        screen = ScriptSettingsScreen(entry)
        app.push_screen(screen)
        await pilot.pause()
        error = str(screen.query_one("#st-prompt-text-error", Static).render())
        assert "offset 9" in error
        assert "�" not in error
        screen.action_close()
        await pilot.pause()
