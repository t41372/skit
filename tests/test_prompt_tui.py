"""The prompt kind's TUI surfaces: the run form's runner picker (mouse AND keyboard),
the Library run/rerun guards, the add lane, and the settings screen's prompt sections.
"""

from __future__ import annotations

import contextlib

import pytest
from textual.widgets import (
    Checkbox,
    DataTable,
    Input,
    OptionList,
    RadioSet,
    Select,
    SelectionList,
    Static,
)

from skit import argstate, config, flows, i18n, launcher, paths, store, tui, tui_footer
from skit.tui_add import AddSourceScreen, KindPickModal, PromptReviewScreen
from skit.tui_form import RunFormScreen
from skit.tui_prompt import PromptCandidatePickerModal
from skit.tui_runner import RunnerAddModal
from skit.tui_settings import DiscardChangesModal, ScriptSettingsScreen


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


async def _click_chip(pilot, widget: Static, label: str) -> None:
    """Click the linked span rather than an arbitrary blank cell in its Static."""
    # Inline chips live in the form's ordinary wheel-scrollable body. Bring this one
    # into the viewport exactly as a mouse user scrolling to it would.
    widget.scroll_visible(animate=False)
    await pilot.pause()
    plain = str(widget.render()).replace(tui_footer.GLUE, " ")
    position = plain.find(label)
    assert position >= 0, plain
    await pilot.click(widget, offset=(position + 1, 0))
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
        prepared=None,
    ):
        calls["values"] = dict(values or {})
        calls["runner"] = runner
        calls["prepared"] = prepared
        return calls.get("code", 0)

    monkeypatch.setattr(launcher, "run_entry", fake_run)
    # The fixture replaces the actual spawn, so make preflight agree that the
    # synthetic runner binaries are launchable.  Individual refusal tests override
    # this seam with the missing binary they exercise.
    monkeypatch.setattr("skit.langs.launch._which", lambda name: f"/bin/{name}")
    monkeypatch.setattr(tui.MenuApp, "suspend", lambda self: _noop_suspend())
    return calls


def _prompt_entry(tmp_path, text="Do {{a}}\n", name="p", pin=""):
    src = tmp_path / f"{name}.prompt.md"
    src.write_text(text, encoding="utf-8")
    entry = store.add_prompt(src, name=name)
    if pin:
        entry = store.write_prompt_runner(entry.slug, pin)
    return entry


async def test_prompt_only_library_uses_entry_taxonomy_everywhere(tmp_path):
    entry = _prompt_entry(tmp_path, text="Review this\n")
    store.update_description(entry.slug, "")
    app = tui.MenuApp()
    async with app.run_test(size=(110, 36)) as pilot:
        await pilot.pause()
        assert app.query_one(DataTable).border_title == "Library"
        assert "1/1 entry" in str(app.query_one("#status", Static).render())
        local = str(app.query_one("#keys-local", Static).render()).replace(tui_footer.GLUE, " ")
        global_keys = str(app.query_one("#keys-global", Static).render()).replace(
            tui_footer.GLUE, " "
        )
        detail = str(app.query_one("#detail-body", Static).render())
        assert "Entry settings" in local
        assert "Edit source" in local
        assert "Add entry" in global_keys
        assert "add one in Entry settings" in detail

        await pilot.press("p")
        await pilot.pause()
        screen = app.screen
        assert isinstance(screen, ScriptSettingsScreen)
        assert screen.query_one("#st-body").border_title == "Entry settings · p"


@pytest.mark.parametrize(
    ("locale", "library", "script_library"),
    [("zh-CN", "工具库", "脚本库"), ("zh-TW", "工具庫", "腳本庫")],
)
async def test_prompt_only_chinese_library_stays_entry_neutral(
    tmp_path, locale, library, script_library
):
    i18n.init(locale)
    _prompt_entry(tmp_path, text="Review this\n")
    app = tui.MenuApp()
    async with app.run_test(size=(110, 36)) as pilot:
        await pilot.pause()
        title = str(app.query_one(DataTable).border_title)
        assert title == library
        assert script_library not in title

        await pilot.press("p")
        await pilot.pause()
        screen = app.screen
        assert isinstance(screen, ScriptSettingsScreen)
        placeholder = screen.query_one("#st-desc", Input).placeholder or ""
        assert library in placeholder
        assert script_library not in placeholder


# --------------------------------------------------------------------------
# run form: the runner picker row
# --------------------------------------------------------------------------


async def test_form_picker_defaults_to_the_pin_and_submits_it(tmp_path, quiet_run):
    _prompt_entry(tmp_path, pin="codex")
    argstate.save_last_runner("opencode")
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
    assert argstate.load_last_runner() == "opencode"  # untouched pin is only a default


