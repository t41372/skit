"""Add flow in the TUI: source step → single review panel.

The review panel is one always-editable surface — no wizard sequence to march through;
Enter accepts everything as reviewed. Detection honesty rules render here: signal-
driven checkbox defaults, the accumulator warning, filename-literal hints, and the
"the script declares its own dependencies" read-only variant.

Every review panel has two faces: pushed from the Library (`a`), and hosted alone by
a _ReviewHost app when a terminal `skit add` runs interactively — same screen, so the
CLI and the TUI can never drift apart. Python gets AddReviewScreen; prompts get its
twin PromptReviewScreen (insertion switch, placeholder ticks, runner pick).
"""

from __future__ import annotations

from pathlib import Path
from typing import override

from rich.markup import escape
from textual import on
from textual.app import App, ComposeResult
from textual.binding import Binding
from textual.containers import Vertical
from textual.screen import Screen
from textual.widgets import Checkbox, Input, RadioButton, RadioSet, Static

from . import (
    analysis,
    argstate,
    config,
    editor,
    pep723,
    store,
    theme,
    tui_footer,
    tui_layout,
    tui_runner,
)
from .i18n import gettext
from .langs.prompt import analyzer as prompt_analyzer
from .params import ParamDecl, is_secret_name


class AddSourceScreen(Screen[str | None]):
    """Step 1: where does the script come from? Returns the new entry's slug, or None."""

    BINDINGS = [
        Binding("escape", "cancel", gettext("Cancel")),
        Binding("ctrl+e", "draft_script", gettext("Write a new script"), priority=True),
        Binding("ctrl+p", "draft_prompt", gettext("Draft a prompt"), priority=True),
        *tui_footer.FIELD_NAV_BINDINGS,
    ]
    # Boot on the path field, not the "*" pick (the body scroll container).
    AUTO_FOCUS = "Input"
    DEFAULT_CSS = """
    /* The border lives on the body, not the Screen: a bordered Screen offsets its
       coordinate space and bottom-docked footer clicks land "outside" it. */
    AddSourceScreen #add-body {
        padding: 1;
        border: round $skit-box-olive;
        border-title-color: ansi_bright_white;
        border-title-style: bold;
    }
    AddSourceScreen .hint { color: $text-muted; }
    /* Chips wrap pill-by-pill; visible lines follow the height tier and anything
       past the cap stays wheel-reachable — see tui_footer.KeysBar. */
    AddSourceScreen KeysBar { dock: bottom; }
    """

    def on_mount(self) -> None:
        self.query_one("#add-body").border_title = gettext("Add a script")

    @override
    def compose(self) -> ComposeResult:
        # FormBody, not a plain Vertical: on a short terminal a fixed body puts the
        # template/name fields under the docked footer with no way to reveal them —
        # the scroll body keeps every field reachable (focus pulls it into view).
        with tui_footer.FormBody(id="add-body"):
            yield Static(gettext("Path to a script, executable, or prompt:"))
            yield Input(placeholder="~/scripts/tool.py", id="add-path")
            yield Static("", id="add-error", markup=True)
            yield Static(
                gettext("…or register a command template below (e.g. ffmpeg -i {input}):"),
                classes="hint",
            )
            yield Input(placeholder="ffmpeg -i {input} {output}", id="add-template")
            yield Input(placeholder=gettext("Name for the command"), id="add-template-name")
            yield Static(gettext("…or start from a blank page:"), classes="hint")
            # The authoring lanes were CLI-only (skit add --edit / --prompt) — a
            # TUI-first user could never discover them (zero-memorization).
            yield Static(
                tui_footer.bar(
                    tui_footer.chip("screen.draft_script", "Ctrl+E", gettext("Write a script…")),
                    tui_footer.chip("screen.draft_prompt", "Ctrl+P", gettext("Draft a prompt…")),
                ),
                id="add-draft",
                markup=True,
            )
        yield tui_footer.KeysBar(
            Static(
                tui_footer.bar(
                    tui_footer.chip("screen.continue_add", "Enter", gettext("Continue")),
                    tui_footer.chip("screen.cancel", "Esc", gettext("Cancel")),
                    tui_footer.nav_chip(),
                ),
                id="add-keys",
                markup=True,
            )
        )

    @on(Input.Submitted, "#add-path")
    def _path_given(self, event: Input.Submitted) -> None:
        self._submit_path()

    def _submit_path(self) -> None:
        """Every scripty kind gets the SAME review panel (identity, storage, deps,
        candidate ticks per its own analyzer) — the add flow must not be four
        different products depending on file extension. Only exe (nothing to detect
        inside a binary) takes the direct lane; unknown files get the honest error."""
        from .langs.registry import spec_for
        from .store import infer_kind

        raw = self.query_one("#add-path", Input).value.strip()
        if not raw:
            return
        path = Path(raw).expanduser()
        error = self.query_one("#add-error", Static)
        if not path.is_file():
            error.update(
                f"[red]{gettext('File not found: %(path)s') % {'path': escape(str(path))}}[/red]"
            )
            return
        kind = infer_kind(path)
        if kind == "unknown" and path.suffix.lower() == ".md":
            # The TUI twin of the CLI's bare-.md ask: the user explicitly picked this
            # file, and a .md that is neither script nor executable is a prompt in all
            # but name — say so instead of dead-ending (mirrors issue #10's direction).
            kind = "prompt"

        def _reviewed(slug: str | None) -> None:
            if slug is not None:
                self.dismiss(slug)

        if kind == "prompt":
            self.app.push_screen(PromptReviewScreen(path), _reviewed)
            return
        kind_spec = spec_for(kind)
        if kind_spec is not None and kind_spec.family == "interpreted":
            self.app.push_screen(AddReviewScreen(path, kind=kind), _reviewed)
            return
        if kind == "exe":
            try:
                entry = store.add_exe(path)
            except store.StoreError as exc:
                error.update(f"[red]{escape(str(exc))}[/red]")
                return
            self.dismiss(entry.slug)
            return
        error.update(
            f"[red]{gettext("%(file)s isn't a script or an executable — pass --prompt for an AI-agent prompt, --exe for a program, or --cmd for a command template.") % {'file': escape(path.name)}}[/red]"
        )

    @on(Input.Submitted, "#add-template")
    @on(Input.Submitted, "#add-template-name")
    def _template_given(self, event: Input.Submitted) -> None:
        self._submit_template()

    def _submit_template(self) -> None:
        template = self.query_one("#add-template", Input).value.strip()
        name = self.query_one("#add-template-name", Input).value.strip()
        error = self.query_one("#add-error", Static)
        if not template:
            return
        if not name:
            error.update(f"[red]{gettext('A name is required.')}[/red]")
            return
        try:
            entry = store.add_command(template, name=name)
        except store.StoreError as exc:
            error.update(f"[red]{escape(str(exc))}[/red]")
            return
        self.dismiss(entry.slug)

    def action_continue_add(self) -> None:
        """Footer/Enter twin: submit whichever field the user filled — the script path
        takes precedence, else the command template."""
        if self.query_one("#add-path", Input).value.strip():
            self._submit_path()
        else:
            self._submit_template()

    def action_draft_script(self) -> None:
        """Ctrl+E / the Write a script… chip: author a brand-new python script in
        $EDITOR, then review it in the same panel a path-add gets."""
        from .cli import _STARTER_SCRIPT  # lazy: cli imports this module lazily too

        self._draft(".py", _STARTER_SCRIPT, "python")

    def action_draft_prompt(self) -> None:
        """Ctrl+P / the Draft a prompt… chip: the prompt twin."""
        from .cli import _starter_prompt

        self._draft(".prompt.md", _starter_prompt(), "prompt")

    def _draft(self, suffix: str, starter: str, kind: str) -> None:
        import os
        import tempfile

        fd, tmp_name = tempfile.mkstemp(suffix=suffix, prefix="skit-new-")  # pragma: no mutate
        os.close(fd)
        tmp = Path(tmp_name)
        tmp.write_text(starter, encoding="utf-8")  # pragma: no mutate
        with self.app.suspend():
            try:
                editor.open_in_editor(tmp)
            except editor.EditorError as exc:
                print(str(exc), flush=True)
        text = tmp.read_text(encoding="utf-8", errors="replace")
        if text.strip() in ("", starter.strip()):
            tmp.unlink(missing_ok=True)  # pragma: no mutate
            self.notify(gettext("Nothing was written, so no script was added."))
            return

        def _reviewed(slug: str | None) -> None:
            tmp.unlink(missing_ok=True)  # pragma: no mutate — the panel copied or cancelled
            if slug is not None:
                self.dismiss(slug)

        review: Screen[str | None] = (
            PromptReviewScreen(tmp, fresh=True)
            if kind == "prompt"
            else AddReviewScreen(tmp, kind=kind, fresh=True)
        )
        self.app.push_screen(review, _reviewed)

    def action_cancel(self) -> None:
        self.dismiss(None)


