"""The ↺ reset-to-default affordance and the input-binding hint, across every surface.

A remembered last-used value overlays the script's own default in the run form's prefill,
which makes that default invisible and (per the zero-memorization rule) unrecoverable — so
each defaulted, non-secret field grows a ``↺ default`` chip and the screen answers Ctrl+O
by restoring ``field.default`` into the live control. These tests pin the OBSERVABLE
contract of that affordance on all four frames that touch it:

- the Textual run form: Ctrl+O from a focused field, the ↺ chip (mouse click AND the
  ``screen.reset_field('<key>')`` action the chip fires), the per-kind restore (text /
  checkbox / radio), the no-default no-op, and the footer advertising Ctrl+O only when
  some field actually has a default to restore;
- the FieldRow compose: the ``input_binding`` help line appears for an intercepted-input
  field and not for a plain const;
- the plain line form (``promptform.collect``): the same input-binding hint is printed;
- the CLI: ``skit params`` shows the SOURCE's live default (not the stale block cache) and
  ``skit show --json`` carries ``delivers_empty`` per field.

The pilot harness mirrors tests/test_tui_form_mut.py: a real ``tui.MenuApp`` hosts a
RunFormScreen pushed with a hand-built FormPlan, and a bare ``_RowApp`` mounts one FieldRow
in isolation. The plain-form / CLI tests reuse the tests/test_forms_cov.py and
tests/test_show.py idioms (a recording Console, a stubbed ``Prompt.ask``, and CliRunner).
"""

from __future__ import annotations

import io
import json
from pathlib import Path
from typing import override

from rich.console import Console
from textual.app import App, ComposeResult
from textual.widgets import Checkbox, Input, RadioButton, Static
from typer.testing import CliRunner

from conftest import footer_text
from skit import cli, flows, promptform, store, tui
from skit.langs.python import metawriter
from skit.models import Entry, ScriptMeta
from skit.params import ParamDecl
from skit.tui_form import FieldRow, RunFormScreen

runner = CliRunner()

INPUT_BINDING_HINT = "Leave empty and the script will ask you in the terminal."


def _entry() -> Entry:
    """A bare command entry: RunFormScreen only reads its slug (for state), name (the
    title), kind (the extra-args label) and resolves a PathContext from it — no file on
    disk is required, so a hand-built Entry drives the form directly."""
    meta = ScriptMeta(name="demo", kind="command", template="echo {m}", params=["m"])
    return Entry(slug="demo", meta=meta, dir=Path("/nonexistent"))


def _plan(*fields: flows.FormField) -> flows.FormPlan:
    return flows.FormPlan(source="command", fields=list(fields))


def _row(screen: RunFormScreen, key: str) -> FieldRow:
    return next(r for r in screen.query(FieldRow) if r.field.key == key)


def _py(tmp_path: Path, body: str, name: str = "job.py") -> Path:
    p = tmp_path / name
    p.write_text(body, encoding="utf-8")
    return p


class _RowApp(App[None]):
    """A bare host so one FieldRow composes in isolation (mirrors test_tui_form_mut)."""

    def __init__(self, field: flows.FormField) -> None:
        super().__init__()
        self._field = field

    @override
    def compose(self) -> ComposeResult:
        yield FieldRow(self._field, "")


# ---------------------------------------------------------------------------
# 1. Ctrl+O from a focused field restores the default over the remembered value
# ---------------------------------------------------------------------------


async def test_ctrl_o_from_focused_field_restores_default_over_remembered_value():
    """A str field whose default is "hello" is prefilled with the DIFFERENT last-used
    "world"; focusing the Input and pressing the advertised Ctrl+O restores "hello" — the
    positive pilot test the footer's chip owes."""
    plan = _plan(
        flows.FormField(key="greeting", label="greeting", default="hello", has_default=True)
    )
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        screen = RunFormScreen(_entry(), plan, {"greeting": "world"})
        app.push_screen(screen)
        await pilot.pause()
        inp = _row(screen, "greeting").query_one(Input)
        assert inp.value == "world"  # the remembered value overlays the default
        inp.focus()
        await pilot.pause()
        await pilot.press("ctrl+o")
        await pilot.pause()
        assert inp.value == "hello"  # Ctrl+O restored the script's own default


# ---------------------------------------------------------------------------
# 2. The ↺ chip: action_reset_field(key) and a real mouse click, per field kind
# ---------------------------------------------------------------------------


