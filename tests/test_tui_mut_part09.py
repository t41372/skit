"""Mutation-kill tests for src/skit/tui.py — chunk 9/10.

Covers the Library action paths: Ctrl+C double-tap quit + toast, edit (no-source
message, editor-error print/prompt/EOF suppression, drift-cache invalidation), the
Script-settings deep-link + close callback, run/rerun guards and value plumbing.

Each test pins real, observable behaviour through the public TUI surface (Textual
`Pilot`) or a direct action call, mirroring tests/test_tui_mut.py.
"""

from __future__ import annotations

import contextlib

import pytest
from textual.containers import VerticalScroll
from textual.widgets import DataTable, Input, Static

from skit import argstate, config, launcher, store, tui
from skit.langs.python import metawriter
from skit.params import ParamDecl
from skit.tui_form import FieldRow, RunFormScreen


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


def _raise_editor_error(_path):
    raise tui.editor.EditorError("editor exploded")


@pytest.fixture
def quiet_run(monkeypatch):
    """Neutralize _execute's terminal-ownership pieces and capture the launch (after_run=stay)."""
    config.save_after_run("stay")
    calls: dict[str, object] = {}

    def fake_run(
        entry,
        extra_args=None,
        *,
        values=None,
        invoke_cwd=None,
        script_override=None,
        env_overlay=None,
    ):
        calls["extra"] = list(extra_args or [])
        calls["values"] = dict(values or {})
        calls["override"] = script_override
        return calls.get("code", 0)

    monkeypatch.setattr(launcher, "run_entry", fake_run)
    monkeypatch.setattr(tui.MenuApp, "suspend", lambda self: _noop_suspend())
    monkeypatch.setattr("builtins.input", lambda *a: "")
    return calls


MANAGED = 'CITY = "Taipei"\nprint(CITY)\n'


def _managed_entry(tmp_path, name="j"):
    text = metawriter.write_params(MANAGED, [ParamDecl(name="CITY", binding="const", type="str")])
    return store.add_python(_py(tmp_path, text, f"{name}.py"), name=name)


ARGPARSE = (
    "import argparse\nap = argparse.ArgumentParser()\n"
    "ap.add_argument('-o', '--output', required=True, help='output path')\n"
    "ap.add_argument('--fast', action='store_true')\n"
    "ap.parse_args()\n"
)


def _argparse_entry(tmp_path):
    return store.add_python(_py(tmp_path, ARGPARSE, "cli.py"), name="cli")


def _detail_text(app) -> str:
    return str(app.query_one("#detail-body", Static).render())


def _status_text(app) -> str:
    return str(app.query_one("#status", Static).render())


# ---------------------------------------------------------------------------
# action_ctrl_c_quit
# ---------------------------------------------------------------------------


async def test_ctrl_c_at_exact_window_boundary_quits(tmp_path):
    """The second Ctrl+C quits when it lands within CTRL_C_WINDOW — inclusive of the
    exact boundary (`<=`). Freeze the clock only across the single synchronous action
    call (no await between patch and restore, so Textual's own timing never sees it)."""
    store.add_python(_py(tmp_path, "print(1)\n"), name="a")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        await pilot.pause()
        app._ctrl_c_at = 0.0  # gap = CTRL_C_WINDOW - 0 == the window exactly
        real_monotonic = tui.time.monotonic
        tui.time.monotonic = lambda: float(tui.MenuApp.CTRL_C_WINDOW)  # ty: ignore[invalid-assignment]
        try:
            app.action_ctrl_c_quit()
        finally:
            tui.time.monotonic = real_monotonic
        assert app.return_value == 0  # `<` would miss the boundary and not quit


async def test_ctrl_c_first_press_shows_quit_toast(tmp_path):
    """A lone Ctrl+C (no prior press) does NOT quit — it arms the double-tap with a
    notification whose exact copy and 2s timeout are the advertised contract."""
    store.add_python(_py(tmp_path, "print(1)\n"), name="a")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        await pilot.pause()
        app.action_ctrl_c_quit()
        await pilot.pause()
        assert app.return_value is None  # first press never quits
        notes = list(app._notifications)
        assert [n.message for n in notes] == ["Press Ctrl+C again to quit"]
        assert notes[0].timeout == app.CTRL_C_WINDOW


# ---------------------------------------------------------------------------
# action_edit
# ---------------------------------------------------------------------------


async def test_edit_command_entry_reports_exact_no_source_message(tmp_path):
    store.add_command("echo hi", name="c")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        await pilot.pause()
        app.action_edit()
        await pilot.pause()
        status = _status_text(app)
        assert status == "c has no editable source (programs and command templates run as-is)."
        assert not status.startswith("XX")


