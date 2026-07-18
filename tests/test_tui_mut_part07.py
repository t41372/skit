"""Exact-behavior tests for MenuApp._refresh_footer (tui.py chunk 7).

The footer is skit's mouse+keyboard contract: every advertised key is also a clickable
pill whose visible glyph is the click target and whose @click action must resolve to the
right handler (AGENTS.md principle 2). _refresh_footer builds those pills. These tests pin
the EXACT rendered chip for every hint in both footer rows — search mode and the normal
table mode — so any drift in a chip's action id, key glyph, or label is caught.

A Static keeps the raw markup it was handed in its `.content` property (`.render()` parses
markup away), so `.content` is the exact `tui_footer.bar(...)` string _refresh_footer wrote.
Each expected pill is rebuilt with the real `tui_footer.chip(...)`, so the GLUE/pill markup
matches byte-for-byte and a substring check pins the whole (action, key, label) triple.
"""

from __future__ import annotations

import pytest
from textual.widgets import Static

from skit import argstate, store, tui, tui_footer


@pytest.fixture(autouse=True)
def _en(monkeypatch):
    # English catalog: msgids are the English source, so gettext("Run") == "Run".
    monkeypatch.setenv("SKIT_LANG", "en")


def _py(tmp_path, body="print(1)\n", name="job.py"):
    p = tmp_path / name
    p.write_text(body, encoding="utf-8")
    return p


def _content(app, wid: str) -> str:
    return str(app.query_one(f"#{wid}", Static).content)


async def test_normal_local_footer_pins_every_chip(tmp_path):
    """Table mode with a script that has run before: the local row advertises run, rerun,
    settings, edit and remove, each as its exact clickable pill."""
    entry = store.add_python(_py(tmp_path), name="a")
    argstate.record_run(entry.slug, 0, at="2026-01-01T00:00:00+00:00")
    assert argstate.load_state(entry.slug)["last_run"]  # so the rerun chip is shown
    app = tui.MenuApp()
    async with app.run_test(size=(160, 40)) as pilot:
        await pilot.pause()
        local = _content(app, "keys-local")
        assert tui_footer.chip("app.run", "Enter", "Run") in local
        assert tui_footer.chip("app.rerun", "r", "Rerun") in local
        assert tui_footer.chip("app.settings", "p", "Script settings") in local
        assert tui_footer.chip("app.edit", "e", "Edit script") in local
        assert tui_footer.chip("app.remove", "Del", "Remove") in local


async def test_rerun_chip_absent_until_a_run_is_recorded(tmp_path):
    """The rerun chip is conditional on a recorded last_run — a fresh script must NOT
    advertise it (the `if load_state(...)["last_run"]` guard)."""
    store.add_python(_py(tmp_path), name="fresh")
    app = tui.MenuApp()
    async with app.run_test(size=(160, 40)) as pilot:
        await pilot.pause()
        local = _content(app, "keys-local")
        assert tui_footer.chip("app.run", "Enter", "Run") in local  # run is unconditional
        assert tui_footer.chip("app.rerun", "r", "Rerun") not in local  # no run yet


async def test_normal_global_footer_pins_every_chip(tmp_path):
    """Table mode: the always-present global row advertises add, presets, search, the
    detail-pane toggle, preferences, health and help — each as its exact clickable pill."""
    store.add_python(_py(tmp_path), name="a")
    app = tui.MenuApp()
    async with app.run_test(size=(160, 40)) as pilot:
        await pilot.pause()
        g = _content(app, "keys-global")
        assert tui_footer.chip("app.add", "a", "Add script") in g
        assert tui_footer.chip("app.presets", "s", "Presets") in g
        assert tui_footer.chip("app.focus_search", "/", "Search") in g
        assert tui_footer.chip("app.toggle_detail", "Tab", "Detail pane") in g
        assert tui_footer.chip("app.preferences", ",", "Preferences") in g
        assert tui_footer.chip("app.health", "D", "Health check") in g
        assert tui_footer.chip("app.help", "?", "Help") in g


async def test_search_mode_footer_pins_its_two_chips_and_blanks_global(tmp_path):
    """With the search box focused, single-letter chips would be dead buttons (the letters
    type text), so the local row collapses to exactly Enter-run and Esc-back-to-list and the
    global row is blanked."""
    store.add_python(_py(tmp_path), name="a")
    app = tui.MenuApp()
    async with app.run_test(size=(160, 40)) as pilot:
        await pilot.pause()
        app.action_focus_search()
        await pilot.pause()
        local = _content(app, "keys-local")
        assert tui_footer.chip("app.run", "Enter", "Run") in local
        assert tui_footer.chip("app.leave_search", "Esc", "Back to list") in local
        # The letter chips are gone while typing, and the global row is empty.
        assert tui_footer.chip("app.settings", "p", "Script settings") not in local
        assert _content(app, "keys-global") == ""