async def test_reset_field_by_key_restores_text_bool_and_choice_defaults():
    """action_reset_field(key) — exactly what the ↺ chip's @click fires — restores the
    default of a NON-focused field, across all three control kinds: a text Input, a
    Checkbox, and a RadioSet. Each is prefilled with a value that differs from its default
    so the restore is observable."""
    plan = _plan(
        flows.FormField(key="greeting", label="greeting", default="hello", has_default=True),
        flows.FormField(key="flag", label="flag", kind="bool", default="false", has_default=True),
        flows.FormField(
            key="mode",
            label="mode",
            kind="choice",
            choices=["a", "b"],
            default="a",
            has_default=True,
        ),
    )
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        screen = RunFormScreen(_entry(), plan, {"greeting": "world", "flag": "true", "mode": "b"})
        app.push_screen(screen)
        await pilot.pause()
        text = _row(screen, "greeting").query_one(Input)
        box = _row(screen, "flag").query_one(Checkbox)
        buttons = list(_row(screen, "mode").query(RadioButton))
        # Prefills overlay every default first.
        assert (text.value, box.value, buttons[0].value, buttons[1].value) == (
            "world",
            True,
            False,
            True,
        )
        screen.action_reset_field("greeting")
        screen.action_reset_field("flag")
        screen.action_reset_field("mode")
        await pilot.pause()
        assert text.value == "hello"  # text default restored
        assert box.value is False  # checkbox back to its default off-state
        assert buttons[0].value is True  # default option "a" reselected
        assert buttons[1].value is False  # ...and "b" released


async def test_reset_chip_mouse_click_restores_the_default():
    """The visible ↺ chip IS the click target (footer grammar): clicking it on the field
    label restores the default, no keyboard involved — the mouse-only path the design
    guarantees for every action."""
    plan = _plan(
        flows.FormField(key="greeting", label="greeting", default="hello", has_default=True)
    )
    app = tui.MenuApp()
    async with app.run_test(size=(120, 40)) as pilot:
        screen = RunFormScreen(_entry(), plan, {"greeting": "world"})
        app.push_screen(screen)
        await pilot.pause()
        row = _row(screen, "greeting")
        label = row.query_one(".field-label", Static)
        rendered = str(label.render())
        idx = rendered.find("↺")
        assert idx >= 0  # the ↺ chip is present in the label to click on
        # Clicking the chip resets THIS field to its default: were its @click wired to any
        # other key, greeting's Input would stay "world" and this would fail — so the click
        # pins the field-keyed routing end to end, mouse-only.
        await pilot.click("#fr-greeting .field-label", offset=(idx, 0))
        await pilot.pause()
        assert row.query_one(Input).value == "hello"


# ---------------------------------------------------------------------------
# 3. The ↺ chip in the label markup, and Ctrl+O in the footer, are conditional
# ---------------------------------------------------------------------------


async def test_reset_chip_present_for_default_absent_for_secret_and_no_default():
    """The ↺ default chip rides the label of a defaulted, non-secret field, and stays off
    (a) a secret field (its default is never echoed into the form) and (b) a field with no
    default (there is nothing to restore)."""
    plan = _plan(
        flows.FormField(key="withdef", label="withdef", default="hi", has_default=True),
        flows.FormField(key="sekret", label="sekret", default="s", has_default=True, secret=True),
        flows.FormField(key="nodef", label="nodef"),
    )
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        screen = RunFormScreen(_entry(), plan, {})
        app.push_screen(screen)
        await pilot.pause()

        def _chip(key: str) -> str:
            return str(_row(screen, key).query_one(".field-label", Static).render())

        assert "↺ default" in _chip("withdef")  # defaulted, non-secret → chip present
        assert "↺ default" not in _chip("sekret")  # secret default is never restorable
        assert "↺ default" not in _chip("nodef")  # nothing to restore → no chip
        # The FieldRow.resettable property agrees with the rendered affordance.
        assert _row(screen, "withdef").resettable is True
        assert _row(screen, "sekret").resettable is False
        assert _row(screen, "nodef").resettable is False