async def test_edit_editor_error_prints_message_and_prompts_return(tmp_path, monkeypatch):
    """When $EDITOR won't launch, e prints the error and pauses on the exact
    'Press Enter to return' prompt before recovering."""
    store.add_python(_py(tmp_path, "print(1)\n"), name="j")
    monkeypatch.setattr(tui.editor, "open_in_editor", _raise_editor_error)
    monkeypatch.setattr(tui.MenuApp, "suspend", lambda self: _noop_suspend())
    printed: list[str] = []
    prompts: list[object] = []
    monkeypatch.setattr("builtins.print", lambda *a, **k: printed.append(" ".join(map(str, a))))

    def fake_input(prompt=""):
        prompts.append(prompt)
        return ""

    monkeypatch.setattr("builtins.input", fake_input)
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        await pilot.pause()
        app.action_edit()
        await pilot.pause()
        assert any("editor exploded" in line for line in printed)
        assert prompts == ["Press Enter to return"]


async def test_edit_editor_error_suppresses_eof_and_recovers(tmp_path, monkeypatch):
    """A ^D (EOFError) at the acknowledge prompt is swallowed: the workbench still
    reloads and reports, rather than crashing out of action_edit."""
    store.add_python(_py(tmp_path, "print(1)\n"), name="j")
    monkeypatch.setattr(tui.editor, "open_in_editor", _raise_editor_error)
    monkeypatch.setattr(tui.MenuApp, "suspend", lambda self: _noop_suspend())
    monkeypatch.setattr("builtins.print", lambda *a, **k: None)

    def raise_eof(*_a):
        raise EOFError

    monkeypatch.setattr("builtins.input", raise_eof)
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        await pilot.pause()
        app.action_edit()  # must NOT propagate the EOFError
        await pilot.pause()
        assert "Edited j." in _status_text(app)


async def test_edit_invalidates_fresh_drift_cache_entry(tmp_path, monkeypatch):
    """The stored copy may change under the editor, so edit drops this slug's drift
    cache entry — even a fresh (mtime-matching) one — forcing a re-derivation. A stale
    'has drift' sentinel that survives would light a false 'script changed' warning."""
    entry = store.add_python(_py(tmp_path, "print(1)\n"), name="a")
    monkeypatch.setattr(tui.editor, "open_in_editor", lambda p: None)
    monkeypatch.setattr(tui.MenuApp, "suspend", lambda self: _noop_suspend())
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        await pilot.pause()
        # A cache entry whose mtime matches the file on disk: only the pop can clear it
        # (an mtime mismatch would recompute regardless, masking the bug).
        mtime = entry.script_path.stat().st_mtime
        app._drift_cache[entry.slug] = (mtime, True)
        app.action_edit()
        await pilot.pause()
        assert "The script changed" not in _detail_text(app)  # re-derived: no real drift


async def test_edit_drift_cache_pop_tolerates_an_absent_slug(tmp_path, monkeypatch):
    """The post-edit pop must not assume the slug is cached — dropping pop's default
    would raise KeyError and crash the edit whenever drift was never computed."""
    store.add_python(_py(tmp_path, "print(1)\n"), name="j")
    monkeypatch.setattr(tui.editor, "open_in_editor", lambda p: None)
    monkeypatch.setattr(tui.MenuApp, "suspend", lambda self: _noop_suspend())
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        await pilot.pause()
        app._drift_cache.clear()  # slug is absent when the pop runs
        app.action_edit()  # a KeyError here would crash the edit
        await pilot.pause()
        assert "Edited j." in _status_text(app)


# ---------------------------------------------------------------------------
# action_settings / action_presets
# ---------------------------------------------------------------------------


def _many_param_entry(tmp_path):
    body = "\n".join(f'C{i} = "v{i}"' for i in range(12)) + "\nprint(1)\n"
    decls = [ParamDecl(name=f"C{i}", binding="const", type="str") for i in range(12)]
    text = metawriter.write_params(body, decls)
    return store.add_python(_py(tmp_path, text, "many.py"), name="many")


async def test_presets_deeplink_scrolls_settings_to_the_presets_section(tmp_path):
    """s (action_presets) opens Script settings deep-linked to the Presets section
    (section='presets'), which scrolls the body to it. Any other section value leaves
    the body at the top."""
    _many_param_entry(tmp_path)
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        await pilot.pause()
        app.action_presets()
        body = None
        for _ in range(40):
            await pilot.pause()
            body = app.screen.query_one("#st-body", VerticalScroll)
            if body.scroll_offset.y > 20:
                break
        assert body is not None
        assert body.scroll_offset.y > 20  # deep-linked to Presets, far below the fold


