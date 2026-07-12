"""Exact-behavior tests for the Library's action paths and the run form screen.

Targets the presentation glue's observable contracts: the remove modal's reassurance
copy, the contextual rerun guard, the execute path's transparency/recording, and the
RunFormScreen's collect/validate/preset mechanics.
"""

from __future__ import annotations

import contextlib

import pytest
from textual.widgets import Checkbox, DataTable, Input, Static

from conftest import click_label, footer_text
from skit import argstate, config, flows, launcher, metawriter, store, tui
from skit.metawriter import ParamSpec
from skit.tui_form import FieldRow, RunFormScreen


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

    Also pins after_run=stay: these tests exercise the workbench loop (run, land back
    in the Library, assert its state), while the out-of-box default is "exit". The
    exit default has its own dedicated tests."""
    config.save_after_run("stay")
    calls: dict[str, object] = {}

    def fake_run(entry, extra_args=None, *, values=None, invoke_cwd=None, script_override=None):
        calls["extra"] = list(extra_args or [])
        calls["values"] = dict(values or {})
        calls["override"] = script_override
        return calls.get("code", 0)

    monkeypatch.setattr(launcher, "run_entry", fake_run)
    monkeypatch.setattr(tui.MenuApp, "suspend", lambda self: _noop_suspend())
    monkeypatch.setattr("builtins.input", lambda *a: "")
    return calls


MANAGED = 'CITY = "Taipei"\nprint(CITY)\n'


def _managed_entry(tmp_path, name="j", default=None):
    text = metawriter.write_params(
        MANAGED, [ParamSpec(name="CITY", kind="const", type="str", default=default)]
    )
    return store.add_python(_py(tmp_path, text, f"{name}.py"), name=name)


# ---------------------------------------------------------------------------
# remove modal
# ---------------------------------------------------------------------------


async def test_remove_modal_carries_the_a5_reassurance(tmp_path):
    store.add_python(_py(tmp_path, "print(1)\n"), name="a")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        app.action_remove()
        await pilot.pause()
        body = "".join(str(w.render()) for w in app.screen.query(Static))
        assert "Your original file will not be deleted." in body
        await pilot.press("escape")
        assert store.list_entries()  # kept


async def test_remove_modal_command_entry_omits_file_reassurance(tmp_path):
    store.add_command("echo hi", name="c")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        app.action_remove()
        await pilot.pause()
        body = "".join(str(w.render()) for w in app.screen.query(Static))
        assert "original file" not in body  # a command has no file; the line would be noise


async def test_remove_confirmed_removes_entry(tmp_path):
    store.add_python(_py(tmp_path, "print(1)\n"), name="a")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        app.action_remove()
        await pilot.pause()
        await pilot.press("y")
        await pilot.pause()
        assert store.list_entries() == []


async def test_backspace_removes_from_the_table(tmp_path):
    """The Mac ⌫ key (the one above Return) sends backspace, not forward-delete, so the
    footer's advertised "Del" must fire on backspace too. With the table focused, backspace
    opens the remove confirmation."""
    store.add_python(_py(tmp_path, "print(1)\n"), name="a")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        await pilot.pause()
        assert app.focused.__class__.__name__ == "DataTable"
        await pilot.press("backspace")
        await pilot.pause()
        assert app.screen.__class__.__name__ == "ConfirmRemove"


async def test_backspace_in_search_edits_text_not_removes(tmp_path):
    """The backspace→remove binding must not hijack the search box: a focused Input owns
    backspace for delete-left, so typing and backspacing in search never triggers remove."""
    store.add_python(_py(tmp_path, "print(1)\n"), name="alpha")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        await pilot.pause()
        app.action_focus_search()
        await pilot.pause()
        search = app.query_one("#search", Input)
        search.value = "alph"
        search.cursor_position = len(search.value)
        await pilot.press("backspace")
        await pilot.pause()
        assert len(app.screen_stack) == 1  # no remove modal opened
        assert search.value == "alp"  # backspace deleted a character instead


# ---------------------------------------------------------------------------
# rerun guard + execute path
# ---------------------------------------------------------------------------


async def test_rerun_before_first_run_points_at_enter(tmp_path):
    store.add_python(_py(tmp_path, "print(1)\n"), name="fresh")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        app.action_rerun()
        await pilot.pause()
        status = str(app.query_one("#status", Static).render())
        assert "fresh hasn't run yet" in status
        assert "press Enter" in status


async def test_rerun_uses_last_values_and_injects(tmp_path, quiet_run):
    entry = _managed_entry(tmp_path)
    argstate.save_last(entry.slug, values={"CITY": "Kaohsiung"})
    argstate.record_run(entry.slug, 0, at="2026-07-09T00:00:00+00:00")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        app.action_rerun()
        await pilot.pause()
        assert quiet_run["override"] is not None  # a value existed → injection happened
        state = argstate.load_state(entry.slug)
        assert state["last_run"]["exit"] == 0


async def test_execute_records_failure_code_and_status(tmp_path, quiet_run):
    entry = store.add_python(_py(tmp_path, "print(1)\n"), name="j")
    quiet_run["code"] = 3
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        app.action_run()
        await pilot.pause()
        assert argstate.load_state(entry.slug)["last_run"]["exit"] == 3
        status = str(app.query_one("#status", Static).render())
        assert "✗ failed (code 3)" in status


async def test_execute_cleans_injected_artifact(tmp_path, quiet_run):
    entry = _managed_entry(tmp_path)
    argstate.save_last(entry.slug, values={"CITY": "Kaohsiung"})
    argstate.record_run(entry.slug, 0, at="2026-07-09T00:00:00+00:00")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        app.action_rerun()
        await pilot.pause()
        assert not list(entry.dir.glob(".injected*"))


async def test_run_with_fields_opens_the_form(tmp_path, quiet_run):
    _managed_entry(tmp_path)
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        app.action_run()
        await pilot.pause()
        assert isinstance(app.screen, RunFormScreen)


async def test_run_without_fields_skips_the_form(tmp_path, quiet_run):
    store.add_python(_py(tmp_path, "print(1)\n"), name="plain")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        app.action_run()
        await pilot.pause()
        assert not isinstance(app.screen, RunFormScreen)
        assert "values" in quiet_run  # launched directly


# ---------------------------------------------------------------------------
# RunFormScreen mechanics
# ---------------------------------------------------------------------------


ARGPARSE = (
    "import argparse\nap = argparse.ArgumentParser()\n"
    "ap.add_argument('-o', '--output', required=True, help='output path')\n"
    "ap.add_argument('--fast', action='store_true')\n"
    "ap.add_argument('--mode', choices=['a', 'b'], default='a')\n"
    "ap.parse_args()\n"
)


def _argparse_entry(tmp_path):
    return store.add_python(_py(tmp_path, ARGPARSE, "cli.py"), name="cli")


# All-optional argparse: every field has a default, so Enter submits without filling anything.
ARGPARSE_ALL_OPTIONAL = (
    "import argparse\nap = argparse.ArgumentParser()\n"
    "ap.add_argument('--width', type=int, default=800)\n"
    "ap.add_argument('--fast', action='store_true')\n"
    "ap.add_argument('--mode', choices=['a', 'b'], default='a')\n"
    "ap.parse_args()\n"
)


async def test_form_renders_title_required_and_help(tmp_path, quiet_run):
    _argparse_entry(tmp_path)
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        app.action_run()
        await pilot.pause()
        screen = app.screen
        assert isinstance(screen, RunFormScreen)
        # The title lives ON the panel border now (btop grammar), not in a Static.
        assert "Run cli" in str(screen.query_one("#form-panel").border_title)
        labels = "".join(str(w.render()) for w in screen.query(".field-label"))
        assert "required" in labels
        helps = "".join(str(w.render()) for w in screen.query(".field-help"))
        assert "output path" in helps


async def test_form_submit_blocks_on_missing_required(tmp_path, quiet_run):
    _argparse_entry(tmp_path)
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        app.action_run()
        await pilot.pause()
        screen = app.screen
        assert isinstance(screen, RunFormScreen)
        screen.action_submit()
        await pilot.pause()
        assert isinstance(app.screen, RunFormScreen)  # still here
        errors = "".join(str(w.render()) for w in screen.query(".field-error"))
        assert "output is required." in errors


async def test_form_submit_launches_with_collected_values(tmp_path, quiet_run):
    _argparse_entry(tmp_path)
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        app.action_run()
        await pilot.pause()
        screen = app.screen
        assert isinstance(screen, RunFormScreen)
        rows = {row.field.key: row for row in screen.query(FieldRow)}
        rows["output"].query_one(Input).value = "out.png"
        rows["fast"].query_one(Checkbox).value = True
        screen.action_submit()
        await pilot.pause()
        # Declared order: output, fast, mode — explicit-pass keeps mode's default.
        assert quiet_run["extra"] == ["--output", "out.png", "--fast", "--mode", "a"]


async def test_form_escape_cancels_without_running(tmp_path, quiet_run):
    _argparse_entry(tmp_path)
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        app.action_run()
        await pilot.pause()
        await pilot.press("escape")
        await pilot.pause()
        assert "values" not in quiet_run  # nothing launched


async def test_form_preset_chips_apply_values(tmp_path, quiet_run):
    entry = _argparse_entry(tmp_path)
    argstate.save_preset(entry.slug, "web", {"output": "web.png"})
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        app.action_run()
        await pilot.pause()
        screen = app.screen
        assert isinstance(screen, RunFormScreen)
        from textual.widgets import RadioSet

        preset_set = screen.query_one("#preset-set", RadioSet)
        buttons = list(preset_set.query("RadioButton"))
        buttons[1].value = True  # click the "web" chip
        await pilot.pause()
        rows = {row.field.key: row for row in screen.query(FieldRow)}
        assert rows["output"].query_one(Input).value == "web.png"


async def test_form_secret_field_masks_input(tmp_path, quiet_run):
    text = metawriter.write_params(
        'API_KEY = "x"\nprint(API_KEY)\n',
        [ParamSpec(name="API_KEY", kind="const", type="str", secret=True)],
    )
    store.add_python(_py(tmp_path, text, "sec.py"), name="sec")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        app.action_run()
        await pilot.pause()
        screen = app.screen
        assert isinstance(screen, RunFormScreen)
        row = next(r for r in screen.query(FieldRow) if r.field.key == "API_KEY")
        assert row.query_one(Input).password is True
        labels = "".join(str(w.render()) for w in screen.query(".field-label"))
        assert "never saved to disk" in labels


async def test_form_ctrl_s_saves_preset_without_running(tmp_path, quiet_run):
    entry = _argparse_entry(tmp_path)
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        app.action_run()
        await pilot.pause()
        screen = app.screen
        assert isinstance(screen, RunFormScreen)
        rows = {row.field.key: row for row in screen.query(FieldRow)}
        rows["output"].query_one(Input).value = "keep.png"
        screen.action_save_preset()
        await pilot.pause()
        modal_input = app.screen.query_one(Input)
        modal_input.value = "web"
        await pilot.press("enter")
        await pilot.pause()
        presets = argstate.load_state(entry.slug)["presets"]
        assert presets["web"]["output"] == "keep.png"
        assert "values" not in quiet_run  # saving is not running


# ---------------------------------------------------------------------------
# footer / inline-window layout (the two "there's no bottom bar" bugs)
# ---------------------------------------------------------------------------


async def test_library_footer_rows_stack_without_overlap(tmp_path):
    """Regression: the two key-hint rows and the status line are three separate widgets.
    Docking each to the bottom independently lands them all on the SAME row (dock does
    not stack), so the key rows hide behind the status line and the footer reads empty —
    "the bottom bar isn't even there". Wrapped in one docked container they must occupy
    three distinct rows, each painted with its advertised content."""
    store.add_python(_py(tmp_path, "print(1)\n"), name="a")
    app = tui.MenuApp()
    async with app.run_test(size=(120, 40)) as pilot:
        await pilot.pause()
        rows = {
            wid: app.screen.query_one(f"#{wid}", Static)
            for wid in ("keys-local", "keys-global", "status")
        }
        ys = {wid: w.region.y for wid, w in rows.items()}
        assert len(set(ys.values())) == 3, ys  # three distinct rows, no overlap
        assert ys["keys-local"] < ys["keys-global"] < ys["status"]  # keys above status
        assert ys["status"] == app.size.height - 1  # the footer is docked at the very bottom
        assert "Run" in footer_text(rows["keys-local"])
        assert "Add script" in footer_text(rows["keys-global"])
        assert "script" in str(rows["status"].render())


async def test_inline_run_form_body_takes_auto_height(tmp_path, quiet_run, monkeypatch):
    """Regression: `skit run` opens the form through Textual inline mode, where the Screen
    is sized to its content height. The scroll body defaults to height:1fr, which collapses
    to a single row in an auto-height parent — the whole form flattens to a 3-line stub and
    the docked footer is clipped ("skit run has no bottom bar"). In inline mode the body
    must take an explicit auto height (so the screen measures true content) and the window
    caps with max-height so a tall form scrolls inside it."""
    _argparse_entry(tmp_path)
    monkeypatch.setattr(tui.App, "is_inline", property(lambda self: True))
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        app.action_run()
        await pilot.pause()
        screen = app.screen
        assert isinstance(screen, RunFormScreen)
        body = screen.query_one("#form-body")
        assert body.styles.height is not None
        assert body.styles.height.is_auto
        assert screen.styles.max_height is not None  # bounded window → the body can scroll
        assert screen.query_one("#form-keys", Static).display  # footer is present, not clipped


async def test_fullscreen_run_form_body_fills_and_pins_footer(tmp_path, quiet_run):
    """The workbench (non-inline) form keeps the 1fr body fill and an unbounded height, so
    the footer pins to the very bottom of the terminal — the inline fix must not leak into
    full-screen mode."""
    _argparse_entry(tmp_path)
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        app.action_run()
        await pilot.pause()
        screen = app.screen
        assert isinstance(screen, RunFormScreen)
        assert not screen.query_one("#form-body").styles.height.is_auto  # 1fr, not collapsed
        assert screen.styles.max_height is None


async def test_library_footer_chips_fire_on_click(tmp_path):
    """Every footer hint is also a button: clicking a Library chip fires the app action it
    advertises (app.* namespace), so a mouse user never has to touch the keyboard."""
    store.add_python(_py(tmp_path, "print(1)\n"), name="a")
    app = tui.MenuApp()
    async with app.run_test(size=(130, 30)) as pilot:
        await pilot.pause()
        await click_label(pilot, "#keys-global", "Add script")
        assert app.screen.__class__.__name__ == "AddSourceScreen"
        await pilot.press("escape")
        await pilot.pause()
        await click_label(pilot, "#keys-global", "Health check")
        assert app.screen.__class__.__name__ == "HealthScreen"


async def test_boot_focus_is_the_table_but_pushed_screens_focus_their_first_field(tmp_path):
    """Two focus rules that must not bleed into each other: the Library boots on the
    TABLE (search would swallow the advertised letter keys), while pushed screens keep
    the app-wide AUTO_FOCUS "*" so their first field owns the keyboard immediately.
    Regression: an app-level AUTO_FOCUS override once left the run form with NOTHING
    focused — the demo tape's keystrokes landed one field off."""
    _argparse_entry(tmp_path)
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        await pilot.pause()
        assert isinstance(app.focused, DataTable)  # library: the table owns the keys
        app.action_run()
        await pilot.pause()
        assert isinstance(app.screen, RunFormScreen)
        # The form boots typeable: its AUTO_FOCUS lands on the first control, never
        # the body scroll. NOTHING focused is the regression this test pins against.
        assert isinstance(app.focused, Input)
        await pilot.press("escape")
        await pilot.pause()
        app.action_add()
        await pilot.pause()
        assert isinstance(app.focused, Input)  # add flow: the path box, ready to type