async def test_choice_default_outside_its_choices_gets_no_chip_and_no_ctrl_o():
    """A script may declare a choice default that is not one of its own choices. There is
    no radio button for it, so ``set_value`` has nothing to press — the chip would be a
    button that visibly does nothing, which is worse than no chip. The off-menu field is
    not resettable, carries no ↺ in its label, and (as the plan's only field) keeps Ctrl+O
    out of the footer entirely; the sane twin whose default IS in choices keeps both."""
    off_menu = flows.FormField(
        key="env",
        label="env",
        kind="choice",
        choices=["dev", "prod"],
        default="staging",
        has_default=True,
    )
    on_menu = flows.FormField(
        key="env",
        label="env",
        kind="choice",
        choices=["dev", "prod"],
        default="dev",
        has_default=True,
    )
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        screen = RunFormScreen(_entry(), _plan(off_menu), {})
        app.push_screen(screen)
        await pilot.pause()
        row = _row(screen, "env")
        assert row.resettable is False
        assert "↺ default" not in str(row.query_one(".field-label", Static).render())
        keys = footer_text(screen.query_one("#form-keys", Static))
        assert "Ctrl+O" not in keys  # no resettable field on the plan → no dead key taught
        assert "Reset to default" not in keys

    app = tui.MenuApp()
    async with app.run_test() as pilot:
        screen = RunFormScreen(_entry(), _plan(on_menu), {"env": "prod"})
        app.push_screen(screen)
        await pilot.pause()
        row = _row(screen, "env")
        assert row.resettable is True
        assert "↺ default" in str(row.query_one(".field-label", Static).render())
        assert "Ctrl+O" in footer_text(screen.query_one("#form-keys", Static))
        buttons = list(row.query(RadioButton))
        assert (buttons[0].value, buttons[1].value) == (False, True)  # prefill overlays
        screen.action_reset_field("env")
        await pilot.pause()
        assert (buttons[0].value, buttons[1].value) == (True, False)  # "dev" restored


async def test_footer_advertises_ctrl_o_only_when_some_field_is_resettable():
    """The footer teaches Ctrl+O exactly when a field can act on it — a chip that refused
    to do anything would teach a dead key. A plan with a defaulted field shows the pill; a
    plan whose only fields are a secret-with-default and a no-default field does not."""
    resettable = _plan(
        flows.FormField(key="g", label="g", default="h", has_default=True),
    )
    none_resettable = _plan(
        flows.FormField(key="s", label="s", default="x", has_default=True, secret=True),
        flows.FormField(key="p", label="p"),
    )
    for plan, expected in ((resettable, True), (none_resettable, False)):
        app = tui.MenuApp()
        async with app.run_test() as pilot:
            screen = RunFormScreen(_entry(), plan, {})
            app.push_screen(screen)
            await pilot.pause()
            keys = footer_text(screen.query_one("#form-keys", Static))
            assert ("Ctrl+O" in keys) is expected
            assert ("Reset to default" in keys) is expected


# ---------------------------------------------------------------------------
# 4. Ctrl+O with no default (and the action's guard branches) is a safe no-op
# ---------------------------------------------------------------------------


async def test_ctrl_o_on_field_without_default_leaves_value_unchanged():
    """Ctrl+O from a field with no default does nothing (the row isn't resettable) — the
    typed value survives, and nothing crashes. The screen with no focus, and a chip action
    naming an unknown key, are likewise safe no-ops (the action's guard branches)."""
    plan = _plan(flows.FormField(key="plain", label="plain"))
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        screen = RunFormScreen(_entry(), plan, {"plain": "typed"})
        app.push_screen(screen)
        await pilot.pause()
        inp = _row(screen, "plain").query_one(Input)
        inp.focus()
        await pilot.pause()
        await pilot.press("ctrl+o")
        await pilot.pause()
        assert inp.value == "typed"  # no default → the field is left exactly as typed
        # No focused control: the chord returns without touching anything.
        screen.set_focus(None)
        screen.action_reset_field()
        # A chip action naming a field that isn't on the form: no row, no crash.
        screen.action_reset_field("ghost")
        await pilot.pause()
        assert inp.value == "typed"


async def test_ctrl_o_with_focus_outside_any_field_row_is_a_no_op():
    """Focus can legitimately sit on a control with NO FieldRow ancestor — the runner
    picker row. The chord must return quietly there (the `next(…, None)` default): were
    the default dropped, the ancestor scan would raise StopIteration instead."""
    plan = _plan(
        flows.FormField(key="greeting", label="greeting", default="hello", has_default=True)
    )
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        screen = RunFormScreen(_entry(), plan, {"greeting": "world"}, runners=["claude"])
        app.push_screen(screen)
        await pilot.pause()
        screen.set_focus(screen.query_one("#runner-select"))
        await pilot.pause()
        screen.action_reset_field()  # direct call: an exception here fails the test
        await pilot.pause()
        assert _row(screen, "greeting").query_one(Input).value == "world"  # untouched


# ---------------------------------------------------------------------------
# 5. The input-binding hint renders in the FieldRow, and only there
# ---------------------------------------------------------------------------


async def test_input_binding_field_renders_the_ask_in_terminal_hint():
    """A field bound to an intercepted input()/read prompt shows the "leave empty and the
    script will ask you" help line — without it the intercept's semantics are invisible."""
    field = flows.FormField(key="q", label="q", input_binding=True)
    async with _RowApp(field).run_test() as pilot:
        helps = [str(s.render()) for s in pilot.app.query(".field-help")]
        assert helps == [INPUT_BINDING_HINT]