async def test_settings_close_reloads_the_library(tmp_path):
    """Closing Script settings runs the _closed callback → _reload, so a store change
    made while it was open surfaces in the Library. A dropped/None callback would leave
    the table stale."""
    store.add_python(_py(tmp_path, "print(1)\n", "a.py"), name="a")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        await pilot.pause()
        app.action_settings()
        await pilot.pause()
        from skit.tui_settings import ScriptSettingsScreen

        assert isinstance(app.screen, ScriptSettingsScreen)
        # A second entry appears while the settings screen is open.
        store.add_python(_py(tmp_path, "print(2)\n", "b.py"), name="b")
        app.screen.dismiss(False)
        await pilot.pause()
        assert app.query_one(DataTable).row_count == 2  # _closed reloaded the table


async def test_settings_close_invalidates_fresh_drift_cache_entry(tmp_path):
    """The _closed callback pops this slug's drift cache (a Resync could have changed
    the definitions), even a fresh mtime-matching entry. A stale sentinel would show a
    bogus 'script changed' warning after closing."""
    entry = store.add_python(_py(tmp_path, "print(1)\n"), name="a")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        await pilot.pause()
        app.action_settings()
        await pilot.pause()
        mtime = entry.script_path.stat().st_mtime
        app._drift_cache[entry.slug] = (mtime, True)
        app.screen.dismiss(False)
        await pilot.pause()
        assert "The script changed" not in _detail_text(app)


async def test_settings_close_drift_pop_tolerates_an_absent_slug(tmp_path):
    """The _closed pop must survive an uncached slug: dropping pop's default raises
    KeyError inside the dismiss callback, which surfaces as an app crash."""
    store.add_python(_py(tmp_path, "print(1)\n"), name="a")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        await pilot.pause()
        app.action_settings()
        await pilot.pause()
        from skit.tui_settings import ScriptSettingsScreen

        assert isinstance(app.screen, ScriptSettingsScreen)
        app._drift_cache.clear()  # slug absent when _closed's pop runs
        app.screen.dismiss(False)
        await pilot.pause()
        # The screen closed cleanly; a KeyError in _closed would fault the app on exit.
        assert not isinstance(app.screen, ScriptSettingsScreen)


# ---------------------------------------------------------------------------
# action_run
# ---------------------------------------------------------------------------


async def test_run_is_blocked_while_another_screen_is_open(tmp_path, quiet_run):
    """action_run is a no-op unless the Library screen is on top (screen_stack == 1):
    a key that bubbles out of a pushed screen must not launch underneath it."""
    store.add_python(_py(tmp_path, "print(1)\n"), name="a")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        await pilot.pause()
        app.push_screen(tui.HelpScreen())  # screen_stack == 2
        await pilot.pause()
        app.action_run()
        await pilot.pause()
        assert "values" not in quiet_run  # nothing launched under the overlay


async def test_run_preflight_error_shows_the_real_message(tmp_path, quiet_run):
    """A failing preflight (missing target) reports the actual LaunchError text, not a
    stringified None."""
    entry = store.add_python(_py(tmp_path, "print(1)\n"), name="a")
    entry.script_path.unlink()  # target gone → TargetMissingError
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        await pilot.pause()
        app.action_run()
        await pilot.pause()
        status = _status_text(app)
        assert "doesn't exist" in status
        assert status != "Error: None"


async def test_run_without_fields_forwards_saved_extra_args(tmp_path, quiet_run):
    """The no-fields fast path launches with THIS slug's remembered extra args."""
    entry = store.add_python(_py(tmp_path, "print(1)\n"), name="plain")
    argstate.save_last(entry.slug, extra_args=["--foo", "bar"])
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        await pilot.pause()
        app.action_run()
        await pilot.pause()
        assert quiet_run["extra"] == ["--foo", "bar"]  # not [] from load_state(None)


async def test_run_form_prefills_from_this_slugs_saved_values(tmp_path, quiet_run):
    """The run form is prefilled from THIS slug's remembered values."""
    entry = _argparse_entry(tmp_path)
    argstate.save_last(entry.slug, values={"output": "saved.png"})
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        await pilot.pause()
        app.action_run()
        await pilot.pause()
        screen = app.screen
        assert isinstance(screen, RunFormScreen)
        row = next(r for r in screen.query(FieldRow) if r.field.key == "output")
        assert row.query_one(Input).value == "saved.png"


