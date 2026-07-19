"""Residual edge-branch coverage for tui.py and tui_form.py.

Sibling to test_tui_cov.py / test_tui_mut.py: those cover the mainline action and
form-rendering paths; this file targets the leftover error paths, empty states, guard
returns, callback branches, and specific key handlers. Every test asserts an observable
behavior (rendered widget content, store/argstate mutations, exit codes, dismiss
results, status text) — never a line executed for its own sake.
"""

from __future__ import annotations

import contextlib
from types import SimpleNamespace

import pytest
from textual.widgets import Checkbox, DataTable, Input, Select, Static

from conftest import footer_text
from skit import argstate, argv_text, config, flows, launcher, store, tui
from skit.langs.python import metawriter
from skit.params import ParamDecl
from skit.tui_form import (
    EnvPickerModal,
    FieldRow,
    PresetNameModal,
    RunFormScreen,
    TokenMenuModal,
    _degraded_notice,
)


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


@contextlib.contextmanager
def _noop_suspend():
    yield


@pytest.fixture
def quiet_run(monkeypatch):
    """Neutralize the terminal-ownership pieces of _execute; capture the launch.

    Pins after_run=stay (the workbench loop these tests assert on); the "exit"
    default has dedicated tests in test_tui_mut.py."""
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
    ):
        calls["extra"] = list(extra_args or [])
        calls["values"] = dict(values or {})
        calls["override"] = script_override
        return calls.get("code", 0)

    monkeypatch.setattr(launcher, "run_entry", fake_run)
    monkeypatch.setattr(tui.MenuApp, "suspend", lambda self: _noop_suspend())
    return calls


MANAGED = 'CITY = "Taipei"\nprint(CITY)\n'


def _managed_entry(tmp_path, name="j", default=None):
    text = metawriter.write_params(
        MANAGED, [ParamDecl(name="CITY", binding="const", type="str", default=default)]
    )
    return store.add_python(_py(tmp_path, text, f"{name}.py"), name=name)


ARGPARSE = (
    "import argparse\nap = argparse.ArgumentParser()\n"
    "ap.add_argument('-o', '--output', required=True, help='output path')\n"
    "ap.add_argument('--fast', action='store_true')\n"
    "ap.add_argument('--mode', choices=['a', 'b'], default='a')\n"
    "ap.parse_args()\n"
)


def _argparse_entry(tmp_path):
    return store.add_python(_py(tmp_path, ARGPARSE, "cli.py"), name="cli")


ARGPARSE_ALL_OPTIONAL = (
    "import argparse\nap = argparse.ArgumentParser()\n"
    "ap.add_argument('--width', type=int, default=800)\n"
    "ap.add_argument('--fast', action='store_true')\n"
    "ap.add_argument('--mode', choices=['a', 'b'], default='a')\n"
    "ap.parse_args()\n"
)


def _static_text(app, selector) -> str:
    return str(app.query_one(selector, Static).render())


# ---------------------------------------------------------------------------
# HelpScreen dismiss
# ---------------------------------------------------------------------------


async def test_help_overlay_escape_dismisses_back_to_library(tmp_path):
    """? opens the reminder overlay; Esc (its only advertised key) closes it and returns
    to the Library, not a dead end."""
    store.add_python(_py(tmp_path, "print(1)\n"), name="a")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        await pilot.press("question_mark")
        assert isinstance(app.screen, tui.HelpScreen)
        await pilot.press("escape")
        await pilot.pause()
        assert not isinstance(app.screen, tui.HelpScreen)
        assert len(app.screen_stack) == 1


# ---------------------------------------------------------------------------
# detail-pane rendering edges
# ---------------------------------------------------------------------------


async def test_detail_blank_when_filter_matches_nothing(tmp_path):
    """Entries exist but the search filters them all out: the detail pane blanks (there's
    nothing selected to describe), rather than falling back to the first-run welcome."""
    store.add_python(_py(tmp_path, "print(1)\n"), name="alpha")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        app.query_one("#search", Input).value = "zzznomatch"
        await pilot.pause()
        assert app.query_one(DataTable).row_count == 0
        assert _static_text(app, "#detail-body") == ""  # blank, not the welcome text


async def test_detail_reference_entry_shows_linked_source(tmp_path):
    """A python reference entry's detail names the linked original (the ↗ counterpart of the
    copy-mode "kept by skit" promise)."""
    p = _py(tmp_path, "print(1)\n", "orig.py")
    store.add_python(p, name="linked", mode="reference")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        await pilot.pause()
        detail = _static_text(app, "#detail-body")
        assert "Linked to the original" in detail
        assert "orig.py" in detail


# ---------------------------------------------------------------------------
# row-selected → run
# ---------------------------------------------------------------------------


async def test_row_selection_triggers_run(tmp_path, monkeypatch):
    """Selecting a row (Enter/double-click on the table) is a Run: DataTable.RowSelected
    routes straight to action_run."""
    store.add_python(_py(tmp_path, "print(1)\n"), name="a")
    fired: list[str] = []
    monkeypatch.setattr(tui.MenuApp, "action_run", lambda self: fired.append("run"))
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        await pilot.pause()
        app.query_one(DataTable).action_select_cursor()
        await pilot.pause()
        assert fired == ["run"]


