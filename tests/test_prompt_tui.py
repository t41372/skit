"""The prompt kind's TUI surfaces: the run form's runner picker (mouse AND keyboard),
the Library run/rerun guards, the add lane, and the settings screen's prompt sections.
"""

from __future__ import annotations

import contextlib

import pytest
from textual.widgets import Checkbox, Input, OptionList, RadioSet, Select, Static

from skit import argstate, config, flows, launcher, store, tui
from skit.tui_add import AddSourceScreen, PromptReviewScreen
from skit.tui_form import RunFormScreen
from skit.tui_runner import RunnerAddModal
from skit.tui_settings import ScriptSettingsScreen


def _value(select: Select[str]) -> str:
    """A runner Select's current value as a plain string. Every runner picker is
    allow_blank=False with an explicit "" option for the "ask on the run form" state, so
    the value is always a real str, never the NULL sentinel — reads need no index math."""
    value = select.value
    assert isinstance(value, str)
    return value


def _option_count(select: Select[str]) -> int:
    """How many options a Select carries. Select has no public option accessor, so we read
    the private _options; allow_blank=False means it holds no synthetic leading blank row."""
    return len(select._options)


async def _click_option(pilot, overlay: OptionList, index: int) -> None:
    """Mouse-click option `index` in an open Select overlay. The overlay draws a one-row
    top border, so option i sits at row i+1 (x=2 lands inside the padded label)."""
    await pilot.click(overlay, offset=(2, index + 1))
    await pilot.pause()


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


def _prompt_entry(tmp_path, text="Do {{a}}\n", name="p", pin=""):
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
        select = screen.query_one("#runner-select", Select)
        assert _value(select) == "codex"  # the pin is the default
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
        select = screen.query_one("#runner-select", Select)
        assert _value(select) == "opencode"  # last-picked prefill
        select.focus()
        await pilot.pause()
        # Keyboard-only operation (policy #2): Enter opens the overlay (the shim), ↓ moves
        # the highlight, Enter chooses it — none of these submit while the Select owns focus.
        await pilot.press("enter")
        await pilot.pause()
        await pilot.press("down")
        await pilot.press("enter")
        await pilot.pause()
        picked = _value(select)
        assert picked != "opencode"  # the keys really moved the selection
        # Step off the Select to a field so Ctrl+R runs (on the Select it would re-open it).
        field = screen.query_one(Input)
        field.value = "x"
        field.focus()
        await pilot.pause()
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
        select = screen.query_one("#runner-select", Select)
        await pilot.click(select)  # mouse-only (policy #2): open the overlay…
        await pilot.pause()
        assert select.expanded
        await _click_option(pilot, select.query_one(OptionList), 1)  # …and click an option
        names = [r.name for r in config.load_prompt_runners()]
        assert _value(select) == names[1]  # the mouse pick landed
        # A pick re-focuses the Select; step off it before running.
        field = screen.query_one(Input)
        field.value = "x"
        field.focus()
        await pilot.pause()
        screen.action_submit()
        await pilot.pause()
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


async def test_run_with_zero_runners_offers_the_new_agent_modal(tmp_path, quiet_run):
    _prompt_entry(tmp_path)
    config.save_prompt_runners([])
    app = tui.MenuApp()
    async with app.run_test(size=(100, 32)) as pilot:
        await pilot.pause()
        app.action_run()
        await pilot.pause()
        # An emptied runner list must not dead-end on a CLI incantation: the New
        # agent modal opens right here. Esc = an honest status, nothing launched.
        screen = app.screen
        assert isinstance(screen, RunnerAddModal)
        await pilot.press("escape")
        await pilot.pause()
        status = app.query_one("#status", Static)
        assert "needs a configured agent" in str(status.render())
    assert "runner" not in quiet_run


async def test_run_with_zero_runners_define_agent_then_run(tmp_path, quiet_run):
    _prompt_entry(tmp_path)
    config.save_prompt_runners([])
    app = tui.MenuApp()
    async with app.run_test(size=(100, 32)) as pilot:
        await pilot.pause()
        app.action_run()
        await pilot.pause()
        modal = app.screen
        assert isinstance(modal, RunnerAddModal)
        modal.query_one("#runner-add-name", Input).value = "mycli"
        modal.query_one("#runner-add-command", Input).value = "mycli run {{prompt}}"
        await pilot.press("enter")  # the advertised Save key
        await pilot.pause()
        await pilot.pause()
        # The run re-enters with the runner configured — straight into the form.
        screen = app.screen
        assert isinstance(screen, RunFormScreen)
        screen.query_one(Input).value = "x"
        screen.action_submit()
        await pilot.pause()
    assert quiet_run["runner"] == config.find_prompt_runner("mycli")
    assert config.find_prompt_runner("mycli") is not None


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