async def test_search_focus_swaps_footer_to_input_mode_and_esc_restores_it(tmp_path):
    """While the search box owns the keyboard every single-letter chip is a lie — the
    letters just type text. The footer must swap both key rows for the input-mode hints
    (Enter run · Esc back to list) and restore the full rows when Esc hands the
    keyboard back to the table (and Esc must leave search, not quit the app)."""
    store.add_python(_py(tmp_path, "print(1)\n"), name="a")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        await pilot.pause()
        assert "Add script" in footer_text(app.query_one("#keys-global", Static))
        app.action_focus_search()
        await pilot.pause()
        local = footer_text(app.query_one("#keys-local", Static))
        assert "Back to list" in local
        assert "Run" in local
        assert "Script settings" not in local  # the letter chips are gone
        assert str(app.query_one("#keys-global", Static).render()) == ""
        await pilot.press("escape")
        await pilot.pause()
        assert app.focused is app.query_one(DataTable)
        assert app.return_value is None  # Esc left the search box; it did NOT quit
        assert "Add script" in footer_text(app.query_one("#keys-global", Static))


async def test_search_mode_esc_chip_returns_to_table_on_click(tmp_path):
    """Mouse path while typing: clicking the Esc chip hands focus back to the table —
    it must never fall through to the table-mode Esc meaning (quit)."""
    store.add_python(_py(tmp_path, "print(1)\n"), name="a")
    app = tui.MenuApp()
    async with app.run_test(size=(130, 30)) as pilot:
        app.action_focus_search()
        await pilot.pause()
        await click_label(pilot, "#keys-local", "Back to list")
        assert app.focused is app.query_one(DataTable)
        assert app.return_value is None  # still running
        assert "Add script" in footer_text(app.query_one("#keys-global", Static))


