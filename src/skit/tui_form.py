"""The run form as Textual widgets: one FormPlan → one pokeable screen.

Shared by the TUI workbench (full screen) and the CLI's inline mini-form (M6): same
widgets, same validation, same keys — one flow, two frames. All logic stays in flows;
this module renders and collects.

Field widgets by kind:
- bool          → Checkbox
- choice        → horizontal RadioSet (←/→ or click)
- secret        → password Input (+ "reads $NAME" note when an env source is set)
- anything else → Input, with live token-expansion preview and glob match count
- degraded      → Input + "leave empty for the script's own default" hint

Type hints render in muted text and only turn loud on a validation error.
"""

from __future__ import annotations

from pathlib import Path
from typing import TYPE_CHECKING, override

from rich.markup import escape
from textual import on
from textual.app import ComposeResult
from textual.binding import Binding
from textual.containers import Horizontal, Vertical
from textual.screen import ModalScreen, Screen
from textual.widgets import Checkbox, Input, Label, OptionList, RadioButton, RadioSet, Static
from textual.widgets.option_list import Option

from . import argstate, flows, tokens, tui_footer
from .i18n import gettext

if TYPE_CHECKING:
    from .models import Entry

# The submit result: (raw field values, extra passthrough args) or None on cancel.
FormResult = tuple[dict[str, str], list[str]] | None

_EXTRA_KEY = "__extra_args__"


def _type_label(kind: str) -> str:
    """The dim type hint next to a field label, translated at render time. The msgids MUST be
    gettext() string literals here, not a dict of labels fed to gettext(kind): a dict lookup is
    invisible to Babel's extractor, so the strings never reach the catalog and silently fall
    back to English in every locale. (That is the exact bug scripts/i18n_coverage.py's
    dynamic-gettext check now guards against.)"""
    return {
        "int": gettext("whole number"),
        "float": gettext("number"),
        "str": gettext("text"),
        "bool": gettext("on/off"),
    }.get(kind, kind)


def _degraded_notice(reason: str) -> str:
    """The honest line for a whole-parser degrade (the form still offers the
    extra-arguments escape field)."""
    if reason == "subparsers":
        return gettext(
            "This script has subcommands skit can't model — type everything into the "
            "extra-arguments field."
        )
    return gettext(
        "skit couldn't read this script's argument declarations — type everything into "
        "the extra-arguments field."
    )