async def test_detail_pane_stale_pin_says_no_longer_configured(tmp_path):
    """A prompt pinned to a runner whose config row is gone: the detail pane says
    '(no longer configured)' — the same honesty Script settings gives (two surfaces, one
    truth), never a bare 'Runs with X' that would launch straight into a 126."""
    _prompt_entry(tmp_path, pin="nonesuch-agent")  # not a configured runner
    app = tui.MenuApp()
    async with app.run_test(size=(120, 34)) as pilot:
        await pilot.pause()
        detail = str(app.query_one("#detail-body", Static).render())
        assert "nonesuch-agent" in detail
        assert "no longer configured" in detail


# --------------------------------------------------------------------------
# add lane
# --------------------------------------------------------------------------


async def test_tui_add_prompt_opens_the_review_panel(tmp_path):
    src = tmp_path / "task.prompt.md"
    src.write_text("# Task\n\nDo {{a}} and {{b}}\n", encoding="utf-8")
    app = tui.MenuApp()
    async with app.run_test(size=(100, 40)) as pilot:
        await pilot.pause()
        app.action_add()
        await pilot.pause()
        screen = app.screen
        assert isinstance(screen, AddSourceScreen)
        screen.query_one("#add-path", Input).value = str(src)
        await pilot.pause()
        screen._submit_path()
        await pilot.pause()
        # Never a blind direct add: the prompt review panel opens, prefilled.
        review = app.screen
        assert isinstance(review, PromptReviewScreen)
        assert review.query_one("#pv-name", Input).value == "task"
        assert review.query_one("#pv-desc", Input).value == "Task"
        boxes = [c for c in review.query(Checkbox) if (c.id or "").startswith("pv-hole-")]
        assert len(boxes) == 2
        assert all(b.value for b in boxes)  # under the cap: everything pre-ticked
        await pilot.press("ctrl+s")  # the advertised Add key
        await pilot.pause()
        await pilot.pause()
    entry = store.resolve("task")
    assert entry.meta.kind == "prompt"
    assert entry.meta.params == ["a", "b"]
    assert entry.meta.workdir == "invoke"
    assert entry.meta.runner == ""  # default: ask on the run form


async def test_tui_add_bare_md_becomes_a_prompt(tmp_path):
    src = tmp_path / "notes.md"
    src.write_text("Summarize {{url}}\n", encoding="utf-8")
    app = tui.MenuApp()
    async with app.run_test(size=(100, 40)) as pilot:
        await pilot.pause()
        app.action_add()
        await pilot.pause()
        screen = app.screen
        assert isinstance(screen, AddSourceScreen)
        screen.query_one("#add-path", Input).value = str(src)
        screen._submit_path()
        await pilot.pause()
        review = app.screen
        assert isinstance(review, PromptReviewScreen)
        review.action_accept()
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
    _prompt_entry(tmp_path, text="{{a}} {{api_key}}\n")
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
    _prompt_entry(tmp_path, text="{{a}}\n")
    app = tui.MenuApp()
    async with app.run_test(size=(110, 40)) as pilot:
        await pilot.pause()
        screen = await _open_settings(app, pilot)
        select = screen.query_one("#st-runner-select", Select)
        assert _value(select) == ""  # "ask on the run form" (no pin)
        names = [r.name for r in config.load_prompt_runners()]
        select.value = names[0]  # the first configured runner
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
        select = screen.query_one("#st-runner-select", Select)
        assert _value(select) == names[0]  # the saved pin is preselected
        select.value = ""  # back to "ask on the run form"
        await pilot.pause()
        screen.action_save()
        await pilot.pause()
    assert store.resolve("p").meta.runner == ""


async def test_settings_runner_section_empty_config_keeps_ask_and_the_door(tmp_path):
    _prompt_entry(tmp_path, text="{{a}}\n")
    config.save_prompt_runners([])
    app = tui.MenuApp()
    async with app.run_test(size=(110, 40)) as pilot:
        await pilot.pause()
        screen = await _open_settings(app, pilot)
        select = screen.query_one("#st-runner-select", Select)
        assert _option_count(select) == 1  # just "ask on the run form"
        assert _value(select) == ""
        assert screen.query("#st-runner-new")  # the New agent… door never disappears
        screen.action_save()  # saving the lone "ask" option is a clean no-op
        await pilot.pause()
    assert store.resolve("p").meta.runner == ""


async def test_settings_ctrl_n_adds_a_custom_agent_ready_to_pin(tmp_path):
    _prompt_entry(tmp_path, text="{{a}}\n")
    app = tui.MenuApp()
    async with app.run_test(size=(110, 40)) as pilot:
        await pilot.pause()
        screen = await _open_settings(app, pilot)
        await pilot.press("ctrl+n")  # the advertised New agent… key
        await pilot.pause()
        modal = app.screen
        assert isinstance(modal, RunnerAddModal)
        modal.query_one("#runner-add-name", Input).value = "mycli"
        modal.query_one("#runner-add-command", Input).value = "mycli go {{prompt}}"
        modal.action_save_runner()
        await pilot.pause()
        select = screen.query_one("#st-runner-select", Select)
        assert _value(select) == "mycli"  # the new agent is selected in place
        screen.action_save()
        await pilot.pause()
    assert store.resolve("p").meta.runner == "mycli"  # the value survived the mid-session add
    assert argstate.load_last_runner() == "mycli"


