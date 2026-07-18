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
import sys
from pathlib import Path
from typing import override

from rich.markup import escape
from textual import on
from textual.app import ComposeResult
from textual.binding import Binding
from textual.containers import Vertical
from textual.screen import ModalScreen, Screen
from textual.widgets import Input, Label, OptionList, RadioButton, RadioSet, Select, Static
from textual.widgets.option_list import Option

from . import config, i18n, tui_footer
from .i18n import gettext, ngettext

_MIRROR_CHOICES = [*config.PYPI_PRESETS, "custom", "off"]


class SkillInstallModal(ModalScreen[str | None]):
    """Pick an agent directory and install the Agent Skill into it (the TUI face of
    `skit agent install`). Dismisses with the written path, or None. Consent stays
    explicit: nothing is written until a directory is picked (principle #6)."""

    AUTO_FOCUS = "OptionList"
    BINDINGS = [Binding("escape", "cancel", gettext("Cancel"))]
    DEFAULT_CSS = """
    SkillInstallModal { align: center middle; }
    SkillInstallModal > Vertical { border: round $accent; padding: 1 2; width: 76;
        max-width: 100%; height: auto; max-height: 100%; background: $background; }
    SkillInstallModal OptionList { height: auto; border: none; }
    SkillInstallModal .hint { color: $text-muted; }
    SkillInstallModal Static { width: auto; margin: 1 0 0 0; }
    """

    @override
    def compose(self) -> ComposeResult:
        from . import agentskill

        home, cwd = agentskill.default_roots()
        self._targets = agentskill.detect_targets(home=home, cwd=cwd)
        scope_names = {"user": gettext("user"), "project": gettext("project")}
        with Vertical():
            yield Label(gettext("Teach an AI agent to use skit"))
            if self._targets:
                yield OptionList(
                    *(
                        Option(
                            f"{escape(t.name)} ({scope_names[t.scope]})  "
                            f"[dim]{escape(str(t.skills_dir))}[/dim]",
                            id=str(i),
                        )
                        for i, t in enumerate(self._targets)
                    )
                )
            else:
                yield Static(
                    gettext(
                        "No agent directories detected (~/.claude, ~/.codex, ./.agents, …). "
                        "Install by hand with: skit agent install --to DIR"
                    ),
                    classes="hint",
                )
            yield Static(
                tui_footer.bar(tui_footer.chip("screen.cancel", "Esc", gettext("Cancel"))),
                markup=True,
            )

    @on(OptionList.OptionSelected)
    def _picked(self, event: OptionList.OptionSelected) -> None:
        from . import agentskill

        target = self._targets[int(str(event.option.id))]
        try:
            written = agentskill.install_into(target.skills_dir, agentskill.skill_text())
        except OSError as exc:
            self.notify(str(exc), severity="error")
            return
        self.dismiss(str(written))

    def action_cancel(self) -> None:
        self.dismiss(None)


