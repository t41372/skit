"""The CLI's inline mini-form: the run form opened in place, in the same terminal.

`skit run NAME` on a TTY (with `form = "tui"`, the default) opens the same RunFormScreen
the TUI uses — via Textual's inline mode, so there's no alternate screen and the
scrollback survives. Submit collapses the form and the script runs right below it;
`--plain` / `form = "plain"` / TERM=dumb fall back to line prompts instead.

The extra-arguments row is hidden here: on the CLI, passthrough args already arrived
via `skit run NAME -- <args>` (argv owns them; two sources would fight). The runner
picker row IS hosted here when the caller passes the configured names — one
interaction paradigm per run, instead of a line prompt glued to a Textual form.
"""

from __future__ import annotations

from typing import TYPE_CHECKING, override

from textual.app import App

from . import flows, theme, tui_layout
from .theme import CLAUDE_THEME
from .tui_form import FormResult, RunFormScreen

if TYPE_CHECKING:
    from .models import Entry


class _InlineFormApp(App[FormResult]):
    ENABLE_COMMAND_PALETTE = False
    # Width tiers only. An inline screen is sized to its CONTENT (capped at 80% of
    # the terminal), so a height tier computed from the screen size would classify a
    # compact form as -h-short on a 50-row terminal and clip its own footer; the
    # RunFormScreen:inline rules already govern the vertical behavior here.
    HORIZONTAL_BREAKPOINTS = tui_layout.HORIZONTAL_BREAKPOINTS
    CSS = theme.CHROME_CSS

    def __init__(
        self,
        entry: Entry,
        plan: flows.FormPlan,
        prefill: dict[str, str],
        runners: list[str] | None = None,
        runner_default: str = "",
    ) -> None:
        super().__init__()
        self._entry: Entry = entry
        self._plan: flows.FormPlan = plan
        self._prefill: dict[str, str] = prefill
        self._runners: list[str] = runners or []
        self._runner_default: str = runner_default

    @override
    def get_css_variables(self) -> dict[str, str]:
        # The first stylesheet parse runs before on_mount activates the theme; the
        # screen CSS needs $skit-box-* resolvable from the very first frame.
        return {**super().get_css_variables(), **theme.BOX_VARIABLES}

    def on_mount(self) -> None:
        self.register_theme(CLAUDE_THEME)
        self.theme = "skit-claude"

        def _done(result: FormResult) -> None:
            self.exit(result)

        self.push_screen(
            RunFormScreen(
                self._entry,
                self._plan,
                self._prefill,
                include_extra=False,
                runners=self._runners,
                runner_default=self._runner_default,
            ),
            _done,
        )


def collect(
    entry: Entry,
    plan: flows.FormPlan,
    prefill: dict[str, str],
    runners: list[str] | None = None,
    runner_default: str = "",
) -> tuple[dict[str, str], str | None] | None:
    """Run the inline form; returns (raw values, picked runner name or None when the
    form had no picker), or None when the user cancelled."""
    app = _InlineFormApp(entry, plan, prefill, runners, runner_default)
    result = app.run(inline=True)
    if result is None:
        return None
    values, _extra, runner = result
    return values, runner