async def test_enter_in_search_runs_the_highlighted_match(tmp_path, quiet_run):
    """The search-mode footer advertises "Enter Run": Enter with the caret in the
    search box runs the highlighted match and returns focus to the table."""
    store.add_python(_py(tmp_path, "print(1)\n"), name="a")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        app.action_focus_search()
        await pilot.pause()
        await pilot.press("enter")
        await pilot.pause()
        assert "values" in quiet_run  # the highlighted script ran
        assert app.focused is app.query_one(DataTable)


async def test_pushed_screen_footer_chips_fire_on_click(tmp_path, quiet_run):
    """A pushed screen's footer chips resolve to that SCREEN's own actions (screen.*
    namespace): clicking Cancel dismisses the run form and launches nothing."""
    _argparse_entry(tmp_path)
    app = tui.MenuApp()
    async with app.run_test(size=(130, 30)) as pilot:
        app.action_run()
        await pilot.pause()
        assert isinstance(app.screen, RunFormScreen)
        await click_label(pilot, "#form-keys", "Cancel")
        assert not isinstance(app.screen, RunFormScreen)  # dismissed by the click
        assert "values" not in quiet_run  # a click on Cancel does not run the script


async def test_enter_submits_from_a_checkbox_field_not_only_a_text_input(tmp_path, quiet_run):
    """B2 lesson (a footer key needs a positive key test): the footer says "Enter Run", but
    a focused Checkbox/RadioSet swallows Enter for its own toggle, so the form binds Enter at
    priority. Pressing Enter with the bool field focused must RUN the form — not toggle the
    checkbox and sit there. (An inline flag-only form has no text Input at all, so this is the
    only submit path besides Ctrl+R.)"""
    store.add_python(_py(tmp_path, ARGPARSE_ALL_OPTIONAL, "opt.py"), name="opt")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        app.action_run()
        await pilot.pause()
        assert isinstance(app.screen, RunFormScreen)
        app.screen.query(Checkbox).first().focus()
        await pilot.pause()
        await pilot.press("enter")
        await pilot.pause()
        assert "values" in quiet_run  # Enter ran the form from the checkbox
        assert not isinstance(app.screen, RunFormScreen)  # and dismissed it