# ---------------------------------------------------------------------------
# search-box arrow forwarding + quit paths
# ---------------------------------------------------------------------------


async def test_arrows_drive_the_table_while_search_is_focused(tmp_path):
    """Up/Down keep browsing the result list even while the caret sits in the search box —
    the table cursor moves though the Input holds focus."""
    store.add_python(_py(tmp_path, "print(1)\n", "a.py"), name="alpha")
    store.add_python(_py(tmp_path, "print(2)\n", "b.py"), name="beta")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        app.action_focus_search()
        await pilot.pause()
        assert app.focused is app.query_one("#search", Input)
        table = app.query_one(DataTable)
        assert table.cursor_row == 0
        await pilot.press("down")
        assert table.cursor_row == 1
        await pilot.press("up")
        assert table.cursor_row == 0


async def test_escape_on_the_table_quits_cleanly(tmp_path):
    """Esc with the table focused (not the search box) exits the Library with code 0."""
    store.add_python(_py(tmp_path, "print(1)\n"), name="a")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        await pilot.pause()
        app.query_one(DataTable).focus()
        await pilot.pause()
        app.action_back_or_quit()
        assert app.return_value == 0


async def test_double_ctrl_c_quits_but_a_single_press_only_warns(tmp_path):
    """First Ctrl+C arms the quit (a notification), a second within the window exits — the
    accidental-quit guard from the footer's "double Ctrl+C"."""
    store.add_python(_py(tmp_path, "print(1)\n"), name="a")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        await pilot.pause()
        app.action_ctrl_c_quit()  # first press: warn only
        assert app.return_value is None
        app.action_ctrl_c_quit()  # second press within the window: exit 0
        assert app.return_value == 0


# ---------------------------------------------------------------------------
# guard returns: pushed screen + empty library
# ---------------------------------------------------------------------------


async def test_library_actions_are_inert_while_a_screen_is_open(tmp_path):
    """Library actions act only on the Library: called directly while a modal is up (keys
    that bubble out of the pushed screen), run/edit/remove must do nothing."""
    store.add_python(_py(tmp_path, "print(1)\n"), name="a")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        app.action_help()
        await pilot.pause()
        assert isinstance(app.screen, tui.HelpScreen)
        app.action_run()
        app.action_edit()
        app.action_remove()
        await pilot.pause()
        assert isinstance(app.screen, tui.HelpScreen)  # nothing opened or ran over it
        assert len(app.screen_stack) == 2


async def test_actions_no_op_on_an_empty_library(tmp_path):
    """With no entry selected (empty library), every entry-scoped action returns quietly —
    no crash, no screen pushed."""
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        await pilot.pause()
        assert app._selected() is None
        app.action_run()
        app.action_rerun()
        app.action_edit()
        app.action_remove()
        app.action_settings()
        await pilot.pause()
        assert len(app.screen_stack) == 1  # nothing was pushed


# ---------------------------------------------------------------------------
# run / rerun error + fallback paths
# ---------------------------------------------------------------------------


async def test_run_reports_preflight_error_in_the_status(tmp_path):
    """A python entry whose script vanished fails preflight before any suspend; the launch
    error surfaces in the status line rather than crashing the TUI."""
    entry = store.add_python(_py(tmp_path, "print(1)\n"), name="ghost")
    entry.script_path.unlink()
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        await pilot.pause()
        app.action_run()
        await pilot.pause()
        status = _static_text(app, "#status")
        assert status.startswith("Error:")
        assert len(app.screen_stack) == 1  # no form opened


async def test_rerun_reports_preflight_error_in_the_status(tmp_path):
    """r honors the same preflight gate as Enter: a vanished target reports an error, never
    a phantom rerun."""
    entry = store.add_python(_py(tmp_path, "print(1)\n"), name="ghost")
    argstate.record_run(entry.slug, 0, at="2026-07-09T00:00:00+00:00")
    entry.script_path.unlink()
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        await pilot.pause()
        app.action_rerun()
        await pilot.pause()
        assert _static_text(app, "#status").startswith("Error:")


async def test_rerun_falls_back_to_the_form_when_last_values_no_longer_validate(
    tmp_path, quiet_run
):
    """The last run's values no longer satisfy the form (a required field with no saved
    value): r must open the form rather than assemble a broken command."""
    entry = _argparse_entry(tmp_path)  # --output is required, no default
    argstate.record_run(entry.slug, 0, at="2026-07-09T00:00:00+00:00")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        await pilot.pause()
        app.action_rerun()
        await pilot.pause()
        assert isinstance(app.screen, RunFormScreen)  # fell back to the form
        assert "values" not in quiet_run  # nothing launched around the hole


async def test_execute_reports_assembly_error_in_the_status(tmp_path):
    """An extra-argument token that can't resolve (an unset env var) is a hard, named
    error at assembly — the status shows it and the run is abandoned before suspending."""
    entry = store.add_python(_py(tmp_path, "print(1)\n"), name="j")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        await pilot.pause()
        app._execute(entry, flows.FormPlan(source="none"), {}, ["{env:SKIT_DEFINITELY_UNSET_XYZ}"])
        await pilot.pause()
        status = _static_text(app, "#status")
        assert status.startswith("Error:")
        assert "isn't set" in status