async def test_plain_const_field_renders_no_input_binding_hint():
    """A plain const field (no input binding, no help) shows no help line at all — the hint
    is specific to the intercepted-input case."""
    field = flows.FormField(key="c", label="c")
    async with _RowApp(field).run_test() as pilot:
        assert [str(s.render()) for s in pilot.app.query(".field-help")] == []


# ---------------------------------------------------------------------------
# 6. The plain line form prints the same input-binding hint
# ---------------------------------------------------------------------------


def test_promptform_prints_input_binding_hint(monkeypatch):
    """promptform.collect (the --plain / dumb-terminal fallback) prints the same
    input-binding hint before asking, and only for the input-binding field."""
    plan = flows.FormPlan(
        source="inject",
        fields=[
            flows.FormField(key="q", label="q", input_binding=True),
            flows.FormField(key="c", label="c"),
        ],
    )
    console = Console(file=io.StringIO(), record=True, width=100)
    monkeypatch.setattr(promptform.Prompt, "ask", lambda *_a, **_k: "typed")
    values = promptform.collect(plan, {}, console=console)

    assert values == {"q": "typed", "c": "typed"}
    text = console.export_text()
    # Exact-line pin, not a substring: a mutated msgid ("XX…XX") still CONTAINS the
    # original sentence, so only whole-line equality proves the literal is intact.
    assert "  " + INPUT_BINDING_HINT in text.splitlines()
    assert text.count("ask you in the terminal") == 1  # only the input-binding field prints it


# ---------------------------------------------------------------------------
# 7. CLI: params shows the source's live default; show --json carries delivers_empty
# ---------------------------------------------------------------------------


def _managed(tmp_path: Path) -> store.Entry:
    """A python entry with two managed consts whose SCRIPT LITERAL was edited after the
    manage-time block was written: NAME's block default is the stale "hello" while the
    script now says "bonjour"; COUNT is an int const."""
    text = metawriter.write_params(
        'NAME = "hello"\nCOUNT = 3\nprint(NAME, COUNT)\n',
        [
            ParamDecl(name="NAME", binding="const", type="str", default="hello"),
            ParamDecl(name="COUNT", binding="const", type="int", default="3"),
        ],
    )
    entry = store.add_python(_py(tmp_path, text, "greet.py"), name="greet")
    sp = entry.script_path
    sp.write_text(
        sp.read_text(encoding="utf-8").replace('NAME = "hello"', 'NAME = "bonjour"'),
        encoding="utf-8",
    )
    return entry


def test_params_default_column_shows_the_sources_live_value(tmp_path):
    """The params table's Default column reads the SOURCE's current default (the live
    script literal "bonjour"), never the stale block cache "hello" — the run form's
    prefill and this table must agree."""
    _managed(tmp_path)
    result = runner.invoke(cli.app, ["params", "greet"])
    assert result.exit_code == 0, result.output
    assert "bonjour" in result.output  # the live literal, shown as the default
    assert "hello" not in result.output  # the stale block default never leaks through


def test_show_json_delivers_empty_true_for_str_const_false_for_int(tmp_path):
    """`skit show --json` carries delivers_empty per field: a defaulted str const is
    WYSIWYG (clearing it delivers ""), an int const is not (empty is never a value there)."""
    _managed(tmp_path)
    result = runner.invoke(cli.app, ["show", "greet", "--json"])
    assert result.exit_code == 0, result.output
    fields = {f["key"]: f for f in json.loads(result.output)["fields"]}
    assert fields["NAME"]["delivers_empty"] is True
    assert fields["COUNT"]["delivers_empty"] is False
    assert fields["NAME"]["default"] == "bonjour"  # the live default rides the JSON too


# ---------------------------------------------------------------------------
# 8. Script settings: the parameter row annotates the SOURCE's live default
# ---------------------------------------------------------------------------


async def test_settings_param_row_shows_the_sources_live_default(tmp_path):
    """The Entry-settings parameter row's dim annotation reads the SOURCE's current
    default ('bonjour'), never the stale block cache ('hello') — the settings pane,
    `skit params`, and the run form must tell one story about one record."""
    entry = _managed(tmp_path)
    from skit.tui_settings import ScriptSettingsScreen

    app = tui.MenuApp()
    async with app.run_test() as pilot:
        await pilot.pause()
        screen = ScriptSettingsScreen(entry)
        app.push_screen(screen)
        await pilot.pause()
        labels = [str(cb.label) for cb in screen.query(Checkbox)]
        name_row = next(label for label in labels if label.startswith("NAME"))
        assert "'bonjour'" in name_row  # the live literal, not the manage-time cache
        assert "hello" not in name_row