def test_every_footer_chip_targets_an_existing_action():
    """Each clickable footer chip names an action ("app.<x>" / "screen.<x>"); a typo would
    fail silently (a click that fires nothing). Pin that every action a footer advertises
    actually exists on its class, so a renamed action can't leave a dead button behind."""
    from skit.tui_add import AddReviewScreen, AddSourceScreen
    from skit.tui_form import PresetNameModal
    from skit.tui_health import HealthScreen
    from skit.tui_prefs import PreferencesScreen
    from skit.tui_settings import DiscardChangesModal, ScriptSettingsScreen

    expected = {
        tui.MenuApp: [
            "run",
            "rerun",
            "settings",
            "edit",
            "remove",
            "add",
            "presets",
            "focus_search",
            "leave_search",
            "preferences",
            "health",
            "help",
        ],
        RunFormScreen: ["submit", "insert_token", "save_preset", "cancel"],
        ScriptSettingsScreen: ["save", "resync", "close"],
        PreferencesScreen: ["save", "close"],
        HealthScreen: ["jump", "rebuild", "close"],
        AddSourceScreen: ["continue_add", "cancel"],
        AddReviewScreen: ["accept", "toggle_candidate", "edit_source", "cancel"],
        # Modals advertise chips too — a mouse user must never hit a keys-only dead end.
        tui.ConfirmRemove: ["confirm", "cancel"],
        tui.HelpScreen: ["dismiss_help"],
        DiscardChangesModal: ["discard", "keep"],
        PresetNameModal: ["save_name", "cancel"],
    }
    for cls, actions in expected.items():
        for name in actions:
            assert callable(getattr(cls, f"action_{name}", None)), (
                f"{cls.__name__}.action_{name} is referenced by a footer chip but missing"
            )


# ---------------------------------------------------------------------------
# design-fidelity fixes (detail pane, reference marker, language, preset empty)
# ---------------------------------------------------------------------------


async def test_detail_pane_tab_toggle_and_narrow_autocollapse(tmp_path):
    """Spec §1: the detail pane auto-collapses when the terminal is narrow AND too short
    to stack it below the list (a narrow-but-tall portrait stacks instead — see
    test_tui_responsive), and Tab pins it. Tab must win over Textual's built-in
    focus-nav (priority), and the pinned choice survives resizes."""
    store.add_python(_py(tmp_path, "print(1)\n"), name="a")
    app = tui.MenuApp()
    async with app.run_test(size=(120, 24)) as pilot:
        await pilot.pause()
        detail = app.screen.query_one("#detail")
        assert detail.display  # wide → shown
        await pilot.press("tab")
        await pilot.pause()
        assert not detail.display  # Tab toggled it off (not a focus move)
        assert app.focused.__class__.__name__ == "DataTable"  # focus stayed on the table
    app2 = tui.MenuApp()
    async with app2.run_test(size=(70, 12)) as pilot:
        await pilot.pause()
        assert not app2.screen.query_one("#detail").display  # narrow + short → collapsed


