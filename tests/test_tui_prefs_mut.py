"""Mutation-hardening tests for the Preferences (,) screen (skit.tui_prefs).

These pin the OBSERVABLE contracts the existing coverage left un-nailed: the panel's
English border title, the "nothing selected -> off" per-axis fallback, the exact string
the save writes for the *default* (else-branch) form/after choices, and the widget each
save actually reads (the after-run radio must be `#pf-after`, not merely the first
RadioSet on screen). Every footer chip/key the screen advertises also gets a positive
pilot test. The verbatim https error copy and the per-axis custom reveal are already
letter-exact in test_tui_prefs_health_cov.py and are not repeated here. English catalog
throughout, so the message assertions read against the original msgids.
"""

from __future__ import annotations

import pytest
from textual.widgets import Input, RadioButton, RadioSet, Static

from conftest import click_label
from skit import config, i18n, tui
from skit.tui_prefs import _MASTER_CHOICES, PreferencesScreen


@pytest.fixture(autouse=True)
def _en(monkeypatch):
    monkeypatch.setenv("SKIT_LANG", "en")
    i18n.set_language("en")


async def _open_prefs(app, on_result=None):
    """Push the Preferences screen and settle it; return the live screen."""
    if on_result is None:
        app.push_screen(PreferencesScreen())
    else:
        app.push_screen(PreferencesScreen(), on_result)
    return app.screen


# ---------------------------------------------------------------------------
# on_mount: the panel border title
# ---------------------------------------------------------------------------


async def test_on_mount_sets_english_border_title(tmp_path):
    """on_mount writes the translated panel title onto the body border. The English
    catalog resolves it to exactly "Preferences" — pins the msgid, the gettext call,
    and that a title is set at all (None/blank/case mutants all diverge)."""
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        screen = await _open_prefs(app)
        await pilot.pause()
        assert str(screen.query_one("#pf-body").border_title) == "Preferences"


# ---------------------------------------------------------------------------
# _axis_choice: the "nothing pressed -> off" fallback
# ---------------------------------------------------------------------------


async def test_axis_choice_falls_back_to_off_when_no_button_pressed(tmp_path):
    """RadioSet.pressed_index is -1 when no button is pressed (its documented "none
    selected" state), which is outside the choices range, so _axis_choice must return
    the safe "off". The master row of a live preset config is deselected to reach that
    state; saving from it then takes the master-off branch — the mirror pauses (enabled
    False) while the stored pypi URL survives, exactly as if "off" had been pressed."""
    config.save_mirror(config.compose(pypi=config.PYPI_PRESETS["tsinghua"]))
    results: list[bool | None] = []
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        screen = await _open_prefs(app, results.append)
        await pilot.pause()
        assert isinstance(screen, PreferencesScreen)
        master_set = screen.query_one("#pf-mirror-master", RadioSet)
        assert master_set.pressed_index == 0  # an enabled config starts on "on"
        master_set._pressed_button = None  # documented "none pressed" state -> index -1
        assert master_set.pressed_index == -1
        assert screen._axis_choice("#pf-mirror-master", _MASTER_CHOICES) == "off"
        screen.action_save()
        await pilot.pause()
    mirror = config.load_mirror()
    assert mirror.enabled is False  # the "off" fallback paused the mirror
    assert mirror.pypi == config.PYPI_PRESETS["tsinghua"]  # pause, don't destroy
    assert results == [True]


# ---------------------------------------------------------------------------
# _toggle_custom: the shared error slot reveals for ANY single custom axis
# ---------------------------------------------------------------------------


async def test_error_slot_appears_for_a_lone_github_custom(tmp_path):
    """The error slot's display is an or-chain over the three axes. The cov suite's
    pypi-first flow never shows the slot for a lone github custom, which leaves the
    `github and npm` collapse of the chain alive — this pins the middle axis alone."""
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        screen = await _open_prefs(app)
        await pilot.pause()
        assert isinstance(screen, PreferencesScreen)
        error_slot = screen.query_one("#pf-mirror-error", Static)
        assert error_slot.display is False  # nothing custom yet
        gh = list(screen.query_one("#pf-mirror-github", RadioSet).query(RadioButton))
        gh[len(config.GITHUB_RELEASE_PRESETS)].value = True  # github "custom", alone
        await pilot.pause()
        assert error_slot.display is True