async def test_settings_ctrl_n_add_preserves_a_stale_pin_option(tmp_path):
    # A stale pin (pinned to a runner no longer configured) plus a mid-session Ctrl+N add:
    # the rebuilt dropdown must STILL carry the stale-pin row, never silently drop it.
    _prompt_entry(tmp_path, text="{{a}}\n", pin="gone")
    config.save_prompt_runners([config.PromptRunner("other", ("other", "{{prompt}}"))])
    app = tui.MenuApp()
    async with app.run_test(size=(110, 40)) as pilot:
        await pilot.pause()
        screen = await _open_settings(app, pilot)
        select = screen.query_one("#st-runner-select", Select)
        assert _value(select) == "gone"  # the stale pin is preselected
        await pilot.press("ctrl+n")
        await pilot.pause()
        modal = app.screen
        assert isinstance(modal, RunnerAddModal)
        modal.query_one("#runner-add-name", Input).value = "fresh"
        modal.query_one("#runner-add-command", Input).value = "fresh go {{prompt}}"
        modal.action_save_runner()
        await pilot.pause()
        select = screen.query_one("#st-runner-select", Select)
        assert _value(select) == "fresh"  # the new agent is selected in place
        # ask + stale "gone" + "other" + new "fresh": count 4 only if the stale row survived.
        assert _option_count(select) == 4


async def test_settings_pin_change_saves_even_with_insertion_off(tmp_path):
    # The declared-params branch is skipped when insertion is off — the pin save must
    # not live inside it, or a pin change on an insertion-off prompt silently drops.
    entry = _prompt_entry(tmp_path, text="{{a}}\n")
    store.write_prompt_interpolate(entry.slug, False)
    app = tui.MenuApp()
    async with app.run_test(size=(110, 40)) as pilot:
        await pilot.pause()
        screen = await _open_settings(app, pilot)
        select = screen.query_one("#st-runner-select", Select)
        names = [r.name for r in config.load_prompt_runners()]
        select.value = names[0]  # the first configured runner
        await pilot.pause()
        screen.action_save()
        await pilot.pause()
    assert store.resolve("p").meta.runner == names[0]


async def test_settings_tick_to_manage_a_detected_placeholder(tmp_path):
    entry = _prompt_entry(tmp_path, text="{{a}} {{b}}\n")
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
    _prompt_entry(tmp_path, text="{{a}} {{b}}\n")
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
    entry = _prompt_entry(tmp_path, text="{{a}} {{b}}\n")
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


async def test_review_prompt_without_placeholders_says_so_and_adds_clean(tmp_path):
    src = tmp_path / "plain.prompt.md"
    src.write_text("No holes.\n", encoding="utf-8")
    app = tui.MenuApp()
    async with app.run_test(size=(100, 40)) as pilot:
        await pilot.pause()
        app.push_screen(PromptReviewScreen(src))
        await pilot.pause()
        review = app.screen
        assert isinstance(review, PromptReviewScreen)
        hints = [str(s.render()) for s in review.query(".hint")]
        assert any("No {{name}} placeholders detected" in h for h in hints)
        assert not [c for c in review.query(Checkbox) if (c.id or "").startswith("pv-hole-")]
        review.action_accept()
        await pilot.pause()
    assert store.resolve("plain").meta.params is None


async def test_settings_save_preserves_a_stale_pin(tmp_path):
    # The pinned runner's config row is gone: opening settings and saving something
    # unrelated must NOT silently clear the pin — its own radio row holds it selected.
    _prompt_entry(tmp_path, text="{{a}}\n", pin="mine")
    config.save_prompt_runners([config.PromptRunner("other", ("other", "{{prompt}}"))])
    app = tui.MenuApp()
    async with app.run_test(size=(110, 40)) as pilot:
        await pilot.pause()
        screen = await _open_settings(app, pilot)
        select = screen.query_one("#st-runner-select", Select)
        assert _value(select) == "mine"  # the stale pin's own row, preselected
        screen.action_save()
        await pilot.pause()
    assert store.resolve("p").meta.runner == "mine"  # preserved, not wiped

    # Explicitly picking the one configured runner replaces it.
    app = tui.MenuApp()
    async with app.run_test(size=(110, 40)) as pilot:
        await pilot.pause()
        screen = await _open_settings(app, pilot)
        select = screen.query_one("#st-runner-select", Select)
        select.value = "other"
        await pilot.pause()
        screen.action_save()
        await pilot.pause()
    assert store.resolve("p").meta.runner == "other"
    assert argstate.load_last_runner() == "other"


