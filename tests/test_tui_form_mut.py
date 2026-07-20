"""Mutation-killing behavioral tests for skit.tui_form.

Every test pins an OBSERVABLE contract of the run-form widgets: the translated type
hints and degrade notices, the FieldRow control widgets (Checkbox / RadioSet / Input)
and how prefills seed them, set_value's write-back, the field-error line, the env-var
picker's 200-item cap, and the RunFormScreen's title/collect/insert-token/save-preset
paths. Pure helpers are called directly; widget behaviour is driven through Textual's
Pilot harness, mirroring tests/test_tui_mut.py and tests/test_forms_cov.py.
"""

from __future__ import annotations

import contextlib
from typing import override

import pytest
from textual.app import App, ComposeResult
from textual.widgets import Checkbox, Input, OptionList, RadioButton, Select, Static

from skit import argstate, config, flows, i18n, launcher, store, tui, tui_form
from skit.langs.python import metawriter
from skit.params import ParamDecl
from skit.tui_form import EnvPickerModal, FieldRow, RunFormScreen, TokenMenuModal


@pytest.fixture(autouse=True)
def _english():
    # Exact-message assertions read the English catalog (msgids are the English source).
    i18n.init("en")


def _py(tmp_path, body: str, name: str = "job.py"):
    p = tmp_path / name
    p.write_text(body, encoding="utf-8")
    return p


@contextlib.contextmanager
def _noop_suspend():
    yield


@pytest.fixture
def quiet_run(monkeypatch):
    """Keep the workbench open after a run and neutralize the real launch (mirrors
    tests/test_tui_mut.py's fixture): action_run then opens the form instead of taking
    over the terminal."""
    config.save_after_run("stay")
    calls: dict[str, object] = {}

    def fake_run(entry, extra_args=None, *, values=None, **_kw):
        calls["extra"] = list(extra_args or [])
        calls["values"] = dict(values or {})
        return calls.get("code", 0)

    monkeypatch.setattr(launcher, "run_entry", fake_run)
    monkeypatch.setattr(tui.MenuApp, "suspend", lambda self: _noop_suspend())
    monkeypatch.setattr("builtins.input", lambda *a: "")
    return calls


ARGPARSE = (
    "import argparse\nap = argparse.ArgumentParser()\n"
    "ap.add_argument('-o', '--output', required=True, help='output path')\n"
    "ap.add_argument('--fast', action='store_true')\n"
    "ap.add_argument('--mode', choices=['a', 'b'], default='a')\n"
    "ap.parse_args()\n"
)


def _argparse_entry(tmp_path):
    return store.add_python(_py(tmp_path, ARGPARSE, "cli.py"), name="cli")


class _RowApp(App[None]):
    """A bare host so one FieldRow can be mounted and driven in isolation."""

    def __init__(self, field: flows.FormField, prefill: str = "") -> None:
        super().__init__()
        self._field = field
        self._prefill = prefill

    @override
    def compose(self) -> ComposeResult:
        yield FieldRow(self._field, self._prefill)


# ---------------------------------------------------------------------------
# _type_label — the translated dim type hint (msgids must be literal gettext calls)
# ---------------------------------------------------------------------------


def test_type_label_translates_each_known_kind_and_echoes_unknown():
    assert tui_form._type_label("int") == "whole number"
    assert tui_form._type_label("float") == "number"
    assert tui_form._type_label("str") == "text"
    assert tui_form._type_label("bool") == "on/off"
    # An unknown kind is echoed back verbatim (the dict .get fallback IS `kind`).
    assert tui_form._type_label("path") == "path"
    assert tui_form._type_label("choice") == "choice"


# ---------------------------------------------------------------------------
# _degraded_notice — the honest whole-parser-degrade line
# ---------------------------------------------------------------------------


def test_degraded_notice_subparsers_branch_text():
    assert tui_form._degraded_notice("subparsers") == (
        "This script has subcommands skit can't model — type everything into the "
        "extra-arguments field."
    )


def test_degraded_notice_generic_branch_text():
    # Any reason other than "subparsers" takes the generic can't-read-declarations line.
    assert tui_form._degraded_notice("dynamic") == (
        "skit couldn't read this script's argument declarations — type everything into "
        "the extra-arguments field."
    )


