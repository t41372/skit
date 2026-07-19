"""The runner (agent) UI: the runner dropdown at scale, the management screen, the action
modal, and the add/edit modal's in-place replace.

Every test asserts an observable outcome — the config row that was written (and its
position), the modal that was pushed, the value a screen dismissed with — never that a
widget merely mounted.
"""

from __future__ import annotations

import contextlib
from types import SimpleNamespace

import pytest
from textual.widgets import Input, OptionList, Select, Static

from skit import argv_text, config, launcher, store, tui, tui_footer
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
        prepared=None,
    ):
        calls["values"] = dict(values or {})
        calls["runner"] = runner
        calls["prepared"] = prepared
        return 0

    monkeypatch.setattr(launcher, "run_entry", fake_run)
    monkeypatch.setattr("skit.langs.launch._which", lambda name: f"/bin/{name}")
    monkeypatch.setattr(tui.MenuApp, "suspend", lambda self: _noop_suspend())
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


async def test_action_modal_ignores_edit_when_row_is_not_repairable(tmp_path):
    app = tui.MenuApp()
    async with app.run_test(size=(100, 32)) as pilot:
        results: list[str | None] = []
        app.push_screen(RunnerActionModal("raw scalar", editable=False), results.append)
        await pilot.pause()
        modal = _as(app.screen, RunnerActionModal)
        modal.action_edit()
        await pilot.pause()
        assert app.screen is modal
        assert results == []
        modal.action_cancel()
        await pilot.pause()
    assert results == [None]


# ---------------------------------------------------------------- RunnerAddModal edit mode


async def test_add_modal_field_navigation_is_visible_keyboard_and_mouse_operable(tmp_path):
    app = tui.MenuApp()
    async with app.run_test(size=(100, 32)) as pilot:
        app.push_screen(RunnerAddModal())
        await pilot.pause()
        modal = _as(app.screen, RunnerAddModal)
        name = modal.query_one("#runner-add-name", Input)
        command = modal.query_one("#runner-add-command", Input)
        footer = modal.query_one("#runner-add-footer", Static)
        plain = str(footer.render()).replace(tui_footer.GLUE, " ")
        assert "Tab/↓" in plain
        assert "Shift+Tab/↑" in plain
        assert name.has_focus

        await pilot.press("down")
        assert command.has_focus
        await pilot.press("up")
        assert name.has_focus

        forward = plain.find("Tab/↓")
        await pilot.click(footer, offset=(forward + 1, 0))
        await pilot.pause()
        assert command.has_focus
        back = plain.find("Shift+Tab/↑")
        await pilot.click(footer, offset=(back + 1, 0))
        await pilot.pause()
        assert name.has_focus


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
        # The command is prefilled via argv_text.join (the split's inverse) — list2cmdline
        # on Windows, shlex.join on POSIX — so it round-trips through the same split.
        assert modal.query_one("#runner-add-command", Input).value == argv_text.join(
            ["codex", "--", "{{prompt}}"]
        )
        # Change only the command, save under the same name.
        modal.query_one("#runner-add-command", Input).value = "codex --model o1 {{prompt}}"
        modal.action_save_runner()
        await pilot.pause()
    runners = config.load_prompt_runners()
    assert [r.name for r in runners] == before  # order and membership unchanged
    assert _find_runner("codex").argv == ("codex", "--model", "o1", "{{prompt}}")  # in place


async def test_add_modal_edit_keeps_the_pin_key_name_immutable(tmp_path):
    config.ensure_prompt_runners_seeded()
    claude_before = _find_runner("claude")
    app = tui.MenuApp()
    async with app.run_test(size=(100, 32)) as pilot:
        results: list[str | None] = []
        app.push_screen(RunnerAddModal(editing="codex"), results.append)
        await pilot.pause()
        modal = _as(app.screen, RunnerAddModal)
        name_input = modal.query_one("#runner-add-name", Input)
        assert name_input.disabled  # the visible contract: pins key off this stable name
        # Defend the persistence path too: a synthetic value change must not rename the key.
        name_input.value = "claude"
        modal.query_one("#runner-add-command", Input).value = "codex --new {{prompt}}"
        modal.action_save_runner()
        await pilot.pause()
    assert results == ["codex"]
    assert _find_runner("codex").argv == ("codex", "--new", "{{prompt}}")
    assert _find_runner("claude") == claude_before