async def test_reference_entry_is_marked_in_the_list(tmp_path):
    """Spec §1: reference-mode entries carry a list-level marker, not only a detail-pane note."""
    from textual.widgets import DataTable

    p = _py(tmp_path, "print(1)\n", "orig.py")
    store.add_python(p, name="linked", mode="reference")
    store.add_python(_py(tmp_path, "print(2)\n", "copied.py"), name="copied")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        await pilot.pause()
        table = app.screen.query_one(DataTable)
        kinds = {table.get_row_at(r)[0]: table.get_row_at(r)[1] for r in range(table.row_count)}
        assert "↗" in kinds["linked"]  # reference marked
        assert "↗" not in kinds["copied"]  # copy not marked


async def test_language_change_retranslates_chrome(tmp_path, monkeypatch):
    """Spec §6: a language change applies on the spot — column headers, search placeholder,
    and window title, not only the data rows."""
    from textual.widgets import DataTable

    from skit import i18n

    store.add_python(_py(tmp_path, "print(1)\n"), name="a")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        await pilot.pause()
        headers_en = [str(c.label) for c in app.screen.query_one(DataTable).ordered_columns]
        assert "Name" in headers_en
        i18n.set_language("zh_TW")
        app._retranslate_chrome()
        await pilot.pause()
        headers_zh = [str(c.label) for c in app.screen.query_one(DataTable).ordered_columns]
        assert "名稱" in headers_zh
        assert "搜尋" in app.screen.query_one("#search", Input).placeholder


async def test_run_form_shows_preset_empty_state_hint(tmp_path, quiet_run):
    """Spec §2: with no presets yet, the form still shows the preset row with a hint that
    teaches the Ctrl+S save affordance — precisely when the user most needs to learn it."""
    _argparse_entry(tmp_path)
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        app.action_run()
        await pilot.pause()
        assert isinstance(app.screen, RunFormScreen)
        hint = str(app.screen.query_one("#preset-empty", Static).render())
        assert "Ctrl+S" in hint


def test_field_row_glob_and_token_feedback(tmp_path):
    # The pure pieces the live preview is built from (full widget path is covered above).
    (tmp_path / "a.png").touch()
    assert flows.glob_feedback("*.png", tmp_path) == 1
    from skit import tokens

    expanded, error = tokens.preview("x_{today}", cwd=tmp_path, env={})
    assert error is None
    assert expanded.startswith("x_")


# ---------------------------------------------------------------------------
# ▾ insert token menu
# ---------------------------------------------------------------------------


async def test_insert_token_menu_inserts_at_cursor(tmp_path, quiet_run):
    from skit.tui_form import TokenMenuModal

    _argparse_entry(tmp_path)
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        app.action_run()
        await pilot.pause()
        screen = app.screen
        assert isinstance(screen, RunFormScreen)
        row = next(r for r in screen.query(FieldRow) if r.field.key == "output")
        box = row.query_one(Input)
        box.value = "x_"
        box.focus()
        await pilot.pause()  # focus lands asynchronously
        box.cursor_position = 2
        screen.action_insert_token()
        await pilot.pause()
        assert isinstance(app.screen, TokenMenuModal)
        # Pick "{today}" (third entry: cwd-dynamic, cwd-fixed, today).
        from textual.widgets import OptionList

        menu = app.screen.query_one(OptionList)
        menu.highlighted = 2
        menu.action_select()
        # select -> modal dismiss -> insert-callback spans several event-loop turns; pump until the
        # token lands rather than betting one pause is enough (same flake class as the env picker).
        for _ in range(50):
            await pilot.pause()
            if box.value == "x_{today}":
                break
        assert box.value == "x_{today}"


async def test_insert_token_env_picker_filters_and_inserts(tmp_path, quiet_run, monkeypatch):
    from skit.tui_form import EnvPickerModal

    monkeypatch.setenv("SKIT_PICK_ME", "1")
    _argparse_entry(tmp_path)
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        app.action_run()
        await pilot.pause()
        screen = app.screen
        assert isinstance(screen, RunFormScreen)
        screen.action_insert_token("output")
        await pilot.pause()
        from textual.widgets import OptionList

        menu = app.screen.query_one(OptionList)
        menu.highlighted = 5  # "Environment variable…"
        menu.action_select()
        await pilot.pause()
        assert isinstance(app.screen, EnvPickerModal)
        app.screen.query_one(Input).value = "SKIT_PICK_ME"
        await pilot.pause()
        env_list = app.screen.query_one(OptionList)
        assert env_list.option_count == 1
        env_list.highlighted = 0
        env_list.action_select()
        row = next(r for r in screen.query(FieldRow) if r.field.key == "output")
        # select -> modal dismiss -> insert-callback spans several event-loop turns; pump until the
        # token lands rather than betting one pause is enough (it flaked on the Windows py3.13 runner).
        for _ in range(50):
            await pilot.pause()
            if row.query_one(Input).value:
                break
        assert row.query_one(Input).value == "{env:SKIT_PICK_ME}"


async def test_insert_token_refused_for_secret_fields(tmp_path, quiet_run):
    text = metawriter.write_params(
        'API_KEY = "x"\nprint(API_KEY)\n',
        [ParamSpec(name="API_KEY", kind="const", type="str", secret=True)],
    )
    store.add_python(_py(tmp_path, text, "sec2.py"), name="sec2")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        app.action_run()
        await pilot.pause()
        screen = app.screen
        assert isinstance(screen, RunFormScreen)
        screen.action_insert_token("API_KEY")
        await pilot.pause()
        assert app.screen is screen  # no menu opened
        secret_row = next(r for r in screen.query(FieldRow) if r.field.key == "API_KEY")
        label = str(secret_row.query_one(".field-label").render())
        assert "▾" not in label  # and no link is offered on the secret row


