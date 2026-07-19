"""Script settings (p): the merged per-script management screen — basics, parameters,
presets, dependencies in one place.

Enter saves everything in one atomic [tool.skit] rewrite; Esc asks when there are
unsaved changes. Reference-mode entries show the parameters read-only (skit never
writes the original file, A7); command entries show the template and placeholders.
"""

from __future__ import annotations

from pathlib import Path
from typing import override

from rich.markup import escape
from textual import on
from textual.app import ComposeResult
from textual.binding import Binding
from textual.containers import Horizontal, Vertical, VerticalScroll
from textual.screen import ModalScreen, Screen
from textual.widgets import Checkbox, Input, Label, RadioButton, RadioSet, Select, Static

from . import analysis, argstate, config, params, pep723, store, tui_footer, tui_runner
from .i18n import gettext
from .langs.registry import spec_for
from .models import Entry
from .params import ParamDecl


class DiscardChangesModal(ModalScreen[bool]):
    BINDINGS = [
        Binding("y", "discard", gettext("Discard")),
        Binding("escape,n", "keep", gettext("Keep editing")),
    ]
    DEFAULT_CSS = """
    DiscardChangesModal { align: center middle; }
    DiscardChangesModal > Vertical { border: round $accent; padding: 1 2; width: auto;
        max-width: 100%; height: auto; max-height: 100%; background: $background; }
    DiscardChangesModal Static { margin: 1 0 0 0; width: auto; }
    """

    @override
    def compose(self) -> ComposeResult:
        with Vertical():
            yield Label(gettext("Discard unsaved changes?"))
            yield Static(
                tui_footer.bar(
                    tui_footer.chip("screen.discard", "y", gettext("Discard")),
                    tui_footer.chip("screen.keep", "Esc", gettext("Keep editing")),
                ),
                markup=True,
            )

    def action_discard(self) -> None:
        self.dismiss(True)

    def action_keep(self) -> None:
        self.dismiss(False)


class ParamRow(Vertical):
    """One managed parameter: keep/unmanage, prompt text, secret + env source."""

    DEFAULT_CSS = """
    ParamRow { height: auto; margin: 0 0 1 2; }
    ParamRow .p-meta { color: $text-muted; width: auto; }
    ParamRow Horizontal { height: auto; }
    ParamRow Horizontal > Checkbox { width: auto; }
    ParamRow Input { width: 1fr; }
    """

    def __init__(self, spec: ParamDecl) -> None:
        super().__init__()
        self.spec: ParamDecl = spec

    @override
    def compose(self) -> ComposeResult:
        s = self.spec
        default = "" if s.default is None else repr(s.default)
        yield Checkbox(f"{escape(s.name)}  [dim]{s.type} {escape(default)}[/dim]", value=True)
        with Horizontal():
            yield Static("  " + gettext("Form label:"), classes="p-meta")
            yield Input(value=s.prompt, placeholder=s.name, classes="p-prompt")
        with Horizontal():
            yield Checkbox(
                gettext("secret (never saved to disk)"), value=s.secret, classes="p-secret"
            )
            yield Input(
                value=s.env_source,
                placeholder=gettext("env variable to read it from (optional)"),
                classes="p-env",
            )
        yield Static("", classes="p-note")

    def collect(self) -> ParamDecl | None:
        """None when unmanaged (checkbox off)."""
        if not self.query_one(Checkbox).value:
            return None
        s = self.spec
        s.prompt = self.query_one(".p-prompt", Input).value.strip()
        s.secret = self.query_one(".p-secret", Checkbox).value
        s.env_source = self.query_one(".p-env", Input).value.strip() if s.secret else ""
        return s

    @on(Checkbox.Changed, ".p-secret")
    def _secret_note(self, event: Checkbox.Changed) -> None:
        note = self.query_one(".p-note", Static)
        if event.value and not self.spec.secret:
            note.update(
                f"[yellow]{gettext('Anything skit previously remembered for this value will be deleted too.')}[/yellow]"
            )
        else:
            note.update("")


def _default_text(value: str | int | float | bool) -> str:
    """A declared default rendered for an editable Input (bool as the true/false words
    coerce_default round-trips, everything else as its plain string)."""
    if isinstance(value, bool):
        return "true" if value else "false"
    return str(value)


