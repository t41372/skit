"""Runner (agent) UI shared by every surface: the pick list, the add/edit modal, and
the management screen.

Every surface that picks a runner (the run form, the prompt add review, Script
settings) uses a value-keyed Select dropdown and mounts the same Ctrl+N chip that
opens RunnerAddModal — the zero-memorization twin of `skit runner add NAME COMMAND…`. The
command is typed as one line and split into argv with platform-appropriate shlex rules
(quotes group words; Windows backslashes stay literal); no shell is ever involved at
launch, the split happens exactly once, here, and the tokens go into config verbatim.
RunnerManageScreen (reached from Preferences) is where the whole registry is visible:
list, edit, remove — never an add-only door.
"""

from __future__ import annotations

from typing import cast, override

from rich.markup import escape
from textual import on
from textual.app import ComposeResult
from textual.binding import Binding
from textual.containers import Vertical
from textual.screen import ModalScreen, Screen
from textual.widgets import Input, Label, OptionList, Static
from textual.widgets.option_list import Option

from . import argv_text, config, store, tui_footer
from .i18n import gettext, ngettext


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


def _row_reason_text(row: config.PromptRunnerRow) -> str:
    """Explain one raw invalid config row in the management surface."""
    return config.prompt_runner_row_reason(row)


class RunnerAddModal(ModalScreen[str | None]):
    """Register (or edit) a custom agent from any runner-picking surface.

    Dismisses with the saved runner's name after writing it into config (seeding the
    presets first, so the row lands next to visible, editable ones), or None on
    cancel. Validation is the same rule the CLI enforces — the two doors write the
    same config rows. Pass ``editing`` to open prefilled on an existing runner: the
    stable name is read-only (prompt pins key off it), and saving replaces that row's
    command in place."""

    AUTO_FOCUS = "Input"
    BINDINGS = [
        *tui_footer.FIELD_NAV_BINDINGS,
        Binding("escape", "cancel", gettext("Cancel")),
    ]
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

    def __init__(
        self,
        editing: str | None = None,
        *,
        initial_argv: tuple[str, ...] | None = None,
        selected_raw: bool = False,
        expected_rows: list[config.PromptRunnerRow] | None = None,
        repair_row: config.PromptRunnerRow | None = None,
    ) -> None:
        super().__init__()
        self._editing: str | None = editing
        self._initial_argv: tuple[str, ...] | None = initial_argv
        self._selected_raw: bool = selected_raw
        self._expected_rows: list[config.PromptRunnerRow] | None = expected_rows
        self._repair_row: config.PromptRunnerRow | None = repair_row

    @override
    def compose(self) -> ComposeResult:
        original = (
            config.find_prompt_runner(self._editing)
            if self._editing and not self._selected_raw
            else None
        )
        editing = self._editing is not None or self._repair_row is not None
        with Vertical():
            yield Label(
                gettext("Edit agent (runner)") if editing else gettext("New agent (runner)")
            )
            yield Input(
                value=original.name if original else (self._editing or ""),
                placeholder=gettext("Name, e.g. aider"),
                id="runner-add-name",
                disabled=self._editing is not None,
            )
            yield Input(
                value=argv_text.join(original.argv if original else (self._initial_argv or ())),
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
                    tui_footer.nav_chip(),
                ),
                id="runner-add-footer",
                markup=True,
            )

    def _error(self, message: str) -> None:
        self.query_one("#runner-add-error", Static).update(f"[red]{escape(message)}[/red]")

    @on(Input.Submitted)
    def _submitted(self, event: Input.Submitted) -> None:
        self.action_save_runner()

    def action_save_runner(self) -> None:
        """Enter / the Save chip: validate, persist, dismiss with the saved name."""
        # Runner names are stable config keys: prompt entries persist the name as their
        # pin. The disabled input is the visible contract; preferring `_editing` here is
        # the defensive half, so a synthetic event cannot orphan every existing pin.
        name = (
            self._editing
            if self._editing is not None
            else self.query_one("#runner-add-name", Input).value.strip()
        )
        command = self.query_one("#runner-add-command", Input).value.strip()
        if not name:
            self._error(gettext("A name is required."))
            self.query_one("#runner-add-name", Input).focus()
            return
        try:
            argv = argv_text.split(command)
        except ValueError:
            self._error(gettext("Unbalanced quotes in the command."))
            return
        reason = config.validate_prompt_runner_argv(argv)
        if reason is not None:
            self._error(_reason_text(reason))
            return
        new_row = config.PromptRunner(name, tuple(argv))
        try:
            if self._repair_row is not None:
                config.replace_prompt_runner_row(
                    cast(int, self._repair_row.index), new_row, expected=self._repair_row
                )
            else:
                config.set_prompt_runner(
                    new_row,
                    replace_existing=self._editing is not None,
                    expected=self._expected_rows,
                )
        except config.PromptRunnerExistsError:
            self._error(
                gettext("The runner %(name)s already exists — pick another name.") % {"name": name}
            )
            return
        except config.PromptRunnerChangedError:
            self._error(
                gettext(
                    "Runner config changed; this edit was not saved. Cancel and select the row again."
                )
            )
            return
        except config.PromptRunnerConfigError as exc:
            self._error(str(exc))
            return
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

    def __init__(
        self,
        name: str,
        *,
        row_index: int | None = None,
        invalid_reason: str | None = None,
        argv: tuple[str, ...] | None = None,
        editable: bool = True,
        selected_raw: bool = False,
    ) -> None:
        super().__init__()
        self._name: str = name
        self._row_index: int | None = row_index
        self._invalid_reason: str | None = invalid_reason
        self._argv: tuple[str, ...] | None = argv
        self._editable: bool = editable
        self._selected_raw: bool = selected_raw

    @override
    def compose(self) -> ComposeResult:
        runner = None if self._selected_raw else config.find_prompt_runner(self._name)
        command = argv_text.join(runner.argv if runner else (self._argv or ()))
        with Vertical():
            yield Label(escape(self._name))
            yield Static(escape(command))
            if self._invalid_reason:
                row = config.PromptRunnerRow(
                    self._row_index,
                    self._name,
                    self._argv,
                    self._invalid_reason,
                    self._name,
                )
                yield Static(f"[red]⚠ {escape(_row_reason_text(row))}[/red]", markup=True)
            chips = []
            if self._editable:
                chips.append(tui_footer.chip("screen.edit", "e", gettext("Edit")))
            chips.extend(
                (
                    tui_footer.chip("screen.remove", "d", gettext("Remove")),
                    tui_footer.chip("screen.cancel", "Esc", gettext("Back")),
                )
            )
            yield Static(
                tui_footer.bar(*chips),
                markup=True,
            )

    def action_edit(self) -> None:
        if self._editable:
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

    def __init__(
        self,
        name: str,
        pinned_count: int = 0,
        *,
        invalid_row: bool = False,
        container: bool = False,
    ) -> None:
        super().__init__()
        self._name: str = name
        self._pinned_count: int = pinned_count
        self._invalid_row: bool = invalid_row
        self._container: bool = container

    @override
    def compose(self) -> ComposeResult:
        with Vertical():
            if self._container:
                question = gettext("Remove the malformed prompt runner container?")
            elif self._invalid_row:
                question = gettext('Remove malformed runner row "%(name)s"?') % {
                    "name": escape(self._name)
                }
            else:
                question = gettext('Remove the agent "%(name)s"?') % {"name": escape(self._name)}
            yield Label(question)
            if self._pinned_count:
                yield Static(
                    ngettext(
                        "%(count)d prompt pins this runner and will need another runner before it can run again.",
                        "%(count)d prompts pin this runner and will need another runner before they can run again.",
                        self._pinned_count,
                    )
                    % {"count": self._pinned_count}
                )
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
    RunnerManageScreen #rm-error { height: auto; color: $error; }
    RunnerManageScreen OptionList { height: auto; border: none; }
    RunnerManageScreen KeysBar { dock: bottom; }
    RunnerManageScreen #rm-keys { color: $text-muted; }
    """

    def __init__(self) -> None:
        super().__init__()
        self._rows_by_id: dict[str, config.PromptRunnerRow] = {}

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
        rows = config.prompt_runner_rows()
        self._rows_by_id = {}
        for display_index, row in enumerate(rows):
            # Textual ids live entirely in the UI's namespace.  A runner name is
            # user-controlled data and may legally equal any prefix we invent, so it
            # must never double as a widget/option id.
            option_id = f"runner-row-{display_index}"
            if row.invalid_reason is None:
                prompt = f"{escape(row.name)}  [dim]{escape(argv_text.join(row.argv or ()))}[/dim]"
            else:
                label = row.name or row.descriptor
                command = argv_text.join(row.argv) if row.argv is not None else ""
                command_text = f"  [dim]{escape(command)}[/dim]" if command else ""
                prompt = (
                    f"⚠ {escape(label)}  [red]{escape(_row_reason_text(row))}[/red]{command_text}"
                )
            self._rows_by_id[option_id] = row
            option_list.add_option(Option(prompt, id=option_id))
        empty = self.query_one("#rm-empty", Static)
        empty.display = not rows

    @override
    def compose(self) -> ComposeResult:
        with tui_footer.FormBody(id="rm-body"):
            yield Static(
                gettext("The agents prompt entries run with. Pick one to edit or remove it."),
                classes="hint",
            )
            yield OptionList()
            yield Static("", id="rm-error")
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
        row = self._rows_by_id[str(event.option.id)]
        name = row.name or row.descriptor
        key_rows = [
            candidate for candidate in self._rows_by_id.values() if candidate.name == row.name
        ]

        def _decided(action: str | None) -> None:
            if action == "edit":
                modal = (
                    RunnerAddModal(
                        editing=row.name,
                        initial_argv=row.argv,
                        selected_raw=True,
                        expected_rows=key_rows,
                    )
                    if row.name
                    else RunnerAddModal(
                        initial_argv=row.argv,
                        selected_raw=True,
                        repair_row=row,
                    )
                )
                self.app.push_screen(modal, lambda _: self._reload())
            elif action == "remove":

                def _confirmed(really: bool | None) -> None:
                    if really:
                        if row.invalid_reason is None:
                            # The valid row represents the stable runner-name key.  A
                            # hand-edited duplicate must not spring to life immediately
                            # after the user removes that agent, so key removal coalesces
                            # every raw row for the name. Invalid-row removal below stays
                            # exact: it is config repair, not removal of the active key.
                            removed = config.remove_prompt_runner(row.name, expected=key_rows)
                        else:
                            removed = config.remove_prompt_runner_row(row.index, expected=row)
                        error = self.query_one("#rm-error", Static)
                        if removed:
                            error.update("")
                        else:
                            error.update(
                                gettext(
                                    "Runner config changed; nothing was removed. Select the row again."
                                )
                            )
                        self._reload()

                pinned_count = (
                    len(store.prompt_entries_pinned_to(row.name))
                    if row.invalid_reason is None
                    else 0
                )
                self.app.push_screen(
                    RunnerRemoveConfirm(
                        name,
                        pinned_count,
                        invalid_row=row.invalid_reason is not None,
                        container=row.index is None,
                    ),
                    _confirmed,
                )

        self.app.push_screen(
            RunnerActionModal(
                name,
                row_index=row.index,
                invalid_reason=row.invalid_reason,
                argv=row.argv,
                editable=row.argv is not None and row.index is not None,
                selected_raw=True,
            ),
            _decided,
        )

    def action_new_runner(self) -> None:
        self.app.push_screen(RunnerAddModal(), lambda _: self._reload())

    def action_close(self) -> None:
        self.dismiss(None)


def new_runner_chip() -> str:
    """The shared "define a custom agent" affordance: one footer-grammar pill (the key
    hint IS the click target) every runner picker places right beside its options."""
    return tui_footer.chip("screen.new_runner", "Ctrl+N", gettext("New agent…"))
