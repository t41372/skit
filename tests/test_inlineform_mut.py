"""Mutation-kill tests for inlineform.py — the CLI's inline mini-form.

`collect` must wire the caller's entry/plan/prefill into the app, and `on_mount` must push the
RunFormScreen with the extra-args row disabled (argv owns passthrough args on the CLI)."""

from __future__ import annotations

from pathlib import Path

from skit import flows, inlineform, store
from skit.models import Entry, ScriptMeta
from skit.tui_form import _EXTRA_KEY, FieldRow, RunFormScreen

ARGPARSE_ALL_OPTIONAL = (
    "import argparse\nap = argparse.ArgumentParser()\n"
    "ap.add_argument('--width', type=int, default=800)\n"
    "ap.parse_args()\n"
)


def _command_entry() -> Entry:
    meta = ScriptMeta(name="c", kind="command", template="echo {m}", params=["m"])
    return Entry(slug="c", meta=meta, dir=Path("/nonexistent"))


def _py(tmp_path: Path, body: str, name: str) -> Path:
    p = tmp_path / name
    p.write_text(body, encoding="utf-8")
    return p


def test_collect_wires_entry_plan_and_prefill_into_the_app(monkeypatch):
    # collect() builds `_InlineFormApp(entry, plan, prefill)`; each argument governs a distinct
    # part of the rendered form (title/state, fields, seed values). Substituting any with None
    # would silently build the wrong form. We construct the app for real and read back exactly
    # what it stored — app.run (the blocking terminal loop) is the only thing faked, and it is not
    # the unit under test; the mutated constructor call runs unchanged.
    entry = _command_entry()
    plan = flows.FormPlan(source="command", fields=[flows.FormField(key="m", label="m")])
    prefill = {"m": "seed"}
    captured: dict[str, object] = {}

    def fake_run(
        app_self: inlineform._InlineFormApp, **kwargs: object
    ) -> tuple[dict[str, str], list[str]]:
        captured["entry"] = app_self._entry
        captured["plan"] = app_self._plan
        captured["prefill"] = app_self._prefill
        return {"m": "x"}, []

    monkeypatch.setattr(inlineform._InlineFormApp, "run", fake_run)
    inlineform.collect(entry, plan, prefill)

    assert captured["entry"] is entry
    assert captured["plan"] is plan
    assert captured["prefill"] is prefill


async def test_inline_form_is_built_with_the_extra_args_row_disabled(tmp_path):
    # on_mount pushes RunFormScreen(..., include_extra=False): the inline (CLI) frame hides the
    # extra-args row because argv already owns passthrough args. The screen must be constructed
    # with EXACTLY False — not None, not True, and the keyword must not be dropped (its default is
    # True) — and the extra FieldRow must be absent from the composed form.
    entry = store.add_python(_py(tmp_path, ARGPARSE_ALL_OPTIONAL, "opt.py"), name="opt")
    plan = flows.plan_for_entry(entry)
    prefill = flows.prefill(plan, entry.slug)
    app = inlineform._InlineFormApp(entry, plan, prefill)
    async with app.run_test() as pilot:
        await pilot.pause()
        screen = app.screen
        assert isinstance(screen, RunFormScreen)
        assert (
            screen._include_extra is False
        )  # exactly False → kills =None, =True, and dropped-kwarg
        keys = {row.field.key for row in screen.query(FieldRow)}
        assert _EXTRA_KEY not in keys  # behavioural: the extra-args row really is hidden