class FieldRow(Vertical):
    """One form field: label row, control, help/feedback lines."""

    DEFAULT_CSS = """
    FieldRow { height: auto; margin: 0 0 1 0; max-width: 100; }
    FieldRow .field-label { color: $foreground; }
    FieldRow .field-help { color: $text-muted; }
    FieldRow .field-error { color: $error; }
    FieldRow .field-preview { color: $text-muted; }
    /* Feedback lines materialize only when they have something to say. As permanent
       empty Statics they cost 2 blank rows per field, which is what stretched the form
       into a sparse, oddly-gapped page. */
    FieldRow .field-preview { display: none; }
    FieldRow .field-error { display: none; }
    FieldRow Input { width: 1fr; }
    FieldRow RadioSet { layout: horizontal; height: auto; border: none; padding: 0; }
    FieldRow RadioSet:focus { border: none; }
    /* Textual gives each RadioButton width:1fr, which scatters two choices across the
       whole terminal row; options belong side by side, reading left to right. */
    FieldRow RadioSet > RadioButton { width: auto; margin: 0 3 0 0; }
    /* A bare Checkbox wears a tall border around a lone glyph — a cryptic floating
       box. Borderless, with an on/off word as its label, it reads as the toggle it is. */
    FieldRow Checkbox { border: none; padding: 0; }
    FieldRow Checkbox:focus { border: none; }
    """

    def __init__(self, field: flows.FormField, prefill: str) -> None:
        # Field keys are identifiers (argparse dests, [tool.skit] names, placeholders),
        # so they are valid Textual ids; the ▾ link targets the row through this id.
        super().__init__(id=f"fr-{field.key}")
        self.field: flows.FormField = field
        self._prefill: str = prefill

    @property
    def insertable(self) -> bool:
        """Whether the ▾ token menu applies: free-text-ish and not a secret (secret
        values skip token expansion by design)."""
        return not self.field.secret and self.field.kind not in ("bool", "choice")

    @override
    def compose(self) -> ComposeResult:
        f = self.field
        pieces = [escape(f.label)]
        if f.required:
            pieces.append(f"[$accent]{gettext('required')}[/]")
        if f.kind in ("int", "float", "bool"):
            pieces.append(f"[dim]{_type_label(f.kind)}[/dim]")
        if f.secret:
            pieces.append(f"[dim]🔒 {gettext('never saved to disk')}[/dim]")
        if self.insertable:
            pieces.append(
                f"[$accent @click=screen.insert_token('{f.key}')]▾ {gettext('insert')}[/]"
            )
        yield Static("  ".join(pieces), classes="field-label", markup=True)
        yield from self._compose_control()
        if f.help:
            yield Static(escape(f.help), classes="field-help")
        if f.degraded:
            yield Static(
                gettext("Leave empty to use the script's own default."), classes="field-help"
            )
        if f.secret and f.env_source:
            yield Static(
                gettext("Leave empty to read it from the environment variable %(env)s.")
                % {"env": escape(f.env_source)},
                classes="field-help",
            )
        yield Static("", classes="field-preview")
        yield Static("", classes="field-error")

    def _compose_control(self) -> ComposeResult:
        f = self.field
        if f.kind == "bool":
            on = self._prefill.strip().lower() in ("true", "1", "yes")
            yield Checkbox(gettext("on") if on else gettext("off"), value=on)
        elif f.kind == "choice" and f.choices:
            with RadioSet():
                for choice in f.choices:
                    yield RadioButton(escape(choice), value=(choice == self._prefill))
        else:
            yield Input(value=self._prefill, password=f.secret)

    @property
    def value(self) -> str:
        f = self.field
        if f.kind == "bool":
            return "true" if self.query_one(Checkbox).value else "false"
        if f.kind == "choice" and f.choices:
            pressed = self.query_one(RadioSet).pressed_index
            return f.choices[pressed] if 0 <= pressed < len(f.choices) else ""
        return self.query_one(Input).value

    def set_value(self, value: str) -> None:
        f = self.field
        if f.kind == "bool":
            self.query_one(Checkbox).value = value.strip().lower() in ("true", "1", "yes")
        elif f.kind == "choice" and f.choices:
            if value in f.choices:
                buttons = list(self.query(RadioButton))
                buttons[f.choices.index(value)].value = True
        else:
            self.query_one(Input).value = value

    def show_error(self, message: str | None) -> None:
        error = self.query_one(".field-error", Static)
        error.update(escape(message) if message else "")
        error.display = bool(message)

    @on(Checkbox.Changed)
    def _bool_word(self, event: Checkbox.Changed) -> None:
        """The label tracks the state (on/off), btop-style — an unchecked box labeled
        "on" would read as a lie."""
        event.checkbox.label = gettext("on") if event.value else gettext("off")

    @on(Input.Changed)
    def _live_feedback(self, event: Input.Changed) -> None:
        """Token-expansion preview and glob match count, refreshed as the user types."""
        f = self.field
        preview = self.query_one(".field-preview", Static)
        value = event.value
        if f.secret or not value:
            preview.update("")
            preview.display = False
            return
        lines: list[str] = []
        if tokens.has_tokens(value):
            expanded, error = tokens.preview(value, cwd=Path.cwd())
            lines.append(f"→ {escape(error if error else expanded)}")
        count = flows.glob_feedback(value, Path.cwd())
        if count is not None:
            lines.append(
                gettext("✓ matches %(count)s file(s)") % {"count": count}
                if count
                else gettext("⚠ matches no files yet")
            )
        preview.update("   ".join(lines))
        preview.display = bool(lines)
        self.show_error(None)