# ---------------------------------------------------------------------------
# edit error path
# ---------------------------------------------------------------------------


@pytest.mark.parametrize("error_type", [tui.editor.EditorError, tui.editor.EditedSourceError])
async def test_edit_errors_return_to_the_tui_without_reading_stdin(
    tmp_path, monkeypatch, error_type
):
    """Both editor failure modes resume the workbench and use its persistent status.

    The edit action is also a clickable footer path, so an error must not introduce a
    hidden keyboard-only ``input()`` gate before Textual regains mouse ownership.
    """
    store.add_python(_py(tmp_path, "print(1)\n"), name="j")
    stdin_reads: list[tuple[object, ...]] = []

    def boom(path, *, kind):
        raise error_type("editor exploded")

    def forbid_stdin(*args):
        stdin_reads.append(args)
        raise AssertionError("Library edit errors must not wait on stdin")

    monkeypatch.setattr(tui.editor, "open_entry_in_editor", boom)
    monkeypatch.setattr(tui.MenuApp, "suspend", lambda self: _noop_suspend())
    monkeypatch.setattr("builtins.input", forbid_stdin)
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        await pilot.pause()
        app.action_edit()
        await pilot.pause()
        assert stdin_reads == []
        assert len(app.screen_stack) == 1
        assert _static_text(app, "#status") == "Error: editor exploded"


# ---------------------------------------------------------------------------
# add / preferences / health callbacks + slug selection
# ---------------------------------------------------------------------------


async def test_add_callback_selects_the_new_entry(tmp_path):
    """When the add flow returns a slug, the Library reloads, moves the cursor onto the new
    entry, and confirms it was added."""
    store.add_python(_py(tmp_path, "print(1)\n", "a.py"), name="a")
    target = store.add_python(_py(tmp_path, "print(2)\n", "b.py"), name="b")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        await pilot.pause()
        app.action_add()
        await pilot.pause()
        app.screen.dismiss(target.slug)
        await pilot.pause()
        selected = app._selected()
        assert selected is not None
        assert selected.slug == target.slug
        assert _static_text(app, "#status") == "✓ added"


async def test_health_callback_jumps_to_the_returned_entry(tmp_path):
    """Closing the health screen on an entry jumps the Library cursor to it."""
    target = store.add_python(_py(tmp_path, "print(1)\n", "a.py"), name="a")
    store.add_python(_py(tmp_path, "print(2)\n", "b.py"), name="b")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        await pilot.pause()
        app.action_health()
        await pilot.pause()
        app.screen.dismiss(target.slug)
        await pilot.pause()
        selected = app._selected()
        assert selected is not None
        assert selected.slug == target.slug


async def test_health_callback_without_a_slug_leaves_the_cursor(tmp_path):
    """Closing the health screen with no selection reloads the Library but doesn't move the
    cursor (the _jump callback's no-slug branch)."""
    store.add_python(_py(tmp_path, "print(1)\n", "a.py"), name="a")
    store.add_python(_py(tmp_path, "print(2)\n", "b.py"), name="b")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        await pilot.pause()
        before = app.query_one(DataTable).cursor_row
        app.action_health()
        await pilot.pause()
        app.screen.dismiss(None)
        await pilot.pause()
        assert app.query_one(DataTable).cursor_row == before  # no jump


async def test_preferences_callback_retranslates_chrome(tmp_path, monkeypatch):
    """Applying a language change in Preferences re-translates the chrome on the spot: the
    _applied callback re-runs _retranslate_chrome, so the column headers switch language."""
    from skit import i18n

    store.add_python(_py(tmp_path, "print(1)\n"), name="a")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        await pilot.pause()
        app.action_preferences()
        await pilot.pause()
        i18n.set_language("zh_TW")
        app.screen.dismiss(None)
        await pilot.pause()
        headers = [str(c.label) for c in app.query_one(DataTable).ordered_columns]
        assert "名稱" in headers  # retranslated by the callback


async def test_select_slug_leaves_cursor_when_slug_is_absent(tmp_path):
    """_select_slug is a no-op when the slug isn't in the visible list (it just refreshes),
    rather than moving the cursor somewhere arbitrary."""
    store.add_python(_py(tmp_path, "print(1)\n"), name="only")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        await pilot.pause()
        table = app.query_one(DataTable)
        before = table.cursor_row
        app._select_slug("no-such-slug")
        await pilot.pause()
        assert table.cursor_row == before  # unchanged


# ---------------------------------------------------------------------------
# detail-pane pin survives a resize
# ---------------------------------------------------------------------------