async def test_insert_link_shown_only_on_insertable_fields(tmp_path, quiet_run):
    _argparse_entry(tmp_path)
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        app.action_run()
        await pilot.pause()
        screen = app.screen
        rows = {r.field.key: r for r in screen.query(FieldRow)}
        assert rows["output"].insertable is True
        assert rows["fast"].insertable is False  # bool
        assert rows["mode"].insertable is False  # choice


# ---------------------------------------------------------------------------
# review fixes: launch-failure honesty, drift on r, dirty check, rescan overrides
# ---------------------------------------------------------------------------


async def test_default_after_run_exits_the_tui_before_running(tmp_path, quiet_run):
    """Out of the box skit is a LAUNCHER: Enter hands a PendingRun back and the app
    exits — the script must NOT start while the TUI still owns the terminal (running
    under suspend and exiting right after repainted the app for a frame: the flash).
    (quiet_run pins stay for the workbench tests, so flip back to the default here.)"""
    config.save_after_run("exit")
    store.add_python(_py(tmp_path, "print(1)\n"), name="a")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        await pilot.pause()
        app.action_run()
        pending = app.return_value
        assert isinstance(pending, tui.PendingRun)
        assert pending.entry.meta.name == "a"
    assert "values" not in quiet_run  # nothing ran inside the TUI


def test_finish_run_executes_records_and_passes_the_code_through(tmp_path, quiet_run, monkeypatch):
    """run_menu's exit-mode tail: the pending script runs on the plain terminal, the
    drift banner still prints (rerun skips the form where it normally lives), the run
    is recorded (r-rerun next launch), and the script's code becomes skit's."""
    entry = store.add_python(_py(tmp_path, "print(1)\n"), name="a")
    plan = flows.plan_for_entry(entry)
    plan.drift_lines = ["the script changed"]
    asm = flows.assemble(plan, {}, [], cwd=tmp_path)
    quiet_run["code"] = 7
    printed: list[str] = []
    monkeypatch.setattr("builtins.print", lambda *a, **k: printed.append(" ".join(map(str, a))))
    pending = tui.PendingRun(entry, plan, asm, {}, [], show_drift=True)
    monkeypatch.setattr(tui.MenuApp, "run", lambda self: pending)
    assert tui.run_menu() == 7
    assert "values" in quiet_run
    assert argstate.load_state(entry.slug)["last_run"]
    assert "the script changed" in printed  # the drift banner survives the exit path


def test_finish_run_launch_failure_uses_docker_codes_and_records_nothing(tmp_path, monkeypatch):
    """A launch that never starts after teardown: docker-convention exit code (127 for
    a missing target — same table as `skit run`), an error line, no phantom history."""
    entry = store.add_python(_py(tmp_path, "print(1)\n"), name="ghost")
    plan = flows.plan_for_entry(entry)
    asm = flows.assemble(plan, {}, [], cwd=tmp_path)
    entry.script_path.unlink()
    printed: list[str] = []
    monkeypatch.setattr("builtins.print", lambda *a, **k: printed.append(" ".join(map(str, a))))
    monkeypatch.setattr(
        tui.MenuApp, "run", lambda self: tui.PendingRun(entry, plan, asm, {}, [], False)
    )
    assert tui.run_menu() == 127
    assert argstate.load_state(entry.slug)["last_run"] == {}
    assert any("Error" in line for line in printed)


async def test_after_run_stay_returns_to_the_library(tmp_path, quiet_run):
    """The workbench opt-in: after_run=stay keeps skit open after a run — the status
    line reports the outcome and the app is still running."""
    store.add_python(_py(tmp_path, "print(1)\n"), name="a")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        await pilot.pause()
        app.action_run()
        await pilot.pause()
        assert app.return_value is None  # still open
        assert "✓ finished" in str(app.query_one("#status", Static).render())


async def test_launch_failure_records_no_phantom_run(tmp_path, monkeypatch):
    config.save_after_run("stay")  # the in-TUI failure banner is the stay-mode path
    entry = store.add_python(_py(tmp_path, "print(1)\n"), name="ghosty")
    entry.script_path.unlink()  # the target is gone: the run can never start

    @contextlib.contextmanager
    def _noop(self):
        yield

    monkeypatch.setattr(tui.MenuApp, "suspend", _noop)
    monkeypatch.setattr("builtins.input", lambda *a: "")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        app._execute(entry, flows.FormPlan(source="none"), {}, [])
        await pilot.pause()
        assert argstate.load_state(entry.slug)["last_run"] == {}  # nothing ran, nothing recorded
        status = str(app.query_one("#status").render())
        assert "couldn't launch" in status
        assert app.return_value is None  # a failed launch never quits the workbench


async def test_rerun_path_prints_drift_lines(tmp_path, quiet_run, monkeypatch):
    # r skips the form (where the banner lives) — the suspend block must say it instead.
    # (print() is intercepted directly: while the test app is live, Textual owns stdout,
    # so capsys can't see what would reach the real terminal after a suspend.)
    printed: list[str] = []
    monkeypatch.setattr("builtins.print", lambda *a, **k: printed.append(" ".join(map(str, a))))
    drifted = metawriter.write_params(
        "CITY = 'x'\nprint(CITY)\n",
        [
            ParamSpec(name="CITY", kind="const", type="str"),
            ParamSpec(name="GONE", kind="const", type="str"),
        ],
    )
    entry = store.add_python(_py(tmp_path, drifted, "drifty.py"), name="drifty")
    argstate.save_last(entry.slug, values={"CITY": "k"})
    argstate.record_run(entry.slug, 0, at="2026-01-01T00:00:00+00:00")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        app.action_rerun()
        await pilot.pause()
    assert any("GONE" in line for line in printed)  # the dropped definition is named


