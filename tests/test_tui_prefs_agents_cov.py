"""Preferences' agent lanes: the JS-runtime radio, the Windows bash path, the agents
count line + Manage/Teach doors, and the SkillInstallModal.

Each test asserts what was persisted, which screen was pushed, or what the modal wrote —
never that a widget merely composed.
"""

from __future__ import annotations

import pytest
from textual.widgets import Input, OptionList, RadioButton, RadioSet, Select, Static

from skit import agentskill, config, tui
from skit.tui_prefs import PreferencesScreen, SkillInstallModal
from skit.tui_runner import RunnerActionModal, RunnerManageScreen, RunnerRemoveConfirm


def _as[S](obj: object, cls: type[S]) -> S:
    """Narrow app.screen to a concrete screen/modal type (ty runs in strict mode)."""
    assert isinstance(obj, cls)
    return obj


@pytest.fixture(autouse=True)
def tmp_store(tmp_path, monkeypatch):
    monkeypatch.setenv("SKIT_DATA_DIR", str(tmp_path / "data"))
    monkeypatch.setenv("SKIT_STATE_DIR", str(tmp_path / "state"))
    monkeypatch.setenv("SKIT_CONFIG_DIR", str(tmp_path / "config"))
    monkeypatch.setenv("SKIT_LANG", "en")


def _text(screen) -> str:
    return "\n".join(str(w.render()) for w in screen.query(Static))


# ---------------------------------------------------------------- JS runtime + bash path


async def test_prefs_js_runner_radio_round_trips(tmp_path):
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        app.push_screen(PreferencesScreen())
        await pilot.pause()
        screen = _as(app.screen, PreferencesScreen)
        buttons = list(screen.query_one("#pf-js", RadioSet).query(RadioButton))
        buttons[2].value = True  # index 0 = auto, 1 = deno, 2 = bun
        await pilot.pause()
        screen.action_save()
        await pilot.pause()
    assert config.load_js_runner() == "bun"


async def test_prefs_js_runner_auto_clears_to_empty(tmp_path):
    config.save_js_runner("bun")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        app.push_screen(PreferencesScreen())
        await pilot.pause()
        screen = _as(app.screen, PreferencesScreen)
        js = screen.query_one("#pf-js", RadioSet)
        assert js.pressed_index == 2  # the saved bun is preselected
        next(iter(js.query(RadioButton))).value = True  # Automatic
        await pilot.pause()
        screen.action_save()
        await pilot.pause()
    assert config.load_js_runner() == ""  # auto == empty


async def test_prefs_bash_section_is_windows_only(tmp_path, monkeypatch):
    """Off Windows the "Shell on Windows" section never composes — a section that can
    never apply is scroll noise. Saving still works (the bash box is simply absent)."""
    monkeypatch.setattr("skit.tui_prefs.sys.platform", "darwin")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        app.push_screen(PreferencesScreen())
        await pilot.pause()
        screen = _as(app.screen, PreferencesScreen)
        assert not screen.query("#pf-bash")  # the whole Windows section is gone
        screen.action_save()
        await pilot.pause()
        assert not isinstance(app.screen, PreferencesScreen)  # saved and dismissed


async def test_prefs_bash_path_persists_on_win32(tmp_path, monkeypatch):
    monkeypatch.setattr("skit.tui_prefs.sys.platform", "win32")
    bash = tmp_path / "bash.exe"
    bash.write_text("", encoding="utf-8")  # a REAL file — the save-side check requires it
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        app.push_screen(PreferencesScreen())
        await pilot.pause()
        screen = _as(app.screen, PreferencesScreen)
        screen.query_one("#pf-bash", Input).value = str(bash)
        await pilot.pause()
        screen.action_save()
        await pilot.pause()
        assert not isinstance(app.screen, PreferencesScreen)  # valid path → saved + dismissed
    assert config.load_bash_path() == str(bash)


async def test_prefs_bash_bad_path_shows_error_and_does_not_save(tmp_path, monkeypatch):
    """The same rule as `skit config shell.bash_path`: a typo'd path must not ride into
    config through the TUI door. The error shows in #pf-bash-error and the screen stays."""
    monkeypatch.setattr("skit.tui_prefs.sys.platform", "win32")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        app.push_screen(PreferencesScreen())
        await pilot.pause()
        screen = _as(app.screen, PreferencesScreen)
        screen.query_one("#pf-bash", Input).value = "/opt/git/bin/nope.exe"
        await pilot.pause()
        screen.action_save()
        await pilot.pause()
        assert isinstance(app.screen, PreferencesScreen)  # refused → stayed open
        assert "No such file" in str(screen.query_one("#pf-bash-error", Static).render())
    assert config.load_bash_path() == ""  # nothing rode into config


