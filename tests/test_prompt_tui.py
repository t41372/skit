"""The prompt kind's TUI surfaces: the run form's runner picker (mouse AND keyboard),
the Library run/rerun guards, the add lane, and the settings screen's prompt sections.
"""

from __future__ import annotations

import contextlib

import pytest
from textual.widgets import Checkbox, Input, RadioSet, Static

from skit import argstate, config, flows, launcher, store, tui
from skit.tui_add import AddSourceScreen
from skit.tui_form import RunFormScreen
from skit.tui_settings import ScriptSettingsScreen


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
        return calls.get("code", 0)

    monkeypatch.setattr(launcher, "run_entry", fake_run)
    monkeypatch.setattr(tui.MenuApp, "suspend", lambda self: _noop_suspend())
    monkeypatch.setattr("builtins.input", lambda *a: "")
    return calls


def _prompt_entry(tmp_path, text="Do {a}\n", name="p", pin=""):
    src = tmp_path / f"{name}.prompt.md"
    src.write_text(text, encoding="utf-8")
    entry = store.add_prompt(src, name=name)
    if pin:
        entry = store.write_prompt_runner(entry.slug, pin)
    return entry


# --------------------------------------------------------------------------
# run form: the runner picker row
# --------------------------------------------------------------------------


async def test_form_picker_defaults_to_the_pin_and_submits_it(tmp_path, quiet_run):
    _prompt_entry(tmp_path, pin="codex")
    app = tui.MenuApp()
    async with app.run_test(size=(100, 32)) as pilot:
        await pilot.pause()
        app.action_run()
        await pilot.pause()
        screen = app.screen
        assert isinstance(screen, RunFormScreen)
        radio = screen.query_one("#runner-set", RadioSet)
        names = [r.name for r in config.load_prompt_runners()]
        assert names[radio.pressed_index] == "codex"
        screen.query_one(Input).value = "hello"
        screen.action_submit()
        await pilot.pause()
    assert quiet_run["values"] == {"a": "hello"}
    assert quiet_run["runner"] == config.find_prompt_runner("codex")


async def test_form_picker_keyboard_pick_runs_and_remembers(tmp_path, quiet_run):
    _prompt_entry(tmp_path)
    argstate.save_last_runner("opencode")
    app = tui.MenuApp()
    async with app.run_test(size=(100, 32)) as pilot:
        await pilot.pause()
        app.action_run()
        await pilot.pause()
        screen = app.screen
        assert isinstance(screen, RunFormScreen)
        radio = screen.query_one("#runner-set", RadioSet)
        names = [r.name for r in config.load_prompt_runners()]
        assert names[radio.pressed_index] == "opencode"  # last-picked prefill
        radio.focus()
        await pilot.pause()
        # Keyboard-only operation (policy #2): arrow to another option, Space picks it
        # (Enter would submit the form — the screen's priority binding owns it).
        await pilot.press("right")
        await pilot.press("space")
        await pilot.pause()
        picked = names[radio.pressed_index]
        assert picked != "opencode"  # the keys really moved the selection
        screen.query_one(Input).value = "x"
        await pilot.press("ctrl+r")
        await pilot.pause()
    assert quiet_run["runner"] == config.find_prompt_runner(picked)
    assert argstate.load_last_runner() == picked  # the pick was remembered


async def test_form_picker_mouse_click_picks_a_runner(tmp_path, quiet_run):
    _prompt_entry(tmp_path)
    app = tui.MenuApp()
    async with app.run_test(size=(110, 34)) as pilot:
        await pilot.pause()
        app.action_run()
        await pilot.pause()
        screen = app.screen
        assert isinstance(screen, RunFormScreen)
        radio = screen.query_one("#runner-set", RadioSet)
        buttons = list(radio.query("RadioButton"))
        await pilot.click(buttons[1])  # mouse-only operation (policy #2)
        await pilot.pause()
        screen.query_one(Input).value = "x"
        screen.action_submit()
        await pilot.pause()
    names = [r.name for r in config.load_prompt_runners()]
    assert quiet_run["runner"] == config.find_prompt_runner(names[1])


async def test_prompt_with_no_placeholders_still_shows_the_form_for_the_picker(tmp_path, quiet_run):
    _prompt_entry(tmp_path, text="No holes here.\n")
    app = tui.MenuApp()
    async with app.run_test(size=(100, 32)) as pilot:
        await pilot.pause()
        app.action_run()
        await pilot.pause()
        # A field-less prompt must NOT take the skip-the-form shortcut: the runner
        # question is still open, and the picker is how it gets answered.
        screen = app.screen
        assert isinstance(screen, RunFormScreen)
        screen.action_submit()
        await pilot.pause()
    assert quiet_run["runner"] is not None