async def test_pinned_detail_pane_ignores_resize_autocollapse(tmp_path):
    """Once Tab pins the detail pane, width-driven auto-collapse stands down: a later resize
    to a wide terminal does not re-show a pane the user chose to hide."""
    store.add_python(_py(tmp_path, "print(1)\n"), name="a")
    app = tui.MenuApp()
    async with app.run_test(size=(120, 24)) as pilot:
        await pilot.pause()
        detail = app.query_one("#detail")
        assert detail.display  # wide → shown by auto
        await pilot.press("tab")  # pin it hidden
        await pilot.pause()
        assert not detail.display
        await pilot.resize_terminal(120, 30)  # still wide; auto would re-show it
        assert not detail.display  # but the pin holds


# ---------------------------------------------------------------------------
# tui_form: degraded notice + field help lines
# ---------------------------------------------------------------------------


def test_generic_degraded_notice_points_at_the_extra_field():
    """A non-subparser parser degrade (skit couldn't read the declarations) still names the
    extra-arguments escape hatch."""
    notice = _degraded_notice("unreadable")
    assert "couldn't read this script's argument declarations" in notice
    assert "extra-arguments field" in notice


async def test_field_help_lines_for_degraded_and_env_secret(tmp_path):
    """A degraded field advertises "leave empty for the default"; a secret field with an env
    source advertises the fallback variable — both as muted help lines under the control."""
    entry = store.add_python(_py(tmp_path, "print(1)\n"), name="j")
    plan = flows.FormPlan(
        source="argparse",
        fields=[
            flows.FormField(key="deg", label="Deg", source="flag", degraded=True),
            flows.FormField(
                key="TOKEN", label="Token", source="flag", secret=True, env_source="MY_ENV"
            ),
        ],
    )
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        await pilot.pause()
        screen = RunFormScreen(entry, plan, {})
        app.push_screen(screen)
        await pilot.pause()
        helps = "".join(str(w.render()) for w in screen.query(".field-help"))
        assert "Leave empty to use the script's own default." in helps
        assert "Leave empty to read it from the environment variable MY_ENV." in helps


# ---------------------------------------------------------------------------
# tui_form: FieldRow.set_value + live glob feedback
# ---------------------------------------------------------------------------


async def test_choice_set_value_ignores_a_value_outside_the_choices(tmp_path, quiet_run):
    """set_value on a choice field is a no-op for a value that isn't one of the choices —
    the current selection stays put instead of clearing to nothing."""
    _argparse_entry(tmp_path)
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        app.action_run()
        await pilot.pause()
        screen = app.screen
        assert isinstance(screen, RunFormScreen)
        mode = next(r for r in screen.query(FieldRow) if r.field.key == "mode")
        assert mode.value == "a"  # argparse default
        mode.set_value("not-a-choice")
        assert mode.value == "a"  # unchanged


async def test_bool_field_reads_checked_state_through_truthy(tmp_path, quiet_run):
    """A bool field reads its checked state through flows.truthy in the widget lane too —
    so a stored "on"/"y" checks the box (the same spelling assembly fires --fast on), never
    unchecked-while-the-run-passes-the-flag. set_value drives the shared rule."""
    _argparse_entry(tmp_path)
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        app.action_run()
        await pilot.pause()
        screen = app.screen
        assert isinstance(screen, RunFormScreen)
        fast = next(r for r in screen.query(FieldRow) if r.field.key == "fast")
        box = fast.query_one(Checkbox)
        fast.set_value("on")
        assert box.value is True  # "on" checks the box
        fast.set_value("y")
        assert box.value is True  # "y" too
        fast.set_value("off")
        assert box.value is False  # and "off" clears it


async def test_live_preview_reports_glob_match_count(tmp_path, quiet_run):
    """Typing a glob into a free-text field shows a live match count. Uses an absolute pattern
    (glob ignores the cwd for it) so the count is deterministic WITHOUT chdir'ing — chdir breaks
    mutmut's stats collection (see the note in test_cli.py)."""
    (tmp_path / "a.png").touch()
    _argparse_entry(tmp_path)
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        app.action_run()
        await pilot.pause()
        screen = app.screen
        assert isinstance(screen, RunFormScreen)
        row = next(r for r in screen.query(FieldRow) if r.field.key == "output")
        row.query_one(Input).value = f"{tmp_path.as_posix()}/*.png"
        await pilot.pause()
        preview = row.query_one(".field-preview", Static)
        assert preview.display
        assert "matches 1 file" in str(preview.render())


# ---------------------------------------------------------------------------
# tui_form: PresetNameModal branches
# ---------------------------------------------------------------------------


async def test_save_preset_captures_set_fixed_fields_from_prefill(tmp_path, quiet_run):
    """The CLI opens a REDUCED form when --set fixed some fields: those values ride in the
    prefill with no composed row. A preset saved here must capture them too (`--save-preset`
    on the identical run does — one feature, one rule), never silently drop the pinned value."""
    entry = _argparse_entry(tmp_path)
    full = flows.plan_for_entry(entry)
    # Reproduce `--set output=fixed.png`: drop output's row, pin its value in the prefill.
    reduced = flows.FormPlan(
        source=full.source,
        fields=[f for f in full.fields if f.key != "output"],
        text=full.text,
    )
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        screen = RunFormScreen(entry, plan=reduced, prefill={"output": "fixed.png"})
        app.push_screen(screen)
        await pilot.pause()
        assert not any(r.field.key == "output" for r in screen.query(FieldRow))  # no row for it
        screen.action_save_preset()
        await pilot.pause()
        modal = app.screen
        assert isinstance(modal, PresetNameModal)
        modal.query_one(Input).value = "pinned"
        modal.action_save_name()
        await pilot.pause()
    saved = argstate.load_state(entry.slug)["presets"]["pinned"]
    assert saved["output"] == "fixed.png"  # the --set-fixed value was captured from prefill


