"""Mutation-targeted behavioural tests for ``promptform`` (the plain line-prompt fallback).

Each test pins an OBSERVABLE contract of ``promptform.collect``: the exact positional
label and ``console`` that reach rich's ``Prompt.ask`` / ``Confirm.ask``, the truthy tokens
that seed a bool field's default, the exact hint text handed to the console, and the gate
that keeps a non-choice field out of the choice prompt. The ask/print calls are stubbed
(the repo idiom — a CliRunner can't drive a live prompt), so what is exercised is the
collector's own argument-building, not rich's TTY.
"""

from __future__ import annotations

import io

from rich.console import Console

from skit import flows, promptform


def _console() -> Console:
    """A recording console with no terminal (line prompts are stubbed)."""
    return Console(file=io.StringIO(), record=True, width=100)


def _recording_console(monkeypatch) -> tuple[Console, list[object]]:
    """A console whose ``print`` records the raw first argument, so a test can assert the
    exact string (markup included) the collector hands over — nothing else prints here."""
    console = _console()
    printed: list[object] = []

    def rec(*a: object, **_k: object) -> None:
        printed.append(a[0] if a else None)

    monkeypatch.setattr(console, "print", rec)
    return console, printed


def _record_ask(monkeypatch, cls, answers):
    """Stub ``cls.ask`` to return each answer in turn, recording the ``(args, kwargs)`` of
    every call so the label/default/choices/console the collector passed can be pinned."""
    it = iter(answers)
    calls: list[tuple[tuple[object, ...], dict[str, object]]] = []

    def fake(*a: object, **kw: object) -> object:
        calls.append((a, kw))
        return next(it)

    monkeypatch.setattr(cls, "ask", fake)
    return calls


# ---------------------------------------------------------------------------
# bool field: label + console reach Confirm.ask; default seeded from truthy tokens
# ---------------------------------------------------------------------------


def test_bool_confirm_gets_label_positional_and_our_console(monkeypatch):
    """The bool question is asked with the escaped ``  label`` as its sole positional prompt
    and the caller's console — not None, and not dropped."""
    plan = flows.FormPlan(
        source="argparse",
        fields=[flows.FormField(key="fast", label="fast", kind="bool")],
    )
    console = _console()
    calls = _record_ask(monkeypatch, promptform.Confirm, [True])
    values = promptform.collect(plan, {}, console=console)

    assert values == {"fast": "true"}
    (args, kwargs) = calls[0]
    assert args == ("  fast",)  # label passed positionally (not None, not omitted)
    assert kwargs.get("console") is console  # our console forwarded (not None, not omitted)


def test_bool_default_true_for_true_and_one_prefills(monkeypatch):
    """A bool field's Confirm default is yes exactly when the prefill (stripped, lowercased)
    is one of the truthy tokens — so "  True  " and "1" both seed default=True."""
    plan = flows.FormPlan(
        source="argparse",
        fields=[
            flows.FormField(key="a", label="a", kind="bool"),
            flows.FormField(key="b", label="b", kind="bool"),
        ],
    )
    console = _console()
    calls = _record_ask(monkeypatch, promptform.Confirm, [True, True])
    promptform.collect(plan, {"a": "  True  ", "b": "1"}, console=console)

    # "  True  " -> "true" is in the truthy set (kills the "XXtrueXX"/"TRUE" token mutations)
    assert calls[0][1]["default"] is True
    # "1" is in the truthy set (kills the "XX1XX" token mutation)
    assert calls[1][1]["default"] is True


# ---------------------------------------------------------------------------
# secret env-source hint: exact text handed to the console
# ---------------------------------------------------------------------------


def test_secret_env_source_hint_printed_verbatim(monkeypatch):
    """A secret with an env source prints exactly one hint line, verbatim including its dim
    markup and the named variable — the whole string, not a fuzzy substring."""
    plan = flows.FormPlan(
        source="inject",
        fields=[
            flows.FormField(key="API_KEY", label="API_KEY", secret=True, env_source="MY_API_KEY")
        ],
    )
    console, printed = _recording_console(monkeypatch)
    calls = _record_ask(monkeypatch, promptform.Prompt, ["sekret"])
    values = promptform.collect(plan, {}, console=console)

    assert values == {"API_KEY": "sekret"}
    assert calls[0][1]["password"] is True
    assert printed == ["  [dim]Enter to read it from the environment variable MY_API_KEY.[/dim]"]


# ---------------------------------------------------------------------------
# degraded free-text hint: exact text handed to the console
# ---------------------------------------------------------------------------


def test_degraded_leave_empty_hint_printed_verbatim(monkeypatch):
    """A degraded field prints the leave-empty hint verbatim, with its dim markup and no
    stray markers — pinned as the full string (a substring check misses the ``XX…XX`` msgid
    mutation, which embeds the original text)."""
    plan = flows.FormPlan(
        source="argparse",
        fields=[flows.FormField(key="bg", label="bg", kind="str", degraded=True)],
    )
    console, printed = _recording_console(monkeypatch)
    _record_ask(monkeypatch, promptform.Prompt, [""])
    values = promptform.collect(plan, {}, console=console)

    assert values == {"bg": ""}
    assert printed == ["  [dim]Leave empty to use the script's own default.[/dim]"]


# ---------------------------------------------------------------------------
# choice gate + choice-prompt arguments
# ---------------------------------------------------------------------------


def test_non_choice_field_with_choices_uses_plain_text_prompt(monkeypatch):
    """The choice branch is entered only when the kind IS "choice" AND choices exist: a
    str-kind field that merely carries choices is asked as plain text, with no ``choices``
    offered (guards the ``and`` gate against becoming ``or``)."""
    plan = flows.FormPlan(
        source="argparse",
        fields=[flows.FormField(key="x", label="x", kind="str", choices=["a", "b"])],
    )
    console = _console()
    calls = _record_ask(monkeypatch, promptform.Prompt, ["typed"])
    values = promptform.collect(plan, {}, console=console)

    assert values == {"x": "typed"}
    (args, kwargs) = calls[0]
    assert args == ("  x",)
    assert "choices" not in kwargs  # plain text prompt, not the choice picker


def test_choice_prompt_gets_label_choices_and_our_console(monkeypatch):
    """A choice field is asked with the escaped ``  label`` positional, its choices, and the
    caller's console — none of them dropped or nulled."""
    plan = flows.FormPlan(
        source="argparse",
        fields=[flows.FormField(key="mode", label="mode", kind="choice", choices=["a", "b"])],
    )
    console = _console()
    calls = _record_ask(monkeypatch, promptform.Prompt, ["b"])
    values = promptform.collect(plan, {}, console=console)

    assert values == {"mode": "b"}
    (args, kwargs) = calls[0]
    assert args == ("  mode",)  # label positional (not None, not omitted)
    assert kwargs["choices"] == ["a", "b"]
    assert kwargs.get("console") is console  # our console forwarded (not None, not omitted)
