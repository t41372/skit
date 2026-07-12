"""Keyboard navigation on the form screens (zero memorization: the footer says how to
move, and arrows work wherever Tab works).

↓/↑ are Tab/Shift+Tab's arrow twins (tui_footer.FIELD_NAV_BINDINGS); they fire only
when the focused widget doesn't claim the arrows itself (a RadioSet keeps them for its
options). Every form screen advertises BOTH directions with the clickable key-only pills
"Tab/↓" and "Shift+Tab/↑", and every form screen boots with its FIRST CONTROL focused —
never the body scroll container. Policy: an advertised key needs a positive pilot test
per surface; this file is that, for every form surface — forward AND back, key AND chip.
Tab/Shift+Tab are pressed literally (not assumed from Textual's built-ins): a future
priority binding could claim either exactly as the Library already claims plain Tab.
"""

from __future__ import annotations

import pytest
from textual.widgets import Input, RadioSet, Select

from conftest import click_label
from skit import store, tui
from skit.langs.python import metawriter
from skit.langs.python.metawriter import ParamSpec
from skit.tui_add import AddReviewScreen
from skit.tui_form import RunFormScreen
from skit.tui_prefs import PreferencesScreen
from skit.tui_settings import ScriptSettingsScreen


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


def _two_field_entry(tmp_path):
    text = metawriter.write_params(
        'CITY = "x"\nNAME = "y"\nprint(CITY, NAME)\n',
        [
            ParamSpec(name="CITY", kind="const", type="str", default="x"),
            ParamSpec(name="NAME", kind="const", type="str", default="y"),
        ],
    )
    return store.add_python(_py(tmp_path, text), name="two")


def _focused_id(app) -> str:
    """ty-friendly focused-widget id (asserts something HAS focus first)."""
    assert app.focused is not None
    return app.focused.id or ""


async def test_run_form_boots_typeable_and_arrows_walk_the_fields(tmp_path):
    _two_field_entry(tmp_path)
    app = tui.MenuApp()
    async with app.run_test(size=(130, 40)) as pilot:
        app.action_run()
        await pilot.pause()
        assert isinstance(app.screen, RunFormScreen)
        first = app.focused
        assert isinstance(first, Input)  # first field, ready to type
        await pilot.press("down")
        second = app.focused
        assert second is not first  # ↓ moved on
        await pilot.press("up")
        assert app.focused is first  # ↑ came back
        await click_label(pilot, "#form-keys", "Tab/↓")
        assert app.focused is second  # the chip is the same action, clickable
        await click_label(pilot, "#form-keys", "Shift+Tab/↑")
        assert app.focused is first  # and the back chip walks the other way
        await pilot.press("tab")  # the advertised keys themselves, not just the chips
        assert app.focused is second
        await pilot.press("shift+tab")
        assert app.focused is first


async def test_add_source_arrows_walk_path_template_name(tmp_path):
    app = tui.MenuApp()
    async with app.run_test(size=(130, 40)) as pilot:
        app.action_add()
        await pilot.pause()
        assert _focused_id(app) == "add-path"
        await pilot.press("down")
        assert _focused_id(app) == "add-template"
        await pilot.press("up")
        assert _focused_id(app) == "add-path"
        await click_label(pilot, "#add-keys", "Tab/↓")
        assert _focused_id(app) == "add-template"
        await click_label(pilot, "#add-keys", "Shift+Tab/↑")
        assert _focused_id(app) == "add-path"
        await pilot.press("tab")  # the advertised keys themselves, not just the chips
        assert _focused_id(app) == "add-template"
        await pilot.press("shift+tab")
        assert _focused_id(app) == "add-path"


async def test_add_review_boots_on_name_and_arrows_move(tmp_path):
    p = _py(tmp_path, 'CITY = "x"\nprint(CITY)\n')
    app = tui.MenuApp()
    async with app.run_test(size=(130, 40)) as pilot:
        app.push_screen(AddReviewScreen(p))
        await pilot.pause()
        assert _focused_id(app) == "rv-name"  # not the body scroll container
        await pilot.press("down")
        assert _focused_id(app) == "rv-desc"
        await pilot.press("up")
        assert _focused_id(app) == "rv-name"
        await click_label(pilot, "#review-keys", "Tab/↓")
        assert _focused_id(app) == "rv-desc"
        await click_label(pilot, "#review-keys", "Shift+Tab/↑")
        assert _focused_id(app) == "rv-name"
        await pilot.press("tab")  # the advertised keys themselves, not just the chips
        assert _focused_id(app) == "rv-desc"
        await pilot.press("shift+tab")
        assert _focused_id(app) == "rv-name"


async def test_prefs_boots_on_language_and_arrows_move(tmp_path):
    app = tui.MenuApp()
    async with app.run_test(size=(130, 40)) as pilot:
        app.push_screen(PreferencesScreen())
        await pilot.pause()
        assert isinstance(app.focused, Select)  # the language dropdown, not the scroll
        app.screen.query_one("#pf-editor", Input).focus()
        await pilot.pause()
        await pilot.press("down")
        radio = app.focused
        assert isinstance(radio, RadioSet)  # moved into the form-style section
        # Inside a RadioSet the arrows belong to the OPTIONS — leaving it is Tab's job
        # (or the chip): the shared bindings must not steal them.
        await pilot.press("down")
        assert app.focused is radio  # still the same widget…
        await click_label(pilot, "#pf-keys", "Tab/↓")
        assert app.focused is not radio  # …and the chip moves on to the next section
        await click_label(pilot, "#pf-keys", "Shift+Tab/↑")
        assert app.focused is radio  # …and the back chip returns to it
        await pilot.press("shift+tab")  # the advertised keys themselves, not just the chips
        assert _focused_id(app) == "pf-editor"
        await pilot.press("tab")
        assert app.focused is radio


async def test_settings_boots_on_name_and_arrows_move(tmp_path):
    entry = _two_field_entry(tmp_path)
    app = tui.MenuApp()
    async with app.run_test(size=(130, 40)) as pilot:
        app.push_screen(ScriptSettingsScreen(entry))
        await pilot.pause()
        assert _focused_id(app) == "st-name"
        await pilot.press("down")
        second = app.focused
        assert second is not None
        assert second.id != "st-name"
        await pilot.press("up")
        assert _focused_id(app) == "st-name"
        await click_label(pilot, "#st-keys", "Tab/↓")
        assert app.focused is second
        await click_label(pilot, "#st-keys", "Shift+Tab/↑")
        assert _focused_id(app) == "st-name"
        await pilot.press("tab")  # the advertised keys themselves, not just the chips
        assert app.focused is second
        await pilot.press("shift+tab")
        assert _focused_id(app) == "st-name"