def test_runner_command_windows_paths_preserve_backslashes_and_roundtrip(monkeypatch):
    monkeypatch.setattr(argv_text, "sys", SimpleNamespace(platform="win32"))
    cases = [
        [],
        [""],
        [r"C:\Program Files\Agent\agent.exe", "--message", "{{prompt}}"],
        ['say "hello"', "space and trailing slash\\"],
        ['one\\"quote', 'two\\\\"quote'],
        ["", "plain", "\t", "{{prompt}}"],
    ]
    for argv in cases:
        assert argv_text.split(argv_text.join(argv)) == argv
    assert argv_text.split(r"C:\tools\agent.exe {{prompt}}") == [
        r"C:\tools\agent.exe",
        "{{prompt}}",
    ]
    assert argv_text.split('"C:\\Program Files\\Agent\\agent.exe" --message="{{prompt}}"') == [
        r"C:\Program Files\Agent\agent.exe",
        "--message={{prompt}}",
    ]
    with pytest.raises(ValueError, match="closing quotation"):
        argv_text.split('"C:\\Program Files\\Agent\\agent.exe')


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


async def test_add_modal_reports_malformed_runner_container_without_dismissing(tmp_path):
    config.save_config({"prompt": "broken"})
    app = tui.MenuApp()
    async with app.run_test(size=(100, 32)) as pilot:
        app.push_screen(RunnerAddModal())
        await pilot.pause()
        modal = _as(app.screen, RunnerAddModal)
        modal.query_one("#runner-add-name", Input).value = "new"
        modal.query_one("#runner-add-command", Input).value = "new {{prompt}}"
        modal.action_save_runner()
        await pilot.pause()
        assert app.screen is modal
        assert "isn't a table" in str(modal.query_one("#runner-add-error", Static).render())
    assert config.load_config()["prompt"] == "broken"


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
    assert any("--prompt=" in p for p in prompts)  # opencode's safe bound value is shown


async def test_manage_screen_shows_invalid_rows_with_reason_and_removes_only_selected(tmp_path):
    untouched = {"name": "anonymous", "argv": "not-a-list"}
    config.save_config(
        {
            "prompt": {
                "runners_seeded": True,
                "runners": [
                    {"name": "good", "argv": ["good", "{{prompt}}"]},
                    {"name": "broken", "argv": ["broken"]},
                    untouched,
                ],
            }
        }
    )
    app = tui.MenuApp()
    async with app.run_test(size=(100, 32)) as pilot:
        screen = await _open_manage(app, pilot)
        options = screen.query_one(OptionList)
        prompts = [str(options.get_option_at_index(i).prompt) for i in range(options.option_count)]
        assert "broken" in prompts[1]
        assert "exactly once" in prompts[1]
        assert "list of text arguments" in prompts[2]

        options.highlighted = 1
        options.action_select()
        await pilot.pause()
        _as(app.screen, RunnerActionModal).action_remove()
        await pilot.pause()
        confirm_text = "\n".join(str(widget.render()) for widget in app.screen.query(Static))
        assert "Remove malformed runner row" in confirm_text
        await pilot.press("y")
        await pilot.pause()

    assert config.load_config()["prompt"]["runners"] == [
        {"name": "good", "argv": ["good", "{{prompt}}"]},
        untouched,
    ]


async def test_manage_screen_can_remove_malformed_container_without_overwriting_other_config(
    tmp_path,
):
    config.save_config({"language": "zh-TW", "prompt": "garbage"})
    app = tui.MenuApp()
    async with app.run_test(size=(100, 32)) as pilot:
        screen = await _open_manage(app, pilot)
        options = screen.query_one(OptionList)
        assert options.option_count == 1
        assert "isn't a table" in str(options.get_option_at_index(0).prompt)
        options.highlighted = 0
        options.action_select()
        await pilot.pause()
        _as(app.screen, RunnerActionModal).action_remove()
        await pilot.pause()
        confirm_text = "\n".join(str(widget.render()) for widget in app.screen.query(Static))
        assert "Remove the malformed prompt runner container" in confirm_text
        await pilot.press("y")
        await pilot.pause()

    assert config.load_config() == {
        "language": "zh-TW",
        "prompt": {"runners_seeded": True, "runners": []},
    }


