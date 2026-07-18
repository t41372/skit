"""The shared "New agent" modal: define a custom PromptRunner without leaving the TUI.

Every surface that picks a runner (the run form, the prompt add review, Script
settings) mounts the same Ctrl+N chip that opens this modal — the zero-memorization
twin of `skit runner add NAME COMMAND…`. The command is typed as one line and split
into argv with shlex (quotes group words); no shell is ever involved at launch, the
split happens exactly once, here, and the tokens go into config verbatim.
"""

from __future__ import annotations

import shlex
from typing import override

from rich.markup import escape
from textual import on
from textual.app import ComposeResult
from textual.binding import Binding
from textual.containers import Vertical
from textual.screen import ModalScreen
from textual.widgets import Input, Label, Static

from . import config, tui_footer
from .i18n import gettext


def _reason_text(code: str) -> str:
    """TUI wording for a runner-argv validation code — same closed set as the CLI's
    _runner_reason, phrased for a form instead of a command line. Resolved at call
    time (a module-level dict would freeze the import-time locale)."""
    return {
        "empty": gettext("Type the agent's command, e.g. mycli run {{prompt}}"),
        "prompt-slot-count": gettext(
            "The command needs the {{prompt}} slot exactly once — that's where the "
            "rendered prompt lands."
        ),
        "prompt-in-binary": gettext(
            "{{prompt}} can't be the command itself — the first word must be the program to run."
        ),
        "stray-hole": gettext(
            "Runner commands take only the {{prompt}} slot — single-brace text is literal, "
            "and other {{holes}} aren't supported."
        ),
    }[code]


class RunnerAddModal(ModalScreen[str | None]):
    """Register a custom agent (PromptRunner) from any runner-picking surface.

    Dismisses with the new runner's name after saving it into config (seeding the
    presets first, so the new row lands next to visible, editable ones), or None on
    cancel. Validation is the same rule the CLI enforces — the two doors write the
    same config rows."""

    AUTO_FOCUS = "Input"
    BINDINGS = [Binding("escape", "cancel", gettext("Cancel"))]
    DEFAULT_CSS = """
    RunnerAddModal { align: center middle; }
    RunnerAddModal > Vertical {
        border: round $accent; padding: 1 2; width: 72; max-width: 100%; height: auto;
        max-height: 100%; background: $background;
    }
    RunnerAddModal .hint { color: $text-muted; }
    RunnerAddModal #runner-add-error { height: auto; }
    RunnerAddModal.-h-short > Vertical, RunnerAddModal.-h-tiny > Vertical { padding: 0 2; }
    RunnerAddModal Static { width: auto; margin: 1 0 0 0; }
    RunnerAddModal.-h-short Static, RunnerAddModal.-h-tiny Static { margin: 0; }
    """

    @override
    def compose(self) -> ComposeResult:
        with Vertical():
            yield Label(gettext("New agent (runner)"))
            yield Input(placeholder=gettext("Name, e.g. aider"), id="runner-add-name")
            yield Input(
                placeholder=gettext("Command, e.g. aider --message {{prompt}}"),
                id="runner-add-command",
            )
            yield Static(
                gettext(
                    "{{prompt}} marks where the prompt text goes. Each word becomes one "
                    "argument — quotes group words, and no shell is involved."
                ),
                classes="hint",
            )
            yield Static("", id="runner-add-error", markup=True)
            yield Static(
                tui_footer.bar(
                    tui_footer.chip("screen.save_runner", "Enter", gettext("Save")),
                    tui_footer.chip("screen.cancel", "Esc", gettext("Cancel")),
                ),
                markup=True,
            )

    def _error(self, message: str) -> None:
        self.query_one("#runner-add-error", Static).update(f"[red]{escape(message)}[/red]")

    @on(Input.Submitted)
    def _submitted(self, event: Input.Submitted) -> None:
        self.action_save_runner()

    def action_save_runner(self) -> None:
        """Enter / the Save chip: validate, persist, dismiss with the new name."""
        name = self.query_one("#runner-add-name", Input).value.strip()
        command = self.query_one("#runner-add-command", Input).value.strip()
        if not name:
            self._error(gettext("A name is required."))
            self.query_one("#runner-add-name", Input).focus()
            return
        try:
            argv = shlex.split(command)
        except ValueError:
            self._error(gettext("Unbalanced quotes in the command."))
            return
        reason = config.validate_prompt_runner_argv(argv)
        if reason is not None:
            self._error(_reason_text(reason))
            return
        # Seed first so the presets stay visible next to the new row, then re-load:
        # the duplicate check must see exactly what will be written back.
        config.ensure_prompt_runners_seeded()
        runners = config.load_prompt_runners()
        if any(r.name == name for r in runners):
            self._error(
                gettext("The runner %(name)s already exists — pick another name.") % {"name": name}
            )
            return
        config.save_prompt_runners([*runners, config.PromptRunner(name, tuple(argv))])
        self.dismiss(name)

    def action_cancel(self) -> None:
        self.dismiss(None)


def new_runner_chip() -> str:
    """The shared "define a custom agent" affordance: one footer-grammar pill (the key
    hint IS the click target) every runner picker places right beside its options."""
    return tui_footer.chip("screen.new_runner", "Ctrl+N", gettext("New agent…"))