async def test_pinned_promptless_prompt_keeps_the_shortcut(tmp_path, quiet_run):
    _prompt_entry(tmp_path, text="No holes here.\n", pin="claude")
    argstate.save_last_runner("")  # ensure the pin, not state, decides
    app = tui.MenuApp()
    async with app.run_test(size=(100, 32)) as pilot:
        await pilot.pause()
        app.action_run()
        await pilot.pause()
        # Still the form (prompt entries always get the picker) — but the default IS
        # the pin, so Enter alone runs it.
        screen = app.screen
        assert isinstance(screen, RunFormScreen)
        screen.action_submit()
        await pilot.pause()
    assert quiet_run["runner"] == config.find_prompt_runner("claude")


async def test_run_with_zero_runners_configured_is_an_honest_status(tmp_path, quiet_run):
    _prompt_entry(tmp_path)
    config.save_prompt_runners([])
    app = tui.MenuApp()
    async with app.run_test(size=(100, 32)) as pilot:
        await pilot.pause()
        app.action_run()
        await pilot.pause()
        assert not isinstance(app.screen, RunFormScreen)
        status = app.query_one("#status", Static)
        assert "No runners configured" in str(status.render())


async def test_rerun_unpinned_prompt_falls_back_to_the_form(tmp_path, quiet_run):
    entry = _prompt_entry(tmp_path)
    argstate.save_last(entry.slug, values={"a": "1"}, extra_args=[])
    argstate.record_run(entry.slug, 0, at="2026-07-17T00:00:00+00:00")
    app = tui.MenuApp()
    async with app.run_test(size=(100, 32)) as pilot:
        await pilot.pause()
        app.action_rerun()
        await pilot.pause()
        # No pin: rerun must never answer the runner question silently.
        assert isinstance(app.screen, RunFormScreen)


async def test_rerun_pinned_prompt_skips_the_form_and_uses_the_pin(
    tmp_path, quiet_run, monkeypatch
):
    entry = _prompt_entry(tmp_path, pin="claude")
    monkeypatch.setattr("skit.langs.launch._which", lambda name: f"/bin/{name}")
    argstate.save_last(entry.slug, values={"a": "1"}, extra_args=[])
    argstate.record_run(entry.slug, 0, at="2026-07-17T00:00:00+00:00")
    app = tui.MenuApp()
    async with app.run_test(size=(100, 32)) as pilot:
        await pilot.pause()
        app.action_rerun()
        await pilot.pause()
    assert quiet_run["values"] == {"a": "1"}
    assert quiet_run["runner"] is None  # the pin resolves inside PromptLaunch.build


async def test_exit_mode_pending_run_carries_the_runner(tmp_path, monkeypatch):
    _prompt_entry(tmp_path, pin="codex")
    config.save_after_run("exit")
    app = tui.MenuApp()
    async with app.run_test(size=(100, 32)) as pilot:
        await pilot.pause()
        app.action_run()
        await pilot.pause()
        screen = app.screen
        assert isinstance(screen, RunFormScreen)
        screen.query_one(Input).value = "v"
        screen.action_submit()
        await pilot.pause()
    pending = app.return_value
    assert isinstance(pending, tui.PendingRun)
    assert pending.runner == config.find_prompt_runner("codex")

    seen: dict[str, object] = {}

    def fake_execute(entry, plan, asm, *, emit, invoke_cwd=None, runner=None):
        seen["runner"] = runner
        return flows.RunOutcome(0)

    monkeypatch.setattr(flows, "execute", fake_execute)
    assert tui._finish_run(pending) == 0
    assert seen["runner"] == config.find_prompt_runner("codex")


async def test_detail_pane_names_the_runner(tmp_path):
    _prompt_entry(tmp_path, pin="claude")
    app = tui.MenuApp()
    async with app.run_test(size=(120, 34)) as pilot:
        await pilot.pause()
        detail = str(app.query_one("#detail-body", Static).render())
        assert "Runs with claude" in detail


async def test_detail_pane_unpinned_prompt_says_the_form_asks(tmp_path):
    _prompt_entry(tmp_path)
    app = tui.MenuApp()
    async with app.run_test(size=(120, 34)) as pilot:
        await pilot.pause()
        detail = str(app.query_one("#detail-body", Static).render())
        assert "Runner picked on the run form" in detail