async def test_settings_interpolate_toggle_off_and_back_on(tmp_path):
    _prompt_entry(tmp_path, text="{{a}}\n")
    app = tui.MenuApp()
    async with app.run_test(size=(110, 40)) as pilot:
        await pilot.pause()
        screen = await _open_settings(app, pilot)
        toggle = screen.query_one("#st-interpolate", Checkbox)
        assert toggle.value is True
        toggle.value = False  # one click…
        await pilot.pause()
        screen.action_save()  # …plus Save turns insertion off
        await pilot.pause()
    off = store.resolve("p")
    assert off.meta.interpolate is False
    assert off.meta.params == ["a"]  # the managed list survives underneath

    # Off state: no rows, no candidates, no add-param input — just the toggle + hint.
    app = tui.MenuApp()
    async with app.run_test(size=(110, 40)) as pilot:
        await pilot.pause()
        screen = await _open_settings(app, pilot)
        from skit.tui_settings import DeclParamRow

        assert not screen.query(DeclParamRow)
        assert not screen.query("#st-add-param")
        toggle = screen.query_one("#st-interpolate", Checkbox)
        assert toggle.value is False
        toggle.value = True
        await pilot.pause()
        screen.action_save()
        await pilot.pause()
    on = store.resolve("p")
    assert on.meta.interpolate is True
    assert on.meta.params == ["a"]  # untouched by the off/on round trip


async def test_settings_candidate_checkboxes_are_flood_capped(tmp_path):
    from skit.langs.prompt.analyzer import LIST_PREVIEW_LIMIT

    many = " ".join("{{u" + str(i) + "}}" for i in range(LIST_PREVIEW_LIMIT + 9))
    entry = _prompt_entry(tmp_path, text="{{a}} " + many + "\n")
    store.write_prompt_managed(entry.slug, ["a"])
    app = tui.MenuApp()
    async with app.run_test(size=(110, 40)) as pilot:
        await pilot.pause()
        screen = await _open_settings(app, pilot)
        boxes = [c for c in screen.query(Checkbox) if (c.id or "").startswith("st-prompt-new-")]
        assert len(boxes) == LIST_PREVIEW_LIMIT


async def test_review_flooded_prompt_previews_capped_and_ticks_nothing(tmp_path):
    from skit.langs.prompt.analyzer import AUTO_MANAGE_LIMIT, LIST_PREVIEW_LIMIT

    many = " ".join("{{h" + str(i) + "}}" for i in range(AUTO_MANAGE_LIMIT + 4))
    src = tmp_path / "big.prompt.md"
    src.write_text(many + "\n", encoding="utf-8")
    app = tui.MenuApp()
    async with app.run_test(size=(100, 40)) as pilot:
        await pilot.pause()
        app.push_screen(PromptReviewScreen(src))
        await pilot.pause()
        review = app.screen
        assert isinstance(review, PromptReviewScreen)
        boxes = [c for c in review.query(Checkbox) if (c.id or "").startswith("pv-hole-")]
        assert len(boxes) == LIST_PREVIEW_LIMIT  # preview, never a wall of checkboxes
        assert not any(b.value for b in boxes)  # flood default: nothing pre-ticked
        warns = [str(s.render()) for s in review.query(".warn")]
        assert any("probably not written for" in w for w in warns)
        hints = [str(s.render()) for s in review.query(".hint")]
        assert any("more" in h for h in hints)  # the honest "+N more" line
        review.action_accept()
        await pilot.pause()
    assert store.resolve("big").meta.params is None  # nothing was asked for


# --------------------------------------------------------------------------
# the prompt review panel
# --------------------------------------------------------------------------


async def test_review_space_untick_keeps_a_subset(tmp_path):
    src = tmp_path / "t.prompt.md"
    src.write_text("{{a}} {{b}}\n", encoding="utf-8")
    app = tui.MenuApp()
    async with app.run_test(size=(100, 40)) as pilot:
        await pilot.pause()
        app.push_screen(PromptReviewScreen(src))
        await pilot.pause()
        review = app.screen
        assert isinstance(review, PromptReviewScreen)
        box_b = review.query_one("#pv-hole-1", Checkbox)
        box_b.focus()
        await pilot.pause()
        await pilot.press("space")  # the advertised Toggle key
        await pilot.pause()
        assert box_b.value is False
        review.action_accept()
        await pilot.pause()
    assert store.resolve("t").meta.params == ["a"]