async def test_form_picker_move_away_then_back_to_pin_is_still_remembered(tmp_path, quiet_run):
    _prompt_entry(tmp_path, pin="codex")
    argstate.save_last_runner("amp")
    app = tui.MenuApp()
    async with app.run_test(size=(100, 32)) as pilot:
        await pilot.pause()
        app.action_run()
        await pilot.pause()
        screen = app.screen
        assert isinstance(screen, RunFormScreen)
        select = screen.query_one("#runner-select", Select)
        assert _value(select) == "codex"
        select.value = "claude"
        await pilot.pause()
        select.value = "codex"
        await pilot.pause()
        screen.query_one(Input).value = "x"
        screen.action_submit()
        await pilot.pause()
    assert quiet_run["runner"] == config.find_prompt_runner("codex")
    assert argstate.load_last_runner() == "codex"


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


async def test_unicode_placeholder_is_a_working_tui_field(tmp_path, quiet_run):
    _prompt_entry(tmp_path, text="审查 {{目标}}\n")
    app = tui.MenuApp()
    async with app.run_test(size=(100, 32)) as pilot:
        await pilot.pause()
        app.action_run()
        await pilot.pause()
        screen = app.screen
        assert isinstance(screen, RunFormScreen)
        field = screen.query_one(Input)
        field.value = "src/app.py"
        screen.action_submit()
        await pilot.pause()
    assert quiet_run["values"] == {"目标": "src/app.py"}


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


async def test_stale_pin_cannot_block_run_form_override(tmp_path, quiet_run, monkeypatch):
    _prompt_entry(tmp_path, pin="removed")
    working = config.PromptRunner("working", ("working-bin", "{{prompt}}"))
    config.save_prompt_runners([working])
    monkeypatch.setattr("skit.langs.launch._which", lambda name: f"/bin/{name}")
    app = tui.MenuApp()
    async with app.run_test(size=(100, 32)) as pilot:
        await pilot.pause()
        app.action_run()
        await pilot.pause()

        # The stale pin used to fail preflight before this picker could open.  With
        # only the configured replacement available, the visible form resolves it.
        screen = app.screen
        assert isinstance(screen, RunFormScreen)
        assert _value(screen.query_one("#runner-select", Select)) == "working"
        screen.query_one(Input).value = "hello"
        screen.action_submit()
        await pilot.pause()
    assert quiet_run["runner"] == working


async def test_missing_pinned_binary_cannot_block_a_different_pick(
    tmp_path, quiet_run, monkeypatch
):
    broken = config.PromptRunner("broken", ("missing-agent", "{{prompt}}"))
    working = config.PromptRunner("working", ("working-agent", "{{prompt}}"))
    config.save_prompt_runners([broken, working])
    _prompt_entry(tmp_path, pin="broken")
    monkeypatch.setattr(
        "skit.langs.launch._which",
        lambda name: None if name == "missing-agent" else f"/bin/{name}",
    )
    app = tui.MenuApp()
    async with app.run_test(size=(100, 32)) as pilot:
        await pilot.pause()
        app.action_run()
        await pilot.pause()

        # The broken pin remains the honest prefill, but it no longer prevents the
        # user from selecting the installed runner for this one launch.
        screen = app.screen
        assert isinstance(screen, RunFormScreen)
        select = screen.query_one("#runner-select", Select)
        assert _value(select) == "broken"
        select.value = "working"
        screen.query_one(Input).value = "hello"
        screen.action_submit()
        await pilot.pause()
    assert quiet_run["runner"] == working