class AddReviewScreen(Screen[str | None]):
    """Step 2: the review panel — everything prefilled, Enter is the only required act."""

    BINDINGS = [
        Binding("escape", "cancel", gettext("Cancel")),
        Binding("ctrl+e", "edit_source", gettext("Edit script"), priority=True),
        Binding("ctrl+a", "accept", gettext("Add"), priority=True),
        *tui_footer.FIELD_NAV_BINDINGS,
    ]
    # Boot on the name field, not the "*" pick (the body scroll container): the panel
    # should be typeable the moment it opens.
    AUTO_FOCUS = "Input"
    DEFAULT_CSS = """
    AddReviewScreen #review-body {
        padding: 0 1;
        border: round $skit-box-olive;
        border-title-color: ansi_bright_white;
        border-title-style: bold;
    }
    AddReviewScreen .section { color: $accent; margin: 1 0 0 0; }
    AddReviewScreen .hint { color: $text-muted; }
    AddReviewScreen .warn { color: $warning; }
    AddReviewScreen KeysBar { dock: bottom; }
    AddReviewScreen #review-keys { color: $text-muted; }
    """

    def __init__(
        self,
        path: Path,
        *,
        kind: str = "python",
        name: str | None = None,
        description: str | None = None,
        reference: bool = False,
        deps: list[str] | None = None,
        requires_python: str = "",
        fresh: bool = False,
    ) -> None:
        """The keyword arguments prefill the panel (the CLI face passes `skit add`'s
        flags through them); everything stays editable on screen.

        Kind-parametric (the audit's worst finding): the panel was python-only while
        shell/js/ts/fish shipped full analyzers that no add lane ever showed — the same
        verb gave python a review panel and shell a toast. Identity/storage are
        universal; deps render per deps_flavor; candidates come from the entry's OWN
        analyzer capability (None → no tick list, identity still reviewed)."""
        super().__init__()
        self._path: Path = path
        self._kind: str = kind
        # A freshly-drafted temp file has no original to link — the storage section is
        # skipped and accept always copies (the CLI refuses --ref there for the same
        # reason).
        self._fresh: bool = fresh
        from .langs.registry import spec_for

        self._spec = spec_for(kind)
        self._text: str = path.read_text(encoding="utf-8", errors="replace")
        self._analysis: analysis.Analysis = self._analyze()
        self._requires_python = requires_python
        # Survives the edit→rescan recompose: the rescan refreshes DETECTION, it must
        # never throw away what the user already typed into the panel.
        self._overrides: dict[str, str] = {}
        if name:
            self._overrides["name"] = name
        if description:
            self._overrides["desc"] = description
        if reference:
            self._overrides["mode"] = "1"
        if deps:
            self._overrides["deps"] = ", ".join(deps)

    def _analyze(self) -> analysis.Analysis:
        if self._spec is None or self._spec.analyzer is None:
            return analysis.Analysis()
        return self._spec.analyzer.analyze(self._text)

    def _suggest_description(self) -> str:
        if self._kind == "python":
            return store.suggest_description(self._text)
        prefix = self._spec.comment.prefix if self._spec and self._spec.comment else "#"
        return store.extract_comment_description(self._text, prefix)

    def on_mount(self) -> None:
        self.query_one("#review-body").border_title = gettext("Add %(name)s") % {
            "name": escape(self._path.name)
        }

    @override
    def compose(self) -> ComposeResult:
        with tui_footer.FormBody(id="review-body"):
            yield Static(gettext("Name"), classes="section")
            yield Input(value=self._overrides.get("name", self._path.stem), id="rv-name")
            yield Static(gettext("Description"), classes="section")
            yield Input(
                value=self._overrides.get("desc", self._suggest_description()),
                placeholder=gettext("(the script has no docstring — you can write one line)"),
                id="rv-desc",
            )
            if not self._fresh:
                # A freshly-drafted temp file has no original to link: no storage ask.
                yield Static(gettext("Storage"), classes="section")
                # "1" == the reference button. Default to copy on first compose (no
                # override); after an edit→rescan, restore whichever the user picked.
                reference = self._overrides.get("mode") == "1"
                with RadioSet(id="rv-mode"):
                    yield RadioButton(
                        gettext(
                            "Keep a copy — skit stores it; your original file is never modified"
                        ),
                        value=not reference,
                    )
                    yield RadioButton(
                        gettext(
                            "Link the original — edits take effect immediately, but skit won't "
                            "write to the file, so parameter definitions are yours to maintain"
                        ),
                        value=reference,
                    )
            yield from self._compose_deps()
            yield from self._compose_params()
        yield tui_footer.KeysBar(
            Static(
                tui_footer.bar(
                    tui_footer.chip("screen.accept", "Ctrl+A", gettext("Add")),
                    tui_footer.chip("screen.toggle_candidate", "Space", gettext("Toggle")),
                    tui_footer.chip("screen.edit_source", "Ctrl+E", gettext("Edit script")),
                    tui_footer.chip("screen.cancel", "Esc", gettext("Cancel")),
                    tui_footer.nav_chip(),
                ),
                id="review-keys",
                markup=True,
            )
        )

    def action_toggle_candidate(self) -> None:
        """Footer/Space twin: flip the focused candidate checkbox (each checkbox is also
        directly clickable). Named to avoid shadowing DOMNode.action_toggle, the built-in
        reactive-attribute toggle that takes an argument."""
        if isinstance(self.focused, Checkbox):
            self.focused.toggle()

    def _compose_deps(self) -> ComposeResult:
        spec = self._spec
        if spec is None or not spec.deps_flavor:
            return  # kinds with no dependency story (shell/fish/the data-driven tail)
        yield Static(gettext("Dependencies"), classes="section")
        if spec.deps_flavor == "npm":
            scanned = spec.dep_scanner(self._text) if spec.dep_scanner else []
            yield Input(
                value=self._overrides.get("deps", ", ".join(scanned)),
                placeholder=gettext("comma separated, e.g. chalk, @scope/pkg"),
                id="rv-deps",
            )
            yield Static(
                gettext("detected from the script's imports — edit freely"), classes="hint"
            )
            return
        if pep723.has_block(self._text):
            meta = pep723.parse_block(self._text) or {}
            deps = meta.get("dependencies") or []
            python = meta.get("requires-python", "")
            yield Static(gettext("The script declares its own dependencies (PEP 723):"))
            if python:
                yield Static(
                    "· " + gettext("needs Python %(python)s") % {"python": escape(str(python))}
                )
            for d in deps:
                yield Static("· " + gettext("installs %(dep)s") % {"dep": escape(str(d))})
            if not deps and not python:
                yield Static(f"[dim]{gettext('(none declared)')}[/dim]", markup=True)
        else:
            suggested = ", ".join(pep723.suggest_dependencies(self._text))
            yield Input(
                value=self._overrides.get("deps", suggested),
                placeholder=gettext("comma separated, e.g. requests>=2,<3, rich"),
                id="rv-deps",
            )
            yield Static(
                gettext("detected from the script's imports — edit freely"), classes="hint"
            )

    def _compose_params(self) -> ComposeResult:
        if self._spec is None or self._spec.analyzer is None:
            return  # no analyzer capability: nothing to tick (identity was still reviewed)
        yield Static(gettext("Parameters"), classes="section")
        reader = self._spec.cli_reader
        spec = reader.read_cli(self._text) if reader is not None else None
        if self._analysis.uses_cli_framework:
            if spec is not None and spec.ok and spec.fields:
                yield Static(
                    gettext(
                        "✓ skit read this script's own arguments (%(count)s fields). Running it "
                        "opens a form — nothing to memorize."
                    )
                    % {"count": len(spec.fields)}
                )
            else:
                yield Static(
                    gettext(
                        "This script parses its own arguments (%(names)s); skit couldn't model "
                        "them statically, so the run form offers a passthrough-arguments field."
                    )
                    % {"names": ", ".join(self._analysis.frameworks)},
                    classes="hint",
                )
            return
        if self._analysis.candidates:
            yield Static(gettext("Tick the ones the run form should ask for:"), classes="hint")
        for i, c in enumerate(self._analysis.candidates):
            label = (
                f"{c.name}  ({c.type} = {c.default!r})"
                if c.binding == "const"
                else gettext("input() #%(n)s: %(prompt)s")
                % {"n": c.order + 1, "prompt": repr(c.prompt)}
            )
            yield Checkbox(escape(label), value=not c.demoted, id=f"rv-cand-{i}")
            if c.demoted:
                yield Static(
                    "  ⚠ " + gettext("looks like a loop accumulator — probably not a parameter"),
                    classes="warn",
                )
        if self._analysis.filename_literals:
            names = ", ".join(repr(s) for s in self._analysis.filename_literals)
            yield Static(
                "💡 "
                + gettext(
                    "%(names)s are written directly inside the code, so skit can't turn them "
                    "into form fields. To manage one, first give it a name at the top of the "
                    "script, e.g. OUTPUT = '…' (Ctrl+E edits it now)."
                )
                % {"names": escape(names)},
                classes="hint",
            )
        if self._analysis.uses_argv:
            yield Static(
                "ℹ "  # noqa: RUF001 — intended info glyph, completing the 💡 tip / ⚠ warning set
                + gettext(
                    "This script reads command-line arguments; the run form has an "
                    "extra-arguments field for them."
                ),
                classes="hint",
            )

    def action_edit_source(self) -> None:
        """Ctrl+E: open the USER'S original file in their editor, then rescan on return
        (the edit→return→rescan loop; A5 is not involved — it's their file, their editor)."""
        self._overrides["name"] = self.query_one("#rv-name", Input).value
        self._overrides["desc"] = self.query_one("#rv-desc", Input).value
        if not self._fresh:
            self._overrides["mode"] = str(self.query_one("#rv-mode", RadioSet).pressed_index)
        deps_box = self.query("#rv-deps")
        if deps_box:
            self._overrides["deps"] = deps_box.first(Input).value
        with self.app.suspend():
            try:
                editor.open_in_editor(self._path)
            except editor.EditorError as exc:
                print(str(exc), flush=True)
        self._text = self._path.read_text(encoding="utf-8", errors="replace")
        self._analysis = self._analyze()
        self.refresh(recompose=True)

    def _collected_deps(self) -> list[str]:
        flavor = self._spec.deps_flavor if self._spec is not None else ""
        if flavor == "npm":
            from .langs.javascript import deps as js_deps

            return js_deps.split_requirements(self.query_one("#rv-deps", Input).value)
        if flavor == "uv" and not pep723.has_block(self._text):
            return pep723.split_requirements(self.query_one("#rv-deps", Input).value)
        return []

    def _store_entry(self, name: str | None, desc: str, reference: bool, deps: list[str]):
        if self._kind == "python":
            return store.add_python(
                self._path,
                name=name,
                mode="reference" if reference else "copy",
                description=desc,
                dependencies=deps or None,
                requires_python=self._requires_python,
            )
        from .langs.registry import shebang_program

        program = shebang_program(self._path)
        interpreter = program if self._spec is not None and program in self._spec.shebangs else ""
        entry = store.add_script(
            self._path,
            kind=self._kind,
            name=name,
            mode="reference" if reference else "copy",
            description=desc,
            interpreter=interpreter,
        )
        if deps and entry.meta.mode == "copy":
            entry = store.update_dependencies(entry.slug, deps)
        return entry

    def action_accept(self) -> None:
        name = self.query_one("#rv-name", Input).value.strip() or None
        desc = self.query_one("#rv-desc", Input).value.strip()
        reference = not self._fresh and self.query_one("#rv-mode", RadioSet).pressed_index == 1
        deps = self._collected_deps()
        try:
            entry = self._store_entry(name, desc, reference, deps)
        except store.StoreError as exc:
            self.notify(str(exc), severity="error")
            return
        # The candidate checkboxes exist only when _compose_params rendered them, i.e. for
        # a copy of a NON-cli-framework script with an analyzer. A script that both parses
        # its own arguments AND defines a managed-looking constant yields
        # uses_cli_framework=True with a non-empty candidates list; _compose_params returns
        # before the checkbox loop in that case, so querying #rv-cand-{i} here would raise
        # NoMatches and crash after the entry was already committed. Gate the collection on
        # the same condition that mounts them.
        if (
            entry.meta.mode == "copy"
            and self._spec is not None
            and self._spec.analyzer is not None
            and self._spec.params_io is not None
            and not self._analysis.uses_cli_framework
        ):
            picked = [
                self._analysis.candidates[i]
                for i in range(len(self._analysis.candidates))
                if self.query_one(f"#rv-cand-{i}", Checkbox).value
            ]
            if picked:
                specs = [ParamDecl.from_candidate(c) for c in picked]
                copy_path = entry.script_path
                current = copy_path.read_text(encoding="utf-8")
                copy_path.write_text(self._spec.params_io.write(current, specs), encoding="utf-8")
        self.dismiss(entry.slug)

    def action_cancel(self) -> None:
        self.dismiss(None)


