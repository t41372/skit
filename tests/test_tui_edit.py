"""Edit-from-Library behavior: source resolution rules and the suspend/editor round trip."""

from __future__ import annotations

import contextlib
from pathlib import Path

import pytest
from textual.widgets import Static

from conftest import plan_cache_key
from skit import flows, launcher, store, tui


@pytest.fixture(autouse=True)
def tmp_store(tmp_path, monkeypatch):
    monkeypatch.setenv("SKIT_DATA_DIR", str(tmp_path / "data"))
    monkeypatch.setenv("SKIT_STATE_DIR", str(tmp_path / "state"))
    monkeypatch.setenv("SKIT_CONFIG_DIR", str(tmp_path / "config"))
    monkeypatch.setenv("SKIT_LANG", "en")


def _py(tmp_path, body: str, name: str = "job.py"):
    p = tmp_path / name
    p.write_text(body, encoding="utf-8")
    return p


@contextlib.contextmanager
def _noop_suspend():
    yield


def test_editable_source_copy_mode_points_at_the_stored_copy(tmp_path):
    # ROUND 12: the target now comes from launcher.plan_edit, shared with `skit edit` —
    # the TUI used to derive it alone and collapsed three refusals into one message.
    entry = store.add_python(_py(tmp_path, "print(1)\n"), name="a")
    assert launcher.plan_edit(entry).target == entry.dir / "script.py"


def test_editable_source_reference_mode_points_at_the_original(tmp_path):
    p = _py(tmp_path, "print(1)\n", "orig.py")
    entry = store.add_python(p, name="r", mode="reference")
    plan = launcher.plan_edit(entry)
    assert plan.target == Path(entry.meta.source)
    assert plan.edits_original is True  # …and the face announces whose file it is


def test_editable_source_command_entry_has_none(tmp_path):
    entry = store.add_command("echo hi", name="c")
    plan = launcher.plan_edit(entry)
    assert plan.target is None
    assert plan.refusal == "not-editable"
    assert tui.MenuApp._can_edit(entry) is False  # …so the row offers no `e` chip


async def test_edit_opens_editor_and_reports(tmp_path, monkeypatch):
    entry = store.add_python(_py(tmp_path, "print(1)\n"), name="a")
    opened: list[Path] = []
    monkeypatch.setattr(tui.editor, "open_in_editor", opened.append)
    monkeypatch.setattr(tui.MenuApp, "suspend", lambda self: _noop_suspend())
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        app.action_edit()
        await pilot.pause()
        assert opened == [entry.dir / "script.py"]
        assert "Edited a." in str(app.query_one("#status", Static).render())


async def test_edit_command_entry_reports_no_source(tmp_path, monkeypatch):
    store.add_command("echo hi", name="c")
    opened: list[Path] = []
    monkeypatch.setattr(tui.editor, "open_in_editor", opened.append)
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        app.action_edit()
        await pilot.pause()
        assert opened == []
        assert "no editable source" in str(app.query_one("#status", Static).render())


async def test_edit_invalidates_the_plan_cache(tmp_path, monkeypatch):
    entry = store.add_python(_py(tmp_path, "print(1)\n"), name="a")
    monkeypatch.setattr(tui.editor, "open_in_editor", lambda p: None)
    monkeypatch.setattr(tui.MenuApp, "suspend", lambda self: _noop_suspend())
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        # A stale sentinel under an impossible key, carrying drift the file doesn't
        # have: only the post-edit pop + re-derivation can replace it.
        stale = flows.FormPlan(source="none", drift_lines=["stale sentinel"])
        app._plan_cache[entry.slug] = ((0, 0, 0, 0, None), stale)
        app.action_edit()
        await pilot.pause()
        # The stale sentinel is gone: the reload re-derived the truth from the files.
        key, plan = app._plan_cache[entry.slug]
        assert key == plan_cache_key(entry)
        assert plan is not stale
        assert plan.drift_lines == []
