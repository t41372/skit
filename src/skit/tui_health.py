"""Health check (D): the checklist that can also act.

Every ⚠ row is selectable — Enter jumps the Library to that script; R rebuilds the
index in place. Checks only what existing capabilities can actually verify (uv, the
registry↔meta correspondence, missing targets, the one deliberate whole-library drift
sweep, mirror state). Nothing invented.
"""

from __future__ import annotations

from typing import override

from rich.markup import escape
from textual import on
from textual.app import ComposeResult
from textual.binding import Binding
from textual.containers import VerticalScroll
from textual.screen import Screen
from textual.widgets import OptionList, Static
from textual.widgets.option_list import Option

from . import config, flows, launcher, store, tui_footer
from .i18n import gettext, ngettext
from .langs.registry import spec_for
from .paths import scripts_dir


class HealthScreen(Screen[str | None]):
    """Dismisses with a slug to select in the Library, or None."""

    BINDINGS = [
        Binding("escape", "close", gettext("Back")),
        Binding("R", "rebuild", gettext("Rebuild index")),
    ]
    DEFAULT_CSS = """
    HealthScreen #hc-body {
        padding: 0 1;
        border: round $skit-box-green;
        border-title-color: ansi_bright_white;
        border-title-style: bold;
    }
    HealthScreen #hc-issues { height: auto; border: none; }
    HealthScreen .ok { color: $success; }
    HealthScreen .warn { color: $warning; }
    HealthScreen .bad { color: $error; }
    HealthScreen .hint { color: $text-muted; }
    HealthScreen KeysBar { dock: bottom; }
    HealthScreen #hc-keys { color: $text-muted; }
    """

    def on_mount(self) -> None:
        self.query_one("#hc-body").border_title = gettext("Health check")

    @override
    def compose(self) -> ComposeResult:
        with VerticalScroll(id="hc-body"):
            uv = launcher.find_uv()
            if uv:
                yield Static(f"✓ {gettext('uv: %(path)s') % {'path': escape(uv)}}", classes="ok")
            else:
                yield Static(
                    f"✗ {gettext('uv: not found. Install it from https://docs.astral.sh/uv/getting-started/installation/')}",
                    classes="bad",
                )
            entries = store.list_entries()
            yield Static(
                "✓ "
                + ngettext(
                    "%(count)s script registered", "%(count)s scripts registered", len(entries)
                )
                % {"count": len(entries)},
                classes="ok",
            )
            issues: list[Option] = [
                Option(
                    f"⚠ {escape(e.meta.name)} — " + gettext("the launch target is gone from disk"),
                    id=e.slug,
                )
                for e in entries
                if launcher.target_missing(e)
            ]
            for e in entries:
                spec = spec_for(e.meta.kind)
                if spec is None or spec.analyzer is None or not e.script_path.exists():
                    continue
                if flows.plan_for_entry(e).drift_lines:
                    issues.append(
                        Option(
                            f"⚠ {escape(e.meta.name)} — "
                            + gettext(
                                "form definitions are out of sync (open Script settings → Resync)"
                            ),
                            id=e.slug,
                        )
                    )
            for e in entries:
                missing_tools = launcher.missing_needs(e)
                if missing_tools:
                    issues.append(
                        Option(
                            f"⚠ {escape(e.meta.name)} — "
                            + gettext("missing external command(s): %(tools)s")
                            % {"tools": ", ".join(escape(t) for t in missing_tools)},
                            id=e.slug,
                        )
                    )
            if issues:
                yield Static(gettext("Issues (Enter jumps to the script):"), classes="warn")
                yield OptionList(*issues, id="hc-issues")
            yield Static("✓ " + escape(config.mirrors_line(config.load_mirror())), classes="ok")
            location = scripts_dir()
            yield Static(
                gettext("Library: %(path)s (%(count)s · %(size)s)")
                % {
                    "path": escape(str(location)),
                    "count": len(entries),
                    "size": store.human_size(store.dir_size(location)),
                },
                classes="hint",
            )
            yield Static("", id="hc-rebuilt", classes="ok")
        yield tui_footer.KeysBar(
            Static(
                tui_footer.bar(
                    tui_footer.chip("screen.jump", "Enter", gettext("Jump to script")),
                    tui_footer.chip("screen.rebuild", "R", gettext("Rebuild index")),
                    tui_footer.chip("screen.close", "Esc", gettext("Back")),
                ),
                id="hc-keys",
                markup=True,
            )
        )

    @on(OptionList.OptionSelected, "#hc-issues")
    def _jump(self, event: OptionList.OptionSelected) -> None:
        self.dismiss(str(event.option.id))

    def action_jump(self) -> None:
        """Footer/Enter twin of clicking an issue line: dismiss to the highlighted
        script. A no-op when the store is healthy (there is no issue list to jump into)."""
        issues = self.query("#hc-issues")
        if not issues:
            return
        option_list = issues.first(OptionList)  # pragma: no mutate — tautological guard
        if option_list.highlighted is not None:
            self.dismiss(str(option_list.get_option_at_index(option_list.highlighted).id))

    def action_rebuild(self) -> None:
        count, problems = store.doctor_rebuild()
        report = self.query_one("#hc-rebuilt", Static)  # pragma: no mutate — tautological guard
        lines = [
            ngettext("Index rebuilt: %(count)s entry", "Index rebuilt: %(count)s entries", count)
            % {"count": count}
        ]
        lines += [escape(p) for p in problems]
        report.update("\n".join(lines))

    def action_close(self) -> None:
        self.dismiss(None)