class PromptReviewScreen(Screen[str | None]):
    """The prompt twin of AddReviewScreen — same contract (everything prefilled, Ctrl+A
    is the only required act), prompt-shaped sections: the insertion master switch, the
    placeholder tick list (flood-capped), and the runner pick with the New agent… door.

    Nothing is stored until Ctrl+A: the flood warning, the tick defaults and the switch
    all happen BEFORE the entry exists, so a long prompt never lands half-configured."""

    BINDINGS = [
        Binding("escape", "cancel", gettext("Cancel")),
        Binding("ctrl+e", "edit_source", gettext("Edit prompt"), priority=True),
        Binding("ctrl+a", "accept", gettext("Add"), priority=True),
        Binding("ctrl+n", "new_runner", gettext("New agent"), show=False, priority=True),
        *tui_footer.FIELD_NAV_BINDINGS,
    ]
    # Boot on the name field, not the "*" pick (the body scroll container).
    AUTO_FOCUS = "Input"
    DEFAULT_CSS = """
    PromptReviewScreen #pv-body {
        padding: 0 1;
        border: round $skit-box-olive;
        border-title-color: ansi_bright_white;
        border-title-style: bold;
    }
    PromptReviewScreen .section { color: $accent; margin: 1 0 0 0; }
    PromptReviewScreen .hint { color: $text-muted; }
    PromptReviewScreen .warn { color: $warning; }
    PromptReviewScreen #pv-holes { height: auto; }
    PromptReviewScreen KeysBar { dock: bottom; }
    PromptReviewScreen #pv-keys { color: $text-muted; }
    """

    def __init__(
        self,
        path: Path,
        *,
        name: str | None = None,
        description: str | None = None,
        reference: bool = False,
        runner: str | None = None,
        interpolate: bool = True,
        fresh: bool = False,
    ) -> None:
        """The keyword arguments prefill the panel (the CLI face passes `skit add`'s
        flags through them); everything stays editable on screen. fresh=True is the
        drafted-in-$EDITOR lane: a temp file with no original to link, so the storage
        section is skipped and accept always copies."""
        super().__init__()
        self._path: Path = path
        self._fresh: bool = fresh
        self._text: str = path.read_text(encoding="utf-8", errors="replace")
        self._detected: list[str] = prompt_analyzer.placeholder_names(self._text)
        self._runner_names: list[str] = []
        # The names behind the tick checkboxes, in compose order (flooded lists show
        # only a preview, so index math must go through this, never self._detected).
        self._shown_names: list[str] = []
        # Survives the edit→rescan recompose: the rescan refreshes DETECTION, it must
        # never throw away what the user already set on the panel.
        self._overrides: dict[str, str] = {}
        if name:
            self._overrides["name"] = name
        if description:
            self._overrides["desc"] = description
        if reference:
            self._overrides["mode"] = "1"
        if runner:
            self._overrides["runner"] = runner
        if not interpolate:
            self._overrides["interpolate"] = "off"

    def on_mount(self) -> None:
        self.query_one("#pv-body").border_title = gettext("Add %(name)s") % {
            "name": escape(self._path.name)
        }
        self.query_one("#pv-holes").display = self.query_one("#pv-interpolate", Checkbox).value

    def _default_runner(self) -> str:
        """Prefill: the CLI's --runner (or the pick kept across a rescan), else the
        last-picked runner, else no pin ("" = ask on the run form)."""
        preferred = self._overrides.get("runner") or argstate.load_last_runner()
        return preferred if preferred in self._runner_names else ""

    @override
    def compose(self) -> ComposeResult:
        self._runner_names = [r.name for r in config.load_prompt_runners()]
        with tui_footer.FormBody(id="pv-body"):
            yield Static(gettext("Name"), classes="section")
            yield Input(
                value=self._overrides.get("name", self._path.stem.removesuffix(".prompt")),
                id="pv-name",
            )
            yield Static(gettext("Description"), classes="section")
            yield Input(
                value=self._overrides.get("desc", store.prompt_description(self._text)),
                placeholder=gettext("(taken from the first line — you can write your own)"),
                id="pv-desc",
            )
            if not self._fresh:
                yield from self._compose_storage()
            yield Static(gettext("Variable insertion"), classes="section")
            yield Checkbox(
                gettext("Fill {{name}} placeholders from a form before each run"),
                value=self._overrides.get("interpolate") != "off",
                id="pv-interpolate",
            )
            with Vertical(id="pv-holes"):
                yield from self._compose_placeholders()
            yield Static(gettext("Runner (the agent this prompt runs with)"), classes="section")
            with tui_runner.PickList(id="pv-runner-set"):
                default = self._default_runner()
                yield RadioButton(gettext("ask on the run form"), value=default == "")
                for runner_name in self._runner_names:
                    yield RadioButton(escape(runner_name), value=(runner_name == default))
            yield Static(tui_runner.new_runner_chip(), id="pv-runner-new", markup=True)
        yield tui_footer.KeysBar(
            Static(
                tui_footer.bar(
                    tui_footer.chip("screen.accept", "Ctrl+A", gettext("Add")),
                    tui_footer.chip("screen.toggle_candidate", "Space", gettext("Toggle")),
                    tui_footer.chip("screen.edit_source", "Ctrl+E", gettext("Edit prompt")),
                    tui_footer.chip("screen.cancel", "Esc", gettext("Cancel")),
                    tui_footer.nav_chip(),
                ),
                id="pv-keys",
                markup=True,
            )
        )

    def _compose_storage(self) -> ComposeResult:
        yield Static(gettext("Storage"), classes="section")
        reference = self._overrides.get("mode") == "1"
        with RadioSet(id="pv-mode"):
            yield RadioButton(
                gettext("Keep a copy — skit stores it; your original file is never modified"),
                value=not reference,
            )
            yield RadioButton(
                gettext(
                    "Link the original — edits take effect immediately; skit never "
                    "writes to the file"
                ),
                value=reference,
            )

    def _compose_placeholders(self) -> ComposeResult:
        """The tick list. Flood honesty (docs/design/prompt.md): past AUTO_MANAGE_LIMIT
        the prompt was clearly not written for insertion — nothing is pre-ticked, only a
        preview is shown, and the warning points at the master switch."""
        detected = self._detected
        flooded = len(detected) > prompt_analyzer.AUTO_MANAGE_LIMIT
        self._shown_names = detected[: prompt_analyzer.LIST_PREVIEW_LIMIT] if flooded else detected
        if not detected:
            yield Static(
                gettext(
                    "No {{name}} placeholders detected — the body travels to the agent as written."
                ),
                classes="hint",
            )
            return
        if flooded:
            yield Static(
                gettext(
                    "Detected %(count)s placeholders — probably not written for "
                    "insertion. Tick only the ones you need, or untick the switch above."
                )
                % {"count": len(detected)},
                classes="warn",
            )
        else:
            yield Static(gettext("Tick the ones the run form should ask for:"), classes="hint")
        for i, hole_name in enumerate(self._shown_names):
            mark = gettext(" (secret)") if is_secret_name(hole_name) else ""
            yield Checkbox(escape(hole_name) + mark, value=not flooded, id=f"pv-hole-{i}")
        if len(detected) > len(self._shown_names):
            yield Static(
                gettext("…and %(count)s more (manage them later in Script settings)")
                % {"count": len(detected) - len(self._shown_names)},
                classes="hint",
            )

    @on(Checkbox.Changed, "#pv-interpolate")
    def _toggle_holes(self, event: Checkbox.Changed) -> None:
        """The master switch folds the tick list away — off means NO insertion machinery,
        and the panel should look like it. Tick states survive underneath for a re-tick."""
        self.query_one("#pv-holes").display = event.value

    def action_toggle_candidate(self) -> None:
        """Footer/Space twin: flip the focused checkbox (each is also clickable)."""
        if isinstance(self.focused, Checkbox):
            self.focused.toggle()

    def _picked_runner(self) -> str:
        """The runner radio's pick ("" = no pin). Index 0 is always "ask on the run
        form"; the rest map straight onto self._runner_names."""
        pressed = self.query_one("#pv-runner-set", RadioSet).pressed_index
        options = ["", *self._runner_names]
        return options[pressed] if 0 <= pressed < len(options) else ""

    def action_new_runner(self) -> None:
        """Ctrl+N / the New agent… chip: define a custom runner without leaving the
        panel — it lands in config, joins the picker, and is selected immediately."""

        async def _added(runner_name: str | None) -> None:
            if not runner_name:
                return
            self._runner_names.append(runner_name)
            radio_set = self.query_one("#pv-runner-set", RadioSet)
            button = RadioButton(escape(runner_name))
            await radio_set.mount(button)
            button.value = True

        self.app.push_screen(tui_runner.RunnerAddModal(), _added)

    def action_edit_source(self) -> None:
        """Ctrl+E: open the USER'S original file in their editor, then rescan on return
        (the same edit→return→rescan loop as the python panel — their file, their editor)."""
        self._overrides["name"] = self.query_one("#pv-name", Input).value
        self._overrides["desc"] = self.query_one("#pv-desc", Input).value
        if not self._fresh:
            self._overrides["mode"] = str(self.query_one("#pv-mode", RadioSet).pressed_index)
        self._overrides["interpolate"] = (
            "on" if self.query_one("#pv-interpolate", Checkbox).value else "off"
        )
        picked = self._picked_runner()
        if picked:
            self._overrides["runner"] = picked
        else:
            self._overrides.pop("runner", None)
        with self.app.suspend():
            try:
                editor.open_in_editor(self._path)
            except editor.EditorError as exc:
                print(str(exc), flush=True)
        self._text = self._path.read_text(encoding="utf-8", errors="replace")
        self._detected = prompt_analyzer.placeholder_names(self._text)
        self.refresh(recompose=True)

    def action_accept(self) -> None:
        name = self.query_one("#pv-name", Input).value.strip() or None
        desc = self.query_one("#pv-desc", Input).value.strip()
        reference = not self._fresh and self.query_one("#pv-mode", RadioSet).pressed_index == 1
        interpolate = self.query_one("#pv-interpolate", Checkbox).value
        managed: list[str] | None = None
        if interpolate:
            # The EXPLICIT kept subset — including the honest empty list. Flooded
            # panels tick nothing by default, and add_prompt honors what was asked.
            managed = [
                self._shown_names[i]
                for i in range(len(self._shown_names))
                if self.query_one(f"#pv-hole-{i}", Checkbox).value
            ]
        runner = self._picked_runner()
        try:
            entry = store.add_prompt(
                self._path,
                name=name,
                mode="reference" if reference else "copy",
                description=desc,
                managed=managed,
                runner=runner,
                interpolate=interpolate,
            )
        except store.StoreError as exc:
            self.notify(str(exc), severity="error")
            return
        if runner:
            # A real pick prefills the next picker (never a non-interactive resolve).
            argstate.save_last_runner(runner)
        self.dismiss(entry.slug)

    def action_cancel(self) -> None:
        self.dismiss(None)