async def test_prefs_save_is_atomic_a_bad_bash_path_persists_nothing(tmp_path, monkeypatch):
    """action_save validates the bash path BEFORE persisting anything: a bad path refuses
    the whole save, so editor/form/after-run/js edited in the same pass stay unchanged on
    disk."""
    monkeypatch.setattr("skit.tui_prefs.sys.platform", "win32")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        app.push_screen(PreferencesScreen())
        await pilot.pause()
        screen = _as(app.screen, PreferencesScreen)
        screen.query_one("#pf-editor", Input).value = "micro"
        list(screen.query_one("#pf-form", RadioSet).query(RadioButton))[1].value = True  # plain
        list(screen.query_one("#pf-after", RadioSet).query(RadioButton))[1].value = True  # stay
        list(screen.query_one("#pf-js", RadioSet).query(RadioButton))[2].value = True  # bun
        screen.query_one("#pf-bash", Input).value = "/opt/git/bin/nope.exe"  # missing → refuse
        await pilot.pause()
        screen.action_save()
        await pilot.pause()
        assert isinstance(app.screen, PreferencesScreen)  # refused → stayed open
        assert "No such file" in str(screen.query_one("#pf-bash-error", Static).render())
    assert config.load_editor() == ""  # nothing rode into config
    assert config.load_form() == "tui"
    assert config.load_after_run() == "exit"
    assert config.load_js_runner() == ""
    assert config.load_bash_path() == ""


async def test_prefs_bash_empty_path_saves_on_win32(tmp_path, monkeypatch):
    """An empty bash path is a valid value (auto-detect) — it saves without the file check."""
    monkeypatch.setattr("skit.tui_prefs.sys.platform", "win32")
    config.save_bash_path("/old/bash.exe")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        app.push_screen(PreferencesScreen())
        await pilot.pause()
        screen = _as(app.screen, PreferencesScreen)
        screen.query_one("#pf-bash", Input).value = ""
        await pilot.pause()
        screen.action_save()
        await pilot.pause()
        assert not isinstance(app.screen, PreferencesScreen)  # saved + dismissed
    assert config.load_bash_path() == ""  # cleared


# ---------------------------------------------------------------- unsaved-changes guard


async def test_prefs_clean_esc_closes_without_asking(tmp_path):
    """A clean Esc (no edits since mount) closes straight away — the discard modal only
    guards actual unsaved work."""
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        app.push_screen(PreferencesScreen())
        await pilot.pause()
        screen = _as(app.screen, PreferencesScreen)
        # _dirt_armed is set via call_after_refresh (arms on the next repaint); a second
        # pause lets that callback run before we assert — the Windows pilot needs the tick.
        await pilot.pause()
        assert screen._dirt_armed is True  # armed after the mount settle
        screen.action_close()
        await pilot.pause()
        assert not isinstance(app.screen, PreferencesScreen)  # closed, no modal


async def test_prefs_dirty_esc_asks_and_keep_editing_stays(tmp_path):
    """Editing a field arms _dirty; Esc then routes through DiscardChangesModal. Keeping
    editing (discard=False) leaves Preferences open."""
    from skit.tui_settings import DiscardChangesModal

    app = tui.MenuApp()
    async with app.run_test() as pilot:
        app.push_screen(PreferencesScreen())
        await pilot.pause()
        screen = _as(app.screen, PreferencesScreen)
        list(screen.query_one("#pf-js", RadioSet).query(RadioButton))[2].value = True  # a real edit
        await pilot.pause()
        assert screen._dirty is True
        screen.action_close()  # dirty → ask
        await pilot.pause()
        confirm = app.screen
        assert isinstance(confirm, DiscardChangesModal)
        confirm.action_keep()  # keep editing → stays
        await pilot.pause()
        assert isinstance(app.screen, PreferencesScreen)