async def test_selected_prompt_runner_preflight_failure_returns_to_library(
    tmp_path, quiet_run, monkeypatch
):
    """The prompt runner cannot be checked until the form resolves the user's pick.  If
    that selected agent is unavailable, submitting the visible form returns to the
    library with the actionable error and never hands the terminal to a child process."""
    _prompt_entry(tmp_path)

    def refuse(entry, invoke_cwd=None, *, runner=None):
        assert entry.meta.kind == "prompt"
        assert runner is not None
        raise launcher.LaunchError("agent [missing] cannot run")

    monkeypatch.setattr(launcher, "preflight", refuse)
    app = tui.MenuApp()
    async with app.run_test(size=(100, 32)) as pilot:
        await pilot.pause()
        app.action_run()
        await pilot.pause()
        screen = app.screen
        assert isinstance(screen, RunFormScreen)
        screen.query_one(Input).value = "hello"
        await pilot.press("ctrl+r")  # the advertised keyboard path submits the form
        await pilot.pause()

        assert app.screen is app.screen_stack[0]
        status = str(app.query_one("#status", Static).render())
        assert status == "Error: agent [missing] cannot run"
    assert "runner" not in quiet_run  # preflight refused before launcher.run_entry


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
    '(no longer configured)' — the same honesty Entry settings gives (two surfaces, one
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


async def test_tui_add_bare_md_asks_before_becoming_a_prompt(tmp_path):
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
        picker = app.screen
        assert isinstance(picker, KindPickModal)
        options = picker.query_one(OptionList)
        prompt_index = next(
            i for i in range(options.option_count) if options.get_option_at_index(i).id == "prompt"
        )
        options.highlighted = prompt_index
        options.action_select()
        await pilot.pause()
        review = app.screen
        assert isinstance(review, PromptReviewScreen)
        review.action_accept()
        await pilot.pause()
    assert store.resolve("notes").meta.kind == "prompt"


async def test_tui_add_bare_md_kind_ask_can_cancel_without_adding(tmp_path):
    src = tmp_path / "notes.md"
    src.write_text("ordinary project notes\n", encoding="utf-8")
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
        assert isinstance(app.screen, KindPickModal)
        await pilot.press("escape")  # the modal's advertised Cancel key
        await pilot.pause()
        assert app.screen is source
    assert store.list_entries() == []


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
    argstate.save_last_runner("opencode")
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
    assert argstate.load_last_runner() == "opencode"

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
    assert argstate.load_last_runner() == "opencode"


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
    argstate.save_last_runner("amp")
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
    assert argstate.load_last_runner() == "amp"  # defining a settings pin is not a run pick


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
    argstate.save_last_runner("amp")
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
    assert argstate.load_last_runner() == "amp"


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

    # Off state hides the precomposed rows. Turning it on reveals them immediately,
    # without the undocumented Save → reopen round trip that used to be required.
    app = tui.MenuApp()
    async with app.run_test(size=(110, 40)) as pilot:
        await pilot.pause()
        screen = await _open_settings(app, pilot)
        from skit.tui_settings import DeclParamRow

        fields = screen.query_one("#st-prompt-fields")
        assert fields.display is False
        assert len(screen.query(DeclParamRow)) == 1  # present but not shown through its parent
        toggle = screen.query_one("#st-interpolate", Checkbox)
        assert toggle.value is False
        toggle.value = True
        await pilot.pause()
        assert fields.display is True
        assert screen.query_one(DeclParamRow).decl.name == "a"
        assert screen.query("#st-add-param")
        screen.action_save()
        await pilot.pause()
    on = store.resolve("p")
    assert on.meta.interpolate is True
    assert on.meta.params == ["a"]  # untouched by the off/on round trip


async def test_settings_off_to_on_can_choose_first_parameters_in_the_same_save(tmp_path):
    entry = _prompt_entry(tmp_path, text="{{a}} {{b}}\n")
    store.write_prompt_managed(entry.slug, [])
    store.write_prompt_interpolate(entry.slug, False)
    app = tui.MenuApp()
    async with app.run_test(size=(110, 40)) as pilot:
        await pilot.pause()
        screen = await _open_settings(app, pilot)
        assert screen.query_one("#st-prompt-fields").display is False

        screen.query_one("#st-interpolate", Checkbox).value = True
        await pilot.pause()
        assert screen.query_one("#st-prompt-fields").display is True
        screen.query_one("#st-prompt-new-1", Checkbox).value = True
        screen.action_save()
        await pilot.pause()

    saved = store.resolve("p")
    assert saved.meta.interpolate is True
    assert saved.meta.params == ["b"]


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


async def test_settings_candidate_picker_reaches_a_hidden_name_and_waits_for_outer_save(tmp_path):
    from skit.langs.prompt.analyzer import LIST_PREVIEW_LIMIT

    names = [f"u{i}" for i in range(LIST_PREVIEW_LIMIT + 9)]
    entry = _prompt_entry(tmp_path, text="{{a}} " + " ".join(f"{{{{{n}}}}}" for n in names))
    store.write_prompt_managed(entry.slug, ["a"])
    app = tui.MenuApp()
    async with app.run_test(size=(110, 40)) as pilot:
        await pilot.pause()
        screen = await _open_settings(app, pilot)
        await pilot.press("ctrl+o")
        await pilot.pause()
        modal = app.screen
        assert isinstance(modal, PromptCandidatePickerModal)

        modal.query_one("#prompt-candidate-filter", Input).value = names[-1]
        await pilot.pause()
        await pilot.press("enter", "space", "ctrl+s")
        await pilot.pause()
        assert app.screen is screen
        assert store.resolve("p").meta.params == ["a"]  # picker Done is not a write

        screen.action_save()
        await pilot.pause()

    assert store.resolve("p").meta.params == ["a", names[-1]]


async def test_settings_candidate_picker_selection_is_discardable(tmp_path):
    from skit.langs.prompt.analyzer import LIST_PREVIEW_LIMIT

    names = [f"u{i}" for i in range(LIST_PREVIEW_LIMIT + 2)]
    entry = _prompt_entry(tmp_path, text="{{a}} " + " ".join(f"{{{{{n}}}}}" for n in names))
    store.write_prompt_managed(entry.slug, ["a"])
    app = tui.MenuApp()
    async with app.run_test(size=(110, 40)) as pilot:
        await pilot.pause()
        screen = await _open_settings(app, pilot)
        await _click_chip(
            pilot,
            screen.query_one("#st-choose-candidates", Static),
            "Choose variables",
        )
        await pilot.pause()
        modal = app.screen
        assert isinstance(modal, PromptCandidatePickerModal)
        modal.query_one("#prompt-candidate-all", Checkbox).value = True
        await pilot.pause()
        modal.action_done()
        await pilot.pause()

        screen.action_close()
        await pilot.pause()
        discard = app.screen
        assert isinstance(discard, DiscardChangesModal)
        discard.action_discard()
        await pilot.pause()

    assert store.resolve("p").meta.params == ["a"]


async def test_settings_candidate_picker_cancel_and_unchanged_done_are_noops(tmp_path):
    from skit.langs.prompt.analyzer import LIST_PREVIEW_LIMIT

    names = [f"u{i}" for i in range(LIST_PREVIEW_LIMIT + 2)]
    entry = _prompt_entry(tmp_path, text=" ".join(f"{{{{{n}}}}}" for n in names))
    store.write_prompt_managed(entry.slug, [])
    app = tui.MenuApp()
    async with app.run_test(size=(110, 40)) as pilot:
        await pilot.pause()
        screen = await _open_settings(app, pilot)

        await pilot.press("ctrl+o", "escape")
        await pilot.pause()
        assert app.screen is screen
        assert screen._pending_prompt_candidates == set()
        assert screen._dirty is False

        await pilot.press("ctrl+o", "ctrl+s")
        await pilot.pause()
        assert app.screen is screen
        assert screen._pending_prompt_candidates == set()
        assert screen._dirty is False
        await pilot.press("escape")
        await pilot.pause()


async def test_settings_candidate_picker_tolerates_preview_recompose(tmp_path):
    """A queued Ctrl+O/Done can straddle a responsive recompose.  Missing old preview
    widgets are skipped by name while the full modal selection still survives."""
    from skit.langs.prompt.analyzer import LIST_PREVIEW_LIMIT

    names = [f"u{i}" for i in range(LIST_PREVIEW_LIMIT + 2)]
    entry = _prompt_entry(tmp_path, text=" ".join(f"{{{{{n}}}}}" for n in names))
    store.write_prompt_managed(entry.slug, [])
    app = tui.MenuApp()
    async with app.run_test(size=(110, 40)) as pilot:
        await pilot.pause()
        screen = await _open_settings(app, pilot)
        await screen.query_one("#st-prompt-new-0", Checkbox).remove()
        await pilot.press("ctrl+o")
        await pilot.pause()
        modal = app.screen
        assert isinstance(modal, PromptCandidatePickerModal)

        await screen.query_one("#st-prompt-new-1", Checkbox).remove()
        await pilot.click("#prompt-candidate-all")
        await pilot.press("ctrl+s")
        await pilot.pause()
        assert app.screen is screen
        assert screen._pending_prompt_candidates == set(names)


async def test_settings_choose_variables_key_is_harmless_when_off_or_short(tmp_path):
    entry = _prompt_entry(tmp_path, text="{{a}} {{b}}")
    store.write_prompt_managed(entry.slug, ["a"])
    app = tui.MenuApp()
    async with app.run_test(size=(110, 40)) as pilot:
        await pilot.pause()
        screen = await _open_settings(app, pilot)
        insertion = screen.query_one("#st-interpolate", Checkbox)
        insertion.value = False
        await pilot.pause()
        await pilot.press("ctrl+o")
        await pilot.pause()
        assert app.screen is screen

        insertion.value = True
        await pilot.pause()
        await pilot.press("ctrl+o")
        await pilot.pause()
        assert app.screen is screen
        await pilot.press("escape")
        await pilot.pause()


async def test_settings_surfaces_prompt_read_failure_from_open_race(tmp_path, monkeypatch):
    entry = _prompt_entry(tmp_path)

    def fail_read(_path):
        raise PermissionError("permission changed")

    monkeypatch.setattr("skit.tui_settings.prompt_text.read", fail_read)
    screen = ScriptSettingsScreen(entry)
    app = tui.MenuApp()
    async with app.run_test(size=(110, 40)) as pilot:
        await pilot.pause()
        app.push_screen(screen)
        await pilot.pause()
        error = str(screen.query_one("#st-prompt-text-error", Static).render())
        assert "permission changed" in error
        await pilot.press("escape")
        await pilot.pause()


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


async def test_review_candidate_picker_keyboard_reaches_a_hidden_name(tmp_path):
    from skit.langs.prompt.analyzer import AUTO_MANAGE_LIMIT

    names = [f"h{i}" for i in range(AUTO_MANAGE_LIMIT + 4)]
    src = tmp_path / "big.prompt.md"
    src.write_text(" ".join(f"{{{{{name}}}}}" for name in names), encoding="utf-8")
    app = tui.MenuApp()
    async with app.run_test(size=(100, 40)) as pilot:
        await pilot.pause()
        app.push_screen(PromptReviewScreen(src))
        await pilot.pause()
        await pilot.press("ctrl+o")  # the advertised full-list keyboard path
        await pilot.pause()
        modal = app.screen
        assert isinstance(modal, PromptCandidatePickerModal)
        modal.query_one("#prompt-candidate-filter", Input).value = names[-1]
        await pilot.pause()
        listing = modal.query_one("#prompt-candidate-list", SelectionList)
        assert listing.option_count == 1
        await pilot.press("enter", "space", "ctrl+s")
        await pilot.pause()

        review = app.screen
        assert isinstance(review, PromptReviewScreen)
        review.action_accept()
        await pilot.pause()

    assert store.resolve("big").meta.params == [names[-1]]


async def test_review_candidate_picker_select_all_and_done_are_mouse_operable(tmp_path):
    from skit.langs.prompt.analyzer import AUTO_MANAGE_LIMIT

    names = [f"h{i}" for i in range(AUTO_MANAGE_LIMIT + 1)]
    src = tmp_path / "all.prompt.md"
    src.write_text(" ".join(f"{{{{{name}}}}}" for name in names), encoding="utf-8")
    app = tui.MenuApp()
    async with app.run_test(size=(100, 40)) as pilot:
        await pilot.pause()
        app.push_screen(PromptReviewScreen(src))
        await pilot.pause()
        review = app.screen
        assert isinstance(review, PromptReviewScreen)
        await _click_chip(
            pilot,
            review.query_one("#pv-choose-candidates", Static),
            "Choose variables",
        )
        modal = app.screen
        assert isinstance(modal, PromptCandidatePickerModal)

        await pilot.click("#prompt-candidate-all")
        await pilot.pause()
        footer = modal.query_one("#prompt-candidate-keys", Static)
        plain = str(footer.render()).replace("⠀", " ")
        done_at = plain.find("Done")
        assert done_at >= 0
        await pilot.click(footer, offset=(done_at + 1, 0))
        await pilot.pause()
        assert app.screen is review
        review.action_accept()
        await pilot.pause()

    assert store.resolve("all").meta.params == names


async def test_review_candidate_picker_keeps_search_and_footer_usable_on_tiny_screen(tmp_path):
    from skit.langs.prompt.analyzer import AUTO_MANAGE_LIMIT

    names = [f"h{i}" for i in range(AUTO_MANAGE_LIMIT + 1)]
    src = tmp_path / "tiny.prompt.md"
    src.write_text(" ".join(f"{{{{{name}}}}}" for name in names), encoding="utf-8")
    app = tui.MenuApp()
    async with app.run_test(size=(42, 10)) as pilot:
        await pilot.pause()
        app.push_screen(PromptReviewScreen(src))
        await pilot.pause()
        await pilot.press("ctrl+o")
        await pilot.pause()
        modal = app.screen
        assert isinstance(modal, PromptCandidatePickerModal)
        footer = modal.query_one("#prompt-candidate-keys", Static)
        assert footer.region.height >= 1
        assert footer.region.bottom <= modal.region.bottom

        modal.query_one("#prompt-candidate-filter", Input).value = names[-1]
        await pilot.pause()
        assert modal.query_one("#prompt-candidate-list", SelectionList).option_count == 1
        await pilot.press("enter", "space", "ctrl+s")
        await pilot.pause()
        review = app.screen
        assert isinstance(review, PromptReviewScreen)
        review.action_cancel()
        await pilot.pause()


async def test_review_candidate_picker_empty_search_and_cancel_are_keyboard_operable(tmp_path):
    from skit.langs.prompt.analyzer import AUTO_MANAGE_LIMIT, LIST_PREVIEW_LIMIT

    names = [f"h{i}" for i in range(AUTO_MANAGE_LIMIT + 1)]
    src = tmp_path / "search.prompt.md"
    src.write_text(" ".join(f"{{{{{name}}}}}" for name in names), encoding="utf-8")
    app = tui.MenuApp()
    async with app.run_test(size=(100, 40)) as pilot:
        await pilot.pause()
        app.push_screen(PromptReviewScreen(src))
        await pilot.pause()
        review = app.screen
        assert isinstance(review, PromptReviewScreen)
        await pilot.press("ctrl+o")
        await pilot.pause()
        modal = app.screen
        assert isinstance(modal, PromptCandidatePickerModal)

        await pilot.press("z", "z", "z", "z")
        await pilot.pause()
        listing = modal.query_one("#prompt-candidate-list", SelectionList)
        assert listing.option_count == 0
        await pilot.press("enter")
        await pilot.pause()
        assert modal.query_one("#prompt-candidate-filter", Input).has_focus
        await pilot.press("escape")
        await pilot.pause()
        assert app.screen is review
        assert not any(
            review.query_one(f"#pv-hole-{i}", Checkbox).value for i in range(LIST_PREVIEW_LIMIT)
        )
        await pilot.press("escape")
        await pilot.pause()


async def test_review_candidate_picker_tolerates_preview_recompose(tmp_path):
    """The modal owns the full selection while the capped preview can recompose behind
    it; Done must not crash merely because one old preview checkbox was unmounted."""
    from skit.langs.prompt.analyzer import AUTO_MANAGE_LIMIT

    names = [f"h{i}" for i in range(AUTO_MANAGE_LIMIT + 1)]
    src = tmp_path / "recompose-picker.prompt.md"
    src.write_text(" ".join(f"{{{{{name}}}}}" for name in names), encoding="utf-8")
    app = tui.MenuApp()
    async with app.run_test(size=(100, 40)) as pilot:
        await pilot.pause()
        review = PromptReviewScreen(src)
        app.push_screen(review)
        await pilot.pause()
        await pilot.press("ctrl+o")
        await pilot.pause()
        modal = app.screen
        assert isinstance(modal, PromptCandidatePickerModal)

        await review.query_one("#pv-hole-0", Checkbox).remove()
        await pilot.click("#prompt-candidate-all")
        await pilot.press("ctrl+s")
        await pilot.pause()
        assert app.screen is review
        assert review._tick_overrides == dict.fromkeys(names, True)
        await pilot.press("escape")
        await pilot.pause()


async def test_review_choose_variables_key_is_harmless_for_a_short_prompt(tmp_path):
    src = tmp_path / "short.prompt.md"
    src.write_text("{{a}} {{b}}", encoding="utf-8")
    app = tui.MenuApp()
    async with app.run_test(size=(100, 40)) as pilot:
        await pilot.pause()
        review = PromptReviewScreen(src)
        app.push_screen(review)
        await pilot.pause()
        await pilot.press("ctrl+o")
        await pilot.pause()
        assert app.screen is review
        await pilot.press("escape")
        await pilot.pause()


async def test_prompt_draft_with_invalid_utf8_reaches_strict_review(tmp_path, monkeypatch):
    monkeypatch.setattr(tui.MenuApp, "suspend", lambda self: _noop_suspend())
    monkeypatch.setattr(
        "skit.tui_add.editor.open_in_editor",
        lambda draft: draft.write_bytes(b"draft:\xff\n"),
    )
    app = tui.MenuApp()
    async with app.run_test(size=(100, 40)) as pilot:
        await pilot.pause()
        source = AddSourceScreen()
        app.push_screen(source)
        await pilot.pause()
        await pilot.press("ctrl+p")
        await pilot.pause()
        review = app.screen
        assert isinstance(review, PromptReviewScreen)
        error = str(review.query_one("#pv-text-error", Static).render())
        assert "offset 6" in error
        assert "�" not in error
        await pilot.press("escape")
        await pilot.pause()
        assert app.screen is source

    kept = list(paths.drafts_dir().glob("skit-new-*.prompt.md"))
    assert len(kept) == 1
    assert kept[0].read_bytes() == b"draft:\xff\n"


async def test_prompt_review_surfaces_initial_and_post_editor_os_errors(tmp_path, monkeypatch):
    missing = tmp_path / "vanished.prompt.md"
    initial = PromptReviewScreen(missing)
    app = tui.MenuApp()
    async with app.run_test(size=(100, 40)) as pilot:
        await pilot.pause()
        app.push_screen(initial)
        await pilot.pause()
        assert "vanished.prompt.md" in str(initial.query_one("#pv-text-error", Static).render())
        await pilot.press("escape")
        await pilot.pause()

        source = tmp_path / "edited.prompt.md"
        source.write_text("{{a}}", encoding="utf-8")
        review = PromptReviewScreen(source)
        app.push_screen(review)
        await pilot.pause()
        monkeypatch.setattr(tui.MenuApp, "suspend", lambda self: _noop_suspend())
        monkeypatch.setattr("skit.tui_add.editor.open_in_editor", lambda path: path.unlink())
        await _click_chip(pilot, review.query_one("#pv-keys", Static), "Edit prompt")
        await pilot.pause()
        error = str(review.query_one("#pv-text-error", Static).render())
        assert "edited.prompt.md" in error
        assert "No such file" in error
        await pilot.press("escape")
        await pilot.pause()


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
        review.action_accept()
        await pilot.pause()
    assert store.resolve("l").meta.runner == "codex"
    assert argstate.load_last_runner() == "amp"  # untouched add-time pin is not a pick


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
        assert any(note.message == "no editor" for note in app._notifications)
        review.action_cancel()
        await pilot.pause()


async def test_review_ctrl_e_keeps_placeholder_ticks_by_name_across_flood_transitions(
    tmp_path, monkeypatch
):
    from skit.langs.prompt.analyzer import AUTO_MANAGE_LIMIT

    src = tmp_path / "ticks.prompt.md"
    src.write_text("{{keep_off}} {{removed}}\n", encoding="utf-8")
    flood_names = ["flood_on", "keep_off"] + [f"new_{i}" for i in range(AUTO_MANAGE_LIMIT - 1)]
    edits = [
        " ".join(f"{{{{{name}}}}}" for name in flood_names) + "\n",
        "{{fresh_below}} {{flood_on}} {{keep_off}}\n",
    ]

    def edit(path):
        path.write_text(edits.pop(0), encoding="utf-8")

    monkeypatch.setattr(tui.MenuApp, "suspend", lambda self: _noop_suspend())
    monkeypatch.setattr("skit.tui_add.editor.open_in_editor", edit)

    def box_for(review: PromptReviewScreen, name: str) -> Checkbox:
        index = review._shown_names.index(name)
        return review.query_one(f"#pv-hole-{index}", Checkbox)

    app = tui.MenuApp()
    async with app.run_test(size=(110, 40)) as pilot:
        await pilot.pause()
        app.push_screen(PromptReviewScreen(src))
        await pilot.pause()
        review = app.screen
        assert isinstance(review, PromptReviewScreen)

        box_for(review, "keep_off").value = False
        review.action_edit_source()
        await pilot.pause()

        assert "removed" not in review._shown_names
        assert box_for(review, "keep_off").value is False  # decision followed the name
        assert box_for(review, "flood_on").value is False  # genuinely new flood holes default off
        box_for(review, "flood_on").value = True

        review.action_edit_source()
        await pilot.pause()

        assert review._shown_names == ["fresh_below", "flood_on", "keep_off"]
        assert box_for(review, "fresh_below").value is True  # new non-flood holes default on
        assert box_for(review, "flood_on").value is True  # explicit flood choice survived
        assert box_for(review, "keep_off").value is False  # reordered survivor stayed off
        review.action_cancel()
        await pilot.pause()


async def test_review_edit_tolerates_a_placeholder_checkbox_unmounted_during_recompose(
    tmp_path, monkeypatch
):
    """A stale footer/key action can arrive while the placeholder list is being
    recomposed.  A name whose old checkbox is already unmounted is skipped; the edit
    still completes and the newly scanned placeholder gets its normal default."""
    src = tmp_path / "recompose.prompt.md"
    src.write_text("{{old}}\n", encoding="utf-8")
    monkeypatch.setattr(tui.MenuApp, "suspend", lambda self: _noop_suspend())
    monkeypatch.setattr(
        "skit.tui_add.editor.open_in_editor",
        lambda path: path.write_text("{{new}}\n", encoding="utf-8"),
    )
    app = tui.MenuApp()
    async with app.run_test(size=(110, 40)) as pilot:
        await pilot.pause()
        app.push_screen(PromptReviewScreen(src))
        await pilot.pause()
        review = app.screen
        assert isinstance(review, PromptReviewScreen)
        old_box = review.query_one("#pv-hole-0", Checkbox)
        await old_box.remove()
        assert review._shown_names == ["old"]
        assert not review.query("#pv-hole-0")

        review.query_one("#pv-interpolate", Checkbox).focus()
        await pilot.press("ctrl+e")
        await pilot.pause()

        assert app.screen is review
        assert review._shown_names == ["new"]
        assert review.query_one("#pv-hole-0", Checkbox).value is True
        assert review._tick_overrides == {}
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


# --------------------------------------------------------------------------
# Library edit → offer to manage the placeholders a body edit introduced
# --------------------------------------------------------------------------


@contextlib.contextmanager
def _noop_suspend():
    yield


def _editor_appending(text: str):
    """A fake $EDITOR: append `text` to the stored body and report success."""

    def fake(path, *, kind):
        path.write_text(path.read_text(encoding="utf-8") + text, encoding="utf-8")
        return 0

    return fake


async def test_library_edit_prompt_offers_picker_and_manages_the_selection(tmp_path, monkeypatch):
    entry = _prompt_entry(tmp_path, text="Say hello.\n", name="greet")
    monkeypatch.setattr(tui.editor, "open_entry_in_editor", _editor_appending("\n{{username}}\n"))
    monkeypatch.setattr(tui.MenuApp, "suspend", lambda self: _noop_suspend())
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        await pilot.pause()
        app.action_edit()
        await pilot.pause()
        modal = app.screen
        assert isinstance(modal, PromptCandidatePickerModal)
        # The single new variable is pre-ticked (not flooded), so Done alone manages it —
        # the exact {{username}}-typed-into-the-body flow the fix is for.
        modal.action_done()
        await pilot.pause()
        assert store.resolve(entry.slug).meta.params == ["username"]
        assert "Now managed: username" in str(app.query_one("#status", Static).render())


async def test_library_edit_prompt_picker_cancel_leaves_it_literal(tmp_path, monkeypatch):
    entry = _prompt_entry(tmp_path, text="Say hello.\n", name="greet")
    monkeypatch.setattr(tui.editor, "open_entry_in_editor", _editor_appending("\n{{username}}\n"))
    monkeypatch.setattr(tui.MenuApp, "suspend", lambda self: _noop_suspend())
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        await pilot.pause()
        app.action_edit()
        await pilot.pause()
        modal = app.screen
        assert isinstance(modal, PromptCandidatePickerModal)
        modal.action_cancel()
        await pilot.pause()
        assert store.resolve(entry.slug).meta.params is None  # unmanaged
        assert "Edited greet." in str(app.query_one("#status", Static).render())


async def test_library_edit_prompt_picker_done_with_no_ticks_manages_nothing(tmp_path, monkeypatch):
    entry = _prompt_entry(tmp_path, text="Say hello.\n", name="greet")
    monkeypatch.setattr(tui.editor, "open_entry_in_editor", _editor_appending("\n{{username}}\n"))
    monkeypatch.setattr(tui.MenuApp, "suspend", lambda self: _noop_suspend())
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        await pilot.pause()
        app.action_edit()
        await pilot.pause()
        modal = app.screen
        assert isinstance(modal, PromptCandidatePickerModal)
        modal.query_one("#prompt-candidate-all", Checkbox).value = False  # untick everything
        await pilot.pause()
        modal.action_done()
        await pilot.pause()
        assert store.resolve(entry.slug).meta.params is None
        assert "Edited greet." in str(app.query_one("#status", Static).render())


async def test_library_edit_prompt_preserves_existing_managed(tmp_path, monkeypatch):
    entry = _prompt_entry(tmp_path, text="{{kept}}\n", name="greet")
    store.write_prompt_managed(entry.slug, ["kept"])
    monkeypatch.setattr(tui.editor, "open_entry_in_editor", _editor_appending("\n{{added}}\n"))
    monkeypatch.setattr(tui.MenuApp, "suspend", lambda self: _noop_suspend())
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        await pilot.pause()
        app.action_edit()
        await pilot.pause()
        modal = app.screen
        assert isinstance(modal, PromptCandidatePickerModal)
        modal.query_one("#prompt-candidate-all", Checkbox).value = True
        await pilot.pause()
        modal.action_done()
        await pilot.pause()
        assert store.resolve(entry.slug).meta.params == ["kept", "added"]


async def test_library_edit_prompt_no_new_placeholder_shows_no_picker(tmp_path, monkeypatch):
    entry = _prompt_entry(tmp_path, text="{{a}}\n", name="greet")
    store.write_prompt_managed(entry.slug, ["a"])
    monkeypatch.setattr(tui.editor, "open_entry_in_editor", _editor_appending("\nmore prose\n"))
    monkeypatch.setattr(tui.MenuApp, "suspend", lambda self: _noop_suspend())
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        await pilot.pause()
        app.action_edit()
        await pilot.pause()
        assert len(app.screen_stack) == 1  # no picker
        assert "Edited greet." in str(app.query_one("#status", Static).render())
        assert store.resolve(entry.slug).meta.params == ["a"]


async def test_library_edit_non_prompt_never_offers_the_picker(tmp_path, monkeypatch):
    script = tmp_path / "job.py"
    script.write_text("print(1)\n", encoding="utf-8")
    store.add_python(script, name="job")
    monkeypatch.setattr(tui.editor, "open_entry_in_editor", lambda p, *, kind: 0)
    monkeypatch.setattr(tui.MenuApp, "suspend", lambda self: _noop_suspend())
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        await pilot.pause()
        app.action_edit()
        await pilot.pause()
        assert len(app.screen_stack) == 1
        assert "Edited job." in str(app.query_one("#status", Static).render())