class _ReviewHost(App[str | None]):
    """A review panel hosted alone (the CLI face). Exits with the new entry's slug, or
    None on cancel — the SAME screen the TUI's `a` pushes, so the two can't drift."""

    ENABLE_COMMAND_PALETTE = False
    HORIZONTAL_BREAKPOINTS = tui_layout.HORIZONTAL_BREAKPOINTS
    VERTICAL_BREAKPOINTS = tui_layout.VERTICAL_BREAKPOINTS

    def __init__(self, screen: Screen[str | None]) -> None:
        super().__init__()
        self._screen: Screen[str | None] = screen

    @override
    def get_css_variables(self) -> dict[str, str]:
        # Same bootstrap as the Library app: the first stylesheet parse runs before
        # on_mount activates the theme and would die on $skit-box-*.
        return {**super().get_css_variables(), **theme.BOX_VARIABLES}

    def on_mount(self) -> None:
        self.register_theme(theme.CLAUDE_THEME)
        self.theme = "skit-claude"
        self.push_screen(self._screen, self.exit)


class AddReviewApp(_ReviewHost):
    """`skit add x.py` / `skit add x.sh` in an interactive terminal."""

    def __init__(
        self,
        path: Path,
        *,
        kind: str = "python",
        name: str | None = None,
        description: str | None = None,
        reference: bool = False,
        deps: list[str] | None = None,
        requires_python: str = "",
        fresh: bool = False,
    ) -> None:
        super().__init__(
            AddReviewScreen(
                path,
                kind=kind,
                name=name,
                description=description,
                reference=reference,
                deps=deps,
                requires_python=requires_python,
            )
        )