async def test_preset_modal_overwrite_hint_and_click_save(tmp_path, quiet_run):
    """Typing an existing preset name warns of the overwrite; a blank name is a no-op; the
    click-twin (action_save_name) saves under the typed name."""
    entry = _argparse_entry(tmp_path)
    argstate.save_preset(entry.slug, "web", {"output": "x"})
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        app.action_run()
        await pilot.pause()
        screen = app.screen
        assert isinstance(screen, RunFormScreen)
        screen.action_save_preset()
        await pilot.pause()
        modal = app.screen
        assert isinstance(modal, PresetNameModal)
        box = modal.query_one(Input)
        box.value = "web"  # collides with the existing preset
        await pilot.pause()
        hint = str(modal.query_one("#preset-hint", Static).render())
        assert "overwrites the existing preset" in hint
        box.value = "   "  # blank → neither Enter nor the click-twin saves
        await pilot.pause()
        await pilot.press("enter")
        await pilot.pause()
        modal.action_save_name()  # click-twin, still blank
        await pilot.pause()
        assert isinstance(app.screen, PresetNameModal)  # still here, nothing saved
        box.value = "clicked"
        modal.action_save_name()  # the mouse twin of Enter, now with a name
        await pilot.pause()
        assert "clicked" in argstate.load_state(entry.slug)["presets"]


async def test_preset_modal_cancel_saves_nothing(tmp_path, quiet_run):
    """Esc on the preset-name modal saves no preset (the save_preset callback ignores a
    None result)."""
    entry = _argparse_entry(tmp_path)
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        app.action_run()
        await pilot.pause()
        screen = app.screen
        assert isinstance(screen, RunFormScreen)
        screen.action_save_preset()
        await pilot.pause()
        assert isinstance(app.screen, PresetNameModal)
        await pilot.press("escape")
        await pilot.pause()
        assert not isinstance(app.screen, PresetNameModal)
        assert argstate.load_state(entry.slug)["presets"] == {}  # nothing saved


def _preset_values(select: Select[str]) -> list[str]:
    # allow_blank=False means no NULL/NoSelection sentinel rides in _options — the
    # isinstance filter is just the type-narrowing ty needs (it never drops a real row).
    return [value for _, value in select._options if isinstance(value, str)]


async def test_form_save_preset_field_less_notifies_and_opens_no_modal(
    tmp_path, quiet_run, monkeypatch
):
    """A field-less form composes NO preset row at all: the row only ever
    existed to teach "fill the form and press Ctrl+S", the exact action Ctrl+S refuses here.
    Ctrl+S still notifies the no-fields sentence (the same one the CLI uses) and opens no modal."""
    store.add_command("echo hi", name="noargs")
    entry = store.resolve("noargs")
    plan = flows.plan_for_entry(entry)
    assert plan.fields == []  # truly field-less
    notes: list[str] = []
    monkeypatch.setattr(RunFormScreen, "notify", lambda self, msg, **kw: notes.append(msg))
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        screen = RunFormScreen(entry, plan, {})
        app.push_screen(screen)
        await pilot.pause()
        # The whole preset row is gone — neither the dropdown nor the empty-state hint composes.
        assert not screen.query("#preset-row")
        assert not screen.query("#preset-select")
        assert not screen.query("#preset-empty")
        # …and the footer must not advertise Ctrl+S either — the visible key hint IS the
        # mouse's click path, so it can't teach the exact action Ctrl+S refuses here.
        keys = footer_text(screen.query_one("#form-keys", Static))
        assert "Save as preset" not in keys
        assert "Ctrl+S" not in keys
        screen.action_save_preset()
        await pilot.pause()
        assert app.screen is screen  # NO modal opened
    assert any("has no form fields, so there's nothing to save." in n for n in notes)
    assert argstate.load_state(entry.slug)["presets"] == {}  # created nothing


async def test_form_footer_advertises_ctrl_s_only_when_fielded(tmp_path, quiet_run):
    """The twin of the field-less footer: a fielded run form KEEPS the Ctrl+S 'Save as
    preset' pill (there is a form to save), so the mouse always has a path to the same
    action the key runs."""
    _argparse_entry(tmp_path)  # has fields
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        app.action_run()
        await pilot.pause()
        screen = app.screen
        assert isinstance(screen, RunFormScreen)
        keys = footer_text(screen.query_one("#form-keys", Static))
        assert "Save as preset" in keys  # fielded → the pill is present
        assert "Ctrl+S" in keys