async def test_settings_esc_with_unsaved_changes_asks(tmp_path):
    from skit.tui_settings import DiscardChangesModal, ScriptSettingsScreen

    store.add_python(_py(tmp_path, "print(1)\n"), name="j")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        await pilot.press("p")
        screen = app.screen
        assert isinstance(screen, ScriptSettingsScreen)
        desc = screen.query_one("#st-desc", Input)
        desc.focus()
        await pilot.pause()
        desc.value = "edited"
        await pilot.pause()
        await pilot.press("escape")
        await pilot.pause()
        assert isinstance(app.screen, DiscardChangesModal)
        await pilot.press("y")  # discard
        await pilot.pause()
        assert not isinstance(app.screen, (ScriptSettingsScreen, DiscardChangesModal))


async def test_add_rescan_preserves_user_edits(tmp_path, monkeypatch):
    from skit.tui_add import AddReviewScreen

    p = _py(tmp_path, "save('out.jpg')\n", "raw.py")
    monkeypatch.setattr("skit.tui_add.editor.open_in_editor", lambda path: None)

    @contextlib.contextmanager
    def _noop(self):
        yield

    monkeypatch.setattr(tui.MenuApp, "suspend", _noop)
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        screen = AddReviewScreen(p)
        app.push_screen(screen)
        await pilot.pause()
        screen.query_one("#rv-name", Input).value = "my-chosen-name"
        screen.query_one("#rv-desc", Input).value = "my description"
        screen.action_edit_source()
        await pilot.pause()
        await pilot.pause()
        assert screen.query_one("#rv-name", Input).value == "my-chosen-name"
        assert screen.query_one("#rv-desc", Input).value == "my description"


async def test_add_panel_rejects_non_executable_file(tmp_path):
    from skit.tui_add import AddSourceScreen

    notes = tmp_path / "notes.txt"
    notes.write_text("hi", encoding="utf-8")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        screen = AddSourceScreen()
        app.push_screen(screen)
        await pilot.pause()
        box = screen.query_one("#add-path", Input)
        box.value = str(notes)
        from textual.widgets import Input as _Input

        screen._path_given(_Input.Submitted(box, str(notes)))
        await pilot.pause()
        error = str(screen.query_one("#add-error").render())
        assert "notes.txt" in error
        assert store.list_entries() == []  # nothing was added


async def test_degraded_parser_still_opens_the_form_with_notice(tmp_path, quiet_run):
    script = (
        "import argparse\nap = argparse.ArgumentParser()\nsub = ap.add_subparsers()\n"
        "p = sub.add_parser('x')\np.add_argument('--y')\n"
    )
    store.add_python(_py(tmp_path, script, "subby.py"), name="subby")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        app.action_run()
        await pilot.pause()
        screen = app.screen
        assert isinstance(screen, RunFormScreen)  # NOT silently executed
        banner = str(screen.query_one("#degraded-notice").render())
        assert "subcommands" in banner
        rows = [r.field.key for r in screen.query(FieldRow)]
        assert rows == ["__extra_args__"]  # the escape field is there


# ---------------------------------------------------------------------------
# btop restyle (clickable modals, one-pill chips, layout regressions)
# ---------------------------------------------------------------------------


async def test_confirm_modal_chips_are_clickable(tmp_path):
    """The remove confirmation must not be a keys-only dead end: clicking its "y Remove"
    chip removes the entry, exactly like pressing y."""
    store.add_python(_py(tmp_path, "print(1)\n"), name="gone")
    app = tui.MenuApp()
    async with app.run_test(size=(110, 30)) as pilot:
        await pilot.pause()
        app.action_remove()
        await pilot.pause()
        assert app.screen.__class__.__name__ == "ConfirmRemove"
        box = app.screen.query_one("#confirm-box")
        chips = box.query(Static).last()
        plain = str(chips.render())
        idx = plain.find("Remove")
        assert idx >= 0, plain
        await pilot.click(chips, offset=(idx + 1, 0))
        await pilot.pause()
        assert store.list_entries() == []  # the click confirmed the removal


async def test_help_overlay_actually_renders_its_rows(tmp_path):
    """Regression: 1fr Statics inside the auto-width help box measured as zero columns,
    so ? showed a tiny empty square. The key list must occupy real cells."""
    store.add_python(_py(tmp_path, "print(1)\n"), name="a")
    app = tui.MenuApp()
    async with app.run_test(size=(110, 30)) as pilot:
        await pilot.pause()
        app.action_help()
        await pilot.pause()
        statics = app.screen.query_one("#help-box").query(Static)
        assert all(w.region.width > 0 and w.region.height > 0 for w in statics)
        body = str(statics.first().render())
        assert "Rerun with last values" in body
        # The overlay's claim is "everything in the footer is also here": the search and
        # detail-pane rows are pinned with their right-aligned key column intact.
        assert f"{'/':>16}  Search" in body
        assert f"{'Tab':>16}  Detail pane" in body


async def test_preset_empty_hint_occupies_real_columns(tmp_path, quiet_run):
    """Regression: widgets default to width:1fr, so the "Preset:" caption swallowed the
    whole preset row and the empty-state hint rendered at zero width — invisible, though
    present in the DOM. The hint must own actual screen columns."""
    _argparse_entry(tmp_path)
    app = tui.MenuApp()
    async with app.run_test(size=(110, 30)) as pilot:
        app.action_run()
        await pilot.pause()
        hint = app.screen.query_one("#preset-empty", Static)
        assert hint.region.width > 0, "empty-state hint is rendered at zero width"


