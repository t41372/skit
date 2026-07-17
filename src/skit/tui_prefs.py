"""Preferences (,): skit's global settings, one screen, everything visible.

Every setting shows what is ACTUALLY in effect right now (the most common question a
settings screen gets is "what happens if I leave this empty"). Language is a dropdown
(the locale list will grow); the form style governs the CLI's parameter prompts; the
after-run choice decides whether skit quits like a launcher or loops like a workbench;
the custom mirror enforces https for the uv binary inline (downloaded-and-executed ⇒
MITM→RCE).
"""

from __future__ import annotations

import os
from typing import override

from rich.markup import escape
from textual import on
from textual.app import ComposeResult
from textual.binding import Binding
from textual.screen import Screen
from textual.widgets import Input, RadioButton, RadioSet, Select, Static

from . import config, i18n, tui_footer
from .i18n import gettext

_MIRROR_CHOICES = [*config.PYPI_PRESETS, "custom", "off"]


class PreferencesScreen(Screen[bool]):
    BINDINGS = [
        Binding("escape", "close", gettext("Back")),
        Binding("ctrl+a", "save", gettext("Save"), priority=True),
        *tui_footer.FIELD_NAV_BINDINGS,
    ]
    # Boot on the language dropdown, not the "*" pick (the body scroll container).
    AUTO_FOCUS = "Select, Input"
    DEFAULT_CSS = """
    PreferencesScreen #pf-body {
        padding: 0 1;
        border: round $skit-box-indigo;
        border-title-color: ansi_bright_white;
        border-title-style: bold;
    }
    PreferencesScreen .section { color: $accent; margin: 1 0 0 0; }
    PreferencesScreen .hint { color: $text-muted; }
    PreferencesScreen .error { color: $error; }
    PreferencesScreen RadioSet { height: auto; border: none; }
    PreferencesScreen RadioSet > RadioButton { width: auto; margin: 0 3 0 0; }
    /* Only the mirror row lays its options side by side — they are single words. The
       form/after sets keep RadioSet's vertical default: their options are sentences,
       and two sentences on one row overflow anything but a very wide terminal. */
    PreferencesScreen #pf-mirror { layout: horizontal; }
    PreferencesScreen.-w-narrow #pf-mirror { layout: vertical; }
    PreferencesScreen KeysBar { dock: bottom; }
    PreferencesScreen #pf-keys { color: $text-muted; }
    """

    def on_mount(self) -> None:
        self.query_one("#pf-body").border_title = gettext("Preferences")
        self._toggle_custom()

    @override
    def compose(self) -> ComposeResult:
        with tui_footer.FormBody(id="pf-body"):
            yield Static(gettext("Interface language"), classes="section")
            current = config.load_config().get("language", "")
            options = [(gettext("Automatic (follow the system)"), "auto")]
            options += [(locale, locale) for locale in i18n.available_locales()]
            yield Select(
                options, value=current if current else "auto", allow_blank=False, id="pf-lang"
            )
            yield Static(
                gettext("Currently in effect: %(locale)s") % {"locale": i18n.current_locale()},
                classes="hint",
            )

            yield Static(gettext("Editor"), classes="section")
            yield Input(
                value=config.load_editor(),
                placeholder=gettext("e.g. code --wait (empty = use $VISUAL / $EDITOR)"),
                id="pf-editor",
            )
            fallback = os.environ.get("VISUAL") or os.environ.get("EDITOR") or ""
            if fallback:
                yield Static(
                    gettext("Empty means: %(cmd)s (from $VISUAL / $EDITOR)")
                    % {"cmd": escape(fallback)},
                    classes="hint",
                )

            yield Static(gettext("Interactive form"), classes="section")
            with RadioSet(id="pf-form"):
                yield RadioButton(
                    gettext("Mini form — opens in place, fully clickable"),
                    value=config.load_form() == "tui",
                )
                yield RadioButton(
                    gettext("Line-by-line prompts — plainest, best over slow terminals"),
                    value=config.load_form() == "plain",
                )
            yield Static(
                gettext(
                    "Used by terminal runs: `skit run` parameter prompts and the "
                    "`skit add` review panel."
                ),
                classes="hint",
            )

            yield Static(gettext("After a run (from this menu)"), classes="section")
            with RadioSet(id="pf-after"):
                yield RadioButton(
                    gettext("Quit skit — leave the script's output in the terminal"),
                    value=config.load_after_run() == "exit",
                )
                yield RadioButton(
                    gettext("Return to the Library"),
                    value=config.load_after_run() == "stay",
                )

            yield Static(
                gettext("Download mirror (mainland-China acceleration)"), classes="section"
            )
            mirror = config.load_mirror()
            if not mirror.enabled:
                selected = "off"
            else:
                selected = next(
                    (k for k, v in config.PYPI_PRESETS.items() if v == mirror.pypi), "custom"
                )
            with RadioSet(id="pf-mirror"):
                for choice in _MIRROR_CHOICES:
                    yield RadioButton(choice, value=(choice == selected))
            yield Static(
                gettext("Affects where dependencies and Python itself are downloaded from."),
                classes="hint",
            )
            yield Input(value=mirror.pypi, placeholder=gettext("PyPI index URL"), id="pf-pypi")
            yield Input(
                value=mirror.python_install,
                placeholder=gettext("Python-install mirror URL"),
                id="pf-pyinstall",
            )
            yield Input(
                value=mirror.uv_binary, placeholder=gettext("uv binary mirror URL"), id="pf-uv"
            )
            yield Input(value=mirror.npm, placeholder=gettext("npm registry URL"), id="pf-npm")
            yield Static("", id="pf-uv-error", classes="error")
        yield tui_footer.KeysBar(
            Static(
                tui_footer.bar(
                    tui_footer.chip("screen.save", "Ctrl+A", gettext("Save")),
                    tui_footer.chip("screen.close", "Esc", gettext("Back")),
                    tui_footer.nav_chip(),
                ),
                id="pf-keys",
                markup=True,
            )
        )

    @on(RadioSet.Changed, "#pf-mirror")
    def _mirror_changed(self, event: RadioSet.Changed) -> None:
        self._toggle_custom()

    def _mirror_choice(self) -> str:
        # query_one("#id", Type) is a tautological guard: the id fixes the widget and its
        # type, so its selector/type arguments carry no behaviour to mutate — pinned.
        mirror_set = self.query_one("#pf-mirror", RadioSet)  # pragma: no mutate
        index = mirror_set.pressed_index
        # pressed_index is a live button index (0..len-1) or -1; the "< len" upper bound is
        # never reached, so the comparison is pinned while the "off" fallback stays mutated.
        if 0 <= index < len(_MIRROR_CHOICES):  # pragma: no mutate
            return _MIRROR_CHOICES[index]
        return "off"

    def _toggle_custom(self) -> None:
        custom = self._mirror_choice() == "custom"
        # query_one("#id", Type) — tautological guards (see _mirror_choice).
        for wid in ("#pf-pypi", "#pf-pyinstall", "#pf-uv", "#pf-npm"):
            self.query_one(wid, Input).display = custom  # pragma: no mutate
        self.query_one("#pf-uv-error", Static).display = custom  # pragma: no mutate

    def action_save(self) -> None:
        # Each query_one("#id", Type) below is a tautological guard (see _mirror_choice),
        # isolated onto its own line so the surrounding save logic stays mutation-tested.
        lang_select = self.query_one("#pf-lang", Select)  # pragma: no mutate
        lang_value = lang_select.value
        i18n.set_language("" if lang_value in ("auto", Select.BLANK) else str(lang_value))
        editor_input = self.query_one("#pf-editor", Input)  # pragma: no mutate
        config.save_editor(editor_input.value)
        form_set = self.query_one("#pf-form", RadioSet)  # pragma: no mutate
        form_index = form_set.pressed_index
        config.save_form("plain" if form_index == 1 else "tui")
        after_set = self.query_one("#pf-after", RadioSet)  # pragma: no mutate
        after_index = after_set.pressed_index
        config.save_after_run("stay" if after_index == 1 else "exit")
        choice = self._mirror_choice()
        if choice == "off":
            config.disable()
        elif choice == "custom":
            uv_input = self.query_one("#pf-uv", Input)  # pragma: no mutate
            uv_url = uv_input.value.strip()
            if uv_url and not uv_url.startswith("https://"):
                error_field = self.query_one("#pf-uv-error", Static)  # pragma: no mutate
                error_field.update(
                    gettext(
                        "The uv binary is downloaded and executed, so its mirror URL must "
                        "use https:// (got: %(url)s)."
                    )
                    % {"url": escape(uv_url)}
                )
                return
            pypi_input = self.query_one("#pf-pypi", Input)  # pragma: no mutate
            pyinstall_input = self.query_one("#pf-pyinstall", Input)  # pragma: no mutate
            npm_input = self.query_one("#pf-npm", Input)  # pragma: no mutate
            config.save_mirror(
                config.MirrorConfig(
                    enabled=True,
                    pypi=pypi_input.value.strip(),
                    python_install=pyinstall_input.value.strip(),
                    uv_binary=uv_url,
                    npm=npm_input.value.strip(),
                )
            )
        else:
            config.save_mirror(config.preset(choice))
        self.dismiss(True)

    def action_close(self) -> None:
        self.dismiss(False)