# --------------------------------------------------------------------------
# add lane
# --------------------------------------------------------------------------


async def test_tui_add_prompt_file_direct_lane(tmp_path):
    src = tmp_path / "task.prompt.md"
    src.write_text("# Task\n\nDo {a} and {b}\n", encoding="utf-8")
    app = tui.MenuApp()
    async with app.run_test(size=(100, 32)) as pilot:
        await pilot.pause()
        app.action_add()
        await pilot.pause()
        screen = app.screen
        assert isinstance(screen, AddSourceScreen)
        screen.query_one("#add-path", Input).value = str(src)
        await pilot.pause()
        screen._submit_path()
        await pilot.pause()
    entry = store.resolve("task")
    assert entry.meta.kind == "prompt"
    assert entry.meta.params == ["a", "b"]
    assert entry.meta.workdir == "invoke"


async def test_tui_add_bare_md_becomes_a_prompt(tmp_path):
    src = tmp_path / "notes.md"
    src.write_text("Summarize {url}\n", encoding="utf-8")
    app = tui.MenuApp()
    async with app.run_test(size=(100, 32)) as pilot:
        await pilot.pause()
        app.action_add()
        await pilot.pause()
        screen = app.screen
        assert isinstance(screen, AddSourceScreen)
        screen.query_one("#add-path", Input).value = str(src)
        screen._submit_path()
        await pilot.pause()
    assert store.resolve("notes").meta.kind == "prompt"


# --------------------------------------------------------------------------
# settings
# --------------------------------------------------------------------------


async def _open_settings(app, pilot):
    app.action_settings()
    await pilot.pause()
    assert isinstance(app.screen, ScriptSettingsScreen)
    return app.screen


async def test_settings_prompt_rows_and_no_flag_input(tmp_path):
    _prompt_entry(tmp_path, text="{a} {api_key}\n")
    app = tui.MenuApp()
    async with app.run_test(size=(110, 40)) as pilot:
        await pilot.pause()
        screen = await _open_settings(app, pilot)
        from skit.tui_settings import DeclParamRow

        rows = list(screen.query(DeclParamRow))
        assert [r.decl.name for r in rows] == ["a", "api_key"]
        assert rows[1].decl.secret  # the name heuristic reaches the editor
        # The trait gate: placeholder kinds never grow a flag input.
        assert not screen.query(".d-flag")


async def test_settings_runner_radio_pins_and_clears(tmp_path):
    _prompt_entry(tmp_path, text="{a}\n")
    app = tui.MenuApp()
    async with app.run_test(size=(110, 40)) as pilot:
        await pilot.pause()
        screen = await _open_settings(app, pilot)
        radio = screen.query_one("#st-runner-set", RadioSet)
        assert radio.pressed_index == 0  # "ask on the run form" (no pin)
        names = [r.name for r in config.load_prompt_runners()]
        buttons = list(radio.query("RadioButton"))
        buttons[1].value = True  # the first configured runner
        await pilot.pause()
        screen.action_save()
        await pilot.pause()
    assert store.resolve("p").meta.runner == names[0]
    assert argstate.load_last_runner() == names[0]

    # And back to "ask each run".
    app = tui.MenuApp()
    async with app.run_test(size=(110, 40)) as pilot:
        await pilot.pause()
        screen = await _open_settings(app, pilot)
        radio = screen.query_one("#st-runner-set", RadioSet)
        assert radio.pressed_index == 1  # the saved pin is preselected
        next(iter(radio.query("RadioButton"))).value = True
        await pilot.pause()
        screen.action_save()
        await pilot.pause()
    assert store.resolve("p").meta.runner == ""


async def test_settings_runner_section_empty_state(tmp_path):
    _prompt_entry(tmp_path, text="{a}\n")
    config.save_prompt_runners([])
    app = tui.MenuApp()
    async with app.run_test(size=(110, 40)) as pilot:
        await pilot.pause()
        screen = await _open_settings(app, pilot)
        assert not screen.query("#st-runner-set")
        assert screen.query_one("#st-body")  # the screen still composes
        screen.action_save()  # and saving without a radio row is a clean no-op
        await pilot.pause()
    assert store.resolve("p").meta.runner == ""