async def test_manage_screen_repairs_a_recognizable_malformed_row_in_place(tmp_path):
    untouched = "not-a-table"
    config.save_config(
        {
            "prompt": {
                "runners_seeded": True,
                "runners": [
                    {"name": "broken", "argv": ["old"]},
                    untouched,
                ],
            }
        }
    )
    app = tui.MenuApp()
    async with app.run_test(size=(100, 32)) as pilot:
        screen = await _open_manage(app, pilot)
        options = screen.query_one(OptionList)
        options.highlighted = 0
        options.action_select()
        await pilot.pause()
        action = _as(app.screen, RunnerActionModal)
        action.action_edit()
        await pilot.pause()
        modal = _as(app.screen, RunnerAddModal)
        assert modal.query_one("#runner-add-name", Input).value == "broken"
        assert modal.query_one("#runner-add-command", Input).value == "old"
        modal.query_one("#runner-add-command", Input).value = "fixed {{prompt}}"
        modal.action_save_runner()
        await pilot.pause()

    assert config.load_config()["prompt"]["runners"] == [
        {"name": "broken", "argv": ["fixed", "{{prompt}}"]},
        untouched,
    ]
    assert config.find_prompt_runner("broken") == config.PromptRunner(
        "broken", ("fixed", "{{prompt}}")
    )


async def test_manage_screen_preserves_and_repairs_anonymous_row_command_by_index(tmp_path):
    command = ["valuable-agent", "--model", "x", "{{prompt}}"]
    untouched = {"name": "other", "argv": ["other", "{{prompt}}"]}
    config.save_config(
        {
            "prompt": {
                "runners_seeded": True,
                "runners": [{"name": "   ", "argv": command}, untouched],
            }
        }
    )
    app = tui.MenuApp()
    async with app.run_test(size=(100, 32)) as pilot:
        screen = await _open_manage(app, pilot)
        options = screen.query_one(OptionList)
        assert "valuable-agent --model x" in str(options.get_option_at_index(0).prompt)
        options.highlighted = 0
        options.action_select()
        await pilot.pause()
        action = _as(app.screen, RunnerActionModal)
        assert "valuable-agent --model x" in "\n".join(
            str(widget.render()) for widget in action.query(Static)
        )
        action.action_edit()
        await pilot.pause()
        modal = _as(app.screen, RunnerAddModal)
        name = modal.query_one("#runner-add-name", Input)
        assert not name.disabled  # no stable key exists yet; repair must let the user create one
        assert modal.query_one("#runner-add-command", Input).value == argv_text.join(command)
        name.value = "valuable"
        modal.action_save_runner()
        await pilot.pause()

    assert config.load_config()["prompt"]["runners"] == [
        {"name": "valuable", "argv": command},
        untouched,
    ]
    assert config.find_prompt_runner("valuable") == config.PromptRunner("valuable", tuple(command))


async def test_manage_screen_user_name_cannot_collide_with_invalid_row_ui_ids(tmp_path):
    malformed = {"name": "broken", "argv": ["broken"]}
    config.save_config(
        {
            "prompt": {
                "runners_seeded": True,
                "runners": [
                    {"name": "__invalid_runner_1", "argv": ["agent", "{{prompt}}"]},
                    malformed,
                ],
            }
        }
    )
    app = tui.MenuApp()
    async with app.run_test(size=(100, 32)) as pilot:
        screen = await _open_manage(app, pilot)  # mounting used to raise DuplicateID
        options = screen.query_one(OptionList)
        assert options.option_count == 2
        assert [options.get_option_at_index(i).id for i in range(2)] == [
            "runner-row-0",
            "runner-row-1",
        ]
        options.highlighted = 0
        options.action_select()
        await pilot.pause()
        _as(app.screen, RunnerActionModal).action_edit()
        await pilot.pause()
        modal = _as(app.screen, RunnerAddModal)
        modal.query_one("#runner-add-command", Input).value = "agent --fixed {{prompt}}"
        modal.action_save_runner()
        await pilot.pause()

    assert config.find_prompt_runner("__invalid_runner_1") == config.PromptRunner(
        "__invalid_runner_1", ("agent", "--fixed", "{{prompt}}")
    )
    assert config.load_config()["prompt"]["runners"][1] == malformed