# ---------------------------------------------------------------------------
# EnvPickerModal._options — the 200-item filtered cap
# ---------------------------------------------------------------------------


def test_env_picker_options_cap_at_200(monkeypatch):
    for i in range(205):
        monkeypatch.setenv(f"ZZQCAP_{i:03d}", "1")
    opts = EnvPickerModal()._options("zzqcap")
    assert len(opts) == 200  # 205 matches, capped to the first 200
    assert all(str(o.id).startswith("ZZQCAP_") for o in opts)


# ---------------------------------------------------------------------------
# FieldRow._compose_control — the control widget per field kind
# ---------------------------------------------------------------------------


async def test_bool_prefill_true_checks_box_and_labels_on():
    f = flows.FormField(key="flag", label="flag", kind="bool")
    async with _RowApp(f, "true").run_test() as pilot:
        box = pilot.app.query_one(Checkbox)
        assert box.value is True  # "true" seeds a checked box
        assert str(box.label) == "on"  # and the label speaks the on-state


async def test_bool_prefill_one_and_yes_are_truthy():
    for prefill in ("1", "yes"):
        f = flows.FormField(key="flag", label="flag", kind="bool")
        async with _RowApp(f, prefill).run_test() as pilot:
            assert pilot.app.query_one(Checkbox).value is True


async def test_bool_prefill_empty_leaves_box_off():
    f = flows.FormField(key="flag", label="flag", kind="bool")
    async with _RowApp(f, "").run_test() as pilot:
        box = pilot.app.query_one(Checkbox)
        assert box.value is False
        assert str(box.label) == "off"


async def test_choice_field_with_empty_choices_falls_back_to_input():
    # kind == "choice" but no choices: the AND guard routes this to the free-text Input
    # (an OR mutant would wrongly enter the empty RadioSet branch, leaving no Input).
    f = flows.FormField(key="c", label="c", kind="choice", choices=[])
    async with _RowApp(f, "seed").run_test() as pilot:
        assert pilot.app.query_one(FieldRow).query_one(Input).value == "seed"


async def test_choice_field_renders_labeled_radio_buttons_with_prefill_selected():
    f = flows.FormField(key="m", label="m", kind="choice", choices=["alpha", "beta"])
    async with _RowApp(f, "alpha").run_test() as pilot:
        buttons = list(pilot.app.query(RadioButton))
        assert [str(b.label) for b in buttons] == ["alpha", "beta"]  # choice text shown
        assert buttons[0].value is True  # prefill "alpha" pre-selects its button
        assert buttons[1].value is False


async def test_text_field_seeds_input_with_prefill():
    f = flows.FormField(key="o", label="o", kind="str")
    async with _RowApp(f, "hello").run_test() as pilot:
        assert pilot.app.query_one(Input).value == "hello"


# ---------------------------------------------------------------------------
# FieldRow.set_value — write a value back into the live control
# ---------------------------------------------------------------------------


async def test_set_value_bool_reads_truthy_strings():
    f = flows.FormField(key="flag", label="flag", kind="bool")
    async with _RowApp(f, "").run_test() as pilot:
        row = pilot.app.query_one(FieldRow)
        box = row.query_one(Checkbox)
        row.set_value("true")
        assert box.value is True
        row.set_value("false")
        assert box.value is False
        row.set_value("1")
        assert box.value is True
        row.set_value("yes")
        assert box.value is True


async def test_set_value_choice_selects_matching_button():
    f = flows.FormField(key="m", label="m", kind="choice", choices=["a", "b"])
    async with _RowApp(f, "a").run_test() as pilot:
        row = pilot.app.query_one(FieldRow)
        buttons = list(row.query(RadioButton))
        row.set_value("b")
        await pilot.pause()
        assert buttons[1].value is True  # "b" now pressed
        assert buttons[0].value is False  # "a" released


async def test_set_value_choice_empty_choices_writes_input():
    f = flows.FormField(key="c", label="c", kind="choice", choices=[])
    async with _RowApp(f, "").run_test() as pilot:
        row = pilot.app.query_one(FieldRow)
        row.set_value("typed")
        assert row.query_one(Input).value == "typed"