class PreferencesScreen(Screen[bool]):
    def __init__(self) -> None:
        super().__init__()
        self._dirty: bool = False
        # Widgets emit Changed while composing; only user edits after mount count.
        self._dirt_armed: bool = False

    BINDINGS = [
        Binding("escape", "close", gettext("Back")),
        Binding("ctrl+a", "save", gettext("Save"), priority=True),
        Binding("ctrl+n", "manage_runners", gettext("Manage agents"), show=False, priority=True),
        Binding("ctrl+t", "install_skill", gettext("Teach an AI agent"), show=False, priority=True),
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
        self._refresh_runner_count()
        self.call_after_refresh(setattr, self, "_dirt_armed", True)

    @on(Input.Changed)
    @on(RadioSet.Changed)
    @on(Select.Changed)
    def _mark_dirty(self) -> None:
        # Same guard Script settings has: eight sections of unsaved edits must not
        # vanish on a stray Esc while its sibling screen asks first.
        if self._dirt_armed:
            self._dirty = True

    def _refresh_runner_count(self) -> None:
        names = [r.name for r in config.load_prompt_runners()]
        self.query_one("#pf-runner-count", Static).update(
            ngettext(
                "%(count)s agent configured: %(names)s",
                "%(count)s agents configured: %(names)s",
                len(names),
            )
            % {"count": len(names), "names": ", ".join(names)}
            if names
            else gettext("No agents configured.")
        )

    def action_manage_runners(self) -> None:
        """Ctrl+N / the Manage agents… chip: the runner registry, whole (list, edit,
        remove, add) — settings must never be reachable only by hand-editing config."""
        from .tui_runner import RunnerManageScreen

        self.app.push_screen(RunnerManageScreen(), lambda _: self._refresh_runner_count())

    def action_install_skill(self) -> None:
        """Ctrl+T / the Teach an AI agent… chip: install the Agent Skill from the TUI —
        a headline README feature that was CLI-only (`skit agent install`)."""

        def _installed(path: str | None) -> None:
            if path:
                self.notify(gettext("Installed the skit Agent Skill: %(path)s") % {"path": path})

        self.app.push_screen(SkillInstallModal(), _installed)

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

            yield Static(gettext("JavaScript runtime"), classes="section")
            js_current = config.load_js_runner()
            with RadioSet(id="pf-js"):
                yield RadioButton(
                    gettext("Automatic — the first of deno / bun / node found"),
                    value=js_current not in config.JS_RUNNERS,
                )
                for js_name in config.JS_RUNNERS:
                    yield RadioButton(js_name, value=(js_name == js_current))
            yield Static(
                gettext("Runs js/ts entries that don't pin their own runtime."),
                classes="hint",
            )

            if sys.platform == "win32":
                # Windows-only fact, Windows-only section: every other OS just runs
                # bash from PATH, and a section that can never apply is scroll noise.
                yield Static(gettext("Shell on Windows"), classes="section")
                yield Input(
                    value=config.load_bash_path(),
                    placeholder=gettext(r"Path to bash.exe (empty = Git Bash / WSL detection)"),
                    id="pf-bash",
                )
                yield Static(
                    gettext("Shell scripts need an explicit bash here."),
                    classes="hint",
                )
                yield Static("", id="pf-bash-error", classes="error")

            yield Static(gettext("Agents (prompt runners)"), classes="section")
            yield Static("", id="pf-runner-count")
            yield Static(
                tui_footer.bar(
                    tui_footer.chip("screen.manage_runners", "Ctrl+N", gettext("Manage agents…")),
                    tui_footer.chip(
                        "screen.install_skill", "Ctrl+T", gettext("Teach an AI agent skit…")
                    ),
                ),
                id="pf-runner-manage",
                markup=True,
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
        index = self.query_one("#pf-mirror", RadioSet).pressed_index
        return _MIRROR_CHOICES[index] if 0 <= index < len(_MIRROR_CHOICES) else "off"

    def _toggle_custom(self) -> None:
        custom = self._mirror_choice() == "custom"
        for wid in ("#pf-pypi", "#pf-pyinstall", "#pf-uv", "#pf-npm"):
            self.query_one(wid, Input).display = custom
        self.query_one("#pf-uv-error", Static).display = custom

    def action_save(self) -> None:
        lang_value = self.query_one("#pf-lang", Select).value
        i18n.set_language("" if lang_value in ("auto", Select.NULL) else str(lang_value))
        config.save_editor(self.query_one("#pf-editor", Input).value)
        form_index = self.query_one("#pf-form", RadioSet).pressed_index
        config.save_form("plain" if form_index == 1 else "tui")
        after_index = self.query_one("#pf-after", RadioSet).pressed_index
        config.save_after_run("stay" if after_index == 1 else "exit")
        js_index = self.query_one("#pf-js", RadioSet).pressed_index
        config.save_js_runner("" if js_index <= 0 else config.JS_RUNNERS[js_index - 1])
        bash_box = self.query("#pf-bash")
        if bash_box:
            bash_value = bash_box.first(Input).value
            # Same rule as `skit config shell.bash_path` — a typo'd path must not
            # ride into config through this door when the CLI door refuses it.
            if bash_value.strip() and not Path(bash_value).expanduser().is_file():
                self.query_one("#pf-bash-error", Static).update(
                    gettext("No such file: %(path)s") % {"path": escape(bash_value)}
                )
                return
            config.save_bash_path(bash_value)
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
                    npm=self.query_one("#pf-npm", Input).value.strip(),
                )
            )
        else:
            config.save_mirror(config.preset(choice))
        self.dismiss(True)

    def action_close(self) -> None:
        if not self._dirty:
            self.dismiss(False)
            return
        from .tui_settings import DiscardChangesModal

        def _decided(discard: bool | None) -> None:
            if discard:
                self.dismiss(False)

        self.app.push_screen(DiscardChangesModal(), _decided)