async def test_review_insertion_switch_off_hides_ticks_and_stores_off(tmp_path):
    src = tmp_path / "raw.prompt.md"
    src.write_text("Use {{tool}} literally\n", encoding="utf-8")
    app = tui.MenuApp()
    async with app.run_test(size=(100, 40)) as pilot:
        await pilot.pause()
        app.push_screen(PromptReviewScreen(src))
        await pilot.pause()
        review = app.screen
        assert isinstance(review, PromptReviewScreen)
        assert review.query_one("#pv-holes").display is True
        review.query_one("#pv-interpolate", Checkbox).value = False
        await pilot.pause()
        assert review.query_one("#pv-holes").display is False  # machinery folds away
        review.action_accept()
        await pilot.pause()
    entry = store.resolve("raw")
    assert entry.meta.interpolate is False
    assert entry.meta.params is None  # nothing managed, body travels verbatim


async def test_review_runner_pick_pins_and_remembers(tmp_path):
    src = tmp_path / "r.prompt.md"
    src.write_text("Go {{a}}\n", encoding="utf-8")
    app = tui.MenuApp()
    async with app.run_test(size=(110, 40)) as pilot:
        await pilot.pause()
        app.push_screen(PromptReviewScreen(src))
        await pilot.pause()
        review = app.screen
        assert isinstance(review, PromptReviewScreen)
        select = review.query_one("#pv-runner-select", Select)
        assert _value(select) == ""  # no pin, no last pick: "ask on the run form"
        names = [r.name for r in config.load_prompt_runners()]
        select.value = names[0]  # the first configured runner
        await pilot.pause()
        review.action_accept()
        await pilot.pause()
    assert store.resolve("r").meta.runner == names[0]
    assert argstate.load_last_runner() == names[0]  # a real pick is remembered


async def test_review_prefills_last_picked_and_explicit_runner_wins(tmp_path):
    argstate.save_last_runner("amp")
    src = tmp_path / "l.prompt.md"
    src.write_text("x {{a}}\n", encoding="utf-8")
    app = tui.MenuApp()
    async with app.run_test(size=(110, 40)) as pilot:
        await pilot.pause()
        app.push_screen(PromptReviewScreen(src))
        await pilot.pause()
        review = app.screen
        assert isinstance(review, PromptReviewScreen)
        select = review.query_one("#pv-runner-select", Select)
        assert _value(select) == "amp"  # last-picked prefill
        review.action_cancel()
        await pilot.pause()

        app.push_screen(PromptReviewScreen(src, runner="codex", interpolate=False))
        await pilot.pause()
        review = app.screen
        assert isinstance(review, PromptReviewScreen)
        select = review.query_one("#pv-runner-select", Select)
        assert _value(select) == "codex"  # the flag wins
        assert review.query_one("#pv-interpolate", Checkbox).value is False
        review.action_cancel()
        await pilot.pause()


async def test_review_escape_adds_nothing(tmp_path):
    src = tmp_path / "e.prompt.md"
    src.write_text("{{a}}\n", encoding="utf-8")
    app = tui.MenuApp()
    async with app.run_test(size=(100, 40)) as pilot:
        await pilot.pause()
        app.push_screen(PromptReviewScreen(src))
        await pilot.pause()
        await pilot.press("escape")  # the advertised Cancel key
        await pilot.pause()
    assert store.list_entries() == []


async def test_review_ctrl_e_rescans_and_keeps_edits(tmp_path, monkeypatch):
    src = tmp_path / "e.prompt.md"
    src.write_text("{{a}}\n", encoding="utf-8")
    monkeypatch.setattr(tui.MenuApp, "suspend", lambda self: _noop_suspend())
    monkeypatch.setattr(
        "skit.tui_add.editor.open_in_editor",
        lambda path: path.write_text("{{a}} {{b}}\n", encoding="utf-8"),
    )
    app = tui.MenuApp()
    async with app.run_test(size=(100, 40)) as pilot:
        await pilot.pause()
        app.push_screen(PromptReviewScreen(src))
        await pilot.pause()
        review = app.screen
        assert isinstance(review, PromptReviewScreen)
        review.query_one("#pv-name", Input).value = "renamed"
        # Ctrl+E is non-priority now (it belongs to the Input mid-edit): step off the
        # Input first — the chord fires from any non-Input focus, the chip always works.
        review.query_one("#pv-interpolate", Checkbox).focus()
        await pilot.pause()
        await pilot.press("ctrl+e")  # the advertised Edit key
        await pilot.pause()
        assert review.query_one("#pv-name", Input).value == "renamed"  # edit survived
        boxes = [c for c in review.query(Checkbox) if (c.id or "").startswith("pv-hole-")]
        assert len(boxes) == 2  # the rescan saw the new hole
        review.action_accept()
        await pilot.pause()
    assert store.resolve("renamed").meta.params == ["a", "b"]


