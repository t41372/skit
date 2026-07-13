"""Script settings (p): the merged per-script management screen — basics, parameters,
presets, dependencies in one place.

Enter saves everything in one atomic [tool.skit] rewrite; Esc asks when there are
unsaved changes. Reference-mode entries show the parameters read-only (skit never
writes the original file, A7); command entries show the template and placeholders.
"""

from __future__ import annotations

from typing import override

from rich.markup import escape
from textual import on
from textual.app import ComposeResult
from textual.binding import Binding
from textual.containers import Horizontal, Vertical, VerticalScroll
from textual.screen import ModalScreen, Screen
from textual.widgets import Checkbox, Input, Label, Static

from . import analysis, argstate, params, pep723, store, tui_footer
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
        Binding("ctrl+a", "save", gettext("Save"), priority=True),
        Binding("ctrl+r", "resync", gettext("Resync"), priority=True),
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
        self._declared_decls: list[ParamDecl] = (
            params.declared_from_meta(entry.meta.parameters) if self._declared else []
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
            yield from self._compose_params()
            yield from self._compose_presets()
            yield from self._compose_deps()
            yield from self._compose_needs()
        yield tui_footer.KeysBar(
            Static(
                tui_footer.bar(
                    tui_footer.chip("screen.save", "Ctrl+A", gettext("Save")),
                    tui_footer.chip("screen.resync", "Ctrl+R", gettext("Resync")),
                    tui_footer.chip("screen.close", "Esc", gettext("Back")),
                    tui_footer.nav_chip(),
                ),
                id="st-keys",
                markup=True,
            )
        )

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
            yield Static(escape(meta.template), classes="hint")
        # A flag only means something where argv is the interface: every non-template kind
        # (binaries AND the interpreted meta-schema kinds), mirroring the CLI's allowed deliveries.
        show_flag = self._spec is not None and self._spec.family != "template"
        for d in self._declared_decls:
            yield DeclParamRow(d, show_flag=show_flag)
        yield Static(gettext("Add a parameter — type a name, then Save:"), classes="hint")
        yield Input(placeholder=gettext("new parameter name"), id="st-add-param")

    def _new_declared(self, name: str) -> ParamDecl:
        """A freshly-added declared parameter's default shape. A template placeholder stays
        required (an empty slot silently assembles a broken command); everything else is an
        optional env/flag value that falls back to the program's own default."""
        placeholders = self._entry.meta.params or []
        if self._spec is not None and self._spec.family == "template":
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
                    "%(name)s is a choice parameter but has no choices; add them with "
                    "`skit params` on the command line."
                )
                % {"name": d.name},
                severity="error",
            )
            return None
        return normalized

    def _cli_driven(self) -> bool:
        """Whether the run form currently comes from the script's own CLI surface — i.e.
        nothing is managed yet AND the script parses its own arguments. (Once anything is
        managed, plan_for_entry already serves the injected form, so there's no trap.)"""
        if self._specs or not self._text:
            return False
        # The ENTRY'S OWN reader: shell getopts, JS util.parseArgs, fish argparse — Python's
        # argspec would see none of them and wrongly offer the manage-a-constant checkboxes on a
        # script that already drives its own CLI.
        reader = self._spec.cli_reader if self._spec is not None else None
        if reader is None:
            return False
        return reader.read_cli(self._text) is not None

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
        for i, (name, values) in enumerate(sorted(presets.items())):
            summary = ", ".join(f"{k}={v}" for k, v in values.items())
            yield Checkbox(
                f"{escape(name)}  [dim]{escape(summary)}[/dim]", value=True, id=f"st-preset-{i}"
            )

    def _compose_deps(self) -> ComposeResult:
        meta = self._entry.meta
        if self._spec is None or not self._spec.supports_deps:
            return
        yield Static(gettext("Dependencies"), classes="section")
        yield Input(
            value=", ".join(meta.dependencies or []),
            placeholder=gettext("comma separated, e.g. requests>=2,<3, rich"),
            id="st-deps",
        )
        yield Input(
            value=meta.requires_python,
            placeholder=gettext('Python constraint, e.g. ">=3.11" (empty = automatic)'),
            id="st-python",
        )

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
    def _mark_dirty(self) -> None:
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

    def action_save(self) -> None:  # noqa: PLR0912 — one atomic save across every section
        entry = self._entry
        new_name = self.query_one("#st-name", Input).value.strip()
        if new_name and new_name != entry.meta.name:
            try:
                store.rename(entry.slug, new_name)
            except store.StoreError as exc:
                self.notify(str(exc), severity="error")
                return  # stay on the screen; nothing else is saved half-way
        description = self.query_one("#st-desc", Input).value.strip()
        if description != entry.meta.description:
            store.update_description(entry.slug, description)
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
        if self._declared:
            decls = self._collect_declared()
            if decls is None:
                return  # a row is invalid; stay on the screen, nothing saved half-way
            store.write_parameters(entry.slug, decls)
            purged = argstate.purge_secret(entry.slug, {d.name for d in decls if d.secret})
            if purged:
                self.notify(
                    gettext("Deleted previously remembered value(s): %(names)s")
                    % {"names": ", ".join(sorted(purged))}
                )
        if self._spec is not None and self._spec.supports_deps:
            deps = pep723.split_requirements(self.query_one("#st-deps", Input).value)
            python = self.query_one("#st-python", Input).value.strip()
            if deps != (entry.meta.dependencies or []) or python != entry.meta.requires_python:
                store.update_dependencies(entry.slug, deps, requires_python=python)
        needs = [
            n.strip() for n in self.query_one("#st-needs", Input).value.split(",") if n.strip()
        ]
        if needs != (entry.meta.needs or []):
            store.update_needs(entry.slug, needs)
        presets = argstate.load_state(entry.slug)["presets"]
        for i, name in enumerate(sorted(presets)):
            box = self.query(f"#st-preset-{i}")
            if box and not box.first(Checkbox).value:
                argstate.delete_preset(entry.slug, name)
        self.dismiss(True)

    def action_close(self) -> None:
        if not self._dirty:
            self.dismiss(False)
            return

        def _decided(discard: bool | None) -> None:
            if discard:
                self.dismiss(False)

        self.app.push_screen(DiscardChangesModal(), _decided)