async def test_run_form_submit_marks_drift_already_shown(tmp_path):
    """The form already displays the drift banner, so its submit executes with
    show_drift=False — carried through to the PendingRun in exit mode."""
    config.save_after_run("exit")
    _argparse_entry(tmp_path)
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        await pilot.pause()
        app.action_run()
        await pilot.pause()
        screen = app.screen
        assert isinstance(screen, RunFormScreen)
        row = next(r for r in screen.query(FieldRow) if r.field.key == "output")
        row.query_one(Input).value = "out.png"
        screen.action_submit()
        await pilot.pause()
        pending = app.return_value
        assert isinstance(pending, tui.PendingRun)
        assert pending.show_drift is False


# ---------------------------------------------------------------------------
# action_rerun
# ---------------------------------------------------------------------------


async def test_rerun_before_first_run_shows_exact_hint(tmp_path):
    """Rerun before any run points the user at Enter — the exact untranslated copy
    (not an XX-wrapped msgid)."""
    store.add_python(_py(tmp_path, "print(1)\n"), name="fresh")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        await pilot.pause()
        app.action_rerun()
        await pilot.pause()
        status = _status_text(app)
        assert status.startswith("fresh hasn't run yet")
        assert "press Enter to fill the form first." in status
        assert not status.startswith("XX")


async def test_rerun_preflight_error_shows_the_real_message(tmp_path, quiet_run):
    """Rerun's preflight failure reports the actual LaunchError, not str(None)."""
    entry = store.add_python(_py(tmp_path, "print(1)\n"), name="a")
    argstate.record_run(entry.slug, 0, at="2026-07-09T00:00:00+00:00")  # pass the first-run guard
    entry.script_path.unlink()  # then the target is gone
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        await pilot.pause()
        app.action_rerun()
        await pilot.pause()
        status = _status_text(app)
        assert "doesn't exist" in status
        assert status != "Error: None"


async def test_rerun_forwards_this_slugs_saved_extra_args(tmp_path, quiet_run):
    """Rerun launches with THIS slug's remembered extra args (not [] from a None slug)."""
    entry = _managed_entry(tmp_path)
    argstate.save_last(entry.slug, values={"CITY": "Kaohsiung"}, extra_args=["--bar"])
    argstate.record_run(entry.slug, 0, at="2026-07-09T00:00:00+00:00")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        await pilot.pause()
        app.action_rerun()
        await pilot.pause()
        assert quiet_run["extra"] == ["--bar"]


# ---------------------------------------------------------------------------
# action_health
# ---------------------------------------------------------------------------


async def test_health_jump_selects_the_returned_slug(tmp_path):
    """Health dismisses with a slug; the _jump callback reloads and moves the Library
    cursor onto that script. A missing callback (or a None slug) leaves it put."""
    store.add_python(_py(tmp_path, "print(1)\n", "a.py"), name="a")
    store.add_python(_py(tmp_path, "print(2)\n", "b.py"), name="b")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        await pilot.pause()
        selected = app._selected()
        assert selected is not None
        before = selected.slug
        target = next(e.slug for e in store.list_entries() if e.slug != before)
        app.action_health()
        await pilot.pause()
        from skit.tui_health import HealthScreen

        assert isinstance(app.screen, HealthScreen)
        app.screen.dismiss(target)
        await pilot.pause()
        jumped = app._selected()
        assert jumped is not None
        assert jumped.slug == target  # jumped to the returned slug


# ---------------------------------------------------------------------------
# action_focus_search / action_back_or_quit (behaviour of the pragma'd helpers)
# ---------------------------------------------------------------------------


async def test_focus_search_moves_the_keyboard_to_the_search_box(tmp_path):
    store.add_python(_py(tmp_path, "print(1)\n"), name="a")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        await pilot.pause()
        assert isinstance(app.focused, DataTable)
        app.action_focus_search()
        await pilot.pause()
        assert app.focused is app.query_one("#search", Input)


async def test_back_or_quit_from_search_returns_to_table_without_quitting(tmp_path):
    store.add_python(_py(tmp_path, "print(1)\n"), name="a")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        await pilot.pause()
        app.action_focus_search()
        await pilot.pause()
        app.action_back_or_quit()
        await pilot.pause()
        assert app.focused is app.query_one(DataTable)
        assert app.return_value is None  # left search, did NOT quit


async def test_back_or_quit_from_table_quits(tmp_path):
    store.add_python(_py(tmp_path, "print(1)\n"), name="a")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        await pilot.pause()
        assert isinstance(app.focused, DataTable)
        app.action_back_or_quit()
        assert app.return_value == 0  # Esc on the table quits cleanly
