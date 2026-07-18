"""Runner (agent) UI shared by every surface: the pick list, the add/edit modal, and
the management screen.

Every surface that picks a runner (the run form, the prompt add review, Script
settings) uses a value-keyed Select dropdown and mounts the same Ctrl+N chip that
opens RunnerAddModal — the zero-memorization twin of `skit runner add NAME COMMAND…`. The
command is typed as one line and split into argv with shlex (quotes group words); no
shell is ever involved at launch, the split happens exactly once, here, and the tokens
go into config verbatim. RunnerManageScreen (reached from Preferences) is where the
whole registry is visible: list, edit, remove — never an add-only door.
"""

from __future__ import annotations

import shlex
from typing import override

from rich.markup import escape
from textual import on
from textual.app import ComposeResult
from textual.binding import Binding
from textual.containers import Vertical
from textual.screen import ModalScreen, Screen
from textual.widgets import Input, Label, OptionList, Static
from textual.widgets.option_list import Option

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
    """Register (or edit) a custom agent from any runner-picking surface.

    Dismisses with the saved runner's name after writing it into config (seeding the
    presets first, so the row lands next to visible, editable ones), or None on
    cancel. Validation is the same rule the CLI enforces — the two doors write the
    same config rows. Pass ``editing`` to open prefilled on an existing runner: the
    save then replaces that row in place (renaming allowed), instead of refusing the
    name as a duplicate."""

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

    def __init__(self, editing: str | None = None) -> None:
        super().__init__()
        self._editing: str | None = editing

    @override
    def compose(self) -> ComposeResult:
        original = config.find_prompt_runner(self._editing) if self._editing else None
        with Vertical():
            yield Label(
                gettext("Edit agent (runner)") if original else gettext("New agent (runner)")
            )
            yield Input(
                value=original.name if original else "",
                placeholder=gettext("Name, e.g. aider"),
                id="runner-add-name",
            )
            yield Input(
                value=shlex.join(original.argv) if original else "",
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
        """Enter / the Save chip: validate, persist, dismiss with the saved name."""
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
        if any(r.name == name for r in runners if r.name != self._editing):
            self._error(
                gettext("The runner %(name)s already exists — pick another name.") % {"name": name}
            )
            return
        new_row = config.PromptRunner(name, tuple(argv))
        if self._editing and any(r.name == self._editing for r in runners):
            # Replace in place — an edit (or rename) keeps the row's position.
            runners = [new_row if r.name == self._editing else r for r in runners]
        else:
            runners = [*runners, new_row]
        config.save_prompt_runners(runners)
        self.dismiss(name)

    def action_cancel(self) -> None:
        self.dismiss(None)


class RunnerActionModal(ModalScreen[str | None]):
    """What to do with one runner: edit or remove (verb keys, chips clickable)."""

    BINDINGS = [
        Binding("e", "edit", gettext("Edit")),
        Binding("d", "remove", gettext("Remove")),
        Binding("escape", "cancel", gettext("Back")),
    ]
    DEFAULT_CSS = """
    RunnerActionModal { align: center middle; }
    RunnerActionModal > Vertical {
        border: round $accent; padding: 1 2; width: auto; height: auto;
        max-width: 100%; max-height: 100%; background: $background;
    }
    RunnerActionModal Static { width: auto; }
    RunnerActionModal > Vertical > Static:last-of-type { margin: 1 0 0 0; }
    """

    def __init__(self, name: str) -> None:
        super().__init__()
        self._name: str = name

    @override
    def compose(self) -> ComposeResult:
        runner = config.find_prompt_runner(self._name)
        command = shlex.join(list(runner.argv)) if runner else ""
        with Vertical():
            yield Label(escape(self._name))
            yield Static(escape(command))
            yield Static(
                tui_footer.bar(
                    tui_footer.chip("screen.edit", "e", gettext("Edit")),
                    tui_footer.chip("screen.remove", "d", gettext("Remove")),
                    tui_footer.chip("screen.cancel", "Esc", gettext("Back")),
                ),
                markup=True,
            )

    def action_edit(self) -> None:
        self.dismiss("edit")

    def action_remove(self) -> None:
        self.dismiss("remove")

    def action_cancel(self) -> None:
        self.dismiss(None)


class RunnerRemoveConfirm(ModalScreen[bool]):
    """Removing a configured agent is destructive config surgery — it gets the same
    ask the Library gives entry removal, not a bare one-keystroke delete."""

    BINDINGS = [
        Binding("y", "confirm", gettext("Remove")),
        Binding("escape,n", "cancel", gettext("Keep")),
    ]
    DEFAULT_CSS = """
    RunnerRemoveConfirm { align: center middle; }
    RunnerRemoveConfirm > Vertical { border: round $accent; padding: 1 2; width: auto;
        max-width: 100%; height: auto; max-height: 100%; background: $background; }
    RunnerRemoveConfirm Static { width: auto; margin: 1 0 0 0; }
    """

    def __init__(self, name: str) -> None:
        super().__init__()
        self._name: str = name

    @override
    def compose(self) -> ComposeResult:
        with Vertical():
            yield Label(gettext('Remove the agent "%(name)s"?') % {"name": self._name})
            yield Static(
                tui_footer.bar(
                    tui_footer.chip("screen.confirm", "y", gettext("Remove")),
                    tui_footer.chip("screen.cancel", "Esc", gettext("Keep")),
                ),
                markup=True,
            )

    def action_confirm(self) -> None:
        self.dismiss(True)

    def action_cancel(self) -> None:
        self.dismiss(False)


class RunnerManageScreen(Screen[None]):
    """The runner registry, whole: every configured agent listed with its command;
    pick one to edit or remove it, Ctrl+N to define a new one. Reached from
    Preferences — the registry must never be an add-only door whose contents can
    only be inspected by hand-reading config.toml."""

    BINDINGS = [
        Binding("escape", "close", gettext("Back")),
        Binding("ctrl+n", "new_runner", gettext("New agent")),
    ]
    AUTO_FOCUS = "OptionList"
    DEFAULT_CSS = """
    RunnerManageScreen #rm-body {
        padding: 0 1;
        border: round $skit-box-olive;
        border-title-color: ansi_bright_white;
        border-title-style: bold;
    }
    RunnerManageScreen .hint { color: $text-muted; }
    RunnerManageScreen OptionList { height: auto; border: none; }
    RunnerManageScreen KeysBar { dock: bottom; }
    RunnerManageScreen #rm-keys { color: $text-muted; }
    """

    def on_mount(self) -> None:
        # The management surface is where the seeds materialize into the user's
        # config — visible and editable, never a hidden built-in list (same rule
        # as `skit runner list`).
        config.ensure_prompt_runners_seeded()
        self.query_one("#rm-body").border_title = gettext("Agents (prompt runners)")
        self._reload()

    def _reload(self) -> None:
        option_list = self.query_one(OptionList)
        option_list.clear_options()
        runners = config.load_prompt_runners()
        for runner in runners:
            option_list.add_option(
                Option(
                    f"{escape(runner.name)}  [dim]{escape(shlex.join(list(runner.argv)))}[/dim]",
                    id=runner.name,
                )
            )
        empty = self.query_one("#rm-empty", Static)
        empty.display = not runners

    @override
    def compose(self) -> ComposeResult:
        with tui_footer.FormBody(id="rm-body"):
            yield Static(
                gettext("The agents prompt entries run with. Pick one to edit or remove it."),
                classes="hint",
            )
            yield OptionList()
            yield Static(gettext("No agents configured yet."), id="rm-empty", classes="hint")
        yield tui_footer.KeysBar(
            Static(
                tui_footer.bar(
                    new_runner_chip(),
                    tui_footer.chip("screen.close", "Esc", gettext("Back")),
                    tui_footer.nav_chip(),
                ),
                id="rm-keys",
                markup=True,
            )
        )

    @on(OptionList.OptionSelected)
    def _picked(self, event: OptionList.OptionSelected) -> None:
        name = str(event.option.id)

        def _decided(action: str | None) -> None:
            if action == "edit":
                self.app.push_screen(RunnerAddModal(editing=name), lambda _: self._reload())
            elif action == "remove":

                def _confirmed(really: bool | None) -> None:
                    if really:
                        config.save_prompt_runners(
                            [r for r in config.load_prompt_runners() if r.name != name]
                        )
                        self._reload()

                self.app.push_screen(RunnerRemoveConfirm(name), _confirmed)

        self.app.push_screen(RunnerActionModal(name), _decided)

    def action_new_runner(self) -> None:
        self.app.push_screen(RunnerAddModal(), lambda _: self._reload())

    def action_close(self) -> None:
        self.dismiss(None)


def new_runner_chip() -> str:
    """The shared "define a custom agent" affordance: one footer-grammar pill (the key
    hint IS the click target) every runner picker places right beside its options."""
    return tui_footer.chip("screen.new_runner", "Ctrl+N", gettext("New agent…"))