async def test_bool_checkbox_label_tracks_its_state(tmp_path, quiet_run):
    """The bool control is a borderless Checkbox whose label speaks its state (btop
    grammar): off shows "off", toggling flips the word to "on"."""
    store.add_python(_py(tmp_path, ARGPARSE_ALL_OPTIONAL, "opt.py"), name="opt")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        app.action_run()
        await pilot.pause()
        box = app.screen.query(Checkbox).first()
        assert str(box.label) == "off"
        box.toggle()
        await pilot.pause()
        assert str(box.label) == "on"


async def test_field_feedback_lines_hide_when_empty(tmp_path, quiet_run):
    """Regression: permanent empty preview/error Statics cost 2 blank rows per field and
    stretched the form into a sparse page. They must display only with content."""
    _argparse_entry(tmp_path)
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        app.action_run()
        await pilot.pause()
        row = next(r for r in app.screen.query(FieldRow) if r.field.key == "output")
        preview = row.query_one(".field-preview", Static)
        error = row.query_one(".field-error", Static)
        assert not preview.display  # empty → no blank rows
        assert not error.display
        row.query_one(Input).value = "x_{today}.png"
        await pilot.pause()
        assert preview.display  # token preview materialized
        row.query_one(Input).value = ""
        await pilot.pause()
        assert not preview.display  # cleared → gone again
        row.show_error("output is required.")
        assert error.display
        row.show_error(None)
        assert not error.display


async def test_command_entry_is_not_marked_as_reference(tmp_path):
    """Command templates carry mode="reference" in their meta, but there is no linked
    file — the ↗ marker is python-reference-only."""
    from textual.widgets import DataTable

    store.add_command("echo {x}", name="cmd")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        await pilot.pause()
        table = app.screen.query_one(DataTable)
        assert "↗" not in table.get_row_at(0)[1]


def test_chip_is_one_link_with_pill_background():
    """One chip = ONE button: a single @click span whose pill background covers both the
    key and the label (the old two-tone underline split each chip into two "buttons")."""
    from skit import tui_footer

    chip = tui_footer.chip("screen.cancel", "Esc", "Cancel")
    assert chip.count("@click=") == 1
    on_pos = chip.find("[on ")
    click_pos = chip.find("@click=screen.cancel")
    assert 0 <= on_pos < click_pos  # the same opening tag carries pill + action
    assert "Esc" in chip
    assert "Cancel" in chip
    assert chip.rstrip().endswith("[/]")


# ---------------------------------------------------------------------------
# pre-commit review fixes (add-accept crash, resync report, rescan mode, footer accent)
# ---------------------------------------------------------------------------


async def test_add_accept_survives_cli_framework_with_constant(tmp_path):
    """A script that BOTH parses its own arguments AND defines a module-level constant sets
    uses_cli_framework=True with a non-empty candidates list; the review panel renders no
    candidate checkboxes, so Accept must not query #rv-cand-{i} (NoMatches would crash the
    TUI after the entry was already committed). Accept adds the entry and dismisses cleanly."""
    from skit.tui_add import AddReviewScreen

    p = _py(tmp_path, "import argparse\nTIMEOUT = 30\nargparse.ArgumentParser()\n", "cli_const.py")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        await pilot.pause()
        screen = AddReviewScreen(p)
        app.push_screen(screen)
        await pilot.pause()
        screen.action_accept()
        await pilot.pause()
        assert not isinstance(app.screen, AddReviewScreen)  # dismissed, not crashed
        assert [e.meta.name for e in store.list_entries()] == ["cli_const"]


async def test_add_review_edit_rescan_preserves_storage_mode(tmp_path, monkeypatch):
    """edit->rescan recomposes the panel; the user's Storage choice must survive it —
    a silent revert of 'Link the original' to 'Keep a copy' would flip A5 semantics."""
    import contextlib

    from textual.widgets import RadioButton, RadioSet

    from skit import editor
    from skit.tui_add import AddReviewScreen

    monkeypatch.setattr(editor, "open_in_editor", lambda p: None)
    monkeypatch.setattr(tui.MenuApp, "suspend", lambda self: contextlib.nullcontext())
    p = _py(tmp_path, "print(1)\n", "s.py")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        await pilot.pause()
        screen = AddReviewScreen(p)
        app.push_screen(screen)
        await pilot.pause()
        list(screen.query_one("#rv-mode", RadioSet).query(RadioButton))[1].value = True  # reference
        await pilot.pause()
        assert screen.query_one("#rv-mode", RadioSet).pressed_index == 1
        screen.action_edit_source()  # recomposes
        await pilot.pause()
        assert screen.query_one("#rv-mode", RadioSet).pressed_index == 1  # still reference


async def test_resync_report_survives_recompose(tmp_path):
    """action_resync refreshes with recompose=True, which rebuilds the screen; the report
    (including safety-rebind warnings) must be re-emitted, not erased to blank."""
    from skit.tui_settings import ScriptSettingsScreen

    entry = _managed_entry(tmp_path)
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        await pilot.pause()
        screen = ScriptSettingsScreen(entry)
        app.push_screen(screen)
        await pilot.pause()
        screen.action_resync()
        await pilot.pause()
        report = str(screen.query_one("#st-resync-report", Static).render())
        assert report.strip()  # not wiped by the recompose


def test_footer_link_color_is_the_accent():
    """Textual paints link-color over the whole @click pill, clobbering an inline [$accent]
    on the chip's key. link-color must therefore BE the accent, so footer keys render in
    terracotta (bold) instead of the default grey."""
    from skit.theme import ACCENT, CLAUDE_THEME

    assert CLAUDE_THEME.variables["link-color"] == ACCENT
