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
from textual.widgets import (
    Checkbox,
    Input,
    Label,
    OptionList,
    RadioButton,
    RadioSet,
    Select,
    Static,
)
from textual.widgets.option_list import Option

from . import argstate, flows, tokens, tui_footer, tui_runner
from .i18n import gettext

if TYPE_CHECKING:
    from .models import Entry

# The submit result: (raw field values, extra passthrough args, picked runner name)
# or None on cancel. The runner element is None unless the form showed a runner picker
# (prompt entries in the TUI workbench); the CLI resolves its runner before the form.
FormResult = tuple[dict[str, str], list[str], str | None] | None

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
            expanded, error = tokens.preview(
                value, cwd=Path.cwd(), brace_escapes=self.field.source != "placeholder"
            )
            if expanded != value or error:
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
        border: round $accent; padding: 1 2; width: 60; max-width: 100%; height: auto;
        max-height: 100%; background: $background;
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
        max-width: 100%; height: auto; max-height: 100%; background: $background; }
    TokenMenuModal OptionList { height: auto; border: none; }
    /* Short terminals: cap the list AND flatten the box's vertical padding so the
       fixed chrome (border+title+chip = 5 rows) plus the list fits the whole tier
       band — the Esc chip must stay on screen (it is the modal's mouse path out);
       the OptionList scrolls internally and keeps its highlight in view. */
    TokenMenuModal.-h-short > Vertical, TokenMenuModal.-h-tiny > Vertical { padding: 0 2; }
    TokenMenuModal.-h-short OptionList { max-height: 4; }
    TokenMenuModal.-h-tiny OptionList { max-height: 2; }
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
        max-width: 100%; height: auto; max-height: 100%; background: $background; }
    EnvPickerModal OptionList { border: none; max-height: 12; }
    /* The fixed chrome costs 10 rows (border+padding+title+input+chip), so the list
       cap must shrink with the tier for the Esc chip — the modal's mouse path out —
       to stay on screen across the WHOLE band, not just its top: 10+6 fits >=16
       (normal); flattening the box padding and the chip margin cuts the chrome to 7,
       so 7+3 fits >=10 (short) and 7+1 fits the tiny floor. (The input keeps its
       border: user CSS — CHROME's Input rules — outranks anything a screen's
       DEFAULT_CSS says, even !important, so it cannot be flattened from here.)
       Filtering still works at every size — typing a full name submits even with
       the list clipped. */
    EnvPickerModal.-h-normal OptionList { max-height: 6; }
    EnvPickerModal.-h-short > Vertical, EnvPickerModal.-h-tiny > Vertical { padding: 0 2; }
    EnvPickerModal.-h-short OptionList { max-height: 3; }
    EnvPickerModal.-h-tiny OptionList { max-height: 1; }
    EnvPickerModal Static { width: auto; margin: 1 0 0 0; }
    EnvPickerModal.-h-short Static, EnvPickerModal.-h-tiny Static { margin: 0; }
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
        Binding("ctrl+n", "new_runner", gettext("New agent"), show=False),
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
    RunFormScreen #preset-row, RunFormScreen #runner-row { height: auto; padding: 0 1; }
    /* Narrow terminals: the caption-beside-list row overflows a Horizontal (it never
       wraps) — stack caption above the list instead. The pickers themselves are
       dropdowns collapse to one row and their overlays scale on their own. */
    RunFormScreen.-w-narrow #preset-row, RunFormScreen.-w-narrow #runner-row { layout: vertical; }
    RunFormScreen.-w-narrow FieldRow RadioSet { layout: vertical; }
    /* Widgets default to width:1fr; in a Horizontal that lets the "Preset:" caption
       swallow the whole row and push the list (or the empty-state hint) clean off the
       screen. Everything in this row hugs its content. */
    RunFormScreen #preset-row Static, RunFormScreen #runner-row Static {
        width: auto; margin: 0 1 0 0;
    }
    /* The empty-state hint is a long sentence: let it take the row's remaining width and
       wrap, rather than width:auto (its full content width) which overflows a narrow form.
       Two ids outrank the `#preset-row Static` width:auto rule above. */
    RunFormScreen #preset-row #preset-empty { color: $text-muted; width: 1fr; height: auto; }
    RunFormScreen #form-body { padding: 0 1; }
    /* Chips wrap pill-by-pill; visible lines follow the height tier and anything
       past the cap stays wheel-reachable — see tui_footer.KeysBar. */
    RunFormScreen KeysBar { dock: bottom; }
    RunFormScreen #form-keys { color: $text-muted; }

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
        runners: list[str] | None = None,
        runner_default: str = "",
    ) -> None:
        super().__init__()
        self._entry: Entry = entry
        self._plan: flows.FormPlan = plan
        self._prefill: dict[str, str] = prefill
        # The inline (CLI) frame hides the extra-args row: argv already owns passthrough
        # args there, and two sources for the same thing would fight.
        self._include_extra: bool = include_extra
        # Prompt entries: the runner picker row (mouse- and keyboard-operable, like the
        # preset chips). The workbench passes the configured names; the CLI's inline
        # frame passes none — it resolved the runner before opening the form.
        self._runners: list[str] = runners or []
        self._runner_default: str = runner_default
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
            if self._runners:
                with Horizontal(id="runner-row"):
                    yield Static(gettext("Runner:"), markup=False)
                    default = (
                        self._runner_default
                        if self._runner_default in self._runners
                        else self._runners[0]
                    )
                    # A dropdown, deliberately: the runner is a SECONDARY control (the
                    # pin or last pick is usually right) — collapsed it costs one row
                    # instead of pushing the actual parameter fields down the screen,
                    # and the overlay scales to any number of agents.
                    yield Select(
                        [(name, name) for name in self._runners],
                        value=default,
                        allow_blank=False,
                        id="runner-select",
                    )
                    # Custom agents are first-class: the picker always carries the door
                    # to define one (footer grammar — the key hint IS the click target).
                    yield Static(tui_runner.new_runner_chip(), id="runner-new", markup=True)
            with Horizontal(id="preset-row"):
                yield Static(gettext("Preset:"), markup=False)
                if self._presets:
                    yield Select(
                        [(gettext("last values"), "")]
                        + [(name, name) for name in sorted(self._presets)],
                        value="",
                        allow_blank=False,
                        id="preset-select",
                    )
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
                    from .langs.registry import spec_for

                    spec = spec_for(self._entry.meta.kind)
                    takes_argv = spec is None or spec.takes_argv
                    extra_field = flows.FormField(
                        key=_EXTRA_KEY,
                        # Prompts: the extra args go to the AGENT's command line, not a
                        # script — the label must not lie about where they land.
                        label=(
                            gettext("Extra arguments (passed to the script as-is)")
                            if takes_argv
                            else gettext("Extra agent arguments (appended to the runner command)")
                        ),
                        source="flag",
                    )
                    # Replay semantics mirror the CLI's takes_argv rule: a kind whose
                    # "arguments" are its placeholders never gets a remembered argv tail
                    # prefilled — the CLI deliberately refuses to replay there, and the
                    # form must not resurrect the same surprise.
                    last_extra = (
                        argstate.load_state(self._entry.slug)["extra_args"] if takes_argv else []
                    )
                    # shlex.join, because collect() shlex.split()s: an argument that
                    # contains spaces must survive the round trip as ONE argument.
                    import shlex

                    yield FieldRow(extra_field, shlex.join(last_extra))
        yield tui_footer.KeysBar(
            Static(
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
        )

    @on(Select.Changed, "#preset-select")
    def _apply_preset(self, event: Select.Changed) -> None:
        """Dropdown switch: overlay the whole preset onto the fields ("last values"
        restores). Value-keyed, so a preset list that changed since compose can never
        shift the mapping."""
        name = str(event.value)
        preset_values = self._presets.get(name, {})
        chosen = self._prefill if not name else {**self._prefill, **preset_values}
        for row in self.query(FieldRow):
            if row.field.key != _EXTRA_KEY:
                row.set_value(chosen.get(row.field.key, ""))

    def _rows(self) -> list[FieldRow]:
        return [row for row in self.query(FieldRow) if row.field.key != _EXTRA_KEY]

    def picked_runner(self) -> str | None:
        """The runner picker's selection, or None when the form has no picker."""
        if not self._runners:
            return None
        value = self.query_one("#runner-select", Select).value
        return None if value is Select.BLANK else str(value)

    def action_new_runner(self) -> None:
        """Ctrl+N / the New agent… chip: define a custom runner without leaving the
        form — it lands in config, joins the picker, and is selected immediately."""
        if not self._runners:
            return  # no picker on this form (non-prompt entry, or the CLI inline frame)

        def _added(name: str | None) -> None:
            if not name:
                return
            self._runners.append(name)
            select = self.query_one("#runner-select", Select)
            select.set_options([(n, n) for n in self._runners])
            select.value = name

        self.app.push_screen(tui_runner.RunnerAddModal(), _added)

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
        # Enter is priority-bound to submit-from-any-field; when a dropdown owns the
        # focus, Enter must keep operating the dropdown instead (open it, or choose
        # the highlighted option in its overlay) — the shim that lets Select coexist
        # with the submit muscle memory.
        focused = self.focused
        if isinstance(focused, Select):
            focused.expanded = not focused.expanded
            return
        if focused is not None and any(isinstance(a, Select) for a in focused.ancestors):
            if isinstance(focused, OptionList) and focused.highlighted is not None:
                focused.action_select()
            return
        values, extra = self.collect()
        errors = flows.validate(self._plan, values)
        if errors:
            for row in self._rows():
                row.show_error(errors.get(row.field.key))
            return
        self.dismiss((values, extra, self.picked_runner()))

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