class PromptReviewApp(_ReviewHost):
    """`skit add x.prompt.md` in an interactive terminal."""

    def __init__(
        self,
        path: Path,
        *,
        name: str | None = None,
        description: str | None = None,
        reference: bool = False,
        runner: str | None = None,
        interpolate: bool = True,
    ) -> None:
        super().__init__(
            PromptReviewScreen(
                path,
                name=name,
                description=description,
                reference=reference,
                runner=runner,
                interpolate=interpolate,
            )
        )


def run_add_review(
    path: Path,
    *,
    kind: str = "python",
    name: str | None = None,
    description: str | None = None,
    reference: bool = False,
    deps: list[str] | None = None,
    requires_python: str = "",
) -> str | None:
    """Blocking CLI entry to the review panel. Returns the new slug, or None."""
    return AddReviewApp(
        path,
        kind=kind,
        name=name,
        description=description,
        reference=reference,
        deps=deps,
        requires_python=requires_python,
    ).run()


def run_prompt_review(
    path: Path,
    *,
    name: str | None = None,
    description: str | None = None,
    reference: bool = False,
    runner: str | None = None,
    interpolate: bool = True,
) -> str | None:
    """Blocking CLI entry to the prompt review panel. Returns the new slug, or None."""
    return PromptReviewApp(
        path,
        name=name,
        description=description,
        reference=reference,
        runner=runner,
        interpolate=interpolate,
    ).run()
