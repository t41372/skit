"""The runner (agent) UI: the runner dropdown at scale, the management screen, the action
modal, and the add/edit modal's in-place replace.

Every test asserts an observable outcome — the config row that was written (and its
position), the modal that was pushed, the value a screen dismissed with — never that a
widget merely mounted.
"""

from __future__ import annotations

import contextlib
import shlex

import pytest
from textual.widgets import Input, OptionList, Select, Static

from skit import config, launcher, store, tui
from skit.tui_form import RunFormScreen
from skit.tui_runner import (
    RunnerActionModal,
    RunnerAddModal,
    RunnerManageScreen,
    RunnerRemoveConfirm,
)


def _as[S](obj: object, cls: type[S]) -> S:
    """Narrow app.screen to a concrete screen/modal type (ty runs in strict mode)."""
    assert isinstance(obj, cls)
    return obj


def _value(select: Select[str]) -> str:
    """A runner/preset Select's current value as a plain string. Every such picker is
    allow_blank=False with an explicit "" option for the blank ("ask"/"last values")
    state, so the value is always a real str, never the NULL sentinel — no index math."""
    value = select.value
    assert isinstance(value, str)
    return value


def _option_count(select: Select[str]) -> int:
    """How many options a Select carries. Select has no public option accessor, so we read
    the private _options; allow_blank=False means it holds no synthetic leading blank row."""
    return len(select._options)


def _find_runner(name: str) -> config.PromptRunner:
    r = config.find_prompt_runner(name)
    assert r is not None
    return r


@pytest.fixture(autouse=True)
def tmp_store(tmp_path, monkeypatch):
    monkeypatch.setenv("SKIT_DATA_DIR", str(tmp_path / "data"))
    monkeypatch.setenv("SKIT_STATE_DIR", str(tmp_path / "state"))
    monkeypatch.setenv("SKIT_CONFIG_DIR", str(tmp_path / "config"))
    monkeypatch.setenv("SKIT_LANG", "en")


@contextlib.contextmanager
def _noop_suspend():
    yield


@pytest.fixture
def quiet_run(monkeypatch):
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
        runner=None,
    ):
        calls["values"] = dict(values or {})
        calls["runner"] = runner
        return 0

    monkeypatch.setattr(launcher, "run_entry", fake_run)
    monkeypatch.setattr(tui.MenuApp, "suspend", lambda self: _noop_suspend())
    monkeypatch.setattr("builtins.input", lambda *a: "")
    return calls


def _prompt(tmp_path, text="Do {{a}}\n", name="p"):
    src = tmp_path / f"{name}.prompt.md"
    src.write_text(text, encoding="utf-8")
    return store.add_prompt(src, name=name)


# ------------------------------------------------------------- runner dropdown at scale


async def test_run_form_runner_select_scales_to_many_runners(tmp_path, quiet_run):
    # Eight runners: a dropdown collapses to one row and its overlay scales to any count
    # (the old horizontal picker CLIPPED past the terminal edge at exactly this number).
    # Value-keyed selection reaches the eighth with no index math and no scroll gymnastics.
    eight = [config.PromptRunner(f"r{i}", (f"r{i}", "{{prompt}}")) for i in range(1, 9)]
    config.save_prompt_runners(eight)
    _prompt(tmp_path)
    app = tui.MenuApp()
    async with app.run_test(size=(100, 32)) as pilot:
        await pilot.pause()
        app.action_run()
        await pilot.pause()
        select = app.screen.query_one("#runner-select", Select)
        assert _option_count(select) == 8  # every runner is an option — none clipped
        select.value = "r8"  # the eighth, exactly where a 5-row fold used to bury it
        await pilot.pause()
        form = _as(app.screen, RunFormScreen)
        form.query_one(Input).value = "hi"
        form.action_submit()
        await pilot.pause()
    assert quiet_run["runner"] == config.find_prompt_runner("r8")