async def test_review_ctrl_e_in_input_is_end_of_line_not_editor(tmp_path, monkeypatch):
    """The prompt review's Ctrl+E is non-priority too: while an Input has focus it is that
    Input's end-of-line, not $EDITOR (the Ctrl+A rule). The chip stays the mouse path."""
    edited: list[int] = []
    monkeypatch.setattr(PromptReviewScreen, "action_edit_source", lambda self: edited.append(1))
    src = tmp_path / "e.prompt.md"
    src.write_text("{{a}}\n", encoding="utf-8")
    app = tui.MenuApp()
    async with app.run_test(size=(100, 40)) as pilot:
        await pilot.pause()
        app.push_screen(PromptReviewScreen(src))
        await pilot.pause()
        review = app.screen
        assert isinstance(review, PromptReviewScreen)
        name = review.query_one("#pv-name", Input)
        name.focus()
        name.value = "hello"
        name.cursor_position = 0
        await pilot.pause()
        await pilot.press("ctrl+e")
        await pilot.pause()
        assert name.cursor_position == len("hello")  # end-of-line, the Input owns it
        assert edited == []  # …never opened the editor


async def test_review_reference_mode_links_the_original(tmp_path):
    src = tmp_path / "linked.prompt.md"
    src.write_text("{{a}}\n", encoding="utf-8")
    app = tui.MenuApp()
    async with app.run_test(size=(100, 40)) as pilot:
        await pilot.pause()
        app.push_screen(PromptReviewScreen(src, reference=True))
        await pilot.pause()
        review = app.screen
        assert isinstance(review, PromptReviewScreen)
        assert review.query_one("#pv-mode", RadioSet).pressed_index == 1  # prefilled
        review.action_accept()
        await pilot.pause()
    entry = store.resolve("linked")
    assert entry.meta.mode == "reference"
    assert entry.script_path == src


async def test_review_duplicate_name_notifies_and_stays(tmp_path):
    _prompt_entry(tmp_path, name="dup")
    src = tmp_path / "x.prompt.md"
    src.write_text("hi\n", encoding="utf-8")
    app = tui.MenuApp()
    async with app.run_test(size=(100, 40)) as pilot:
        await pilot.pause()
        app.push_screen(PromptReviewScreen(src, name="dup"))
        await pilot.pause()
        review = app.screen
        assert isinstance(review, PromptReviewScreen)
        review.action_accept()
        await pilot.pause()
        assert app.screen is review  # the error keeps the panel open
    assert len(store.list_entries()) == 1  # nothing new landed


async def test_review_ctrl_n_defines_a_custom_agent_and_selects_it(tmp_path):
    src = tmp_path / "n.prompt.md"
    src.write_text("{{a}}\n", encoding="utf-8")
    app = tui.MenuApp()
    async with app.run_test(size=(110, 40)) as pilot:
        await pilot.pause()
        app.push_screen(PromptReviewScreen(src))
        await pilot.pause()
        review = app.screen
        assert isinstance(review, PromptReviewScreen)
        await pilot.press("ctrl+n")  # the advertised New agent… key
        await pilot.pause()
        modal = app.screen
        assert isinstance(modal, RunnerAddModal)
        modal.query_one("#runner-add-name", Input).value = "aider"
        modal.query_one("#runner-add-command", Input).value = "aider --message {{prompt}}"
        modal.action_save_runner()  # the Save chip's click twin
        await pilot.pause()
        select = review.query_one("#pv-runner-select", Select)
        assert _value(select) == "aider"  # new agent selected in place
        review.action_accept()
        await pilot.pause()
    assert store.resolve("n").meta.runner == "aider"
    assert config.find_prompt_runner("aider") is not None  # persisted to config


async def test_review_escape_returns_to_the_add_source_screen(tmp_path):
    src = tmp_path / "back.prompt.md"
    src.write_text("{{a}}\n", encoding="utf-8")
    app = tui.MenuApp()
    async with app.run_test(size=(100, 40)) as pilot:
        await pilot.pause()
        app.action_add()
        await pilot.pause()
        source = app.screen
        assert isinstance(source, AddSourceScreen)
        source.query_one("#add-path", Input).value = str(src)
        source._submit_path()
        await pilot.pause()
        assert isinstance(app.screen, PromptReviewScreen)
        await pilot.press("escape")
        await pilot.pause()
        # Cancelling the review lands back on the source step, not the Library.
        assert app.screen is source
    assert store.list_entries() == []


async def test_review_description_prefill_and_toggle_action(tmp_path):
    src = tmp_path / "d.prompt.md"
    src.write_text("{{a}}\n", encoding="utf-8")
    app = tui.MenuApp()
    async with app.run_test(size=(100, 40)) as pilot:
        await pilot.pause()
        app.push_screen(PromptReviewScreen(src, description="hand-written"))
        await pilot.pause()
        review = app.screen
        assert isinstance(review, PromptReviewScreen)
        assert review.query_one("#pv-desc", Input).value == "hand-written"
        box = review.query_one("#pv-hole-0", Checkbox)
        box.focus()
        await pilot.pause()
        review.action_toggle_candidate()  # the footer chip's click twin
        assert box.value is False
        review.query_one("#pv-name", Input).focus()
        await pilot.pause()
        review.action_toggle_candidate()  # non-checkbox focus: a clean no-op
        assert box.value is False
        review.action_cancel()
        await pilot.pause()