async def test_settings_tick_to_manage_a_detected_placeholder(tmp_path):
    entry = _prompt_entry(tmp_path, text="{a} {b}\n")
    store.write_prompt_managed(entry.slug, ["a"])
    app = tui.MenuApp()
    async with app.run_test(size=(110, 40)) as pilot:
        await pilot.pause()
        screen = await _open_settings(app, pilot)
        box = screen.query_one("#st-prompt-new-0", Checkbox)
        assert "b" in str(box.label)
        box.value = True
        await pilot.pause()
        screen.action_save()
        await pilot.pause()
    reloaded = store.resolve("p")
    assert reloaded.meta.params == ["a", "b"]  # managed, in body order
    assert any(d.name == "b" and d.delivery == "placeholder" for d in store.read_parameters("p"))


async def test_settings_unticking_a_row_unmanages_it(tmp_path):
    _prompt_entry(tmp_path, text="{a} {b}\n")
    app = tui.MenuApp()
    async with app.run_test(size=(110, 40)) as pilot:
        await pilot.pause()
        screen = await _open_settings(app, pilot)
        from skit.tui_settings import DeclParamRow

        rows = list(screen.query(DeclParamRow))
        rows[0].query_one(".d-keep", Checkbox).value = False  # drop `a`
        await pilot.pause()
        screen.action_save()
        await pilot.pause()
    assert store.resolve("p").meta.params == ["b"]


async def test_settings_typing_a_body_hole_name_manages_it(tmp_path):
    entry = _prompt_entry(tmp_path, text="{a} {b}\n")
    store.write_prompt_managed(entry.slug, ["a"])
    app = tui.MenuApp()
    async with app.run_test(size=(110, 40)) as pilot:
        await pilot.pause()
        screen = await _open_settings(app, pilot)
        screen.query_one("#st-add-param", Input).value = "b"
        screen.action_save()
        await pilot.pause()
    assert store.resolve("p").meta.params == ["a", "b"]


async def test_form_submit_with_a_runner_removed_mid_flight_is_honest(tmp_path, quiet_run):
    _prompt_entry(tmp_path, pin="codex")
    app = tui.MenuApp()
    async with app.run_test(size=(100, 32)) as pilot:
        await pilot.pause()
        app.action_run()
        await pilot.pause()
        app.screen.query_one(Input).value = "x"
        config.save_prompt_runners([])  # yanked while the form was open
        screen = app.screen
        assert isinstance(screen, RunFormScreen)
        screen.action_submit()
        await pilot.pause()
        status = app.query_one("#status", Static)
        assert "no longer configured" in str(status.render())
    assert "runner" not in quiet_run  # nothing launched


async def test_tui_add_prompt_without_placeholders_skips_the_toast(tmp_path):
    src = tmp_path / "plain.prompt.md"
    src.write_text("No holes.\n", encoding="utf-8")
    app = tui.MenuApp()
    async with app.run_test(size=(100, 32)) as pilot:
        await pilot.pause()
        app.action_add()
        await pilot.pause()
        screen = app.screen
        assert isinstance(screen, AddSourceScreen)
        screen.query_one("#add-path", Input).value = str(src)
        screen._submit_path()
        await pilot.pause()
    assert store.resolve("plain").meta.params is None


async def test_settings_save_preserves_a_stale_pin(tmp_path):
    # The pinned runner's config row is gone: opening settings and saving something
    # unrelated must NOT silently clear the pin — its own radio row holds it selected.
    _prompt_entry(tmp_path, text="{a}\n", pin="mine")
    config.save_prompt_runners([config.PromptRunner("other", ("other", "{prompt}"))])
    app = tui.MenuApp()
    async with app.run_test(size=(110, 40)) as pilot:
        await pilot.pause()
        screen = await _open_settings(app, pilot)
        radio = screen.query_one("#st-runner-set", RadioSet)
        assert radio.pressed_index == 1  # the stale pin's own row, preselected
        screen.action_save()
        await pilot.pause()
    assert store.resolve("p").meta.runner == "mine"  # preserved, not wiped

    # Explicitly picking a configured runner (index 2 = "other") replaces it.
    app = tui.MenuApp()
    async with app.run_test(size=(110, 40)) as pilot:
        await pilot.pause()
        screen = await _open_settings(app, pilot)
        radio = screen.query_one("#st-runner-set", RadioSet)
        buttons = list(radio.query("RadioButton"))
        buttons[2].value = True
        await pilot.pause()
        screen.action_save()
        await pilot.pause()
    assert store.resolve("p").meta.runner == "other"
    assert argstate.load_last_runner() == "other"