async def test_editing_invalid_duplicate_uses_selected_raw_command_then_coalesces_key(tmp_path):
    config.save_config(
        {
            "prompt": {
                "runners_seeded": True,
                "runners": [
                    {"name": "same", "argv": ["good", "{{prompt}}"]},
                    {"name": "same", "argv": ["broken"]},
                    {"name": "other", "argv": ["other", "{{prompt}}"]},
                ],
            }
        }
    )
    app = tui.MenuApp()
    async with app.run_test(size=(100, 32)) as pilot:
        screen = await _open_manage(app, pilot)
        options = screen.query_one(OptionList)
        options.highlighted = 1  # the malformed duplicate, not the valid first row
        options.action_select()
        await pilot.pause()
        action = _as(app.screen, RunnerActionModal)
        action_text = "\n".join(str(widget.render()) for widget in action.query(Static))
        assert "broken" in action_text
        assert "good {{prompt}}" not in action_text
        action.action_edit()
        await pilot.pause()
        modal = _as(app.screen, RunnerAddModal)
        assert modal.query_one("#runner-add-command", Input).value == "broken"
        modal.query_one("#runner-add-command", Input).value = "fixed {{prompt}}"
        modal.action_save_runner()
        await pilot.pause()

    assert config.load_config()["prompt"]["runners"] == [
        {"name": "same", "argv": ["fixed", "{{prompt}}"]},
        {"name": "other", "argv": ["other", "{{prompt}}"]},
    ]


async def test_removing_active_runner_key_cannot_promote_a_duplicate_row(tmp_path):
    config.save_config(
        {
            "prompt": {
                "runners_seeded": True,
                "runners": [
                    {"name": "same", "argv": ["first", "{{prompt}}"]},
                    {"name": "same", "argv": ["second", "{{prompt}}"]},
                    {"name": "other", "argv": ["other", "{{prompt}}"]},
                ],
            }
        }
    )
    app = tui.MenuApp()
    async with app.run_test(size=(100, 32)) as pilot:
        screen = await _open_manage(app, pilot)
        options = screen.query_one(OptionList)
        options.highlighted = 0  # the active `same` key
        options.action_select()
        await pilot.pause()
        _as(app.screen, RunnerActionModal).action_remove()
        await pilot.pause()
        await pilot.press("y")
        await pilot.pause()

    assert config.find_prompt_runner("same") is None
    assert config.load_config()["prompt"]["runners"] == [
        {"name": "other", "argv": ["other", "{{prompt}}"]}
    ]


async def test_invalid_row_remove_refuses_if_index_shifts_while_confirm_is_open(tmp_path):
    original = [
        {"name": "good", "argv": ["good", "{{prompt}}"]},
        {"name": "target", "argv": ["target"]},
        {"name": "other", "argv": ["other", "{{prompt}}"]},
    ]
    config.save_config({"prompt": {"runners_seeded": True, "runners": original}})
    inserted = {"name": "inserted", "argv": ["inserted", "{{prompt}}"]}
    app = tui.MenuApp()
    async with app.run_test(size=(100, 32)) as pilot:
        screen = await _open_manage(app, pilot)
        options = screen.query_one(OptionList)
        options.highlighted = 1
        options.action_select()
        await pilot.pause()
        _as(app.screen, RunnerActionModal).action_remove()
        await pilot.pause()
        assert isinstance(app.screen, RunnerRemoveConfirm)

        # Simulate another process editing config during the destructive-confirmation
        # window. The old index now points at `good`, never the selected `target`.
        doc = config.load_config()
        doc["prompt"]["runners"].insert(0, inserted)
        config.save_config(doc)
        await pilot.press("y")
        await pilot.pause()

        assert isinstance(app.screen, RunnerManageScreen)
        assert "nothing was removed" in str(screen.query_one("#rm-error", Static).render())

    assert config.load_config()["prompt"]["runners"] == [inserted, *original]


async def test_active_name_remove_refuses_if_key_is_replaced_while_confirm_is_open(tmp_path):
    original = config.PromptRunner("victim", ("old", "{{prompt}}"))
    replacement = config.PromptRunner("victim", ("new", "--important", "{{prompt}}"))
    config.save_prompt_runners([original])
    app = tui.MenuApp()
    async with app.run_test(size=(100, 32)) as pilot:
        screen = await _open_manage(app, pilot)
        options = screen.query_one(OptionList)
        options.highlighted = 0
        options.action_select()
        await pilot.pause()
        _as(app.screen, RunnerActionModal).action_remove()
        await pilot.pause()
        assert isinstance(app.screen, RunnerRemoveConfirm)

        config.set_prompt_runner(replacement, replace_existing=True)
        await pilot.press("y")
        await pilot.pause()

        assert isinstance(app.screen, RunnerManageScreen)
        assert "nothing was removed" in str(screen.query_one("#rm-error", Static).render())

    assert config.find_prompt_runner("victim") == replacement


