"""Plain form renderer: the unified form as line prompts (rich Prompt/Confirm).

One of the FormPlan renderers ("one flow, N renderings"): the TUI renders widgets,
the inline mini-form renders a Textual window, and this renders the humble line-by-line
questionnaire — the deliberate fallback for SSH, dumb terminals, and the `--plain`
flag / `form = "plain"` preference. All logic (prefill, validation, assembly) lives in
flows; this module only asks."""

from __future__ import annotations

from rich.console import Console
from rich.markup import escape
from rich.prompt import Confirm, Prompt

from . import flows
from .i18n import gettext


def collect(
    plan: flows.FormPlan,
    prefill: dict[str, str],
    *,
    console: Console,
) -> dict[str, str]:
    """Ask for every field, re-prompting on a validation error. Returns raw (token/glob
    original) values keyed by field key."""
    values: dict[str, str] = {}
    for f in plan.fields:
        if f.help:
            console.print(f"  [dim]{escape(f.help)}[/dim]")
        if f.degraded:
            console.print(f"  [dim]{gettext("Leave empty to use the script's own default.")}[/dim]")
        values[f.key] = _ask_until_valid(f, prefill.get(f.key, ""), console)
    return values


def _ask_until_valid(f: flows.FormField, default: str, console: Console) -> str:
    while True:
        value = _ask_once(f, default, console)
        error = flows.validate_value(f, value)
        if error is None:
            return value
        console.print(f"  [red]{escape(error)}[/red]")


def _ask_once(f: flows.FormField, default: str, console: Console) -> str:
    label = escape(f.label)
    if f.kind == "bool":
        checked = Confirm.ask(
            f"  {label}", default=default.strip().lower() in ("true", "1", "yes"), console=console
        )
        return "true" if checked else "false"
    if f.secret:
        if f.env_source:
            console.print(
                "  [dim]"
                + gettext("Enter to read it from the environment variable %(env)s.")
                % {"env": escape(f.env_source)}
                + "[/dim]"
            )
        return Prompt.ask(f"  {label}", password=True, console=console)
    if f.kind == "choice" and f.choices:
        return Prompt.ask(
            f"  {label}", choices=f.choices, default=default or f.choices[0], console=console
        )
    answer = Prompt.ask(f"  {label}", default=default or None, console=console)
    return (answer or "").strip()