async def test_run_form_enter_shim_is_a_full_keyboard_journey(tmp_path, quiet_run):
    # Policy #2: every advertised key needs a positive pilot test, and Enter is advertised
    # as Run. The shim lets that muscle memory coexist with a focused dropdown — with the
    # Select focused Enter OPERATES it (open, then choose in the overlay); only from a plain
    # field does Enter submit.
    eight = [config.PromptRunner(f"r{i}", (f"r{i}", "{{prompt}}")) for i in range(1, 9)]
    config.save_prompt_runners(eight)
    _prompt(tmp_path)
    app = tui.MenuApp()
    async with app.run_test(size=(100, 32)) as pilot:
        await pilot.pause()
        app.action_run()
        await pilot.pause()
        screen = _as(app.screen, RunFormScreen)
        select = screen.query_one("#runner-select", Select)
        select.focus()
        await pilot.pause()
        start = _value(select)
        # 1) Enter on the focused Select opens its overlay — it must NOT submit.
        await pilot.press("enter")
        await pilot.pause()
        assert select.expanded  # the overlay is open
        assert "values" not in quiet_run  # nothing ran
        # 2) Arrow within the overlay, then Enter picks the highlighted option (still no run).
        await pilot.press("down")
        await pilot.press("enter")
        await pilot.pause()
        assert not select.expanded  # choosing folded the overlay
        assert _value(select) != start  # the highlighted option really landed
        assert "values" not in quiet_run  # Enter CHOSE, it did not submit
        picked = _value(select)
        # 3) Enter from a plain field DOES submit (the muscle-memory path is intact).
        field = screen.query_one(Input)
        field.value = "hi"
        field.focus()
        await pilot.pause()
        await pilot.press("enter")
        await pilot.pause()
    assert quiet_run["values"] == {"a": "hi"}
    assert quiet_run["runner"] == config.find_prompt_runner(picked)


async def test_run_form_enter_on_overlay_without_a_highlight_is_a_safe_noop(tmp_path, quiet_run):
    # The shim's inner guard: Enter routed into an open overlay that has nothing highlighted
    # must neither pick nor submit — fold nothing, stay put, never crash.
    _prompt(tmp_path)
    app = tui.MenuApp()
    async with app.run_test(size=(100, 32)) as pilot:
        await pilot.pause()
        app.action_run()
        await pilot.pause()
        screen = _as(app.screen, RunFormScreen)
        select = screen.query_one("#runner-select", Select)
        select.expanded = True
        await pilot.pause()
        overlay = select.query_one(OptionList)
        overlay.highlighted = None  # the no-highlight state the guard defends against
        overlay.focus()
        await pilot.pause()
        screen.action_submit()
        await pilot.pause()
        assert "values" not in quiet_run  # guard was False → returned without submitting


# ---------------------------------------------------------------- RunnerActionModal


async def test_action_modal_shows_command_and_dismisses_by_verb(tmp_path):
    config.ensure_prompt_runners_seeded()
    app = tui.MenuApp()
    async with app.run_test(size=(100, 32)) as pilot:
        for verb, expected in (
            ("action_edit", "edit"),
            ("action_remove", "remove"),
            ("action_cancel", None),
        ):
            results: list[str | None] = []
            app.push_screen(RunnerActionModal("opencode"), results.append)
            await pilot.pause()
            modal = app.screen
            assert isinstance(modal, RunnerActionModal)
            text = "\n".join(str(s.render()) for s in modal.query(Static))
            assert "opencode" in text
            assert "--prompt" in text  # the row shows its shlex-joined command
            getattr(modal, verb)()
            await pilot.pause()
            assert results == [expected]


# ---------------------------------------------------------------- RunnerAddModal edit mode


async def test_add_modal_edit_prefills_and_replaces_in_place(tmp_path):
    config.ensure_prompt_runners_seeded()
    before = [r.name for r in config.load_prompt_runners()]
    app = tui.MenuApp()
    async with app.run_test(size=(100, 32)) as pilot:
        app.push_screen(RunnerAddModal(editing="codex"))
        await pilot.pause()
        modal = app.screen
        assert isinstance(modal, RunnerAddModal)
        assert modal.query_one("#runner-add-name", Input).value == "codex"
        # The command is prefilled shlex-joined (so it round-trips through the same split).
        assert modal.query_one("#runner-add-command", Input).value == shlex.join(
            ["codex", "{{prompt}}"]
        )
        # Change only the command, save under the same name.
        modal.query_one("#runner-add-command", Input).value = "codex --model o1 {{prompt}}"
        modal.action_save_runner()
        await pilot.pause()
    runners = config.load_prompt_runners()
    assert [r.name for r in runners] == before  # order and membership unchanged
    assert _find_runner("codex").argv == ("codex", "--model", "o1", "{{prompt}}")  # in place