# ---------------------------------------------------------------------------
# action_save: the default (else-branch) form / after-run literals
# ---------------------------------------------------------------------------


async def test_save_persists_default_tui_form_and_exit_after_run(tmp_path):
    """A fresh screen has the first radio of each pair pressed (index 0), so save takes
    the *else* branch of both ternaries: form -> "tui", after-run -> "exit". Asserted
    against the RAW config file, because load_form()/load_after_run() normalise unknown
    values back to those same defaults and would mask a corrupted literal."""
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        screen = await _open_prefs(app)
        await pilot.pause()
        assert isinstance(screen, PreferencesScreen)
        assert screen.query_one("#pf-form", RadioSet).pressed_index == 0
        assert screen.query_one("#pf-after", RadioSet).pressed_index == 0
        screen.action_save()
        await pilot.pause()
    raw = config.load_config()
    assert raw["form"] == "tui"
    assert raw["after_run"] == "exit"


async def test_save_reads_after_run_from_pf_after_radioset(tmp_path):
    """The after-run save must read `#pf-after`, not the first RadioSet on the screen
    (that is `#pf-form`). Set the two to different indices — after=stay(1), form=tui(0) —
    so a query that fell back to the first RadioSet would persist "exit" instead of the
    "stay" the user actually chose."""
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        screen = await _open_prefs(app)
        await pilot.pause()
        assert isinstance(screen, PreferencesScreen)
        list(screen.query_one("#pf-after", RadioSet).query(RadioButton))[1].value = True  # stay
        await pilot.pause()
        assert screen.query_one("#pf-form", RadioSet).pressed_index == 0  # differs from after
        assert screen.query_one("#pf-after", RadioSet).pressed_index == 1
        screen.action_save()
        await pilot.pause()
    assert config.load_after_run() == "stay"
    assert config.load_config()["form"] == "tui"  # form still read from its own radio


# ---------------------------------------------------------------------------
# footer chips / key bindings — every advertised path is operable
# ---------------------------------------------------------------------------


async def test_footer_save_chip_saves_and_dismisses(tmp_path):
    """The "Ctrl+A Save" footer chip is a button: clicking it fires screen.save, which
    persists and dismisses True (mouse-only operability of the advertised action)."""
    results: list[bool | None] = []
    app = tui.MenuApp()
    async with app.run_test(size=(130, 30)) as pilot:
        screen = await _open_prefs(app, results.append)
        await pilot.pause()
        screen.query_one("#pf-editor", Input).value = "micro"
        await pilot.pause()
        await click_label(pilot, "#pf-keys", "Save")
    assert config.load_editor() == "micro"
    assert results == [True]


async def test_footer_back_chip_dismisses_false(tmp_path):
    """The "Esc Back" footer chip fires screen.close, dismissing False without saving."""
    results: list[bool | None] = []
    app = tui.MenuApp()
    async with app.run_test(size=(130, 30)) as pilot:
        await _open_prefs(app, results.append)
        await pilot.pause()
        await click_label(pilot, "#pf-keys", "Back")
    assert results == [False]


async def test_ctrl_a_key_saves_and_escape_closes(tmp_path):
    """Keyboard twins of the two chips: Ctrl+A (priority binding) saves and dismisses
    True; on a fresh screen Esc closes and dismisses False."""
    saved: list[bool | None] = []
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        screen = await _open_prefs(app, saved.append)
        await pilot.pause()
        screen.query_one("#pf-editor", Input).value = "nvim"
        await pilot.pause()
        await pilot.press("ctrl+a")
        await pilot.pause()
    assert config.load_editor() == "nvim"
    assert saved == [True]

    closed: list[bool | None] = []
    app2 = tui.MenuApp()
    async with app2.run_test() as pilot:
        await _open_prefs(app2, closed.append)
        await pilot.pause()
        await pilot.press("escape")
        await pilot.pause()
    assert closed == [False]