async def test_review_modal_cancel_leaves_the_picker_alone(tmp_path):
    src = tmp_path / "c.prompt.md"
    src.write_text("{{a}}\n", encoding="utf-8")
    app = tui.MenuApp()
    async with app.run_test(size=(100, 40)) as pilot:
        await pilot.pause()
        app.push_screen(PromptReviewScreen(src))
        await pilot.pause()
        review = app.screen
        assert isinstance(review, PromptReviewScreen)
        before = _option_count(review.query_one("#pv-runner-select", Select))
        await pilot.press("ctrl+n")
        await pilot.pause()
        assert isinstance(app.screen, RunnerAddModal)
        await pilot.press("escape")
        await pilot.pause()
        after = _option_count(review.query_one("#pv-runner-select", Select))
        assert after == before  # a cancelled add adds no option
        review.action_cancel()
        await pilot.pause()


async def test_review_ctrl_e_keeps_the_runner_pick_and_reports_editor_errors(tmp_path, monkeypatch):
    from skit import editor as editor_mod

    src = tmp_path / "k.prompt.md"
    src.write_text("{{a}}\n", encoding="utf-8")
    monkeypatch.setattr(tui.MenuApp, "suspend", lambda self: _noop_suspend())
    monkeypatch.setattr("skit.tui_add.editor.open_in_editor", lambda path: None)
    names = [r.name for r in config.load_prompt_runners()]
    app = tui.MenuApp()
    async with app.run_test(size=(110, 40)) as pilot:
        await pilot.pause()
        app.push_screen(PromptReviewScreen(src))
        await pilot.pause()
        review = app.screen
        assert isinstance(review, PromptReviewScreen)
        select = review.query_one("#pv-runner-select", Select)
        select.value = names[1]  # pick the second configured runner
        await pilot.pause()
        review.action_edit_source()
        await pilot.pause()
        select = review.query_one("#pv-runner-select", Select)
        assert _value(select) == names[1]  # the pick survived the rescan

        # An editor failure is reported, never a crash out of the panel.
        monkeypatch.setattr(
            "skit.tui_add.editor.open_in_editor",
            lambda path: (_ for _ in ()).throw(editor_mod.EditorError("no editor")),
        )
        review.action_edit_source()
        await pilot.pause()
        assert isinstance(app.screen, PromptReviewScreen)  # still standing
        review.action_cancel()
        await pilot.pause()


async def test_form_ctrl_n_is_a_noop_without_a_picker(tmp_path, quiet_run):
    store.add_command("echo {x}", name="plaincmd")  # a form with fields, no runner row
    app = tui.MenuApp()
    async with app.run_test(size=(100, 32)) as pilot:
        await pilot.pause()
        app.action_run()
        await pilot.pause()
        screen = app.screen
        assert isinstance(screen, RunFormScreen)
        await pilot.press("ctrl+n")  # no runner picker on a python form
        await pilot.pause()
        assert app.screen is screen  # no modal opened
        screen.action_cancel()
        await pilot.pause()


async def test_form_modal_cancel_leaves_the_picker_alone(tmp_path, quiet_run):
    _prompt_entry(tmp_path)
    app = tui.MenuApp()
    async with app.run_test(size=(110, 34)) as pilot:
        await pilot.pause()
        app.action_run()
        await pilot.pause()
        screen = app.screen
        assert isinstance(screen, RunFormScreen)
        select = screen.query_one("#runner-select", Select)
        before = _option_count(select)
        await pilot.press("ctrl+n")
        await pilot.pause()
        assert isinstance(app.screen, RunnerAddModal)
        await pilot.press("escape")
        await pilot.pause()
        assert _option_count(select) == before  # a cancelled add adds no option
        screen.action_cancel()
        await pilot.pause()


async def test_settings_ctrl_n_is_a_noop_on_non_prompt_entries(tmp_path):
    py = tmp_path / "s.py"
    py.write_text('X = "1"\nprint(X)\n', encoding="utf-8")
    store.add_python(py, name="plainpy")
    app = tui.MenuApp()
    async with app.run_test(size=(110, 40)) as pilot:
        await pilot.pause()
        screen = await _open_settings(app, pilot)
        await pilot.press("ctrl+n")
        await pilot.pause()
        assert app.screen is screen  # no modal on a python entry's settings