async def test_add_modal_edit_rename_onto_another_name_is_refused(tmp_path):
    config.ensure_prompt_runners_seeded()
    app = tui.MenuApp()
    async with app.run_test(size=(100, 32)) as pilot:
        results: list[str | None] = []
        app.push_screen(RunnerAddModal(editing="codex"), results.append)
        await pilot.pause()
        modal = _as(app.screen, RunnerAddModal)
        modal.query_one("#runner-add-name", Input).value = "claude"  # already taken
        modal.action_save_runner()
        await pilot.pause()
        assert isinstance(app.screen, RunnerAddModal)  # not dismissed — error shown
        assert results == []
        err = "\n".join(str(s.render()) for s in modal.query(Static))
        assert "already exists" in err
    # claude was not overwritten by codex's argv.
    assert _find_runner("claude").argv == ("claude", "{{prompt}}")


async def test_add_modal_edit_save_under_same_name_is_allowed(tmp_path):
    config.ensure_prompt_runners_seeded()
    app = tui.MenuApp()
    async with app.run_test(size=(100, 32)) as pilot:
        results: list[str | None] = []
        app.push_screen(RunnerAddModal(editing="amp"), results.append)
        await pilot.pause()
        modal = _as(app.screen, RunnerAddModal)
        modal.query_one("#runner-add-command", Input).value = "amp -x {{prompt}}"
        modal.action_save_runner()
        await pilot.pause()
    assert results == ["amp"]  # dismissed with the saved name


# ---------------------------------------------------------------- RunnerManageScreen


async def _open_manage(app, pilot):
    app.push_screen(RunnerManageScreen())
    await pilot.pause()
    assert isinstance(app.screen, RunnerManageScreen)
    return app.screen


async def test_manage_screen_lists_rows_with_name_and_command(tmp_path):
    config.ensure_prompt_runners_seeded()
    app = tui.MenuApp()
    async with app.run_test(size=(100, 32)) as pilot:
        screen = await _open_manage(app, pilot)
        options = screen.query_one(OptionList)
        prompts = [str(options.get_option_at_index(i).prompt) for i in range(options.option_count)]
    assert any("claude" in p for p in prompts)
    assert any("--prompt" in p for p in prompts)  # opencode's command is shown


async def test_manage_screen_pick_then_edit_replaces_in_place(tmp_path):
    config.ensure_prompt_runners_seeded()
    before = [r.name for r in config.load_prompt_runners()]
    app = tui.MenuApp()
    async with app.run_test(size=(100, 32)) as pilot:
        screen = await _open_manage(app, pilot)
        options = screen.query_one(OptionList)
        options.highlighted = 1  # codex
        options.action_select()
        await pilot.pause()
        action_modal = app.screen
        assert isinstance(action_modal, RunnerActionModal)
        action_modal.action_edit()  # dismiss "edit" → the manage screen pushes the add modal
        await pilot.pause()
        add_modal = app.screen
        assert isinstance(add_modal, RunnerAddModal)
        add_modal.query_one("#runner-add-command", Input).value = "codex --new {{prompt}}"
        add_modal.action_save_runner()
        await pilot.pause()
        assert isinstance(app.screen, RunnerManageScreen)  # back on the list
    assert [r.name for r in config.load_prompt_runners()] == before  # position held
    assert _find_runner("codex").argv == ("codex", "--new", "{{prompt}}")