async def test_prefs_dirty_esc_discard_closes(tmp_path):
    """Discarding (y) from the modal closes Preferences without saving the edit."""
    from skit.tui_settings import DiscardChangesModal

    app = tui.MenuApp()
    async with app.run_test() as pilot:
        app.push_screen(PreferencesScreen())
        await pilot.pause()
        screen = _as(app.screen, PreferencesScreen)
        list(screen.query_one("#pf-js", RadioSet).query(RadioButton))[2].value = True
        await pilot.pause()
        assert screen._dirty is True
        screen.action_close()
        await pilot.pause()
        _as(app.screen, DiscardChangesModal).action_discard()  # y → close
        await pilot.pause()
        assert not isinstance(app.screen, PreferencesScreen)
    assert config.load_js_runner() == ""  # the unsaved edit never persisted


# ------------------------------------------------- chord grammar: Ctrl+O / Ctrl+K


async def test_prefs_ctrl_k_in_an_input_edits_the_field_not_a_modal(tmp_path):
    """Ctrl+O/Ctrl+K are NON-priority: with focus in a prefs Input, Ctrl+K
    is the Input's own delete-to-end-of-line — the screen must NOT answer a text-editing
    chord with a modal on a screen full of text fields."""
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        app.push_screen(PreferencesScreen())
        await pilot.pause()
        screen = _as(app.screen, PreferencesScreen)
        editor_box = screen.query_one("#pf-editor", Input)
        editor_box.value = "micro --wait"
        editor_box.focus()
        await pilot.pause()
        editor_box.cursor_position = 5  # after "micro"
        await pilot.press("ctrl+k")
        await pilot.pause()
        assert not isinstance(app.screen, SkillInstallModal)  # no modal hijacked the chord
        assert app.screen is screen  # still on Preferences
        assert editor_box.value == "micro"  # delete-to-end ran in the Input


async def test_prefs_ctrl_k_from_non_input_focus_opens_the_skill_modal(tmp_path, monkeypatch):
    """The twin: from a non-Input focus (the language dropdown boots there) Ctrl+K still
    opens the Teach-an-AI-agent modal — the chord fires, just never over an Input."""
    monkeypatch.setattr(agentskill, "detect_targets", lambda *, home, cwd: [])
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        app.push_screen(PreferencesScreen())
        await pilot.pause()
        screen = _as(app.screen, PreferencesScreen)
        screen.query_one("#pf-lang", Select).focus()  # a non-Input widget
        await pilot.pause()
        await pilot.press("ctrl+k")
        await pilot.pause()
        assert isinstance(app.screen, SkillInstallModal)


async def test_prefs_ctrl_o_from_non_input_focus_opens_manage_runners(tmp_path):
    """Ctrl+O from a non-Input focus opens the Manage agents screen."""
    from skit.tui_runner import RunnerManageScreen

    app = tui.MenuApp()
    async with app.run_test() as pilot:
        app.push_screen(PreferencesScreen())
        await pilot.pause()
        screen = _as(app.screen, PreferencesScreen)
        screen.query_one("#pf-lang", Select).focus()
        await pilot.pause()
        await pilot.press("ctrl+o")
        await pilot.pause()
        assert isinstance(app.screen, RunnerManageScreen)


# ---------------------------------------------------------------- agents section


async def test_prefs_agents_count_updates_after_managing(tmp_path):
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        app.push_screen(PreferencesScreen())
        await pilot.pause()
        screen = _as(app.screen, PreferencesScreen)
        assert "7 agents configured" in str(screen.query_one("#pf-runner-count", Static).render())
        # Ctrl+O opens the manage screen; remove one runner (confirm) and come back.
        await pilot.press("ctrl+o")
        await pilot.pause()
        manage = app.screen
        assert isinstance(manage, RunnerManageScreen)
        options = manage.query_one(OptionList)
        options.highlighted = 0
        options.action_select()
        await pilot.pause()
        _as(app.screen, RunnerActionModal).action_remove()  # → remove
        await pilot.pause()
        _as(app.screen, RunnerRemoveConfirm).action_confirm()  # the new destructive-op ask
        await pilot.pause()
        manage.action_close()
        await pilot.pause()
        # Back on Preferences: the count line refreshed via the return callback.
        assert isinstance(app.screen, PreferencesScreen)
        assert "6 agents configured" in str(
            app.screen.query_one("#pf-runner-count", Static).render()
        )


