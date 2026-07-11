"""Behavioral coverage for the two CLI run-form collectors.

`skit run NAME` gathers field values through one of two collectors:

- ``promptform.collect`` — the humble line-by-line questionnaire (``--plain`` /
  ``form = "plain"`` / ``TERM=dumb``). It prints help/hints, asks each field with rich's
  Prompt/Confirm, re-asks on a validation error, and returns the raw values dict.
- ``inlineform.collect`` — the same RunFormScreen opened in place via Textual's inline
  mode. It returns the submitted values, or ``None`` when the user cancelled.

Every test here asserts an OBSERVABLE contract: the exact returned dict, the prompt/hint
text shown, the default seeded from the prefill, and the None-on-cancel path. The
line-prompt tests stub ``Prompt.ask`` / ``Confirm.ask`` (the repo idiom — CliRunner can't
drive a live prompt), so the collector's own logic is what's exercised, not rich's TTY.
"""

from __future__ import annotations

import io
from pathlib import Path

from rich.console import Console

from skit import flows, inlineform, promptform, store
from skit.models import Entry, ScriptMeta
from skit.tui_form import RunFormScreen


def _console() -> Console:
    """A recording console with no terminal: line prompts are stubbed, so this only has to
    capture the help/hint/error text the collector prints."""
    return Console(file=io.StringIO(), record=True, width=100)


def _script_ask(monkeypatch, cls, answers):
    """Stub ``cls.ask`` (rich Prompt/Confirm) to return each answer in turn, recording the
    (args, kwargs) of every call so a test can pin the label/default/choices passed."""
    it = iter(answers)
    calls: list[tuple[tuple[object, ...], dict[str, object]]] = []

    def fake(*a: object, **kw: object) -> object:
        calls.append((a, kw))
        return next(it)

    monkeypatch.setattr(cls, "ask", fake)
    return calls


def _command_entry() -> Entry:
    meta = ScriptMeta(name="c", kind="command", template="echo {m}", params=["m"])
    return Entry(slug="c", meta=meta, dir=Path("/nonexistent"))


def _py(tmp_path: Path, body: str, name: str = "job.py") -> Path:
    p = tmp_path / name
    p.write_text(body, encoding="utf-8")
    return p


# All-optional argparse: every field has a default, so submit succeeds without filling
# anything — the inline form dismisses with a value dict on the first Enter/submit.
ARGPARSE_ALL_OPTIONAL = (
    "import argparse\nap = argparse.ArgumentParser()\n"
    "ap.add_argument('--width', type=int, default=800)\n"
    "ap.add_argument('--fast', action='store_true')\n"
    "ap.add_argument('--mode', choices=['a', 'b'], default='a')\n"
    "ap.parse_args()\n"
)


# ---------------------------------------------------------------------------
# promptform.collect — the plain line-prompt fallback
# ---------------------------------------------------------------------------


def test_promptform_text_fields_keep_default_or_take_typed(monkeypatch):
    """A text field seeds Prompt.ask with the prefill as its default; an empty answer keeps
    it (rich returns the default), a typed answer overrides. A field without help prints no
    help line, and a field with no prefill/default is asked with default=None."""
    plan = flows.FormPlan(
        source="argparse",
        fields=[
            flows.FormField(key="output", label="output", kind="str", help="output path"),
            flows.FormField(key="name", label="name", kind="str"),
        ],
    )
    console = _console()
    seq = iter(["__keep__", "typed-name"])
    calls: list[dict[str, object]] = []

    def fake(*_a: object, **kw: object) -> object:
        calls.append(kw)
        answer = next(seq)
        # "__keep__" simulates pressing Enter: rich hands back the default it was given.
        return kw["default"] if answer == "__keep__" else answer

    monkeypatch.setattr(promptform.Prompt, "ask", fake)
    values = promptform.collect(plan, {"output": "prev.png"}, console=console)

    assert values == {"output": "prev.png", "name": "typed-name"}
    assert calls[0]["default"] == "prev.png"  # prefill forwarded as the default
    assert calls[1]["default"] is None  # no prefill, no field default -> None, not ""
    text = console.export_text()
    assert "output path" in text  # help printed for the field that has it
    assert text.count("output path") == 1  # ...and only for that one (the no-help field is silent)


def test_promptform_reprompts_on_validation_error(monkeypatch):
    """A required field re-asks after an invalid (empty) answer, printing the validation error,
    and returns only once a valid value arrives."""
    plan = flows.FormPlan(
        source="argparse",
        fields=[flows.FormField(key="output", label="output", kind="str", required=True)],
    )
    console = _console()
    calls = _script_ask(monkeypatch, promptform.Prompt, ["", "final.png"])
    values = promptform.collect(plan, {}, console=console)

    assert values == {"output": "final.png"}
    assert len(calls) == 2  # re-prompted once after the empty answer
    assert "output is required." in console.export_text()  # the error line was shown


def test_promptform_bool_field_maps_confirm_to_true_false(monkeypatch):
    """A bool field is a yes/no Confirm whose default is seeded from the prefill's truthiness,
    and whose answer stores the lowercase string "true"/"false"."""
    plan = flows.FormPlan(
        source="argparse",
        fields=[
            flows.FormField(key="fast", label="fast", kind="bool"),
            flows.FormField(key="slow", label="slow", kind="bool"),
        ],
    )
    console = _console()
    calls = _script_ask(monkeypatch, promptform.Confirm, [True, False])
    values = promptform.collect(plan, {"fast": "yes"}, console=console)

    assert values == {"fast": "true", "slow": "false"}
    assert calls[0][1]["default"] is True  # prefill "yes" -> Confirm defaults to yes
    assert calls[1][1]["default"] is False  # no prefill -> Confirm defaults to no