async def test_manage_screen_pick_then_remove_confirms_then_deletes(tmp_path):
    """Removing an agent is destructive config surgery, so it now ASKS first — the
    RunnerRemoveConfirm. Pressing y confirms and deletes the row."""
    config.ensure_prompt_runners_seeded()
    app = tui.MenuApp()
    async with app.run_test(size=(100, 32)) as pilot:
        screen = await _open_manage(app, pilot)
        options = screen.query_one(OptionList)
        options.highlighted = 0  # claude
        options.action_select()
        await pilot.pause()
        _as(app.screen, RunnerActionModal).action_remove()  # RunnerActionModal → "remove"
        await pilot.pause()
        confirm = app.screen
        assert isinstance(confirm, RunnerRemoveConfirm)  # asked before deleting
        assert config.find_prompt_runner("claude") is not None  # not yet gone
        await pilot.press("y")  # confirm
        await pilot.pause()
        assert isinstance(app.screen, RunnerManageScreen)
        # the list reloaded without claude
        remaining = screen.query_one(OptionList)
        ids = [remaining.get_option_at_index(i).id for i in range(remaining.option_count)]
    assert config.find_prompt_runner("claude") is None
    assert "claude" not in ids


async def test_manage_screen_remove_confirm_kept_deletes_nothing(tmp_path):
    """Esc on the confirm keeps the agent — the "really is False" branch: nothing is
    removed and the list is unchanged."""
    config.ensure_prompt_runners_seeded()
    before = config.load_prompt_runners()
    app = tui.MenuApp()
    async with app.run_test(size=(100, 32)) as pilot:
        screen = await _open_manage(app, pilot)
        options = screen.query_one(OptionList)
        options.highlighted = 0  # claude
        options.action_select()
        await pilot.pause()
        _as(app.screen, RunnerActionModal).action_remove()
        await pilot.pause()
        assert isinstance(app.screen, RunnerRemoveConfirm)
        await pilot.press("escape")  # Esc → keep
        await pilot.pause()
        assert isinstance(app.screen, RunnerManageScreen)
    assert config.load_prompt_runners() == before  # nothing removed


async def test_manage_screen_pick_then_cancel_changes_nothing(tmp_path):
    config.ensure_prompt_runners_seeded()
    before = config.load_prompt_runners()
    app = tui.MenuApp()
    async with app.run_test(size=(100, 32)) as pilot:
        screen = await _open_manage(app, pilot)
        options = screen.query_one(OptionList)
        options.highlighted = 0
        options.action_select()
        await pilot.pause()
        _as(app.screen, RunnerActionModal).action_cancel()  # None: neither edit nor remove
        await pilot.pause()
        assert isinstance(app.screen, RunnerManageScreen)
    assert config.load_prompt_runners() == before  # nothing added, edited, or removed


async def test_manage_screen_ctrl_n_opens_the_add_modal(tmp_path):
    config.ensure_prompt_runners_seeded()
    app = tui.MenuApp()
    async with app.run_test(size=(100, 32)) as pilot:
        await _open_manage(app, pilot)
        await pilot.press("ctrl+n")
        await pilot.pause()
        assert isinstance(app.screen, RunnerAddModal)


async def test_manage_screen_esc_dismisses(tmp_path):
    config.ensure_prompt_runners_seeded()
    app = tui.MenuApp()
    async with app.run_test(size=(100, 32)) as pilot:
        results: list[None] = []
        app.push_screen(RunnerManageScreen(), results.append)
        await pilot.pause()
        _as(app.screen, RunnerManageScreen).action_close()
        await pilot.pause()
    assert results == [None]


async def test_manage_screen_empty_state_shows_when_no_runners(tmp_path, monkeypatch):
    # A deliberately-emptied list: on_mount seeds, so force the emptied marker afterwards
    # by patching the seed to a no-op and saving [].
    monkeypatch.setattr(config, "ensure_prompt_runners_seeded", lambda: None)
    config.save_prompt_runners([])
    app = tui.MenuApp()
    async with app.run_test(size=(100, 32)) as pilot:
        screen = await _open_manage(app, pilot)
        empty = screen.query_one("#rm-empty", Static)
        assert empty.display is True
        assert screen.query_one(OptionList).option_count == 0