async def test_form_save_preset_from_empty_state_mounts_a_select(tmp_path, quiet_run):
    """The first preset save replaces the "none yet — press Ctrl+S" hint with a real
    dropdown, selected on the just-saved preset."""
    entry = _argparse_entry(tmp_path)
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        screen = RunFormScreen(entry, plan=flows.plan_for_entry(entry), prefill={})
        app.push_screen(screen)
        await pilot.pause()
        assert screen.query("#preset-empty")  # the teaching hint is showing
        assert not screen.query("#preset-select")
        screen.action_save_preset()
        await pilot.pause()
        modal = app.screen
        assert isinstance(modal, PresetNameModal)
        modal.query_one(Input).value = "quick"
        modal.action_save_name()
        await pilot.pause()
        assert app.screen is screen  # back on the form
        assert not screen.query("#preset-empty")  # the hint is gone
        select = screen.query_one("#preset-select", Select)
        assert select.value == "quick"  # the just-saved preset is selected
        assert "quick" in _preset_values(select)
    assert "quick" in argstate.load_state(entry.slug)["presets"]


async def test_form_save_preset_existing_select_gains_the_name(tmp_path, quiet_run):
    """When a dropdown already exists, a new save adds the name and selects it — the row
    never contradicts the user's own just-made save."""
    entry = _argparse_entry(tmp_path)
    argstate.save_preset(entry.slug, "web", {"output": "x"})  # a preset already exists
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        screen = RunFormScreen(entry, plan=flows.plan_for_entry(entry), prefill={})
        app.push_screen(screen)
        await pilot.pause()
        select = screen.query_one("#preset-select", Select)  # dropdown already present
        assert _preset_values(select) == ["", "web"]
        screen.action_save_preset()
        await pilot.pause()
        modal = app.screen
        assert isinstance(modal, PresetNameModal)
        modal.query_one(Input).value = "quick"
        modal.action_save_name()
        await pilot.pause()
        select = screen.query_one("#preset-select", Select)
        assert select.value == "quick"  # selects the new one
        # options are the blank row + the presets sorted by name (quick < web)
        assert _preset_values(select) == ["", "quick", "web"]  # old preset kept, new added
    assert set(argstate.load_state(entry.slug)["presets"]) == {"web", "quick"}


async def test_form_save_preset_does_not_resurrect_a_cleared_field(tmp_path, quiet_run):
    """Clearing a prefilled field, then Ctrl+S + naming the preset, must not resurrect
    the field. The refresh swallows EVERY Changed until the just-saved name lands —
    set_options first resets the picker to "last values", and that intermediate
    Changed("") is exactly how a single-value one-shot let this bug ship three times
    (it re-applied the prefill overlay to every field). Hand-picks still apply."""
    entry = _argparse_entry(tmp_path)
    argstate.save_preset(entry.slug, "web", {"output": "web.png"})  # a #preset-select exists
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        screen = RunFormScreen(
            entry, plan=flows.plan_for_entry(entry), prefill={"output": "orig.png"}
        )
        app.push_screen(screen)
        await pilot.pause()
        rows = {r.field.key: r for r in screen.query(FieldRow)}
        assert rows["output"].query_one(Input).value == "orig.png"  # prefilled
        rows["output"].set_value("")  # the user clears it
        screen.action_save_preset()
        await pilot.pause()
        modal = app.screen
        assert isinstance(modal, PresetNameModal)
        modal.query_one(Input).value = "clean"
        modal.action_save_name()
        await pilot.pause()
        await pilot.pause()
        assert "output" not in argstate.load_state(entry.slug)["presets"]["clean"]
        select = screen.query_one("#preset-select", Select)
        assert select.value == "clean"
        # THE headline observable: the cleared field STAYS cleared.
        assert rows["output"].query_one(Input).value == ""
        # Hand-picks still apply, including of the just-saved name later.
        select.value = "web"
        await pilot.pause()
        assert rows["output"].query_one(Input).value == "web.png"
        select.value = "clean"
        await pilot.pause()
        assert rows["output"].query_one(Input).value == "orig.png"  # prefill overlay


async def test_form_save_preset_while_another_preset_is_selected(tmp_path, quiet_run):
    """The journey that exercises the one-shot token: hand-pick a preset, EDIT a
    field, save under a new name — the fields must keep the user's edits (set_options'
    intermediate blank reset must not re-apply the prefill overlay), and the picker
    must land on the new name."""
    entry = _argparse_entry(tmp_path)
    argstate.save_preset(entry.slug, "web", {"output": "web.png"})
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        screen = RunFormScreen(
            entry, plan=flows.plan_for_entry(entry), prefill={"output": "orig.png"}
        )
        app.push_screen(screen)
        await pilot.pause()
        select = screen.query_one("#preset-select", Select)
        select.value = "web"  # hand-pick: applies web.png
        await pilot.pause()
        rows = {r.field.key: r for r in screen.query(FieldRow)}
        assert rows["output"].query_one(Input).value == "web.png"
        rows["output"].set_value("final.png")  # the user edits on top
        screen.action_save_preset()
        await pilot.pause()
        modal = app.screen
        assert isinstance(modal, PresetNameModal)
        modal.query_one(Input).value = "clean"
        modal.action_save_name()
        await pilot.pause()
        await pilot.pause()
        assert argstate.load_state(entry.slug)["presets"]["clean"]["output"] == "final.png"
        assert select.value == "clean"
        # The field keeps the user's edit — not stomped back to the prefill.
        assert rows["output"].query_one(Input).value == "final.png"