async def test_prefs_agents_count_empty_state(tmp_path, monkeypatch):
    monkeypatch.setattr(config, "ensure_prompt_runners_seeded", lambda: None)
    config.save_prompt_runners([])
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        app.push_screen(PreferencesScreen())
        await pilot.pause()
        assert "No agents configured." in str(
            app.screen.query_one("#pf-runner-count", Static).render()
        )


async def test_prefs_ctrl_t_opens_then_cancel_notifies_nothing(tmp_path, monkeypatch):
    monkeypatch.setattr(agentskill, "detect_targets", lambda *, home, cwd: [])
    notes: list[str] = []
    monkeypatch.setattr(
        PreferencesScreen, "notify", lambda self, message, **kw: notes.append(message)
    )
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        app.push_screen(PreferencesScreen())
        await pilot.pause()
        await pilot.press("ctrl+k")
        await pilot.pause()
        assert isinstance(app.screen, SkillInstallModal)
        _as(app.screen, SkillInstallModal).action_cancel()  # dismiss(None): no notify
        await pilot.pause()
        assert isinstance(app.screen, PreferencesScreen)
    assert notes == []


# ---------------------------------------------------------------- SkillInstallModal


def _fake_target(tmp_path):
    return agentskill.Target(name="claude", scope="user", base=tmp_path / ".claude")


async def test_skill_modal_installs_into_selected_target(tmp_path, monkeypatch):
    monkeypatch.setattr(agentskill, "detect_targets", lambda *, home, cwd: [_fake_target(tmp_path)])
    results: list[str | None] = []
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        app.push_screen(SkillInstallModal(), results.append)
        await pilot.pause()
        modal = app.screen
        assert isinstance(modal, SkillInstallModal)
        options = modal.query_one(OptionList)
        options.highlighted = 0
        options.action_select()
        await pilot.pause()
    written = tmp_path / ".claude" / "skills" / "skit" / "SKILL.md"
    assert written.is_file()  # consent → the file was actually written
    assert results == [str(written)]  # dismissed with the path


async def test_skill_modal_reports_install_error(tmp_path, monkeypatch):
    monkeypatch.setattr(agentskill, "detect_targets", lambda *, home, cwd: [_fake_target(tmp_path)])

    def boom(_dir, _text):
        raise OSError("permission denied")

    monkeypatch.setattr(agentskill, "install_into", boom)
    results: list[str | None] = []
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        app.push_screen(SkillInstallModal(), results.append)
        await pilot.pause()
        options = app.screen.query_one(OptionList)
        options.highlighted = 0
        options.action_select()
        await pilot.pause()
        assert isinstance(app.screen, SkillInstallModal)  # stayed open on error
    assert results == []


async def test_skill_modal_empty_targets_shows_hint(tmp_path, monkeypatch):
    monkeypatch.setattr(agentskill, "detect_targets", lambda *, home, cwd: [])
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        app.push_screen(SkillInstallModal())
        await pilot.pause()
        assert "No agent directories detected" in _text(app.screen)
        assert not app.screen.query(OptionList)


async def test_prefs_ctrl_t_install_notifies_with_the_written_path(tmp_path, monkeypatch):
    monkeypatch.setattr(agentskill, "detect_targets", lambda *, home, cwd: [_fake_target(tmp_path)])
    notes: list[str] = []
    monkeypatch.setattr(
        PreferencesScreen, "notify", lambda self, message, **kw: notes.append(message)
    )
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        app.push_screen(PreferencesScreen())
        await pilot.pause()
        await pilot.press("ctrl+k")
        await pilot.pause()
        options = app.screen.query_one(OptionList)
        options.highlighted = 0
        options.action_select()  # install → modal dismisses path → Preferences notifies
        await pilot.pause()
    written = tmp_path / ".claude" / "skills" / "skit" / "SKILL.md"
    assert written.is_file()
    assert any(str(written) in m for m in notes)


async def test_skill_modal_esc_cancels(tmp_path, monkeypatch):
    monkeypatch.setattr(agentskill, "detect_targets", lambda *, home, cwd: [])
    results: list[str | None] = []
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        app.push_screen(SkillInstallModal(), results.append)
        await pilot.pause()
        _as(app.screen, SkillInstallModal).action_cancel()
        await pilot.pause()
    assert results == [None]
