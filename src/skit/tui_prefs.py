"""Preferences (,): skit's global settings, one screen, everything visible.

Every setting shows what is ACTUALLY in effect right now (the most common question a
settings screen gets is "what happens if I leave this empty"). Language is a dropdown
(the locale list will grow); the form style governs every interactive flow; the custom
mirror enforces https for the uv binary inline (downloaded-and-executed ⇒ MITM→RCE).
"""

from __future__ import annotations

import os
from typing import override

from rich.markup import escape
from textual import on
from textual.app import ComposeResult
from textual.binding import Binding
from textual.containers import VerticalScroll
from textual.screen import Screen
from textual.widgets import Input, RadioButton, RadioSet, Select, Static

from . import config, i18n, tui_footer
from .i18n import gettext

_MIRROR_CHOICES = [*config.PYPI_PRESETS, "custom", "off"]


class PreferencesScreen(Screen[bool]):
    BINDINGS = [
        Binding("escape", "close", gettext("Back")),
        Binding("ctrl+a", "save", gettext("Save"), priority=True),
    ]
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
    PreferencesScreen RadioSet { layout: horizontal; height: auto; border: none; }
    PreferencesScreen RadioSet > RadioButton { width: auto; margin: 0 3 0 0; }
    PreferencesScreen #pf-keys { dock: bottom; height: 1; color: $text-muted; padding: 0 1; }
    """

    def on_mount(self) -> None:
        self.query_one("#pf-body").border_title = gettext("Preferences")
        self._toggle_custom()

    @override
    def compose(self) -> ComposeResult:
        with VerticalScroll(id="pf-body"):
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
            yield Static("", id="pf-uv-error", classes="error")
        yield Static(
            tui_footer.bar(
                tui_footer.chip("screen.save", "Ctrl+A", gettext("Save")),
                tui_footer.chip("screen.close", "Esc", gettext("Back")),
            ),
            id="pf-keys",
            markup=True,
        )

    @on(RadioSet.Changed, "#pf-mirror")
    def _mirror_changed(self, event: RadioSet.Changed) -> None:
        self._toggle_custom()

    def _mirror_choice(self) -> str:
        index = self.query_one("#pf-mirror", RadioSet).pressed_index
        return _MIRROR_CHOICES[index] if 0 <= index < len(_MIRROR_CHOICES) else "off"

    def _toggle_custom(self) -> None:
        custom = self._mirror_choice() == "custom"
        for wid in ("#pf-pypi", "#pf-pyinstall", "#pf-uv"):
            self.query_one(wid, Input).display = custom
        self.query_one("#pf-uv-error", Static).display = custom

    def action_save(self) -> None:
        lang_value = self.query_one("#pf-lang", Select).value
        i18n.set_language("" if lang_value in ("auto", Select.BLANK) else str(lang_value))
        config.save_editor(self.query_one("#pf-editor", Input).value)
        form_index = self.query_one("#pf-form", RadioSet).pressed_index
        config.save_form("plain" if form_index == 1 else "tui")
        choice = self._mirror_choice()
        if choice == "off":
            config.disable()
        elif choice == "custom":
            uv_url = self.query_one("#pf-uv", Input).value.strip()
            if uv_url and not uv_url.startswith("https://"):
                self.query_one("#pf-uv-error", Static).update(
                    gettext(
                        "The uv binary is downloaded and executed, so its mirror URL must "
                        "use https:// (got: %(url)s)."
                    )
                    % {"url": escape(uv_url)}
                )
                return
            config.save_mirror(
                config.MirrorConfig(
                    enabled=True,
                    pypi=self.query_one("#pf-pypi", Input).value.strip(),
                    python_install=self.query_one("#pf-pyinstall", Input).value.strip(),
                    uv_binary=uv_url,
                )
            )
        else:
            config.save_mirror(config.preset(choice))
        self.dismiss(True)

    def action_close(self) -> None:
        self.dismiss(False)