class DeclParamRow(Vertical):
    """One declared parameter (exe/command entries): keep/remove, type, default, flag
    (binary kinds only), required, form label, secret + env source. Its delivery is fixed
    at add time and shown read-only in the header (the CLI's --deliver changes it)."""

    DEFAULT_CSS = """
    DeclParamRow { height: auto; margin: 0 0 1 2; }
    DeclParamRow .p-meta { color: $text-muted; width: auto; }
    DeclParamRow Horizontal { height: auto; }
    DeclParamRow Horizontal > Checkbox { width: auto; }
    DeclParamRow Input { width: 1fr; }
    """

    def __init__(self, decl: ParamDecl, *, show_flag: bool) -> None:
        super().__init__()
        self.decl: ParamDecl = decl
        self._show_flag: bool = show_flag

    @override
    def compose(self) -> ComposeResult:
        d = self.decl
        default = "" if d.default is None else _default_text(d.default)
        yield Checkbox(
            f"{escape(d.name)}  [dim]{escape(d.delivery)}[/dim]", value=True, classes="d-keep"
        )
        with Horizontal():
            yield Static("  " + gettext("Type:"), classes="p-meta")
            yield Input(
                value=d.type,
                placeholder=gettext("type: str / int / float / bool / choice"),
                classes="d-type",
            )
        with Horizontal():
            yield Static("  " + gettext("Default:"), classes="p-meta")
            yield Input(
                value=default,
                placeholder=gettext("default value (optional)"),
                classes="d-default",
            )
        with Horizontal():
            yield Static("  " + gettext("Choices:"), classes="p-meta")
            yield Input(
                value=", ".join(str(c) for c in d.choices or []),
                placeholder=gettext("comma separated (for type: choice)"),
                classes="d-choices",
            )
        with Horizontal():
            yield Static("  " + gettext("Help:"), classes="p-meta")
            yield Input(
                value=d.help,
                placeholder=gettext("one-line help shown under the field (optional)"),
                classes="d-help",
            )
        if self._show_flag:
            with Horizontal():
                yield Static("  " + gettext("Flag:"), classes="p-meta")
                yield Input(
                    value=d.flag,
                    placeholder=gettext("--flag (empty = positional)"),
                    classes="d-flag",
                )
        yield Checkbox(gettext("required"), value=d.required, classes="d-required")
        with Horizontal():
            yield Static("  " + gettext("Form label:"), classes="p-meta")
            yield Input(value=d.prompt, placeholder=d.name, classes="p-prompt")
        with Horizontal():
            yield Checkbox(
                gettext("secret (never saved to disk)"), value=d.secret, classes="p-secret"
            )
            yield Input(
                value=d.env_source,
                placeholder=gettext("env variable to read it from (optional)"),
                classes="p-env",
            )
        yield Static("", classes="p-note")

    @property
    def type_text(self) -> str:
        """The raw text in the type field (validated by the screen, which owns the ParamType
        literal narrowing, so an invalid type can be rejected rather than silently coerced)."""
        return self.query_one(".d-type", Input).value.strip() or "str"

    def collect(self) -> ParamDecl | None:
        """None when removed (keep checkbox off). Gathers every field EXCEPT the type onto a
        copy of the decl; the screen reads `type_text`, validates it, and sets the type."""
        if not self.query_one(".d-keep", Checkbox).value:
            return None
        d = self.decl
        d.default = self.query_one(".d-default", Input).value.strip() or None
        d.choices = tuple(
            c.strip() for c in self.query_one(".d-choices", Input).value.split(",") if c.strip()
        )
        d.help = self.query_one(".d-help", Input).value.strip()
        if self._show_flag:
            d.flag = self.query_one(".d-flag", Input).value.strip()
        d.required = self.query_one(".d-required", Checkbox).value
        d.prompt = self.query_one(".p-prompt", Input).value.strip()
        d.secret = self.query_one(".p-secret", Checkbox).value
        d.env_source = self.query_one(".p-env", Input).value.strip() if d.secret else ""
        return d

    @on(Checkbox.Changed, ".p-secret")
    def _secret_note(self, event: Checkbox.Changed) -> None:
        note = self.query_one(".p-note", Static)
        if event.value and not self.decl.secret:
            note.update(
                f"[yellow]{gettext('Anything skit previously remembered for this value will be deleted too.')}[/yellow]"
            )
        else:
            note.update("")