class PresetNameModal(ModalScreen[str | None]):
    """Ctrl+S: name (or overwrite) a preset from the current form values."""

    AUTO_FOCUS = "Input"  # type immediately; without this, Enter lands nowhere
    BINDINGS = [Binding("escape", "cancel", gettext("Cancel"))]
    DEFAULT_CSS = """
    PresetNameModal { align: center middle; }
    PresetNameModal > Vertical {
        border: round $accent; padding: 1 2; width: 60; height: auto;
        background: $background;
    }
    """

    def __init__(self, existing: set[str]) -> None:
        super().__init__()
        self._existing: set[str] = existing

    @override
    def compose(self) -> ComposeResult:
        with Vertical():
            yield Label(gettext("Save as preset"))
            yield Input(placeholder=gettext("Preset name"))
            yield Static("", id="preset-hint", markup=True)
            yield Static(
                tui_footer.bar(
                    tui_footer.chip("screen.save_name", "Enter", gettext("Save")),
                    tui_footer.chip("screen.cancel", "Esc", gettext("Cancel")),
                ),
                markup=True,
            )

    @on(Input.Changed)
    def _overwrite_hint(self, event: Input.Changed) -> None:
        hint = self.query_one("#preset-hint", Static)
        if event.value.strip() in self._existing:
            hint.update(
                f"[yellow]{gettext('This overwrites the existing preset %(name)s.') % {'name': escape(event.value.strip())}}[/yellow]"
            )
        else:
            hint.update("")

    @on(Input.Submitted)
    def _save(self, event: Input.Submitted) -> None:
        name = event.value.strip()
        if name:
            self.dismiss(name)

    def action_save_name(self) -> None:
        """Click twin of Enter: save under the typed name (a blank name stays put)."""
        name = self.query_one(Input).value.strip()
        if name:
            self.dismiss(name)

    def action_cancel(self) -> None:
        self.dismiss(None)


class TokenMenuModal(ModalScreen[str | None]):
    """The ▾insert menu: run-time value tokens, spelled out so users learn the syntax
    passively (the same philosophy as showing the assembled command). The first two
    entries ARE the design's intent-vs-frozen fork: "wherever I run from" ({cwd}) sits
    right next to "this directory, as a fixed path"."""

    AUTO_FOCUS = "OptionList"
    BINDINGS = [Binding("escape", "cancel", gettext("Cancel"))]
    DEFAULT_CSS = """
    TokenMenuModal { align: center middle; }
    TokenMenuModal > Vertical { border: round $accent; padding: 1 2; width: 64;
        height: auto; background: $background; }
    TokenMenuModal OptionList { height: auto; border: none; }
    TokenMenuModal Static { width: auto; margin: 1 0 0 0; }
    """

    _ENV_SENTINEL = "__env__"

    @override
    def compose(self) -> ComposeResult:
        entries: list[tuple[str, str, str]] = [
            ("{cwd}", gettext("Directory at run time (changes with where you run)"), "{cwd}"),
            (str(Path.cwd()), gettext("This directory, as a fixed path"), str(Path.cwd())),
            ("{today}", gettext("Today's date"), "{today}"),
            ("{now}", gettext("Current time"), "{now}"),
            ("~", gettext("Home directory"), "~"),
        ]
        options = [
            Option(f"{escape(label)}  [dim]{escape(shown)}[/dim]", id=insert)
            for shown, label, insert in entries
        ]
        options.append(
            Option(
                f"{gettext('Environment variable…')}  [dim]{{env:NAME}}[/dim]",
                id=self._ENV_SENTINEL,
            )
        )
        with Vertical():
            yield Label(gettext("Insert a run-time value"))
            yield OptionList(*options)
            yield Static(
                tui_footer.bar(tui_footer.chip("screen.cancel", "Esc", gettext("Cancel"))),
                markup=True,
            )

    @on(OptionList.OptionSelected)
    def _picked(self, event: OptionList.OptionSelected) -> None:
        if event.option.id == self._ENV_SENTINEL:

            def _env_done(token: str | None) -> None:
                self.dismiss(token)

            self.app.push_screen(EnvPickerModal(), _env_done)
            return
        self.dismiss(str(event.option.id))

    def action_cancel(self) -> None:
        self.dismiss(None)


