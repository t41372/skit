"""Edit-from-Library behavior: source resolution rules and the suspend/editor round trip."""

from __future__ import annotations

import contextlib
from pathlib import Path

import pytest
from textual.widgets import Static

from skit import store, tui


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
    entry = store.add_python(_py(tmp_path, "print(1)\n"), name="a")
    app = tui.MenuApp()
    assert app._editable_source(entry) == entry.dir / "script.py"


def test_editable_source_reference_mode_points_at_the_original(tmp_path):
    p = _py(tmp_path, "print(1)\n", "orig.py")
    entry = store.add_python(p, name="r", mode="reference")
    app = tui.MenuApp()
    assert app._editable_source(entry) == Path(entry.meta.source)


def test_editable_source_command_entry_has_none(tmp_path):
    entry = store.add_command("echo hi", name="c")
    app = tui.MenuApp()
    assert app._editable_source(entry) is None


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


async def test_edit_invalidates_the_drift_cache(tmp_path, monkeypatch):
    entry = store.add_python(_py(tmp_path, "print(1)\n"), name="a")
    monkeypatch.setattr(tui.editor, "open_in_editor", lambda p: None)
    monkeypatch.setattr(tui.MenuApp, "suspend", lambda self: _noop_suspend())
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        app._drift_cache[entry.slug] = (0.0, True)
        app.action_edit()
        await pilot.pause()
        # The stale sentinel is gone: the reload re-derived the truth from the file.
        mtime, drift = app._drift_cache[entry.slug]
        assert mtime != 0.0
        assert drift is False
