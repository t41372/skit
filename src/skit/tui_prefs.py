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
from dataclasses import replace
from typing import override

from rich.markup import escape
from textual import on
from textual.app import ComposeResult
from textual.binding import Binding
from textual.screen import Screen
from textual.widgets import Input, RadioButton, RadioSet, Select, Static

from . import config, i18n, tui_footer
from .i18n import gettext

# One choice list per mirror axis: each ecosystem has its own vendor landscape, so the
# radios never share a vocabulary (a PyPI vendor name must not pretend to cover npm).
# The master row is the pause switch: "off" keeps the stored URLs (the CLI's
# `skit config mirror off` twin), so a TUI save can never destroy a paused config.
_MASTER_CHOICES = ["on", "off"]
_PYPI_CHOICES = [*config.PYPI_PRESETS, "custom", "off"]
_GITHUB_CHOICES = [*config.GITHUB_RELEASE_PRESETS, "custom", "off"]
_NPM_CHOICES = [*config.NPM_PRESETS, "custom", "off"]


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
    /* Only the mirror rows lay their options side by side — they are single words. The
       form/after sets keep RadioSet's vertical default: their options are sentences,
       and two sentences on one row overflow anything but a very wide terminal. */
    PreferencesScreen .pf-mirror-row { layout: horizontal; }
    PreferencesScreen.-w-narrow .pf-mirror-row { layout: vertical; }
    PreferencesScreen .pf-axis { margin: 1 0 0 0; }
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
                gettext("Download mirrors (mainland-China acceleration)"), classes="section"
            )
            yield Static(
                gettext("Each ecosystem is its own choice — mirror vendors differ per axis."),
                classes="hint",
            )
            mirror = config.load_mirror()
            yield Static(
                gettext('Master switch — "off" pauses mirrors but keeps the saved URLs.'),
                classes="pf-axis",
            )
            # Fresh configs default to "on" so picking any preset just works; "off" is only
            # pre-selected for an explicitly paused config (URLs saved, master off). Known
            # collapse: "off" saved with no URLs reads back as "on" — the two states are
            # behaviorally identical (nothing to pause, mirror_env is empty either way).
            master_on = mirror.enabled or not (
                mirror.pypi or mirror.python_install or mirror.uv_binary or mirror.npm
            )
            with RadioSet(id="pf-mirror-master", classes="pf-mirror-row"):
                for choice in _MASTER_CHOICES:
                    yield RadioButton(choice, value=((choice == "on") == master_on))
            yield Static(gettext("PyPI index (Python packages)"), classes="pf-axis")
            with RadioSet(id="pf-mirror-pypi", classes="pf-mirror-row"):
                for choice in _PYPI_CHOICES:
                    yield RadioButton(choice, value=(choice == config.pypi_choice(mirror)))
            yield Input(value=mirror.pypi, placeholder=gettext("PyPI index URL"), id="pf-pypi")
            yield Static(
                gettext("GitHub releases (Python builds, the uv binary)"), classes="pf-axis"
            )
            with RadioSet(id="pf-mirror-github", classes="pf-mirror-row"):
                for choice in _GITHUB_CHOICES:
                    yield RadioButton(choice, value=(choice == config.github_choice(mirror)))
            yield Input(
                value=config.github_base(mirror),
                placeholder=gettext("github-release mirror base URL"),
                id="pf-github",
            )
            yield Static(gettext("npm registry (JS/TS packages)"), classes="pf-axis")
            with RadioSet(id="pf-mirror-npm", classes="pf-mirror-row"):
                for choice in _NPM_CHOICES:
                    yield RadioButton(choice, value=(choice == config.npm_choice(mirror)))
            yield Input(value=mirror.npm, placeholder=gettext("npm registry URL"), id="pf-npm")
            yield Static("", id="pf-mirror-error", classes="error")
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

    @on(RadioSet.Changed, ".pf-mirror-row")
    def _mirror_changed(self, event: RadioSet.Changed) -> None:
        self._toggle_custom()

    def _axis_choice(self, selector: str, choices: list[str]) -> str:
        index = self.query_one(selector, RadioSet).pressed_index
        return choices[index] if 0 <= index < len(choices) else "off"

    def _toggle_custom(self) -> None:
        """Each axis unhides its own URL input only while that axis sits on "custom"."""
        pypi = self._axis_choice("#pf-mirror-pypi", _PYPI_CHOICES) == "custom"
        github = self._axis_choice("#pf-mirror-github", _GITHUB_CHOICES) == "custom"
        npm = self._axis_choice("#pf-mirror-npm", _NPM_CHOICES) == "custom"
        self.query_one("#pf-pypi", Input).display = pypi
        self.query_one("#pf-github", Input).display = github
        self.query_one("#pf-npm", Input).display = npm
        self.query_one("#pf-mirror-error", Static).display = pypi or github or npm

    def _mirror_error(self, message: str) -> None:
        self.query_one("#pf-mirror-error", Static).update(message)

    def _resolve_mirror(self) -> config.MirrorConfig | None:
        """Resolve the whole mirror block from the form, or None after showing an inline
        error. Runs before ANY write, so a refused save never half-applies."""
        urls: dict[str, str] = {}
        for selector, choices, presets, input_id, key in (
            ("#pf-mirror-pypi", _PYPI_CHOICES, config.PYPI_PRESETS, "#pf-pypi", "pypi"),
            ("#pf-mirror-npm", _NPM_CHOICES, config.NPM_PRESETS, "#pf-npm", "npm"),
        ):
            choice = self._axis_choice(selector, choices)
            if choice == "off":
                urls[key] = ""
            elif choice == "custom":
                value = self.query_one(input_id, Input).value.strip()
                # Same URL gate as the CLI axis keys and the wizard: an empty custom must
                # not silently save as off (the radio would lie), and a non-URL typo must
                # not persist to surface later as a broken UV_DEFAULT_INDEX.
                if not config.is_url_token(value):
                    self._mirror_error(gettext("A custom choice needs a URL."))
                    return None
                urls[key] = value
            else:
                urls[key] = presets[choice]
        github_pair = self._resolve_github()
        if github_pair is None:
            return None
        python_install, uv_binary = github_pair
        resolved = config.compose(python_install=python_install, uv_binary=uv_binary, **urls)
        if self._axis_choice("#pf-mirror-master", _MASTER_CHOICES) == "off":
            # Pause, don't destroy: the stored URLs survive for `mirror on` / the return trip.
            resolved = replace(resolved, enabled=False)
        return resolved

    def _resolve_github(self) -> tuple[str, str] | None:
        """The github axis's (python_install, uv_binary) pair from the form, or None after
        showing an inline error."""
        github = self._axis_choice("#pf-mirror-github", _GITHUB_CHOICES)
        if github == "off":
            return ("", "")
        if github != "custom":
            return config.github_release_urls(config.GITHUB_RELEASE_PRESETS[github])
        base = self.query_one("#pf-github", Input).value.strip()
        stored = config.load_mirror()
        if (
            not base
            and not config.github_base(stored)
            and (stored.python_install or stored.uv_binary)
        ):
            # A hand-edited pair no base derives: the input prefills empty, so an untouched
            # save (e.g. changing only the language) passes the stored pair through as-is
            # instead of refusing the whole form over an axis the user never touched.
            return (stored.python_install, stored.uv_binary)
        if not config.is_url_token(base):
            # Empty, or garbage (whitespace, a vendor name): the same shared gate as the
            # CLI and wizard — a base with a space would sail through the https check and
            # blow up much later, inside the uv bootstrap.
            self._mirror_error(gettext("A custom choice needs a URL."))
            return None
        if not base.startswith("https://"):
            self._mirror_error(
                gettext(
                    "The uv binary is downloaded and executed, so the github-release "
                    "base URL must use https:// (got: %(url)s)."
                )
                % {"url": escape(base)}
            )
            return None
        return config.github_release_urls(base)

    def action_save(self) -> None:
        # Resolve + validate the mirror block BEFORE any write: a refused save must not
        # half-apply (language/editor/form silently persisting while the mirror errors).
        mirror_cfg = self._resolve_mirror()
        if mirror_cfg is None:
            return
        lang_value = self.query_one("#pf-lang", Select).value
        i18n.set_language("" if lang_value in ("auto", Select.BLANK) else str(lang_value))
        editor_input = self.query_one("#pf-editor", Input)  # pragma: no mutate
        config.save_editor(editor_input.value)
        form_set = self.query_one("#pf-form", RadioSet)  # pragma: no mutate
        form_index = form_set.pressed_index
        config.save_form("plain" if form_index == 1 else "tui")
        after_set = self.query_one("#pf-after", RadioSet)  # pragma: no mutate
        after_index = after_set.pressed_index
        config.save_after_run("stay" if after_index == 1 else "exit")
        config.save_mirror(mirror_cfg)
        self.dismiss(True)

    def action_close(self) -> None:
        self.dismiss(False)