class EnvPickerModal(ModalScreen[str | None]):
    """Pick an environment variable by name (type to filter os.environ)."""

    AUTO_FOCUS = "Input"
    BINDINGS = [Binding("escape", "cancel", gettext("Cancel"))]
    DEFAULT_CSS = """
    EnvPickerModal { align: center middle; }
    EnvPickerModal > Vertical { border: round $accent; padding: 1 2; width: 64;
        height: auto; max-height: 80%; background: $background; }
    EnvPickerModal OptionList { border: none; max-height: 12; }
    EnvPickerModal Static { width: auto; margin: 1 0 0 0; }
    """

    @override
    def compose(self) -> ComposeResult:
        with Vertical():
            yield Label(gettext("Environment variable"))
            yield Input(placeholder=gettext("type to filter…"))
            yield OptionList(*self._options(""))
            yield Static(
                tui_footer.bar(tui_footer.chip("screen.cancel", "Esc", gettext("Cancel"))),
                markup=True,
            )

    def _options(self, needle: str) -> list[Option]:
        import os

        names = sorted(n for n in os.environ if needle.lower() in n.lower())
        return [Option(escape(n), id=n) for n in names[:200]]

    @on(Input.Changed)
    def _filter(self, event: Input.Changed) -> None:
        option_list = self.query_one(OptionList)
        option_list.clear_options()
        option_list.add_options(self._options(event.value.strip()))

    @on(Input.Submitted)
    def _typed(self, event: Input.Submitted) -> None:
        # A name typed in full is accepted even if it isn't set YET — the token resolves
        # at run time, and assembly reports a missing variable by name.
        name = event.value.strip()
        if name.isidentifier():
            self.dismiss("{env:" + name + "}")

    @on(OptionList.OptionSelected)
    def _picked(self, event: OptionList.OptionSelected) -> None:
        self.dismiss("{env:" + str(event.option.id) + "}")

    def action_cancel(self) -> None:
        self.dismiss(None)