def test_promptform_secret_field_notes_env_source_and_masks(monkeypatch):
    """A secret field is asked with password=True and, when it declares an env source, prints
    the "Enter to read it from $VAR" hint naming that variable."""
    plan = flows.FormPlan(
        source="inject",
        fields=[
            flows.FormField(key="API_KEY", label="API_KEY", secret=True, env_source="MY_API_KEY")
        ],
    )
    console = _console()
    calls = _script_ask(monkeypatch, promptform.Prompt, ["typed-secret"])
    values = promptform.collect(plan, {}, console=console)

    assert values == {"API_KEY": "typed-secret"}
    assert calls[0][1]["password"] is True  # input is masked
    assert "MY_API_KEY" in console.export_text()  # the env-source hint names the variable


def test_promptform_secret_without_env_source_prints_no_hint(monkeypatch):
    """A secret with no env source is still masked, but prints no environment hint."""
    plan = flows.FormPlan(
        source="inject",
        fields=[flows.FormField(key="TOKEN", label="TOKEN", secret=True)],
    )
    console = _console()
    calls = _script_ask(monkeypatch, promptform.Prompt, ["s3cr3t"])
    values = promptform.collect(plan, {}, console=console)

    assert values == {"TOKEN": "s3cr3t"}
    assert calls[0][1]["password"] is True
    assert "environment variable" not in console.export_text()  # no source -> no hint


def test_promptform_choice_field_offers_the_choices(monkeypatch):
    """A choice field is asked with its choices and defaults to the first when nothing is
    prefilled; the picked choice is returned verbatim."""
    plan = flows.FormPlan(
        source="argparse",
        fields=[flows.FormField(key="mode", label="mode", kind="choice", choices=["a", "b"])],
    )
    console = _console()
    calls = _script_ask(monkeypatch, promptform.Prompt, ["b"])
    values = promptform.collect(plan, {}, console=console)

    assert values == {"mode": "b"}
    assert calls[0][1]["choices"] == ["a", "b"]
    assert calls[0][1]["default"] == "a"  # no prefill -> first choice is the default


def test_promptform_degraded_field_prints_leave_empty_hint(monkeypatch):
    """A degraded free-text field prints the "leave empty for the script's own default" hint,
    and an empty answer is stored as "" (the field is optional, so no re-prompt)."""
    plan = flows.FormPlan(
        source="argparse",
        fields=[flows.FormField(key="bg", label="bg", kind="str", degraded=True)],
    )
    console = _console()
    _script_ask(monkeypatch, promptform.Prompt, [""])
    values = promptform.collect(plan, {}, console=console)

    assert values == {"bg": ""}
    assert "Leave empty to use the script's own default." in console.export_text()


# ---------------------------------------------------------------------------
# inlineform.collect — the inline mini-form (Textual inline mode)
# ---------------------------------------------------------------------------


def test_inline_collect_returns_values_when_form_submits(monkeypatch):
    """collect opens the app in inline mode and, on submit, unpacks the (values, extra) result
    down to just the values dict (the inline frame's extra-args are dropped — argv owns them)."""
    entry = _command_entry()
    plan = flows.FormPlan(source="command", fields=[flows.FormField(key="m", label="m")])

    def fake_run(_self: object, **kwargs: object) -> tuple[dict[str, str], list[str]]:
        assert kwargs.get("inline") is True  # opened in inline mode, not fullscreen
        return {"m": "hi"}, ["--extra"]

    monkeypatch.setattr(inlineform._InlineFormApp, "run", fake_run)
    result = inlineform.collect(entry, plan, {"m": "seed"})

    assert result == {"m": "hi"}  # values returned, the extra list discarded


def test_inline_collect_returns_none_when_cancelled(monkeypatch):
    """A cancelled inline form (app.run yields None) makes collect return None, not an empty
    dict — the caller distinguishes "cancelled" from "submitted nothing"."""
    entry = _command_entry()
    plan = flows.FormPlan(source="command", fields=[flows.FormField(key="m", label="m")])
    monkeypatch.setattr(inlineform._InlineFormApp, "run", lambda _self, **_k: None)

    assert inlineform.collect(entry, plan, {}) is None


async def test_inline_app_pushes_form_and_submit_exits_with_result(tmp_path):
    """Driving the real _InlineFormApp: on_mount registers + activates the Claude theme and
    pushes the RunFormScreen with the extra-args row hidden; get_css_variables exposes the
    $skit-box-* tints from the first frame; and submitting the form routes the result through
    the _done callback into app.exit."""
    entry = store.add_python(_py(tmp_path, ARGPARSE_ALL_OPTIONAL, "opt.py"), name="opt")
    plan = flows.plan_for_entry(entry)
    prefill = flows.prefill(plan, entry.slug)
    app = inlineform._InlineFormApp(entry, plan, prefill)
    async with app.run_test() as pilot:
        await pilot.pause()
        assert isinstance(app.screen, RunFormScreen)  # on_mount pushed the run form
        assert app.theme == "skit-claude"  # the Claude theme was registered and activated
        assert "skit-box-maroon" in app.get_css_variables()  # box tints resolvable up front
        app.screen.action_submit()  # all-optional form -> dismisses with (values, [])
        await pilot.pause()

    result = app.return_value
    assert result is not None  # _done forwarded the submit result into app.exit
    values, extra = result
    assert extra == []  # include_extra=False: the inline frame hides the extra-args row
    assert set(values) == {"width", "fast", "mode"}  # the plan's fields were collected