class ScriptSettingsScreen(Screen[bool]):
    """Four sections in one screen; `s` in the Library deep-links to Presets."""

    BINDINGS = [
        Binding("escape", "close", gettext("Back")),
        Binding("ctrl+s", "save", gettext("Save"), priority=True),
        Binding("ctrl+r", "resync", gettext("Resync"), priority=True),
        Binding("ctrl+n", "new_runner", gettext("New agent"), show=False, priority=True),
        *tui_footer.FIELD_NAV_BINDINGS,
    ]
    # Boot on the name field, not the "*" pick (the body scroll container).
    AUTO_FOCUS = "Input, Checkbox"
    DEFAULT_CSS = """
    ScriptSettingsScreen #st-body {
        padding: 0 1;
        border: round $skit-box-indigo;
        border-title-color: ansi_bright_white;
        border-title-style: bold;
    }
    ScriptSettingsScreen .section { color: $accent; margin: 1 0 0 0; }
    ScriptSettingsScreen .hint { color: $text-muted; }
    ScriptSettingsScreen KeysBar { dock: bottom; }
    ScriptSettingsScreen #st-keys { color: $text-muted; }
    """

    def __init__(self, entry: Entry, initial_section: str = "") -> None:
        super().__init__()
        self._entry: Entry = entry
        self._initial: str = initial_section
        self._dirty: bool = False
        # Widgets can emit Changed while the screen is still composing (initial values
        # settling); only user edits after mount count as dirt.
        self._dirt_armed: bool = False
        self._text: str = ""
        self._spec = spec_for(entry.meta.kind)
        params_io = self._spec.params_io if self._spec is not None else None
        if params_io is not None and entry.script_path.exists():
            self._text = entry.script_path.read_text(encoding="utf-8", errors="replace")
        # The ENTRY'S OWN params_io — never Python's. shell/fish carry their block in the same
        # '#' engine, but JS/TS carry it behind '//', so the Python reader would return [] for a
        # perfectly valid managed JS entry (TUI↔CLI parity: cli._edit_params already routes this way).
        self._specs: list[ParamDecl] = params_io.read(self._text) if params_io is not None else []
        # Every kind whose schema lives in meta.toml [[parameters]] — exe, command, AND the
        # interpreted kinds with no in-file block (ruby/perl/lua/r/powershell) — is edited through
        # the declared-params editor rather than the analyzer-driven [tool.skit] flow above.
        self._declared: bool = self._spec is not None and self._spec.params_io is None
        self._is_prompt: bool = entry.meta.kind == "prompt"
        # Value-keyed option list behind the workdir radio (kind-aware; "custom" last).
        self._workdir_choices: list[str] = []
        # The preset names as composed — the delete pass maps checkbox indices
        # through THIS list, never a fresh state read (a preset added/deleted
        # mid-session must not shift which name an untick deletes).
        self._preset_names: list[str] = []
        if self._is_prompt:
            # Every MANAGED placeholder gets an editable row (undeclared ones as their
            # synthesized schema), plus declared env riders — the exact field list the
            # run form serves, so what you edit is what you get.
            self._declared_decls: list[ParamDecl] = params.declared_for_template(
                entry.meta.parameters, entry.meta.params or []
            )
        else:
            self._declared_decls = (
                params.declared_from_meta(entry.meta.parameters) if self._declared else []
            )
        self._prompt_body_names: list[str] = []
        if self._is_prompt and not entry.meta.interpolate:
            self._declared_decls = []
        elif self._is_prompt and entry.script_path.exists():
            from .langs.prompt import analyzer as prompt_analyzer

            self._prompt_body_names = prompt_analyzer.placeholder_names(
                entry.script_path.read_text(encoding="utf-8", errors="replace")
            )
        # The resync outcome (incl. safety-rebind warnings) must survive the recompose that
        # action_resync triggers — a widget updated in place would be thrown away and rebuilt
        # empty. Kept on the instance so compose can re-emit it. Already escape()'d for markup.
        self._resync_report: str = ""

    def _reconcile(self) -> analysis.Report | None:
        """Reconcile the stored definitions against the script — through the ENTRY'S OWN analyzer.

        Hardcoding Python's here silently broke every other analyzable kind: the Python analyzer
        raises SyntaxError on shell/JS/fish source, so the screen showed zero detected candidates
        and reported a valid script as unparseable. None when the kind has no analyzer, or when
        there is no text to analyze."""
        analyzer = self._spec.analyzer if self._spec is not None else None
        if analyzer is None or not self._text:
            return None
        return analyzer.reconcile(self._text, self._specs)

    @override
    def compose(self) -> ComposeResult:
        with tui_footer.FormBody(id="st-body"):
            yield Static(gettext("Basics"), classes="section")
            yield Input(value=self._entry.meta.name, id="st-name")
            yield Static(
                gettext("Renaming keeps everything — remembered values, presets, the stored copy."),
                classes="hint",
            )
            yield Input(
                value=self._entry.meta.description,
                placeholder=gettext("Description (shown in the Library)"),
                id="st-desc",
            )
            yield from self._compose_storage()
            yield from self._compose_launch()
            yield from self._compose_runner()
            yield from self._compose_params()
            yield from self._compose_presets()
            yield from self._compose_deps()
            yield from self._compose_needs()
        chips = [tui_footer.chip("screen.save", "Ctrl+S", gettext("Save"))]
        if (
            self._spec is not None
            and self._spec.analyzer is not None
            and self._entry.meta.mode == "copy"
        ):
            # The same guard action_resync applies: advertising a key that silently
            # no-ops (prompt/exe/command/reference entries) teaches a dead chord.
            chips.append(tui_footer.chip("screen.resync", "Ctrl+R", gettext("Resync")))
        chips += [
            tui_footer.chip("screen.close", "Esc", gettext("Back")),
            tui_footer.nav_chip(),
        ]
        yield tui_footer.KeysBar(Static(tui_footer.bar(*chips), id="st-keys", markup=True))

    def _compose_storage(self) -> ComposeResult:
        meta = self._entry.meta
        if self._spec is None or not self._spec.supports_modes:
            return
        yield Static(gettext("Storage"), classes="section")
        if meta.mode == "copy":
            yield Static(
                gettext("Keep a copy — your original file is never modified. Source: %(path)s")
                % {"path": escape(meta.source)},
                classes="hint",
            )
        else:
            yield Static(
                gettext("Linked to the original: %(path)s") % {"path": escape(meta.source)},
                classes="hint",
            )

    def _compose_launch(self) -> ComposeResult:
        """Where the entry runs, and (for interpreted kinds) what runs it — launcher
        policies the product previously implemented in full but exposed NOWHERE: the
        only way to change them was hand-editing meta.toml."""
        meta = self._entry.meta
        if self._spec is None:
            return  # unknown kind: don't offer policies a newer skit defined
        yield Static(gettext("Run in (working directory)"), classes="section")
        # Kind-aware options: a command template has no "own folder" (no file), and a
        # reference-only exe has no stored copy — offering either would be a label
        # that reads as one thing and silently resolves as another.
        choices: list[tuple[str, str]] = []
        if self._spec.has_original_file:
            choices.append((gettext("The script's own folder"), "origin"))
        if self._spec.stored_name:
            choices.append((gettext("skit's stored-copy folder"), "store"))
        choices.append((gettext("Wherever skit is run from"), "invoke"))
        choices.append((gettext("A fixed folder (type it below)"), "custom"))
        self._workdir_choices = [value for _, value in choices]
        known_values = {value for _, value in choices if value != "custom"}
        custom = meta.workdir not in known_values
        with RadioSet(id="st-workdir"):
            for label, value in choices:
                yield RadioButton(
                    label, value=(meta.workdir == value if value != "custom" else custom)
                )
        yield Input(
            value=meta.workdir if custom else "",
            placeholder=gettext("/absolute/path"),
            id="st-workdir-path",
        )
        if self._spec.family == "interpreted" and meta.kind not in ("python", "prompt"):
            yield Static(gettext("Interpreter / runtime"), classes="section")
            yield Input(
                value=meta.interpreter,
                placeholder=gettext("empty = automatic (shebang, then detection order)"),
                id="st-interpreter",
            )

    @on(RadioSet.Changed, "#st-workdir")
    def _workdir_changed(self, event: RadioSet.Changed) -> None:
        self._toggle_workdir_path()

    def _toggle_workdir_path(self) -> None:
        box = self.query("#st-workdir")
        if box:
            pressed = box.first(RadioSet).pressed_index
            is_custom = (
                0 <= pressed < len(self._workdir_choices)
                and self._workdir_choices[pressed] == "custom"
            )
            self.query_one("#st-workdir-path", Input).display = is_custom

    def _validated_launch(self) -> tuple[str, str] | None:
        """The validation half: (workdir, interpreter) to persist, or None on invalid
        input — computed and CHECKED before any write anywhere in the save."""
        box = self.query("#st-workdir")
        if not box:
            return ("", "")
        pressed = box.first(RadioSet).pressed_index
        picked = (
            self._workdir_choices[pressed]
            if 0 <= pressed < len(self._workdir_choices)
            else "custom"
        )
        if picked != "custom":
            new_workdir = picked
        else:
            new_workdir = self.query_one("#st-workdir-path", Input).value.strip()
            if not new_workdir:
                # "Fixed folder" picked but nothing typed: keep what is stored rather
                # than guessing (an empty path is not a policy).
                new_workdir = self._entry.meta.workdir
            if new_workdir not in ("origin", "store", "invoke") and not (
                Path(new_workdir).expanduser().is_absolute()
            ):
                # The same rule store.write_workdir enforces, checked BEFORE any write.
                self.notify(
                    gettext(
                        "The working directory must be origin, store, invoke, or an absolute path."
                    ),
                    severity="error",
                )
                return None
        interp = ""
        interp_box = self.query("#st-interpreter")
        if interp_box:
            interp = interp_box.first(Input).value.strip()
        return (new_workdir, interp)

    def _write_launch(self, launch: tuple[str, str]) -> None:
        """The write half — inputs already validated."""
        new_workdir, new_interp = launch
        box = self.query("#st-workdir")
        if not box:
            return
        if new_workdir != self._entry.meta.workdir:
            store.write_workdir(self._entry.slug, new_workdir)
        if self.query("#st-interpreter") and new_interp != self._entry.meta.interpreter:
            store.write_interpreter(self._entry.slug, new_interp)

    def _compose_runner(self) -> ComposeResult:
        if not self._is_prompt:
            return
        yield Static(gettext("Runner (the agent this prompt runs with)"), classes="section")
        names = [r.name for r in config.load_prompt_runners()]
        pin = self._entry.meta.runner
        # A pin whose runner row was removed from config still gets an option — and it
        # stays SELECTED, so opening settings and saving something unrelated never
        # silently clears it (an unrequested data change). Picking another option (or
        # "ask on the run form") is the explicit way out.
        stale_pin = pin and all(name != pin for name in names)
        # A VALUE-keyed dropdown: the save path reads Select.value, so a runner list
        # that changed while the screen was open can never shift an index mapping.
        options: list[tuple[str, str]] = [(gettext("ask on the run form"), "")]
        if stale_pin:
            options.append((gettext("%(runner)s (no longer configured)") % {"runner": pin}, pin))
        options += [(name, name) for name in names]
        yield Select(options, value=pin, allow_blank=False, id="st-runner-select")
        # Custom agents are first-class: the picker always carries the door to define
        # one (footer grammar — the key hint IS the click target), even when the
        # configured list was deliberately emptied.
        yield Static(tui_runner.new_runner_chip(), id="st-runner-new", markup=True)

    def action_new_runner(self) -> None:
        """Ctrl+N / the New agent… chip: define a custom runner right from settings —
        it lands in config, joins the picker, and is selected, ready to pin on save."""
        if not self._is_prompt:
            return

        def _added(name: str | None) -> None:
            if not name:
                return
            # Rebuild exactly as compose does — the new runner is in config now.
            names = [r.name for r in config.load_prompt_runners()]
            pin = self._entry.meta.runner
            options: list[tuple[str, str]] = [(gettext("ask on the run form"), "")]
            if pin and pin not in names:
                options.append(
                    (gettext("%(runner)s (no longer configured)") % {"runner": pin}, pin)
                )
            options += [(n, n) for n in names]
            select = self.query_one("#st-runner-select", Select)
            select.set_options(options)
            select.value = name

        self.app.push_screen(tui_runner.RunnerAddModal(), _added)

    def _compose_params(self) -> ComposeResult:
        yield Static(gettext("Parameters (the run form's fields)"), classes="section")
        meta = self._entry.meta
        if self._declared:
            yield from self._compose_declared_editor()
            return
        if self._spec is None or self._spec.analyzer is None:
            yield Static(gettext("(programs have no managed parameters)"), classes="hint")
            return
        if meta.mode == "reference":
            yield Static(
                gettext(
                    "skit doesn't write to this file — maintain the [tool.skit] definitions "
                    "in the source directly."
                ),
                classes="hint",
            )
            for s in self._specs:
                yield Static(f"· {escape(s.name)} ({s.type})")
            return
        for s in self._specs:
            yield ParamRow(s)
        if self._cli_driven():
            # This script's form already comes from its own argparse/click/typer surface.
            # Managing a hardcoded constant would write a [tool.skit] block that shadows
            # that whole form (plan_for_entry prefers managed params) — a source-flip
            # trap. Explain instead of offering the manage-these-constants checkboxes.
            yield Static(
                gettext(
                    "This script's run form comes from its own command-line arguments. "
                    "Managing a hardcoded constant here would replace that form — leave it as is."
                ),
                classes="hint",
            )
        else:
            report = self._reconcile()
            if report is not None and report.new:
                yield Static(
                    gettext("Detected but not yet managed — tick to manage:"), classes="hint"
                )
                for i, c in enumerate(report.new):
                    yield Checkbox(
                        f"{escape(c.name)}  [dim]{c.type} = {escape(repr(c.default))}[/dim]",
                        value=False,
                        id=f"st-new-{i}",
                    )
        if self._specs and all(s.binding == "input" for s in self._specs):
            yield Static(
                gettext("Every input() is managed — this script can now run with --no-input."),
                classes="hint",
            )
        yield Static(self._resync_report, id="st-resync-report", classes="hint", markup=True)

    def _compose_declared_editor(self) -> ComposeResult:
        """The exe/command declared-schema editor: one row per declared parameter, plus an
        add-a-parameter field. A template shows its command line read-only above the rows
        (the placeholders it names are visible there); a declared row overrides a
        placeholder's schema or rides along as an env variable."""
        meta = self._entry.meta
        if self._spec is not None and self._spec.family == "template":
            # Editable — the template IS the program; freezing it forever while every
            # other kind's payload has `skit edit` forced remove + re-add over a typo.
            yield Input(value=meta.template, id="st-template")
            yield Static(
                gettext("Saving re-reads the {placeholders} from the template."),
                classes="hint",
            )
        # A flag only means something where argv is the interface: every kind whose form
        # is NOT placeholders (binaries AND the interpreted meta-schema kinds), mirroring
        # the CLI's allowed deliveries. The trait — not the family — is the gate, so a
        # prompt (family "interpreted") never grows a meaningless flag input.
        show_flag = self._spec is not None and not self._spec.placeholder_params
        if self._is_prompt:
            # The per-prompt master switch: one click + Save turns insertion off outright
            # (the escape hatch for long prompts full of {{lookalikes}}); the managed
            # list survives underneath for a later switch-on.
            yield Checkbox(
                gettext("Variable insertion ({{name}} placeholders become form fields)"),
                value=meta.interpolate,
                id="st-interpolate",
            )
            if not meta.interpolate:
                yield Static(
                    gettext("Off — the body travels to the agent exactly as written."),
                    classes="hint",
                )
                return
        for d in self._declared_decls:
            yield DeclParamRow(d, show_flag=show_flag)
        if self._is_prompt:
            from .langs.prompt import analyzer as prompt_analyzer

            unmanaged = [n for n in self._prompt_body_names if n not in (meta.params or [])]
            if unmanaged:
                yield Static(
                    gettext("Detected but not yet managed — tick to manage:"), classes="hint"
                )
                for i, candidate in enumerate(unmanaged[: prompt_analyzer.LIST_PREVIEW_LIMIT]):
                    yield Checkbox(escape(candidate), value=False, id=f"st-prompt-new-{i}")
                if len(unmanaged) > prompt_analyzer.LIST_PREVIEW_LIMIT:
                    yield Static(
                        gettext(
                            "…and %(count)s more (manage them with: skit params %(name)s --add NAME)"
                        )
                        % {
                            "count": len(unmanaged) - prompt_analyzer.LIST_PREVIEW_LIMIT,
                            "name": escape(meta.name),
                        },
                        classes="hint",
                    )
        yield Static(gettext("Add a parameter — type a name, then Save:"), classes="hint")
        yield Input(placeholder=gettext("new parameter name"), id="st-add-param")

    def _new_declared(self, name: str) -> ParamDecl:
        """A freshly-added declared parameter's default shape. A template placeholder stays
        required (an empty slot silently assembles a broken command); everything else is an
        optional env/flag value that falls back to the program's own default."""
        placeholders = self._prompt_body_names if self._is_prompt else self._entry.meta.params or []
        if self._spec is not None and self._spec.placeholder_params:
            if name in placeholders:
                return ParamDecl(name=name, binding="none", delivery="placeholder", required=True)
            return ParamDecl(name=name, binding="none", delivery="env")
        return ParamDecl(name=name, binding="none", delivery="flag")

    def _collect_declared(self) -> list[ParamDecl] | None:
        """Gather every declared row (+ the add-a-parameter field) into a validated decl
        list, or None when a row is invalid — a notify() explains and the save is aborted so
        an inconsistent schema is never written."""
        out: list[ParamDecl] = []
        seen: set[str] = set()
        for row in self.query(DeclParamRow):
            d = row.collect()
            if d is None:
                continue
            checked = self._validate_declared(d, row.type_text)
            if checked is None:
                return None
            out.append(checked)
            seen.add(checked.name)
        new_name = self.query_one("#st-add-param", Input).value.strip()
        if new_name and new_name not in seen:
            out.append(self._new_declared(new_name))
        return out

    def _validate_declared(self, d: ParamDecl, type_text: str) -> ParamDecl | None:
        """Validate one collected row's type/default/invariants; notify + return None on the
        first problem (the screen aborts the save)."""
        param_type = params.as_param_type(type_text)
        if param_type is None:
            self.notify(
                gettext("%(name)s has an unknown type — use str, int, float, bool, or choice.")
                % {"name": d.name},
                severity="error",
            )
            return None
        d.type = param_type
        if d.default is not None:
            try:
                d.default = params.coerce_default(str(d.default), d.type)
            except ValueError:
                self.notify(
                    gettext("%(name)s: the default doesn't match its type.") % {"name": d.name},
                    severity="error",
                )
                return None
        normalized = params.normalize(d)
        if params.validate_invariants(normalized) is not None:
            self.notify(
                gettext(
                    "%(name)s is a choice parameter but has no choices — fill its Choices field."
                )
                % {"name": d.name},
                severity="error",
            )
            return None
        return normalized

    def _cli_driven(self) -> bool:
        """Whether the run form currently comes from a MODELED read of the script's own
        CLI surface — i.e. nothing is managed yet AND the entry's own reader models a
        form (flows.reader_fields, the one trap predicate every surface shares). Once
        anything is managed, plan_for_entry already serves the injected form; and for
        self-parsing skit couldn't model (docopt/fire, a dynamic optstring) the form is
        passthrough-only, so managed constants are additive — no trap, offer stays."""
        if self._specs or not self._text:
            return False
        from . import flows

        return flows.reader_fields(self._spec, self._text) > 0

    def _compose_presets(self) -> ComposeResult:
        yield Static(gettext("Presets"), classes="section", id="st-presets-section")
        presets = argstate.load_state(self._entry.slug)["presets"]
        if not presets:
            yield Static(
                gettext("None yet — press Ctrl+S inside the run form to save one."),
                classes="hint",
            )
            return
        yield Static(gettext("Untick a preset to delete it on save:"), classes="hint")
        # NAME-keyed via the list captured here (the runner Select's rule): a preset
        # added or deleted mid-session (a concurrent skit preset save — the product's
        # own agent-coexistence story) must never shift which name an untick deletes.
        self._preset_names = sorted(presets)
        for i, name in enumerate(self._preset_names):
            summary = ", ".join(f"{k}={v}" for k, v in presets[name].items())
            yield Checkbox(
                f"{escape(name)}  [dim]{escape(summary)}[/dim]", value=True, id=f"st-preset-{i}"
            )

    def _compose_deps(self) -> ComposeResult:
        meta = self._entry.meta
        if not self._deps_editable():
            return
        yield Static(gettext("Dependencies"), classes="section")
        yield Input(
            value=", ".join(meta.dependencies or []),
            placeholder=gettext("comma separated, e.g. requests>=2,<3, rich")
            if self._spec is not None and self._spec.deps_flavor == "uv"
            else gettext("comma separated, e.g. chalk@^5, zod"),
            id="st-deps",
        )
        if self._spec is not None and self._spec.deps_flavor == "uv":
            yield Input(
                value=meta.requires_python,
                placeholder=gettext('Python constraint, e.g. ">=3.11" (empty = automatic)'),
                id="st-python",
            )

    def _deps_editable(self) -> bool:
        """Whether this entry's package dependencies can be edited here: any uv-flavor entry
        (reference mode records them in meta and delivers via --with), but an npm-flavor entry
        only in copy mode — a reference entry runs from its own project, whose node_modules
        already serves it, so offering the field would record deps the launch never uses."""
        if self._spec is None or not self._spec.supports_deps:
            return False
        return self._spec.deps_flavor != "npm" or self._entry.meta.mode == "copy"

    def _compose_needs(self) -> ComposeResult:
        # Every kind can declare needs: a shell script or a command template can require
        # ffmpeg/jq on PATH just as a python script can (checked before launch, exit 126).
        yield Static(gettext("Needs (external commands)"), classes="section")
        yield Input(
            value=", ".join(self._entry.meta.needs or []),
            placeholder=gettext("comma separated, e.g. ffmpeg, jq"),
            id="st-needs",
        )

    def on_mount(self) -> None:
        self.query_one("#st-body").border_title = gettext("Script settings · %(name)s") % {
            "name": escape(self._entry.meta.name)
        }
        self._toggle_workdir_path()
        self.call_after_refresh(setattr, self, "_dirt_armed", True)
        if self._initial == "presets":
            # `s` in the Library deep-links here: land the eye on the Presets section.
            section = self.query("#st-presets-section")
            if section:
                self.call_after_refresh(
                    self.query_one("#st-body", VerticalScroll).scroll_to_widget, section.first()
                )

    @on(Input.Changed)
    @on(Checkbox.Changed)
    @on(RadioSet.Changed)
    @on(Select.Changed)
    def _mark_dirty(self) -> None:
        # RadioSet included: the runner pin radio is a real edit — without it, a
        # pin-only change followed by Esc was discarded with no unsaved-changes ask.
        if self._dirt_armed:
            self._dirty = True

    # ----------------------------------------------------------------- save

    def action_resync(self) -> None:
        # One narrowing point (same idiom as action_save): the spec and its analyzer are proven
        # together, so no second, unreachable None-guard is needed afterwards.
        spec = self._spec
        if spec is None or spec.analyzer is None or self._entry.meta.mode != "copy":
            return
        result = analysis.edit_specs(
            self._text, self._specs, resync=True, analyze=spec.analyzer.analyze
        )
        self._specs = result.specs
        # Stash the outcome before recompose rebuilds the screen (updating the live Static
        # would be lost — recompose replaces it). compose re-emits self._resync_report.
        if result.warnings:
            self._resync_report = "\n".join(
                escape(analysis.render_warning(w)) for w in result.warnings
            )
        else:
            self._resync_report = gettext("Everything still matches the script.")
        self.refresh(recompose=True)

    def action_save(self) -> None:  # noqa: PLR0911, PLR0912, PLR0915 — one atomic save across every section
        """VALIDATE EVERYTHING FIRST, write only after every check passes: a refusal
        that lands after earlier sections were persisted makes both the Esc guard's
        "unsaved changes" and this method's own "nothing saved half-way" a lie (the
        exact half-commit Preferences was cured of; the rename write goes first, so
        even its unpre-checkable name-conflict failure aborts with zero prior writes)."""
        entry = self._entry
        new_name = self.query_one("#st-name", Input).value.strip()
        # ---- validation pass: no writes below may run unless ALL of these pass ----
        launch = self._validated_launch()
        if launch is None:
            return  # invalid workdir input; nothing was written
        new_template: str | None = None
        template_box = self.query("#st-template")
        if template_box:
            new_template = template_box.first(Input).value
            if new_template != entry.meta.template and not new_template.strip():
                self.notify(gettext("Command template must not be empty"), severity="error")
                return  # an empty template is not a program; nothing was written
        pending_decls: list[ParamDecl] | None = None
        if self._declared and not (self._is_prompt and not entry.meta.interpolate):
            pending_decls = self._collect_declared()
            if pending_decls is None:
                return  # a row is invalid; nothing was written
        pending_deps: tuple[list[str], str | None] | None = None
        if self._deps_editable():
            raw_deps = self.query_one("#st-deps", Input).value
            if self._spec is not None and self._spec.deps_flavor == "uv":
                deps = pep723.split_requirements(raw_deps)
                python: str | None = self.query_one("#st-python", Input).value.strip()
                if python and python.lower() in ("-", "none"):
                    # The add ask's token for "automatic", honored on this intake too.
                    python = ""
                # Validate HERE, in the validation pass — the store chokepoint would
                # refuse too, but only after rename/description/params had already
                # persisted, breaking this screen's own write-nothing-on-invalid
                # contract. Same validators as every other uv intake.
                for d in deps:
                    if (error := pep723.requirement_error(d)) is not None:
                        self.notify(error, severity="error")
                        return  # invalid requirement; nothing was written
                if python and (error := pep723.requires_python_error(python)) is not None:
                    self.notify(error, severity="error")
                    return  # invalid constraint; nothing was written
            else:
                # npm-shaped split — the PEP 508 splitter would merge a scoped package into
                # its neighbor ("chalk, @scope/pkg" -> one bogus requirement). No Python
                # constraint either (and no #st-python widget to read), and no PEP 508
                # validation: npm grammar belongs to the npm installer.
                from .langs.javascript import deps as js_deps

                deps = js_deps.split_requirements(raw_deps)
                python = None
            pending_deps = (deps, python)
        # ---- write pass ----
        if new_name and new_name != entry.meta.name:
            try:
                store.rename(entry.slug, new_name)
            except store.StoreError as exc:
                self.notify(str(exc), severity="error")
                return  # first write failed; nothing else was saved
        description = self.query_one("#st-desc", Input).value.strip()
        if description != entry.meta.description:
            store.update_description(entry.slug, description)
        self._write_launch(launch)
        if new_template is not None and new_template != entry.meta.template:
            store.update_template(entry.slug, new_template)
        # One narrowing point: an analyzable kind always carries params_io too (the registry pairs
        # them), and a non-empty text with a live analyzer means reconcile always returns a report —
        # so the capabilities are read straight off the narrowed spec, with no dead None-guards.
        spec = self._spec
        if (
            spec is not None
            and spec.analyzer is not None
            and spec.params_io is not None
            and entry.meta.mode == "copy"
            and self._text
        ):
            new_specs = [s for row in self.query(ParamRow) if (s := row.collect()) is not None]
            report = spec.analyzer.reconcile(self._text, self._specs)
            for i, c in enumerate(report.new):
                box = self.query(f"#st-new-{i}")
                if box and box.first(Checkbox).value:
                    new_specs.append(ParamDecl.from_candidate(c))
            entry.script_path.write_text(
                spec.params_io.write(self._text, new_specs), encoding="utf-8"
            )
            purged = argstate.purge_secret(entry.slug, {s.name for s in new_specs if s.secret})
            if purged:
                self.notify(
                    gettext("Deleted previously remembered value(s): %(names)s")
                    % {"names": ", ".join(sorted(purged))}
                )
        if self._is_prompt:
            # The toggle is composed unconditionally for prompts, so query_one is safe.
            wants_interpolate = self.query_one("#st-interpolate", Checkbox).value
            if wants_interpolate != entry.meta.interpolate:
                store.write_prompt_interpolate(entry.slug, wants_interpolate)
            # The pin save lives HERE, not in the declared branch below: that branch is
            # skipped when insertion is off, and a pin change must never be dropped for it.
            self._save_runner_pin()
        if pending_decls is not None:
            decls = pending_decls
            if self._is_prompt:
                decls += self._ticked_prompt_candidates({d.name for d in decls})
                self._save_prompt_managed(decls)
            store.write_parameters(entry.slug, decls)
            purged = argstate.purge_secret(entry.slug, {d.name for d in decls if d.secret})
            if purged:
                self.notify(
                    gettext("Deleted previously remembered value(s): %(names)s")
                    % {"names": ", ".join(sorted(purged))}
                )
        if pending_deps is not None:
            deps, python = pending_deps
            if deps != (entry.meta.dependencies or []) or (
                python is not None and python != entry.meta.requires_python
            ):
                try:
                    store.update_dependencies(entry.slug, deps, requires_python=python)
                except store.StoreError as exc:
                    # Clearing npm deps also sweeps node_modules from disk, which can fail
                    # (a held-open file, a read-only remnant) — same treatment as a failed
                    # rename: report and stay, never crash the app out from under the user.
                    self.notify(str(exc), severity="error")
                    return
        needs = [
            n.strip() for n in self.query_one("#st-needs", Input).value.split(",") if n.strip()
        ]
        if needs != (entry.meta.needs or []):
            store.update_needs(entry.slug, needs)
        for i, name in enumerate(self._preset_names):
            # The compose-time names, not a fresh state read: index i belongs to the
            # checkbox the user actually saw.
            box = self.query(f"#st-preset-{i}")
            if box and not box.first(Checkbox).value:
                argstate.delete_preset(entry.slug, name)
        self.dismiss(True)

    def _ticked_prompt_candidates(self, taken: set[str]) -> list[ParamDecl]:
        """The tick-to-manage checkboxes' yield: each ticked, still-unclaimed candidate
        becomes a synthesized placeholder row (required, like every managed placeholder)."""
        managed = self._entry.meta.params or []
        unmanaged = [n for n in self._prompt_body_names if n not in managed]
        out: list[ParamDecl] = []
        for i, candidate in enumerate(unmanaged):
            box = self.query(f"#st-prompt-new-{i}")
            if box and box.first(Checkbox).value and candidate not in taken:
                out.append(
                    ParamDecl(
                        name=candidate,
                        binding="none",
                        delivery="placeholder",
                        required=True,
                        secret=params.is_secret_name(candidate),
                    )
                )
        return out

    def _save_prompt_managed(self, decls: list[ParamDecl]) -> None:
        """The managed list follows the kept placeholder rows: body order first, then any
        managed name the body has lost but the user kept (drift stays visible, not grown)."""
        entry = self._entry
        keep = [d.name for d in decls if d.delivery == "placeholder"]
        keep_set = set(keep)
        new_managed = [n for n in self._prompt_body_names if n in keep_set]
        new_managed += [n for n in keep if n not in set(self._prompt_body_names)]
        store.write_prompt_managed(entry.slug, new_managed)

    def _save_runner_pin(self) -> None:
        """Persist the runner dropdown's pick. Value-keyed — no index mapping exists
        to shift, however the runner list changed while the screen was open."""
        entry = self._entry
        # The dropdown always composes for a prompt (even an emptied runner list keeps
        # the "ask" option), and this only runs for prompts — query_one is safe.
        current = entry.meta.runner
        value = self.query_one("#st-runner-select", Select).value
        pin = "" if value is Select.NULL else str(value)
        if pin != current:
            store.write_prompt_runner(entry.slug, pin)
            # A pick of a real configured runner prefills future pickers; re-keeping a
            # stale pin (or "ask") is not a pick.
            if pin and config.find_prompt_runner(pin) is not None:
                argstate.save_last_runner(pin)

    def action_close(self) -> None:
        if not self._dirty:
            self.dismiss(False)
            return

        def _decided(discard: bool | None) -> None:
            if discard:
                self.dismiss(False)

        self.app.push_screen(DiscardChangesModal(), _decided)