class RunFormScreen(Screen[FormResult]):
    """The full-size run form (title answers "where am I": Run <name>)."""

    BINDINGS = [
        Binding("escape", "cancel", gettext("Cancel")),
        Binding("ctrl+s", "save_preset", gettext("Save as preset")),
        # Enter runs the form from ANY field. Without priority, a focused Checkbox/RadioSet
        # swallows Enter for its own toggle/select and the footer's "Enter Run" silently does
        # nothing (and an inline flag/choice-only form has no Input at all, so Enter could
        # never submit). priority makes the screen intercept Enter first; Space still toggles
        # a checkbox. Ctrl+R stays as the explicit muscle-memory chord.
        Binding("enter", "submit", gettext("Run"), priority=True, show=False),
        Binding("ctrl+r", "submit", gettext("Run"), priority=True),
        Binding("ctrl+t", "insert_token", gettext("Insert value")),
        *tui_footer.FIELD_NAV_BINDINGS,
    ]
    # Boot straight into the first control (not the body scroll container, which the
    # app-wide "*" would pick): the form should be typeable the moment it opens.
    AUTO_FOCUS = "Input, Checkbox, RadioSet"
    DEFAULT_CSS = """
    RunFormScreen { background: $background; }
    /* btop grammar: the form is one rounded panel with the "Run <name>" title ON the
       border (proc-box maroon — this is the panel where things run). The border lives
       on an inner container, not the Screen: a bordered Screen shifts its coordinate
       space and clicks on the bottom-docked footer land "outside" the screen. */
    RunFormScreen #form-panel {
        border: round $skit-box-maroon;
        border-title-color: ansi_bright_white;
        border-title-style: bold;
    }
    RunFormScreen #drift-banner, RunFormScreen #degraded-notice { color: $warning; padding: 0 1; }
    RunFormScreen #preset-row { height: auto; padding: 0 1; }
    /* Widgets default to width:1fr; in a Horizontal that lets the "Preset:" caption
       swallow the whole row and push the chips (or the empty-state hint) clean off the
       screen. Everything in this row hugs its content. */
    RunFormScreen #preset-row Static { width: auto; margin: 0 1 0 0; }
    RunFormScreen #preset-row RadioSet { width: auto; }
    /* The empty-state hint is a long sentence: let it take the row's remaining width and
       wrap, rather than width:auto (its full content width) which overflows a narrow form.
       Two ids outrank the `#preset-row Static` width:auto rule above. */
    RunFormScreen #preset-row #preset-empty { color: $text-muted; width: 1fr; height: auto; }
    RunFormScreen #preset-row RadioSet { layout: horizontal; height: auto; border: none; }
    RunFormScreen #preset-row RadioSet > RadioButton { width: auto; margin: 0 2 0 0; }
    RunFormScreen #form-body { padding: 0 1; }
    RunFormScreen #form-keys { dock: bottom; height: 1; color: $text-muted; padding: 0 1; }

    /* Inline mode (the CLI's `skit run` mini-window): an inline Screen is sized to its
       content height, but the VerticalScroll body defaults to height:1fr, which collapses
       to a single row in an auto-height parent — the whole form flattens to a 3-line stub
       and the docked footer gets clipped. Give the panel and body an explicit auto height
       so the screen measures its true content, and cap the window so a tall form scrolls
       inside it (footer stays docked and visible) instead of taking over the terminal. In
       full-screen mode the body keeps its 1fr fill and the footer pins to the bottom. */
    RunFormScreen:inline { height: auto; max-height: 80%; }
    RunFormScreen:inline #form-panel { height: auto; }
    RunFormScreen:inline #form-body { height: auto; }
    """

    def __init__(
        self,
        entry: Entry,
        plan: flows.FormPlan,
        prefill: dict[str, str],
        include_extra: bool = True,
    ) -> None:
        super().__init__()
        self._entry: Entry = entry
        self._plan: flows.FormPlan = plan
        self._prefill: dict[str, str] = prefill
        # The inline (CLI) frame hides the extra-args row: argv already owns passthrough
        # args there, and two sources for the same thing would fight.
        self._include_extra: bool = include_extra
        self._presets: dict[str, dict[str, str]] = argstate.load_state(entry.slug)["presets"]

    def on_mount(self) -> None:
        self.query_one("#form-panel").border_title = gettext("Run %(name)s") % {
            "name": escape(self._entry.meta.name)
        }

    @override
    def compose(self) -> ComposeResult:
        with Vertical(id="form-panel"):
            if self._plan.drift_lines:
                yield Static(
                    "\n".join(escape(line) for line in self._plan.drift_lines), id="drift-banner"
                )
            if self._plan.degraded_reason:
                yield Static(_degraded_notice(self._plan.degraded_reason), id="degraded-notice")
            with Horizontal(id="preset-row"):
                yield Static(gettext("Preset:"), markup=False)
                if self._presets:
                    with RadioSet(id="preset-set"):
                        yield RadioButton(gettext("last values"), value=True)
                        for name in sorted(self._presets):
                            yield RadioButton(escape(name))
                else:
                    # Empty state teaches the Ctrl+S affordance precisely when the user
                    # has no presets and most needs to learn it (spec §2).
                    yield Static(
                        gettext("none yet — fill the form and press Ctrl+S to save one"),
                        id="preset-empty",
                    )
            with tui_footer.FormBody(id="form-body"):
                for f in self._plan.fields:
                    yield FieldRow(f, self._prefill.get(f.key, ""))
                if self._include_extra:
                    extra_field = flows.FormField(
                        key=_EXTRA_KEY,
                        label=gettext("Extra arguments (passed to the script as-is)"),
                        source="flag",
                    )
                    last_extra = argstate.load_state(self._entry.slug)["extra_args"]
                    # shlex.join, because collect() shlex.split()s: an argument that
                    # contains spaces must survive the round trip as ONE argument.
                    import shlex

                    yield FieldRow(extra_field, shlex.join(last_extra))
        yield Static(
            tui_footer.bar(
                tui_footer.chip("screen.submit", "Enter", gettext("Run")),
                tui_footer.chip("screen.insert_token", "Ctrl+T", gettext("Insert value")),
                tui_footer.chip("screen.save_preset", "Ctrl+S", gettext("Save as preset")),
                tui_footer.chip("screen.cancel", "Esc", gettext("Cancel")),
                tui_footer.nav_chip(),
            ),
            id="form-keys",
            markup=True,
        )

    @on(RadioSet.Changed, "#preset-set")
    def _apply_preset(self, event: RadioSet.Changed) -> None:
        """Chip switch: overlay the whole preset onto the fields ("last values" restores)."""
        index = event.radio_set.pressed_index
        names = sorted(self._presets)
        chosen = (
            self._prefill if index == 0 else {**self._prefill, **self._presets[names[index - 1]]}
        )
        for row in self.query(FieldRow):
            if row.field.key != _EXTRA_KEY:
                row.set_value(chosen.get(row.field.key, ""))

    def _rows(self) -> list[FieldRow]:
        return [row for row in self.query(FieldRow) if row.field.key != _EXTRA_KEY]

    def collect(self) -> tuple[dict[str, str], list[str]]:
        import shlex

        values = {row.field.key: row.value for row in self._rows()}
        extra_row = next((row for row in self.query(FieldRow) if row.field.key == _EXTRA_KEY), None)
        if extra_row is None:
            return values, []
        try:
            extra = shlex.split(extra_row.value)
        except ValueError:
            extra = [extra_row.value] if extra_row.value else []
        return values, extra

    def action_submit(self) -> None:
        values, extra = self.collect()
        errors = flows.validate(self._plan, values)
        if errors:
            for row in self._rows():
                row.show_error(errors.get(row.field.key))
            return
        self.dismiss((values, extra))

    def action_cancel(self) -> None:
        self.dismiss(None)

    def action_insert_token(self, key: str = "") -> None:
        """Ctrl+T (or the ▾ link on a field): insert a run-time value token at the
        cursor of the focused text field. Secrets are excluded — their values skip
        token expansion by design."""
        if key:
            row = self.query_one(f"#fr-{key}", FieldRow)
        else:
            focused = self.focused
            if not isinstance(focused, Input):
                return
            row = next((a for a in focused.ancestors if isinstance(a, FieldRow)), None)
            if row is None:
                return
        if not row.insertable:
            return
        target = row.query_one(Input)
        target.focus()

        def _insert(token: str | None) -> None:
            if token:
                target.insert_text_at_cursor(token)
                target.focus()

        self.app.push_screen(TokenMenuModal(), _insert)

    def action_save_preset(self) -> None:
        values, _extra = self.collect()

        def _named(name: str | None) -> None:
            if not name:
                return
            argstate.save_preset(
                self._entry.slug,
                name,
                {k: v for k, v in values.items() if v},
                secret_names=self._plan.secret_names,
            )
            self._presets = argstate.load_state(self._entry.slug)["presets"]
            self.notify(
                gettext('Preset "%(preset)s" saved.') % {"preset": name}, severity="information"
            )

        self.app.push_screen(PresetNameModal(set(self._presets)), _named)