async def test_form_overwrite_selected_preset_keeps_a_cleared_field(tmp_path, quiet_run):
    """Overwrite variant: hand-pick 'web', clear the field, Ctrl+S under the SAME name
    — the cleared field stays cleared and the stored preset drops the value."""
    entry = _argparse_entry(tmp_path)
    argstate.save_preset(entry.slug, "web", {"output": "web.png"})
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        screen = RunFormScreen(
            entry, plan=flows.plan_for_entry(entry), prefill={"output": "orig.png"}
        )
        app.push_screen(screen)
        await pilot.pause()
        select = screen.query_one("#preset-select", Select)
        select.value = "web"
        await pilot.pause()
        rows = {r.field.key: r for r in screen.query(FieldRow)}
        rows["output"].set_value("")
        screen.action_save_preset()
        await pilot.pause()
        modal = app.screen
        assert isinstance(modal, PresetNameModal)
        modal.query_one(Input).value = "web"
        modal.action_save_name()
        await pilot.pause()
        await pilot.pause()
        assert "output" not in argstate.load_state(entry.slug)["presets"]["web"]
        assert rows["output"].query_one(Input).value == ""  # not resurrected


# ---------------------------------------------------------------------------
# tui_form: token menu cancel + env picker typed/cancel
# ---------------------------------------------------------------------------


async def test_token_menu_cancel_inserts_nothing(tmp_path, quiet_run):
    """Cancelling the ▾ insert menu leaves the target field untouched (the insert callback
    ignores a None token)."""
    _argparse_entry(tmp_path)
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        app.action_run()
        await pilot.pause()
        screen = app.screen
        assert isinstance(screen, RunFormScreen)
        row = next(r for r in screen.query(FieldRow) if r.field.key == "output")
        row.query_one(Input).value = "keep"
        screen.action_insert_token("output")
        await pilot.pause()
        assert isinstance(app.screen, TokenMenuModal)
        await pilot.press("escape")
        await pilot.pause()
        assert not isinstance(app.screen, TokenMenuModal)
        assert row.query_one(Input).value == "keep"  # unchanged


async def test_env_picker_typed_name_is_accepted(tmp_path):
    """A full env-variable name typed and submitted is accepted even if unset yet — it
    resolves at run time — and dismisses as a {env:NAME} token."""
    store.add_python(_py(tmp_path, "print(1)\n"), name="j")
    app = tui.MenuApp()
    result: list[str | None] = []
    async with app.run_test() as pilot:
        await pilot.pause()
        app.push_screen(EnvPickerModal(), result.append)
        await pilot.pause()
        app.screen.query_one(Input).value = "MY_TYPED_VAR"
        await pilot.pause()
        await pilot.press("enter")
        await pilot.pause()
        assert result == ["{env:MY_TYPED_VAR}"]


async def test_env_picker_rejects_a_non_identifier_name(tmp_path):
    """A submitted name that isn't a valid variable identifier is refused — the picker stays
    open rather than dismissing an unusable token."""
    store.add_python(_py(tmp_path, "print(1)\n"), name="j")
    app = tui.MenuApp()
    result: list[str | None] = []
    async with app.run_test() as pilot:
        await pilot.pause()
        app.push_screen(EnvPickerModal(), result.append)
        await pilot.pause()
        app.screen.query_one(Input).value = "not an identifier"
        await pilot.pause()
        await pilot.press("enter")
        await pilot.pause()
        assert isinstance(app.screen, EnvPickerModal)  # still open
        assert result == []  # nothing dismissed


async def test_env_picker_cancel_returns_none(tmp_path):
    """Esc on the env picker dismisses with no token."""
    store.add_python(_py(tmp_path, "print(1)\n"), name="j")
    app = tui.MenuApp()
    result: list[str | None] = []
    async with app.run_test() as pilot:
        await pilot.pause()
        app.push_screen(EnvPickerModal(), result.append)
        await pilot.pause()
        await pilot.press("escape")
        await pilot.pause()
        assert result == [None]


# ---------------------------------------------------------------------------
# tui_form: insert-token guards (no text field / non-field input)
# ---------------------------------------------------------------------------


async def test_insert_token_ignores_a_non_text_focus(tmp_path, quiet_run):
    """Ctrl+T with a non-Input focused (e.g. a checkbox) opens no menu — there's no text
    field to insert into."""
    store.add_python(_py(tmp_path, ARGPARSE_ALL_OPTIONAL, "opt.py"), name="opt")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        app.action_run()
        await pilot.pause()
        screen = app.screen
        assert isinstance(screen, RunFormScreen)
        screen.query(Checkbox).first().focus()
        await pilot.pause()
        screen.action_insert_token()  # no key, focus is the checkbox
        await pilot.pause()
        assert app.screen is screen  # no menu opened