# ---------------------------------------------------------------------------
# FieldRow.show_error — the terse validation line
# ---------------------------------------------------------------------------


async def test_show_error_reveals_message_then_clears():
    f = flows.FormField(key="o", label="o", kind="str")
    async with _RowApp(f, "").run_test() as pilot:
        row = pilot.app.query_one(FieldRow)
        err = row.query_one(".field-error", Static)
        row.show_error("bad value")
        assert err.display is True
        assert "bad value" in str(err.render())
        row.show_error(None)
        assert err.display is False
        assert str(err.render()) == ""  # cleared to empty, not left with stray text


# ---------------------------------------------------------------------------
# RunFormScreen — title, extra-args collection, token insert, preset save
# ---------------------------------------------------------------------------


async def test_form_border_title_is_exactly_run_name(tmp_path, quiet_run):
    _argparse_entry(tmp_path)
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        app.action_run()
        await pilot.pause()
        screen = app.screen
        assert isinstance(screen, RunFormScreen)
        assert str(screen.query_one("#form-panel").border_title) == "Run cli"


async def test_collect_shlex_splits_the_extra_args_field(tmp_path, quiet_run):
    _argparse_entry(tmp_path)
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        app.action_run()
        await pilot.pause()
        screen = app.screen
        assert isinstance(screen, RunFormScreen)
        extra_row = next(r for r in screen.query(FieldRow) if r.field.key == tui_form._EXTRA_KEY)
        extra_row.query_one(Input).value = "--foo bar"
        _values, extra = screen.collect()
        assert extra == ["--foo", "bar"]  # the field's text is shlex-split into argv


async def test_insert_token_opens_menu_for_the_named_field(tmp_path, quiet_run):
    _argparse_entry(tmp_path)
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        app.action_run()
        await pilot.pause()
        screen = app.screen
        assert isinstance(screen, RunFormScreen)
        rows = {r.field.key: r for r in screen.query(FieldRow)}
        # Target the extra-args row (LAST, insertable): a query that ignored the #fr-<key>
        # id and grabbed the first FieldRow instead would wrongly act on "output".
        screen.action_insert_token(tui_form._EXTRA_KEY)
        await pilot.pause()
        assert isinstance(app.screen, TokenMenuModal)
        menu = app.screen.query_one(OptionList)
        menu.highlighted = 0  # "{cwd}"
        menu.action_select()
        # select -> dismiss -> insert callback spans several loop turns; pump until it lands.
        for _ in range(50):
            await pilot.pause()
            if rows[tui_form._EXTRA_KEY].query_one(Input).value:
                break
        # the token landed in the named (extra-args) field, NOT the first field.
        assert rows[tui_form._EXTRA_KEY].query_one(Input).value == "{cwd}"
        assert rows["output"].query_one(Input).value == ""


async def test_save_preset_notifies_and_reloads_presets(tmp_path, quiet_run):
    entry = _argparse_entry(tmp_path)
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        app.action_run()
        await pilot.pause()
        screen = app.screen
        assert isinstance(screen, RunFormScreen)
        rows = {r.field.key: r for r in screen.query(FieldRow)}
        rows["output"].query_one(Input).value = "keep.png"
        screen.action_save_preset()
        await pilot.pause()
        app.screen.query_one(Input).value = "web"
        await pilot.press("enter")
        for _ in range(20):
            await pilot.pause()
            if "web" in screen._presets:
                break
        # the preset persisted AND was reloaded onto the screen from the entry's slug
        assert "web" in screen._presets
        assert screen._presets["web"]["output"] == "keep.png"
        assert argstate.load_state(entry.slug)["presets"]["web"]["output"] == "keep.png"
        # the confirmation toast carries the exact english message
        messages = [n.message for n in app._notifications]
        assert 'Preset "web" saved.' in messages