async def test_manage_edit_refuses_stale_target_and_keeps_the_users_modal_open(tmp_path):
    original = config.PromptRunner("victim", ("old", "{{prompt}}"))
    external = config.PromptRunner("victim", ("external", "--important", "{{prompt}}"))
    config.save_prompt_runners([original])
    app = tui.MenuApp()
    async with app.run_test(size=(100, 32)) as pilot:
        screen = await _open_manage(app, pilot)
        options = screen.query_one(OptionList)
        options.highlighted = 0
        options.action_select()
        await pilot.pause()
        _as(app.screen, RunnerActionModal).action_edit()
        await pilot.pause()
        modal = _as(app.screen, RunnerAddModal)
        modal.query_one("#runner-add-command", Input).value = "my-edit {{prompt}}"

        config.set_prompt_runner(external, replace_existing=True)
        modal.action_save_runner()
        await pilot.pause()

        assert app.screen is modal  # typed work remains visible; no silent dismiss/loss
        assert "config changed" in str(modal.query_one("#runner-add-error", Static).render())
    assert config.find_prompt_runner("victim") == external


async def test_manage_edit_allows_an_unrelated_concurrent_runner_change(tmp_path):
    victim = config.PromptRunner("victim", ("old", "{{prompt}}"))
    other = config.PromptRunner("other", ("other", "{{prompt}}"))
    external_other = config.PromptRunner("other", ("external", "{{prompt}}"))
    config.save_prompt_runners([victim, other])
    app = tui.MenuApp()
    async with app.run_test(size=(100, 32)) as pilot:
        screen = await _open_manage(app, pilot)
        options = screen.query_one(OptionList)
        options.highlighted = 0
        options.action_select()
        await pilot.pause()
        _as(app.screen, RunnerActionModal).action_edit()
        await pilot.pause()
        modal = _as(app.screen, RunnerAddModal)
        modal.query_one("#runner-add-command", Input).value = "mine {{prompt}}"

        config.set_prompt_runner(external_other, replace_existing=True)
        modal.action_save_runner()
        await pilot.pause()

        assert isinstance(app.screen, RunnerManageScreen)
    assert config.find_prompt_runner("victim") == config.PromptRunner(
        "victim", ("mine", "{{prompt}}")
    )
    assert config.find_prompt_runner("other") == external_other


async def test_duplicate_row_removal_does_not_warn_that_active_pins_will_break(tmp_path):
    config.save_config(
        {
            "prompt": {
                "runners_seeded": True,
                "runners": [
                    {"name": "same", "argv": ["first", "{{prompt}}"]},
                    {"name": "same", "argv": ["second", "{{prompt}}"]},
                ],
            }
        }
    )
    entry = _prompt(tmp_path)
    store.write_prompt_runner(entry.slug, "same")
    app = tui.MenuApp()
    async with app.run_test(size=(100, 32)) as pilot:
        screen = await _open_manage(app, pilot)
        options = screen.query_one(OptionList)
        options.highlighted = 1
        options.action_select()
        await pilot.pause()
        _as(app.screen, RunnerActionModal).action_remove()
        await pilot.pause()
        confirm = _as(app.screen, RunnerRemoveConfirm)
        warning = "\n".join(str(widget.render()) for widget in confirm.query(Static))
        assert "pins this runner" not in warning
        await pilot.press("y")
        await pilot.pause()

    assert config.find_prompt_runner("same") == config.PromptRunner("same", ("first", "{{prompt}}"))
    assert store.resolve(entry.slug).meta.runner == "same"


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
    first = _prompt(tmp_path, name="first")
    second = _prompt(tmp_path, name="second")
    store.write_prompt_runner(first.slug, "claude")
    store.write_prompt_runner(second.slug, "claude")
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
        warning = "\n".join(str(s.render()) for s in confirm.query(Static))
        assert "2 prompts pin this runner" in warning
        await pilot.press("y")  # confirm
        await pilot.pause()
        assert isinstance(app.screen, RunnerManageScreen)
        # the list reloaded without claude
        remaining = screen.query_one(OptionList)
        prompts = [
            str(remaining.get_option_at_index(i).prompt) for i in range(remaining.option_count)
        ]
    assert config.find_prompt_runner("claude") is None
    assert not any("claude" in prompt for prompt in prompts)
    assert store.resolve(first.slug).meta.runner == "claude"
    assert store.resolve(second.slug).meta.runner == "claude"


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