async def test_insert_token_ignores_an_input_outside_any_field(tmp_path, quiet_run):
    """Ctrl+T from an Input that isn't part of a FieldRow finds no row and opens no menu."""
    _argparse_entry(tmp_path)
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        app.action_run()
        await pilot.pause()
        screen = app.screen
        assert isinstance(screen, RunFormScreen)
        stray = Input(id="stray")
        await screen.mount(stray)
        stray.focus()
        await pilot.pause()
        screen.action_insert_token()
        await pilot.pause()
        assert app.screen is screen  # no menu opened


async def test_insert_token_ignores_a_stale_field_click_action(tmp_path, quiet_run):
    """A ▾ chip carries its field key.  If an already-queued click is delivered after
    that row was replaced, its stale key is a safe no-op: no modal and no mutation of
    the still-visible field."""
    _argparse_entry(tmp_path)
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        app.action_run()
        await pilot.pause()
        screen = app.screen
        assert isinstance(screen, RunFormScreen)
        output = next(r for r in screen.query(FieldRow) if r.field.key == "output")
        output_input = output.query_one(Input)
        output_input.value = "keep.png"
        output_input.focus()
        await pilot.pause()

        screen.action_insert_token("row-that-no-longer-exists")
        await pilot.pause()

        assert app.screen is screen
        assert output_input.value == "keep.png"
        assert screen.focused is output_input


# ---------------------------------------------------------------------------
# tui_form: RunFormScreen include_extra + collect edges
# ---------------------------------------------------------------------------


async def test_inline_form_without_extra_field_collects_no_extra(tmp_path):
    """The inline (CLI) frame hides the extra-args row — argv already owns passthrough
    there — so the form has no __extra_args__ field and collect() returns an empty tail."""
    entry = _argparse_entry(tmp_path)
    plan = flows.plan_for_entry(entry)
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        await pilot.pause()
        screen = RunFormScreen(entry, plan, {}, include_extra=False)
        app.push_screen(screen)
        await pilot.pause()
        keys = [r.field.key for r in screen.query(FieldRow)]
        assert "__extra_args__" not in keys
        _values, extra = screen.collect()
        assert extra == []


async def test_extra_argument_labels_name_the_actual_receiver(tmp_path):
    command = store.add_command("echo ready", name="cmd")
    prompt_path = tmp_path / "review.prompt.md"
    prompt_path.write_text("Review this\n", encoding="utf-8")
    prompt = store.add_prompt(prompt_path, name="review")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        for entry, expected in (
            (command, "Extra command arguments"),
            (prompt, "Extra agent arguments"),
        ):
            screen = RunFormScreen(entry, flows.plan_for_entry(entry), {})
            app.push_screen(screen)
            await pilot.pause()
            extra_row = next(
                row for row in screen.query(FieldRow) if row.field.key == "__extra_args__"
            )
            assert extra_row.field.label == expected
            app.pop_screen()
            await pilot.pause()


async def test_collect_keeps_unbalanced_extra_as_one_argument(tmp_path, quiet_run):
    """Extra args with an unbalanced quote can't be shlex-split; collect falls back to
    passing the raw text as a single argument rather than dropping it."""
    _argparse_entry(tmp_path)
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        app.action_run()
        await pilot.pause()
        screen = app.screen
        assert isinstance(screen, RunFormScreen)
        extra_row = next(r for r in screen.query(FieldRow) if r.field.key == "__extra_args__")
        extra_row.query_one(Input).value = '"unclosed'
        _values, extra = screen.collect()
        assert extra == ['"unclosed']


async def test_extra_args_windows_paths_roundtrip_through_the_form(tmp_path, monkeypatch):
    entry = _argparse_entry(tmp_path)
    expected = [r"C:\Program Files\tool\input.txt", "two words"]
    argstate.save_last(entry.slug, extra_args=expected)
    monkeypatch.setattr(argv_text, "sys", SimpleNamespace(platform="win32"))
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        screen = RunFormScreen(entry, flows.plan_for_entry(entry), {})
        app.push_screen(screen)
        await pilot.pause()
        extra_row = next(r for r in screen.query(FieldRow) if r.field.key == "__extra_args__")
        _values, extra = screen.collect()
        assert extra == expected  # saved argv joined for editing, then split back byte-for-byte
        extra_row.query_one(Input).value = r"C:\tools\input.txt --fast"
        _values, extra = screen.collect()
        assert extra == [r"C:\tools\input.txt", "--fast"]


# ---------------------------------------------------------------------------
# tui_form: drift banner inside the form
# ---------------------------------------------------------------------------


async def test_form_shows_drift_banner_when_the_plan_drifted(tmp_path, quiet_run):
    """When a managed script drifted from its definitions, opening the form (Enter) shows
    the drift as a banner, naming the dropped definition."""
    drifted = metawriter.write_params(
        "CITY = 'x'\nprint(CITY)\n",
        [
            ParamDecl(name="CITY", binding="const", type="str"),
            ParamDecl(name="GONE", binding="const", type="str"),
        ],
    )
    store.add_python(_py(tmp_path, drifted, "drifty.py"), name="drifty")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        app.action_run()
        await pilot.pause()
        screen = app.screen
        assert isinstance(screen, RunFormScreen)
        banner = str(screen.query_one("#drift-banner", Static).render())
        assert "GONE" in banner
