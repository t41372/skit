"""Preferences' agent lanes: the JS-runtime radio, the Windows bash path, the agents
count line + Manage/Teach doors, and the SkillInstallModal.

Each test asserts what was persisted, which screen was pushed, or what the modal wrote —
never that a widget merely composed.
"""

from __future__ import annotations

import pytest
from textual.widgets import Input, OptionList, RadioButton, RadioSet, Static

from skit import agentskill, config, tui
from skit.tui_prefs import PreferencesScreen, SkillInstallModal
from skit.tui_runner import RunnerActionModal, RunnerManageScreen


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


async def test_prefs_bash_path_persists(tmp_path):
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        app.push_screen(PreferencesScreen())
        await pilot.pause()
        screen = _as(app.screen, PreferencesScreen)
        screen.query_one("#pf-bash", Input).value = "/opt/git/bin/bash.exe"
        await pilot.pause()
        screen.action_save()
        await pilot.pause()
    assert config.load_bash_path() == "/opt/git/bin/bash.exe"


# ---------------------------------------------------------------- agents section


async def test_prefs_agents_count_updates_after_managing(tmp_path):
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        app.push_screen(PreferencesScreen())
        await pilot.pause()
        screen = _as(app.screen, PreferencesScreen)
        assert "5 agents configured" in str(screen.query_one("#pf-runner-count", Static).render())
        # Ctrl+N opens the manage screen; remove one runner and come back.
        await pilot.press("ctrl+n")
        await pilot.pause()
        manage = app.screen
        assert isinstance(manage, RunnerManageScreen)
        options = manage.query_one(OptionList)
        options.highlighted = 0
        options.action_select()
        await pilot.pause()
        _as(app.screen, RunnerActionModal).action_remove()  # → remove
        await pilot.pause()
        manage.action_close()
        await pilot.pause()
        # Back on Preferences: the count line refreshed via the return callback.
        assert isinstance(app.screen, PreferencesScreen)
        assert "4 agents configured" in str(
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
        await pilot.press("ctrl+t")
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
        await pilot.press("ctrl+t")
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