async def test_save_preset_strips_secret_values_from_disk(tmp_path, quiet_run):
    text = metawriter.write_params(
        'API_KEY = "x"\nprint(API_KEY)\n',
        [ParamDecl(name="API_KEY", binding="const", type="str", secret=True)],
    )
    entry = store.add_python(_py(tmp_path, text, "sec.py"), name="sec")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        app.action_run()
        await pilot.pause()
        screen = app.screen
        assert isinstance(screen, RunFormScreen)
        row = next(r for r in screen.query(FieldRow) if r.field.key == "API_KEY")
        row.query_one(Input).value = "s3cr3t"
        screen.action_save_preset()
        await pilot.pause()
        app.screen.query_one(Input).value = "vault"
        await pilot.press("enter")
        for _ in range(20):
            await pilot.pause()
            if "vault" in argstate.load_state(entry.slug)["presets"]:
                break
        preset = argstate.load_state(entry.slug)["presets"]["vault"]
        assert "API_KEY" not in preset  # secret_names kept the secret off disk


async def test_save_preset_field_less_warns_with_exact_message_and_severity(
    tmp_path, quiet_run, monkeypatch
):
    """A field-less form has nothing to save: Ctrl+S refuses with the CLI's exact sentence
    at WARNING severity (not information, not None, not a dropped kwarg) and opens no modal.
    The notify call is captured whole so the exact msgid AND severity are both pinned."""
    store.add_command("echo hi", name="noargs")
    entry = store.resolve("noargs")
    plan = flows.plan_for_entry(entry)
    assert plan.fields == []  # truly field-less
    calls: list[tuple[str, dict[str, object]]] = []
    monkeypatch.setattr(
        RunFormScreen, "notify", lambda self, message, **kw: calls.append((message, kw))
    )
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        screen = RunFormScreen(entry, plan, {})
        app.push_screen(screen)
        await pilot.pause()
        screen.action_save_preset()
        await pilot.pause()
        assert app.screen is screen  # no modal opened
    assert calls == [
        ("noargs has no form fields, so there's nothing to save.", {"severity": "warning"})
    ]


async def test_save_preset_saved_notify_carries_information_severity(
    tmp_path, quiet_run, monkeypatch
):
    """The save confirmation toast is the exact sentence at INFORMATION severity. Capturing
    the notify call (not the rendered toast) pins the severity kwarg was passed explicitly —
    a dropped kwarg would fall back to the same default and hide otherwise."""
    _argparse_entry(tmp_path)
    calls: list[tuple[str, dict[str, object]]] = []
    monkeypatch.setattr(
        RunFormScreen, "notify", lambda self, message, **kw: calls.append((message, kw))
    )
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        app.action_run()
        await pilot.pause()
        screen = app.screen
        assert isinstance(screen, RunFormScreen)
        rows = {r.field.key: r for r in screen.query(FieldRow)}
        rows["output"].query_one(Input).value = "keep.png"
        screen.action_save_preset()
        await pilot.pause()
        app.screen.query_one(Input).value = "web"
        await pilot.press("enter")
        for _ in range(20):
            await pilot.pause()
            if calls:
                break
    assert calls == [('Preset "web" saved.', {"severity": "information"})]


async def test_first_preset_save_mounts_select_with_last_values_row_and_no_blank(
    tmp_path, quiet_run
):
    """The first save swaps the empty-state hint for a real dropdown whose leading row is
    skit's localized "↩ last values" restore option (exact copy) and whose allow_blank is
    False — no synthetic NoSelection row rides in."""
    entry = _argparse_entry(tmp_path)
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        screen = RunFormScreen(entry, plan=flows.plan_for_entry(entry), prefill={})
        app.push_screen(screen)
        await pilot.pause()
        assert screen.query("#preset-empty")  # empty-state hint is showing
        screen.action_save_preset()
        await pilot.pause()
        app.screen.query_one(Input).value = "web"
        await pilot.press("enter")
        for _ in range(20):
            await pilot.pause()
            if screen.query("#preset-select"):
                break
        select = screen.query_one("#preset-select", Select)
        # the localized "last values" restore row leads the options, verbatim (kills XX / case)
        assert select._options[0] == ("↩ last values", "")
        # allow_blank stays False — no synthetic NoSelection row (kills None / True / dropped)
        assert select._allow_blank is False
