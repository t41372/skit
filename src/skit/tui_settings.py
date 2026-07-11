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

from . import argspec, argstate, metawriter, pep723, reconcile, store, tui_footer
from .i18n import gettext
from .models import Entry


class DiscardChangesModal(ModalScreen[bool]):
    BINDINGS = [
        Binding("y", "discard", gettext("Discard")),
        Binding("escape,n", "keep", gettext("Keep editing")),
    ]
    DEFAULT_CSS = """
    DiscardChangesModal { align: center middle; }
    DiscardChangesModal > Vertical { border: round $accent; padding: 1 2; width: auto;
        height: auto; background: $background; }
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

    def __init__(self, spec: metawriter.ParamSpec) -> None:
        super().__init__()
        self.spec: metawriter.ParamSpec = spec

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

    def collect(self) -> metawriter.ParamSpec | None:
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


class ScriptSettingsScreen(Screen[bool]):
    """Four sections in one screen; `s` in the Library deep-links to Presets."""

    BINDINGS = [
        Binding("escape", "close", gettext("Back")),
        Binding("ctrl+a", "save", gettext("Save"), priority=True),
        Binding("ctrl+r", "resync", gettext("Resync"), priority=True),
    ]
    DEFAULT_CSS = """
    ScriptSettingsScreen #st-body {
        padding: 0 1;
        border: round $skit-box-indigo;
        border-title-color: ansi_bright_white;
        border-title-style: bold;
    }
    ScriptSettingsScreen .section { color: $accent; margin: 1 0 0 0; }
    ScriptSettingsScreen .hint { color: $text-muted; }
    ScriptSettingsScreen #st-keys { dock: bottom; height: 1; color: $text-muted; padding: 0 1; }
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
        if entry.meta.kind == "python" and entry.script_path.exists():
            self._text = entry.script_path.read_text(encoding="utf-8", errors="replace")
        self._specs: list[metawriter.ParamSpec] = metawriter.read_params(self._text)
        # The resync outcome (incl. safety-rebind warnings) must survive the recompose that
        # action_resync triggers — a widget updated in place would be thrown away and rebuilt
        # empty. Kept on the instance so compose can re-emit it. Already escape()'d for markup.
        self._resync_report: str = ""

    @override
    def compose(self) -> ComposeResult:
        with VerticalScroll(id="st-body"):
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
        yield Static(
            tui_footer.bar(
                tui_footer.chip("screen.save", "Ctrl+A", gettext("Save")),
                tui_footer.chip("screen.resync", "Ctrl+R", gettext("Resync")),
                tui_footer.chip("screen.close", "Esc", gettext("Back")),
            ),
            id="st-keys",
            markup=True,
        )

    def _compose_storage(self) -> ComposeResult:
        meta = self._entry.meta
        if meta.kind != "python":
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
        if meta.kind == "command":
            yield Static(escape(meta.template), classes="hint")
            for p in meta.params or []:
                yield Static(f"· {escape(p)}")
            return
        if meta.kind != "python":
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
            report = reconcile.reconcile(self._text, self._specs) if self._text else None
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
        if self._specs and all(s.kind == "input" for s in self._specs):
            yield Static(
                gettext("Every input() is managed — this script can now run with --no-input."),
                classes="hint",
            )
        yield Static(self._resync_report, id="st-resync-report", classes="hint", markup=True)

    def _cli_driven(self) -> bool:
        """Whether the run form currently comes from the script's own CLI surface — i.e.
        nothing is managed yet AND the script parses its own arguments. (Once anything is
        managed, plan_for_entry already serves the injected form, so there's no trap.)"""
        if self._specs or not self._text:
            return False
        return argspec.read_cli(self._text) is not None

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
        if meta.kind != "python":
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
        if self._entry.meta.kind != "python" or self._entry.meta.mode != "copy":
            return
        result = reconcile.edit_specs(self._text, self._specs, resync=True)
        self._specs = result.specs
        # Stash the outcome before recompose rebuilds the screen (updating the live Static
        # would be lost — recompose replaces it). compose re-emits self._resync_report.
        if result.warnings:
            self._resync_report = "\n".join(
                escape(reconcile.render_warning(w)) for w in result.warnings
            )
        else:
            self._resync_report = gettext("Everything still matches the script.")
        self.refresh(recompose=True)

    def action_save(self) -> None:
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
        if entry.meta.kind == "python" and entry.meta.mode == "copy" and self._text:
            new_specs = [s for row in self.query(ParamRow) if (s := row.collect()) is not None]
            report = reconcile.reconcile(self._text, self._specs) if self._text else None
            if report is not None:
                for i, c in enumerate(report.new):
                    box = self.query(f"#st-new-{i}")
                    if box and box.first(Checkbox).value:
                        new_specs.append(metawriter.ParamSpec.from_candidate(c))
            copy_path = entry.dir / "script.py"
            copy_path.write_text(metawriter.write_params(self._text, new_specs), encoding="utf-8")
            purged = argstate.purge_secret(entry.slug, {s.name for s in new_specs if s.secret})
            if purged:
                self.notify(
                    gettext("Deleted previously remembered value(s): %(names)s")
                    % {"names": ", ".join(sorted(purged))}
                )
        if entry.meta.kind == "python":
            deps = pep723.split_requirements(self.query_one("#st-deps", Input).value)
            python = self.query_one("#st-python", Input).value.strip()
            if deps != (entry.meta.dependencies or []) or python != entry.meta.requires_python:
                store.update_dependencies(entry.slug, deps, requires_python=python)
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
