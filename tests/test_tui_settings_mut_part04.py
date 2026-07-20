"""Mutation-kill tests for ScriptSettingsScreen._compose_storage / _compose_presets.

Every assertion pins a real rendered string or CSS class of the live Script settings
screen, composed through a Textual `Pilot` exactly as a user sees it — the Storage
section (copy vs. reference wording) and the Presets section (empty-state hint, the
"untick to delete" hint, and the per-preset checkbox summary line).
"""

from __future__ import annotations

import pytest
from textual.widgets import Checkbox, Static

from skit import argstate, i18n, store
from skit.tui import MenuApp
from skit.tui_settings import ScriptSettingsScreen


@pytest.fixture(autouse=True)
def tmp_store(tmp_path, monkeypatch):
    monkeypatch.setenv("SKIT_DATA_DIR", str(tmp_path / "data"))
    monkeypatch.setenv("SKIT_STATE_DIR", str(tmp_path / "state"))
    monkeypatch.setenv("SKIT_CONFIG_DIR", str(tmp_path / "config"))
    monkeypatch.setenv("SKIT_LANG", "en")
    i18n.init("en")  # message assertions read the English (msgid) catalog


def _py(tmp_path, name="job.py", body="print(1)\n"):
    p = tmp_path / name
    p.write_text(body, encoding="utf-8")
    return p


def _section_labels(screen) -> list[str]:
    """The exact text of every widget carrying the ``section`` CSS class."""
    return [str(w.render()).strip() for w in screen.query(".section")]


def _find_static(screen, prefix: str) -> Static | None:
    """The first Static whose rendered text starts with ``prefix`` (any class)."""
    for w in screen.query(Static):
        if str(w.render()).startswith(prefix):
            return w
    return None


# ---------------------------------------------------------------------------
# _compose_storage
# ---------------------------------------------------------------------------


async def test_storage_section_copy_mode_wording_and_classes(tmp_path):
    """Copy-mode entry: the 'Storage' section header (text + .section class) and the
    'Keep a copy …' hint (exact wording + source path + .hint class)."""
    entry = store.add_python(_py(tmp_path, "orig.py"), name="j")  # default mode = copy
    assert entry.meta.mode == "copy"
    app = MenuApp()
    async with app.run_test() as pilot:
        await pilot.pause()
        screen = ScriptSettingsScreen(entry)
        app.push_screen(screen)
        await pilot.pause()

        # Header text AND its .section class in one exact-membership check.
        assert "Storage" in _section_labels(screen)

        # The copy-mode branch (meta.mode == "copy") shows the "keep a copy" reassurance,
        # with the real source path substituted in and the .hint class applied.
        hint = _find_static(screen, "Keep a copy — your original file is never modified. Source:")
        assert hint is not None
        assert "orig.py" in str(hint.render())  # %(path)s substituted with the source
        assert hint.has_class("hint")
        # The reference-branch wording must NOT appear for a copy entry.
        assert _find_static(screen, "Linked to the original:") is None


async def test_storage_section_reference_mode_wording_and_classes(tmp_path):
    """Reference-mode entry takes the else branch: the 'Linked to the original …' hint
    (exact wording + source path + .hint class), and NOT the copy wording."""
    entry = store.add_python(_py(tmp_path, "linked_orig.py"), name="linked", mode="reference")
    assert entry.meta.mode == "reference"
    app = MenuApp()
    async with app.run_test() as pilot:
        await pilot.pause()
        screen = ScriptSettingsScreen(entry)
        app.push_screen(screen)
        await pilot.pause()

        link = _find_static(screen, "Linked to the original:")
        assert link is not None
        assert "linked_orig.py" in str(link.render())  # %(path)s substituted
        assert link.has_class("hint")
        # The copy-mode wording must NOT appear for a reference entry.
        assert _find_static(screen, "Keep a copy") is None


# ---------------------------------------------------------------------------
# _compose_presets
# ---------------------------------------------------------------------------


async def test_presets_section_empty_state(tmp_path):
    """No presets yet: the 'Presets' section header (text + .section class) and the
    empty-state 'None yet — press Ctrl+S …' hint (exact wording + .hint class)."""
    entry = store.add_python(_py(tmp_path), name="j")
    assert argstate.load_state(entry.slug)["presets"] == {}
    app = MenuApp()
    async with app.run_test() as pilot:
        await pilot.pause()
        screen = ScriptSettingsScreen(entry)
        app.push_screen(screen)
        await pilot.pause()

        assert "Presets" in _section_labels(screen)

        none_hint = _find_static(screen, "None yet — press Ctrl+S inside the run form to save one.")
        assert none_hint is not None
        assert none_hint.has_class("hint")
        # With no presets, the delete-affordance hint and preset checkboxes are absent.
        assert _find_static(screen, "Untick a preset") is None
        assert not screen.query("#st-preset-0")


async def test_presets_section_with_presets_lists_checkbox_summary(tmp_path):
    """With presets: the 'Untick a preset to delete it on save:' hint (wording + .hint
    class) and a checkbox per preset whose label carries the ", "-joined value summary."""
    entry = store.add_python(_py(tmp_path), name="j")
    argstate.save_preset(entry.slug, "p1", {"CITY": "Taipei", "N": "3"})
    app = MenuApp()
    async with app.run_test() as pilot:
        await pilot.pause()
        screen = ScriptSettingsScreen(entry)
        app.push_screen(screen)
        await pilot.pause()

        untick = _find_static(screen, "Untick a preset to delete it on save:")
        assert untick is not None
        assert untick.has_class("hint")
        # The empty-state hint must NOT appear once a preset exists.
        assert _find_static(screen, "None yet") is None

        # One checkbox per preset; its label is 'name  k=v, k=v' — the ", " separator joins
        # the two values (mutating it to "XX, XX" corrupts the summary text).
        cb = screen.query_one("#st-preset-0", Checkbox)
        assert str(cb.label) in ("p1  CITY=Taipei, N=3", "p1  N=3, CITY=Taipei")