async def test_settings_runner_select_change_arms_the_discard_ask(tmp_path):
    # A pin-only edit is a real edit: Esc must raise the unsaved-changes ask, never
    # silently drop it (Select.Changed is what arms the dirty flag for the runner pin).
    from skit.tui_settings import DiscardChangesModal

    _prompt_entry(tmp_path, text="{{a}}\n")
    app = tui.MenuApp()
    async with app.run_test(size=(110, 40)) as pilot:
        await pilot.pause()
        screen = await _open_settings(app, pilot)
        select = screen.query_one("#st-runner-select", Select)
        names = [r.name for r in config.load_prompt_runners()]
        select.value = names[0]  # a pin-only edit
        await pilot.pause()
        assert screen._dirty is True
        await pilot.press("escape")
        await pilot.pause()
        assert isinstance(app.screen, DiscardChangesModal)
        await pilot.press("escape")  # keep editing
        await pilot.pause()
    assert store.resolve("p").meta.runner == ""  # nothing silently written either


async def test_settings_modal_cancel_leaves_the_picker_alone(tmp_path):
    _prompt_entry(tmp_path, text="{{a}}\n")
    app = tui.MenuApp()
    async with app.run_test(size=(110, 40)) as pilot:
        await pilot.pause()
        screen = await _open_settings(app, pilot)
        select = screen.query_one("#st-runner-select", Select)
        before = _option_count(select)
        await pilot.press("ctrl+n")
        await pilot.pause()
        assert isinstance(app.screen, RunnerAddModal)
        await pilot.press("escape")
        await pilot.pause()
        assert _option_count(select) == before  # a cancelled add adds no option


def test_run_prompt_review_returns_the_apps_result(tmp_path, monkeypatch):
    from skit import tui_add

    src = tmp_path / "h.prompt.md"
    src.write_text("x\n", encoding="utf-8")
    monkeypatch.setattr(tui_add.PromptReviewApp, "run", lambda self: "slug-sentinel")
    assert tui_add.run_prompt_review(src, name="n") == "slug-sentinel"


# --------------------------------------------------------------------------
# the New agent modal
# --------------------------------------------------------------------------


async def test_form_ctrl_n_defines_a_custom_agent_and_runs_with_it(tmp_path, quiet_run):
    _prompt_entry(tmp_path)
    app = tui.MenuApp()
    async with app.run_test(size=(110, 34)) as pilot:
        await pilot.pause()
        app.action_run()
        await pilot.pause()
        screen = app.screen
        assert isinstance(screen, RunFormScreen)
        await pilot.press("ctrl+n")  # advertised on the runner row's chip
        await pilot.pause()
        modal = app.screen
        assert isinstance(modal, RunnerAddModal)
        modal.query_one("#runner-add-name", Input).value = "aider"
        modal.query_one("#runner-add-command", Input).value = "aider --message {{prompt}}"
        modal.action_save_runner()
        await pilot.pause()
        select = screen.query_one("#runner-select", Select)
        assert _value(select) == "aider"  # joined the picker, selected in place
        field = screen.query_one(Input)
        field.value = "x"
        field.focus()  # step off the Select so Enter/submit runs (not re-open the overlay)
        await pilot.pause()
        screen.action_submit()
        await pilot.pause()
    assert quiet_run["runner"] == config.find_prompt_runner("aider")
    assert config.prompt_runners_seeded()  # the seeds materialized alongside


async def test_runner_modal_validation_covers_every_refusal(tmp_path):
    app = tui.MenuApp()
    async with app.run_test(size=(100, 32)) as pilot:
        await pilot.pause()
        app.push_screen(RunnerAddModal())
        await pilot.pause()
        modal = app.screen
        assert isinstance(modal, RunnerAddModal)
        error = modal.query_one("#runner-add-error", Static)
        name_box = modal.query_one("#runner-add-name", Input)
        cmd_box = modal.query_one("#runner-add-command", Input)

        modal.action_save_runner()  # empty name
        assert "name is required" in str(error.render())
        name_box.value = "claude"  # collides with a seed
        cmd_box.value = "claude {{prompt}}"
        modal.action_save_runner()
        assert "already exists" in str(error.render())
        name_box.value = "mycli"
        cmd_box.value = ""  # no command at all
        modal.action_save_runner()
        assert "mycli run {{prompt}}" in str(error.render())
        cmd_box.value = "mycli run"  # no slot
        modal.action_save_runner()
        assert "exactly once" in str(error.render())
        cmd_box.value = "{{prompt}}"  # the slot as the binary
        modal.action_save_runner()
        assert "first word" in str(error.render())
        cmd_box.value = "mycli {{prompt}} {{extra}}"  # a stray hole
        modal.action_save_runner()
        assert "only the {{prompt}} slot" in str(error.render())
        cmd_box.value = "mycli 'run {{prompt}}"  # unbalanced quote
        modal.action_save_runner()
        assert "Unbalanced quotes" in str(error.render())
        modal.action_cancel()
        await pilot.pause()
    assert config.find_prompt_runner("mycli") is None  # nothing was written
