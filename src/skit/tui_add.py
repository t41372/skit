"""Add flow in the TUI: source step → single review panel.

The review panel is one always-editable surface — no wizard sequence to march through;
Enter accepts everything as reviewed. Detection honesty rules render here: signal-
driven checkbox defaults, the accumulator warning, filename-literal hints, and the
"the script declares its own dependencies" read-only variant.

The panel has two faces: pushed from the Library (`a`), and hosted alone by
`AddReviewApp` when a terminal `skit add x.py` runs interactively — same screen, so
the CLI and the TUI can never drift apart.
"""

from __future__ import annotations

from pathlib import Path
from typing import override

from rich.markup import escape
from textual import on
from textual.app import App, ComposeResult
from textual.binding import Binding
from textual.screen import Screen
from textual.widgets import Checkbox, Input, RadioButton, RadioSet, Static

from . import analysis, editor, pep723, store, theme, tui_footer, tui_layout
from .i18n import gettext
from .langs.python import analyzer, argspec, metawriter
from .params import ParamDecl


class AddSourceScreen(Screen[str | None]):
    """Step 1: where does the script come from? Returns the new entry's slug, or None."""

    BINDINGS = [
        Binding("escape", "cancel", gettext("Cancel")),
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
            yield Static(gettext("Path to a script or executable:"))
            yield Input(placeholder="~/scripts/tool.py", id="add-path")
            yield Static("", id="add-error", markup=True)
            yield Static(
                gettext("…or register a command template below (e.g. ffmpeg -i {input}):"),
                classes="hint",
            )
            yield Input(placeholder="ffmpeg -i {input} {output}", id="add-template")
            yield Input(placeholder=gettext("Name for the command"), id="add-template-name")
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

    def _add_non_python(self, path: Path, error: Static) -> None:
        """The direct-add lane (no review panel): exe entries have nothing to detect inside them,
        and every other kind is added straight away.

        Note that shell/js/ts/fish DO have analyzers — the review panel is simply Python-shaped
        (it renders PEP 723 dependency completion alongside the candidates), so they take the
        direct lane and surface their candidates afterwards in Script settings (`p`) and
        `skit params`, which are both language-neutral. Interpreted adds record the shebang's
        interpreter and a comment-extracted description via store.add_script."""
        from .langs.registry import shebang_program, spec_for
        from .store import infer_kind

        kind = infer_kind(path)
        kind_spec = spec_for(kind)
        try:
            if kind == "exe":
                entry = store.add_exe(path)
            elif kind_spec is not None and kind_spec.family == "interpreted":
                program = shebang_program(path)
                interpreter = program if program in kind_spec.shebangs else ""
                entry = store.add_script(path, kind=kind, interpreter=interpreter)
                # npm-flavor copy adds record the script's own imports as dependencies — the
                # direct lane has no review step, so this mirrors the CLI's non-interactive
                # "accept the suggestions as-is"; Script settings (`p`) edits them afterwards.
                if (
                    kind_spec.deps_flavor == "npm"
                    and entry.meta.mode == "copy"
                    and kind_spec.dep_scanner is not None
                ):
                    try:
                        # The codec spelling is inert here (import specifiers are ASCII, so
                        # "utf-8"/"UTF-8"/dropped→locale-default all decode identically); only
                        # errors="replace" is load-bearing, and its stray-byte tolerance is
                        # pinned by test_non_python_tolerates_non_utf8_bytes. mutmut is
                        # line-granular, so the whole read is pragma'd (fmt:skip keeps the
                        # inline pragma on the statement line, where mutmut honors it).
                        text = path.read_text(encoding="utf-8", errors="replace")  # pragma: no mutate  # fmt: skip
                    except OSError:
                        # Unreachable: add_script above already decoded this same path with the
                        # same codec, so a re-read cannot raise; and ""/None/"XXXX" are all falsy
                        # or import-free, so the `if text` guard yields scanned=[] regardless.
                        text = ""  # pragma: no mutate
                    scanned = kind_spec.dep_scanner(text) if text else []
                    if scanned:
                        entry = store.update_dependencies(entry.slug, scanned)
                        # The CLI's add summary prints the recorded deps; the direct lane's
                        # only surface is a toast — recording something that will download
                        # packages on first run must never be invisible.
                        self.notify(
                            gettext("Dependencies recorded: %(deps)s (edit in Script settings)")
                            % {"deps": ", ".join(scanned)}
                        )
            else:
                error.update(
                    f"[red]{gettext("%(file)s isn't a script or an executable — pass --exe for a program, or --cmd for a command template.") % {'file': escape(path.name)}}[/red]"
                )
                return
        except store.StoreError as exc:
            error.update(f"[red]{escape(str(exc))}[/red]")
            return
        self.dismiss(entry.slug)

    def _submit_path(self) -> None:
        # query_one's expect_type is a redundant runtime guard here (the node is always the
        # declared type) and "#add-path"/"#add-error" are each the first widget of their type,
        # so dropping the type / the selector cannot change what is returned — equivalent.
        raw = self.query_one("#add-path", Input).value.strip()  # pragma: no mutate
        if not raw:
            return
        path = Path(raw).expanduser()
        error = self.query_one("#add-error", Static)  # pragma: no mutate
        if not path.is_file():
            error.update(
                f"[red]{gettext('File not found: %(path)s') % {'path': escape(str(path))}}[/red]"
            )
            return
        if path.suffix.lower() != ".py":
            self._add_non_python(path, error)
            return

        def _reviewed(slug: str | None) -> None:
            if slug is not None:
                self.dismiss(slug)

        self.app.push_screen(AddReviewScreen(path), _reviewed)

    @on(Input.Submitted, "#add-template")
    @on(Input.Submitted, "#add-template-name")
    def _template_given(self, event: Input.Submitted) -> None:
        self._submit_template()

    def _submit_template(self) -> None:
        # See _submit_path: query_one's type guard is inert and each id is the sole match, so
        # the type-drop / None-type mutations of these three calls are equivalent.
        template = self.query_one("#add-template", Input).value.strip()  # pragma: no mutate
        name = self.query_one("#add-template-name", Input).value.strip()  # pragma: no mutate
        error = self.query_one("#add-error", Static)  # pragma: no mutate
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
        # query_one type guard inert; "#add-path" is the first Input, so the type-drop/None
        # mutations return the same widget — equivalent.
        if self.query_one("#add-path", Input).value.strip():  # pragma: no mutate
            self._submit_path()
        else:
            self._submit_template()

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
        name: str | None = None,
        description: str | None = None,
        reference: bool = False,
        deps: list[str] | None = None,
        requires_python: str = "",
    ) -> None:
        """The keyword arguments prefill the panel (the CLI face passes `skit add`'s
        flags through them); everything stays editable on screen."""
        super().__init__()
        self._path: Path = path
        self._text: str = path.read_text(encoding="utf-8", errors="replace")  # pragma: no mutate — encoding None/utf-8/UTF-8 decode identically under skit's UTF-8-mode runtime (equivalent); the errors="replace" handler stays behaviourally pinned by test_add_review_screen_reads_invalid_utf8_with_replace  # fmt: skip
        self._analysis: analysis.Analysis = analyzer.analyze(self._text)
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
                value=self._overrides.get("desc", store.suggest_description(self._text)),
                placeholder=gettext("(the script has no docstring — you can write one line)"),
                id="rv-desc",
            )
            yield Static(gettext("Storage"), classes="section")
            # "1" == the reference button. Default to copy on first compose (no override);
            # after an edit→rescan, restore whichever the user had picked.
            reference = self._overrides.get("mode") == "1"
            with RadioSet(id="rv-mode"):
                yield RadioButton(
                    gettext("Keep a copy — skit stores it; your original file is never modified"),
                    value=not reference,
                )
                yield RadioButton(
                    gettext(
                        "Link the original — edits take effect immediately, but skit won't write "
                        "to the file, so parameter definitions are yours to maintain"
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
        yield Static(gettext("Dependencies"), classes="section")
        if pep723.has_block(self._text):
            meta = pep723.parse_block(self._text) or {}
            deps = meta.get("dependencies") or []
            python = meta.get("requires-python")
            yield Static(gettext("The script declares its own dependencies (PEP 723):"))
            if python:
                yield Static(
                    "· " + gettext("needs Python %(python)s") % {"python": escape(str(python))}
                )
            for d in deps:
                yield Static("· " + gettext("installs %(dep)s") % {"dep": escape(str(d))})
            if not deps and not python:
                yield Static(f"[dim]{gettext('(none declared)')}[/dim]")
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
        yield Static(gettext("Parameters"), classes="section")
        spec = argspec.read_cli(self._text)
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
        self._overrides["name"] = self.query_one("#rv-name", Input).value  # pragma: no mutate — expect_type is a pure runtime assertion (equivalent); the override is pinned by test_edit_source_preserves_name_desc_and_mode_overrides  # fmt: skip
        self._overrides["desc"] = self.query_one("#rv-desc", Input).value  # pragma: no mutate — expect_type equivalent; pinned by test_edit_source_preserves_name_desc_and_mode_overrides  # fmt: skip
        self._overrides["mode"] = str(self.query_one("#rv-mode", RadioSet).pressed_index)  # pragma: no mutate — expect_type/type-selector equivalent; pinned by test_edit_source_preserves_name_desc_and_mode_overrides  # fmt: skip
        deps_box = self.query("#rv-deps")
        if deps_box:
            deps_input = deps_box.first(Input)  # pragma: no mutate — expect_type equivalent (unique #rv-deps match)  # fmt: skip
            self._overrides["deps"] = deps_input.value
        with self.app.suspend():
            try:
                editor.open_in_editor(self._path)
            except editor.EditorError as exc:
                print(str(exc), flush=True)
        self._text = self._path.read_text(encoding="utf-8", errors="replace")  # pragma: no mutate — encoding None/utf-8/UTF-8 decode identically under skit's UTF-8-mode runtime (equivalent); errors="replace" is pinned by test_edit_source_rescans_non_utf8_original_with_replace  # fmt: skip
        self._analysis = analyzer.analyze(self._text)
        self.refresh(recompose=True)

    def action_accept(self) -> None:
        name = self.query_one("#rv-name", Input).value.strip() or None  # pragma: no mutate — expect_type/type-selector equivalent (unique first #rv-name Input); pinned by test_accept_copy_uses_typed_name_desc_and_deps  # fmt: skip
        desc = self.query_one("#rv-desc", Input).value.strip()  # pragma: no mutate — expect_type equivalent; pinned by test_accept_copy_uses_typed_name_desc_and_deps  # fmt: skip
        mode_set = self.query_one("#rv-mode", RadioSet)  # pragma: no mutate — expect_type/type-selector equivalent (unique #rv-mode)  # fmt: skip
        reference = mode_set.pressed_index == 1
        deps: list[
            str
        ] = []  # pragma: no mutate — [] vs None is unobservable here: read only via deps-or-None on the PEP 723 branch, where both collapse to None
        if not pep723.has_block(self._text):
            deps = pep723.split_requirements(self.query_one("#rv-deps", Input).value)  # pragma: no mutate — expect_type equivalent (unique #rv-deps); pinned by test_accept_copy_uses_typed_name_desc_and_deps  # fmt: skip
        try:
            entry = store.add_python(
                self._path,
                name=name,
                mode="reference" if reference else "copy",
                description=desc,
                dependencies=deps or None,
                requires_python=self._requires_python,
            )
        except store.StoreError as exc:
            self.notify(str(exc), severity="error")
            return
        # The candidate checkboxes exist only when _compose_params rendered them, i.e. for
        # a copy of a NON-cli-framework script. A script that both parses its own arguments
        # AND defines a module-level constant / input() yields uses_cli_framework=True with a
        # non-empty candidates list; _compose_params returns before the checkbox loop in that
        # case, so querying #rv-cand-{i} here would raise NoMatches and crash after the entry
        # was already committed. Gate the collection on the same condition that mounts them.
        if entry.meta.mode == "copy" and not self._analysis.uses_cli_framework:
            picked = []
            for i in range(len(self._analysis.candidates)):
                checkbox = self.query_one(f"#rv-cand-{i}", Checkbox)  # pragma: no mutate — expect_type/per-index selector are query mechanics; the selection is pinned by test_accept_writes_only_checked_candidate_params  # fmt: skip
                if checkbox.value:
                    picked.append(self._analysis.candidates[i])
            if picked:
                specs = [ParamDecl.from_candidate(c) for c in picked]
                copy_path = entry.script_path
                current = copy_path.read_text(
                    encoding="utf-8"
                )  # pragma: no mutate — utf-8 equivalence
                copy_path.write_text(
                    metawriter.write_params(current, specs), encoding="utf-8"
                )  # pragma: no mutate — utf-8 equivalence
        self.dismiss(entry.slug)

    def action_cancel(self) -> None:
        self.dismiss(None)


class AddReviewApp(App[str | None]):
    """`skit add x.py` in an interactive terminal: the SAME review panel the TUI's
    `a` opens, hosted alone. Exits with the new entry's slug, or None on cancel."""

    ENABLE_COMMAND_PALETTE = False
    HORIZONTAL_BREAKPOINTS = tui_layout.HORIZONTAL_BREAKPOINTS
    VERTICAL_BREAKPOINTS = tui_layout.VERTICAL_BREAKPOINTS

    def __init__(
        self,
        path: Path,
        *,
        name: str | None = None,
        description: str | None = None,
        reference: bool = False,
        deps: list[str] | None = None,
        requires_python: str = "",
    ) -> None:
        super().__init__()
        self._screen = AddReviewScreen(
            path,
            name=name,
            description=description,
            reference=reference,
            deps=deps,
            requires_python=requires_python,
        )

    @override
    def get_css_variables(self) -> dict[str, str]:
        # Same bootstrap as the Library app: the first stylesheet parse runs before
        # on_mount activates the theme and would die on $skit-box-*.
        return {**super().get_css_variables(), **theme.BOX_VARIABLES}

    def on_mount(self) -> None:
        self.register_theme(theme.CLAUDE_THEME)
        self.theme = "skit-claude"
        self.push_screen(self._screen, self.exit)


def run_add_review(
    path: Path,
    *,
    name: str | None = None,
    description: str | None = None,
    reference: bool = False,
    deps: list[str] | None = None,
    requires_python: str = "",
) -> str | None:
    """Blocking CLI entry to the review panel. Returns the new slug, or None."""
    return AddReviewApp(
        path,
        name=name,
        description=description,
        reference=reference,
        deps=deps,
        requires_python=requires_python,
    ).run()
