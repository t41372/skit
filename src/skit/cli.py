"""CLI entry point (Typer): the v2 command surface.

Running `skit` with no subcommand opens the TUI workbench. The commands here are the
automation/SSH/muscle-memory shortcuts; every interactive flow goes through the shared
form layer (flows) so CLI and TUI behave identically.

Command-surface contracts:
- Exit codes (docker convention): `run` passes the script's exit code through PURE;
  skit's own failures are 125, a target that exists but isn't executable is 126, a
  missing target/name is 127, usage errors are 2. Other commands: 0/1/2.
- Every output has a --json twin where output exists.
- Lists are repeatable flags (--dep), never comma-joined (PEP 508 specifiers contain
  commas).
- Non-interactive contract: on a pipe/CI/--no-input, never prompt, never guess, never
  silently assemble a broken command.

Every user-visible string goes through i18n.gettext()/ngettext(). Help strings resolve at
import time, so i18n initializes lazily on module import (see i18n.py).
"""

from __future__ import annotations

import dataclasses
import json
import os
import sys
from pathlib import Path
from typing import TYPE_CHECKING

import typer
from rich.console import Console
from rich.markup import escape
from rich.prompt import Confirm, Prompt

from . import (
    __version__,
    agentskill,
    analysis,
    argstate,
    config,
    editor,
    flows,
    healthcheck,
    i18n,
    launcher,
    models,
    pep723,
    promptform,
    store,
)
from .i18n import gettext, ngettext
from .langs.python import analyzer, metawriter
from .langs.registry import KNOWN_KINDS, spec_for
from .params import ParamDecl, declared_from_meta, edit_declared, is_secret_name

if TYPE_CHECKING:
    from collections.abc import Callable

    from .langs.base import LangSpec

app = typer.Typer(
    name="skit",
    help=gettext(
        "skit — a script launcher and parameter manager. Run it without a subcommand to open the main menu."
    ),
    add_completion=True,
    no_args_is_help=False,
)
console = Console()
err_console = Console(stderr=True)

# Exit-code contract for `skit run` (docker convention; the script's own code passes
# through untouched, so these must stay out of the 0-124 range scripts commonly use).
EXIT_USAGE = 2
EXIT_SKIT = 125
EXIT_NOT_EXECUTABLE = 126
EXIT_NOT_FOUND = 127
EXIT_CANCELLED = 130  # user cancelled the form (128+SIGINT convention) — not a skit failure

# Rich closing tags are case-insensitive, so a mutated-case variant is behaviorally identical.
_DIM_CLOSE = "[/dim]"  # pragma: no mutate
_RED_CLOSE = "[/red]"  # pragma: no mutate


def _fail(message: str, code: int) -> typer.Exit:
    err_console.print(f"[red]{escape(message)}[/red]")
    return typer.Exit(code)


def _is_interactive() -> bool:
    return sys.stdin.isatty() and sys.stdout.isatty()


# --------------------------------------------------------------------------
# dynamic completion (north star: nothing to memorize — not even your own names)
# --------------------------------------------------------------------------


def _complete_script(incomplete: str) -> list[str]:
    try:
        entries = store.list_entries()
    except Exception:  # completion must never crash the shell
        return []
    out = {e.meta.name for e in entries} | {e.slug for e in entries}
    return sorted(c for c in out if c.startswith(incomplete))


def _complete_preset(ctx: typer.Context, incomplete: str) -> list[str]:
    name = ctx.params.get("name")
    if not name:
        return []
    try:
        entry = store.resolve(name)
        presets = argstate.load_state(entry.slug)["presets"]
    except Exception:  # completion must never crash the shell
        return []
    return sorted(p for p in presets if p.startswith(incomplete))


def _complete_runner(incomplete: str) -> list[str]:
    try:
        names = [r.name for r in config.load_prompt_runners()]
    except Exception:  # completion must never crash the shell
        return []
    return sorted(n for n in names if n.startswith(incomplete))


_SCRIPT_ARG = typer.Argument(
    ..., help=gettext("Script name or slug"), autocompletion=_complete_script
)


@app.callback(invoke_without_command=True)
def main(
    ctx: typer.Context,
    version: bool = typer.Option(False, "--version", "-V", help=gettext("Show version")),
) -> None:
    if version:
        console.print(f"skit {__version__}")
        raise typer.Exit()
    if ctx.invoked_subcommand is None:
        _maybe_first_run_setup()
        from .tui import run_menu

        raise typer.Exit(run_menu())


# --------------------------------------------------------------------------
# add
# --------------------------------------------------------------------------


def _resolve_python_metadata(
    text: str, deps_opt: list[str] | None, python_opt: str | None, no_input: bool
) -> tuple[list[str], str]:
    """Decide the (dependencies, requires_python) to fill in.

    - Script already has a PEP 723 block: don't ask, don't fill (the block is the source of truth).
    - Explicit --dep / --python: use them directly, no prompting.
    - Interactive: only ask when the AST reveals likely third-party imports; ask nothing when
      there are no dependencies at all.
    """
    if pep723.has_block(text):
        meta = pep723.parse_block(text) or {}
        deps = meta.get("dependencies")
        if deps:
            console.print(
                gettext("The script declares its own dependencies (PEP 723): %(deps)s")
                % {"deps": ", ".join(escape(d) for d in deps)}
            )
        return [], ""
    if deps_opt is not None or python_opt is not None:
        # Strip/drop empties, matching the interactive path and _resolve_npm_dependencies: an
        # empty "" requirement makes PEP 508 refuse the whole [tool.uv] block ("Empty field is
        # not allowed"), and a whitespace-only --python is an unparseable version constraint —
        # either bricks every subsequent run before the script starts.
        return [d.strip() for d in (deps_opt or []) if d.strip()], (python_opt or "").strip()
    suggested = pep723.suggest_dependencies(text)
    if not suggested:
        return [], ""  # No dependencies: nothing to ask
    if no_input or not sys.stdin.isatty():
        return suggested, ""  # Non-interactive: accept the suggestions as-is
    answer = Prompt.ask(
        gettext("Dependencies to install (Enter to accept, edit the list, or '-' for none)"),
        default=", ".join(suggested),
        console=console,
    )
    if answer.strip().lower() in ("-", "none"):
        deps_list: list[str] = []
    else:
        deps_list = pep723.split_requirements(answer)
    py = Prompt.ask(
        gettext("Python version (leave empty for automatic)"), default="", console=console
    )
    return deps_list, py.strip()


def _resolve_npm_dependencies(
    script: Path,
    deps_opt: list[str] | None,
    no_input: bool,
    scanner: Callable[[str], list[str]] | None,
) -> list[str]:
    """The npm dependency list a js/ts copy-mode add should record — the js analogue of
    `_resolve_python_metadata`, sharing its contract exactly: explicit --dep wins without a
    question; otherwise the script's own imports are the suggestion; non-interactive accepts the
    suggestions as-is; interactively the list is offered for editing. No scanner (grammar failed
    to import) or an unreadable file suggests nothing — honest degradation, never a blocked add."""
    if deps_opt is not None:
        # Drop empty/whitespace --dep values: an empty package name is junk in the --json
        # contract and would fake a "cleared" list without sweeping node_modules.
        return [d.strip() for d in deps_opt if d.strip()]
    if scanner is None:
        return []
    try:
        text = script.read_text(encoding="utf-8", errors="replace")
    except OSError:
        text = ""
    suggested = scanner(text) if text else []
    if not suggested:
        return []
    if no_input or not sys.stdin.isatty():
        return suggested  # Non-interactive: accept the suggestions as-is
    answer = Prompt.ask(
        gettext("Dependencies to install (Enter to accept, edit the list, or '-' for none)"),
        default=", ".join(suggested),
        console=console,
    )
    if answer.strip().lower() in ("-", "none"):
        return []
    # npm-shaped split, NOT pep723.split_requirements: the PEP 508 splitter would merge a
    # scoped package into its neighbor ("chalk, @scope/pkg" -> one bogus requirement).
    from .langs.javascript import deps as js_deps

    return js_deps.split_requirements(answer)


def _refuse_unusable_add_flags(
    kind: str, kind_spec: LangSpec | None, ref: bool, dep: list[str] | None, python: str | None
) -> None:
    """An explicit flag the add can't honor is refused, never dropped (the non-interactive
    contract — silently assembling an entry that ignores what the caller asked for is exactly
    the guessing it forbids). The uv flavor honors both flags; npm honors --dep on copies
    only; every other kind honors neither."""
    flavor = kind_spec.deps_flavor if kind_spec is not None else ""
    if flavor == "uv":
        return
    if dep is not None and (flavor != "npm" or ref):
        message = (
            gettext(
                "Reference-mode entries take no managed dependencies — they run from their own project. Add it as a copy, or drop --dep."
            )
            if flavor == "npm"
            else gettext("%(kind)s entries don't take package dependencies — drop --dep.")
            % {"kind": kind}
        )
        err_console.print(f"[red]{message}[/red]")
        raise typer.Exit(EXIT_USAGE)
    if python is not None:
        err_console.print(
            f"[red]{gettext("A Python constraint doesn't apply to %(kind)s scripts.") % {'kind': kind}}[/red]"
        )
        raise typer.Exit(EXIT_USAGE)


def _prompt_identity(
    p: Path, text: str, name: str | None, description: str | None, no_input: bool
) -> tuple[str | None, str | None]:
    """Interactive name + description prompts for `add`. `None` means "let the store derive it"."""
    if no_input or not sys.stdin.isatty():
        return name, description
    if name is None:
        name = Prompt.ask(gettext("Name in skit"), default=p.stem, console=console).strip() or None
    if description is None:
        description = Prompt.ask(
            gettext("Description (optional)"),
            default=store.suggest_description(text),
            console=console,
        ).strip()
    return name, description


def _require_file(resolved: Path) -> None:
    if not resolved.is_file():
        raise store.StoreError(gettext("File not found: %(path)s") % {"path": str(resolved)})


def _parse_selection(answer: str, count: int) -> list[int]:
    """Parse an onboarding selection: 'all' / 'none' (or empty) / '1,3,5'."""
    answer = answer.strip().lower()
    if answer == "all":
        return list(range(count))
    picked: list[int] = []
    for raw_part in answer.split(","):
        part = raw_part.strip()
        # isdecimal() is the predicate whose truth guarantees int() succeeds (isdigit()
        # also accepts superscripts/circled digits that int() rejects).
        if part.isdecimal() and 1 <= int(part) <= count and (int(part) - 1) not in picked:
            picked.append(int(part) - 1)
    return picked


def _default_selection(candidates: list[analysis.Candidate]) -> str:
    """Signal-driven default (UX spec §0): clean candidates in, demoted candidates out."""
    clean = [i for i, c in enumerate(candidates, start=1) if not c.demoted]
    if len(clean) == len(candidates):
        return "all"
    if not clean:
        return "none"
    return ",".join(str(i) for i in clean)


def _print_candidate(i: int, c: analysis.Candidate) -> None:
    mark = gettext(" (secret)") if c.secret else ""
    if c.binding == "const":
        console.print(
            "  "
            + gettext("%(num)s. %(name)s (%(type)s) = %(value)s%(secret)s")
            % {
                "num": i,
                "name": escape(c.name),
                "type": c.type,
                "value": escape(repr(c.default)),
                "secret": mark,
            }
        )
    else:
        console.print(
            "  "
            + gettext("%(num)s. input() #%(ordinal)s: %(prompt)s%(secret)s")
            % {"num": i, "ordinal": c.order + 1, "prompt": escape(repr(c.prompt)), "secret": mark}
        )
    if c.demoted:
        console.print(
            f"     [yellow]{gettext('⚠ looks like a loop accumulator — probably not a parameter')}[/yellow]"
        )


def _print_add_hints(result: analysis.Analysis, script_name: str) -> None:
    """The honest, rule-backed hints (UX spec §0): argv passthrough, extractable filenames."""
    if result.uses_argv:
        console.print(
            "[dim]"
            + gettext(
                "This script reads command-line arguments; the run form has an extra-arguments field for them."
            )
            + _DIM_CLOSE
        )
    if result.filename_literals:
        names = ", ".join(escape(repr(s)) for s in result.filename_literals)
        console.print(
            "[dim]"
            + gettext(
                "💡 %(names)s are written directly inside the code, so skit can't turn them into form fields. To manage one, first give it a name at the top of the script, e.g. OUTPUT = '…' (skit edit %(script)s)."
            )
            % {"names": names, "script": escape(script_name)}
            + _DIM_CLOSE
        )


def _onboard_params(text: str, script_name: str, no_input: bool) -> list[ParamDecl]:
    """Parameter onboarding at add time (A4: which constant counts as a parameter is a UX call).

    - argparse detected: nothing to manage — the run form is read statically from the
      script's own argument declarations (the unified form model).
    - Non-interactive: don't guess, don't select, return empty (honesty beats clever).
    """
    result = analyzer.analyze(text)
    if result.uses_cli_framework:
        from .langs.python import argspec

        spec = argspec.read_cli(text)
        if spec is not None and spec.ok and spec.fields:
            console.print(
                gettext(
                    "✓ skit read this script's own arguments (%(count)s fields). Running it opens a form — nothing to memorize."
                )
                % {"count": len(spec.fields)}
            )
        else:
            console.print(
                "[dim]"
                + gettext(
                    "This script parses its own arguments (%(names)s); skit couldn't model them statically, so the run form offers a passthrough-arguments field."
                )
                % {"names": ", ".join(result.frameworks)}
                + _DIM_CLOSE
            )
        return []
    _print_add_hints(result, script_name)
    if not result.candidates or no_input or not sys.stdin.isatty():
        return []
    console.print(
        ngettext(
            "Found %(count)s parameter candidate (constants / input() calls):",
            "Found %(count)s parameter candidates (constants / input() calls):",
            len(result.candidates),
        )
        % {"count": len(result.candidates)}
    )
    for i, c in enumerate(result.candidates, start=1):
        _print_candidate(i, c)
    answer = Prompt.ask(
        gettext("Which ones should skit manage? (e.g. 1,3 / all / none)"),
        default=_default_selection(result.candidates),
        console=console,
    )
    picked = _parse_selection(answer, len(result.candidates))
    return [ParamDecl.from_candidate(result.candidates[i]) for i in picked]


def _onboard_script_params(entry: store.Entry, kind_spec: LangSpec, no_input: bool) -> list[str]:
    """Line-mode parameter onboarding for analyzable interpreted kinds (shell/js/ts/
    fish) — the same candidate tick python gets. The analyzers shipped a whole PR
    without any add lane ever surfacing their findings; users concluded the language
    support was fake. Copy mode only (reference never writes the original, A7);
    non-interactive selects nothing (python's rule: honesty beats clever)."""
    if entry.meta.mode != "copy" or kind_spec.analyzer is None or kind_spec.params_io is None:
        return []
    text = entry.script_path.read_text(encoding="utf-8", errors="replace")
    result = kind_spec.analyzer.analyze(text)
    if result.uses_cli_framework:
        return []  # the script's own parser is the form — nothing to manage
    _print_add_hints(result, entry.meta.name)
    if not result.candidates or no_input or not sys.stdin.isatty():
        return []
    console.print(
        ngettext(
            "Found %(count)s parameter candidate (constants / input() calls):",
            "Found %(count)s parameter candidates (constants / input() calls):",
            len(result.candidates),
        )
        % {"count": len(result.candidates)}
    )
    for i, c in enumerate(result.candidates, start=1):
        _print_candidate(i, c)
    answer = Prompt.ask(
        gettext("Which ones should skit manage? (e.g. 1,3 / all / none)"),
        default=_default_selection(result.candidates),
        console=console,
    )
    picked = _parse_selection(answer, len(result.candidates))
    specs = [ParamDecl.from_candidate(result.candidates[i]) for i in picked]
    if not specs:
        return []
    copy_path = entry.script_path
    current = copy_path.read_text(encoding="utf-8")  # pragma: no mutate — utf-8 equivalence
    copy_path.write_text(
        kind_spec.params_io.write(current, specs), encoding="utf-8"
    )  # pragma: no mutate
    return [s.name for s in specs]


# A brand-new script starts from just a shebang; if the editor is closed with nothing more
# than this (or empty), we treat it as "cancelled" and add nothing.
_STARTER_SCRIPT = "#!/usr/bin/env python3\n"


def _onboard_python(
    p: Path,
    text: str,
    *,
    name: str | None,
    description: str | None = None,
    ref: bool = False,
    deps_opt: list[str] | None = None,
    python_opt: str | None = None,
    no_input: bool = False,
) -> tuple[store.Entry, list[str], list[str], list[str]]:
    """Shared add/create pipeline: identity -> dependencies -> store add -> parameter
    onboarding. Returns (entry, deps, managed_names, secret_names) for the summary."""
    name, description = _prompt_identity(p, text, name, description, no_input)
    final_deps, final_py = _resolve_python_metadata(text, deps_opt, python_opt, no_input)
    entry = store.add_python(
        p,
        name=name,
        mode="reference" if ref else "copy",
        description=description,
        dependencies=final_deps or None,
        requires_python=final_py,
    )
    managed: list[str] = []
    secrets: list[str] = []
    if entry.meta.mode == "reference":
        console.print(
            f"[dim]{gettext('Reference mode never touches the original file, so parameter setup was skipped.')}[/dim]"
        )
    else:
        params_specs = _onboard_params(text, entry.meta.name, no_input)
        if params_specs:
            copy_path = entry.script_path
            current = copy_path.read_text(encoding="utf-8")  # pragma: no mutate — utf-8 equivalence
            new_text = metawriter.write_params(current, params_specs)
            copy_path.write_text(new_text, encoding="utf-8")  # pragma: no mutate
            managed = [s.name for s in params_specs]
            secrets = [s.name for s in params_specs if s.secret]
    return entry, final_deps, managed, secrets


def _create_python_in_editor(
    name: str | None, deps_opt: list[str] | None = None, python_opt: str | None = None
) -> None:
    """Write a starter script to a temp file, open the user's editor, then ingest whatever
    they saved. Explicit --dep / --python ride through to the onboarding exactly as a
    path-based python add would honor them."""
    import tempfile

    if not _is_interactive():
        err_console.print(
            f"[red]{gettext('Writing a new script in an editor needs an interactive terminal.')}[/red]"
        )
        raise typer.Exit(EXIT_USAGE)
    if not name:
        name = Prompt.ask(gettext("Name in skit"), console=console).strip()
        if not name:
            err_console.print(f"[red]{gettext('A name is required.')}[/red]")
            raise typer.Exit(EXIT_USAGE)
    fd, tmp_name = tempfile.mkstemp(suffix=".py", prefix="skit-new-")  # pragma: no mutate
    os.close(fd)
    tmp = Path(tmp_name)
    tmp.write_text(_STARTER_SCRIPT, encoding="utf-8")  # pragma: no mutate
    try:
        console.print(f"[dim]{gettext('Opening your editor…')}[/dim]")
        editor.open_in_editor(tmp)
        text = tmp.read_text(encoding="utf-8", errors="replace")  # pragma: no mutate — utf-8 equiv
        if text.strip() in ("", _STARTER_SCRIPT.strip()):
            console.print(gettext("Nothing was written, so no script was added."))
            return
        from .langs.registry import kind_for_shebang

        drafted_kind = kind_for_shebang(tmp) or "python"
        if drafted_kind != "python":
            # A changed shebang is an explicit signal, honored by the SAME rule the
            # TUI draft lane uses — a bash draft must never be stored as a broken
            # python entry (nor refused after the user already wrote it).
            entry = store.add_script(tmp, kind=drafted_kind, name=name, mode="copy")
            spec = spec_for(drafted_kind)
            deps = []
            if (
                spec is not None
                and spec.deps_flavor == "npm"
                and spec.dep_scanner is not None
                and (deps := spec.dep_scanner(text))
            ):
                entry = store.update_dependencies(entry.slug, deps)
            managed, secrets = [], []
        else:
            entry, deps, managed, secrets = _onboard_python(
                tmp, text, name=name, deps_opt=deps_opt, python_opt=python_opt
            )
    except (editor.EditorError, store.StoreError) as exc:
        raise _fail(str(exc), 1) from exc
    finally:
        tmp.unlink(missing_ok=True)  # pragma: no mutate — the temp file always exists here
    _print_add_summary(entry, deps, managed, secrets)


def _add_from_stdin(
    name: str | None,
    description: str | None,
    deps_opt: list[str] | None = None,
    python_opt: str | None = None,
) -> None:
    """`skit add -`: ingest a script from stdin (e.g. `pbpaste | skit add - -n clip`).
    stdin is the script, so there is nobody to prompt: the non-interactive contract
    applies, and a name is required up front."""
    import tempfile

    if not name:
        err_console.print(
            f"[red]{gettext('Reading the script from stdin needs an explicit --name.')}[/red]"
        )
        raise typer.Exit(EXIT_USAGE)
    text = sys.stdin.read()
    if not text.strip():
        err_console.print(
            f"[red]{gettext('Nothing arrived on stdin, so there is nothing to add.')}[/red]"
        )
        raise typer.Exit(1)
    fd, tmp_name = tempfile.mkstemp(suffix=".py", prefix="skit-stdin-")  # pragma: no mutate
    os.close(fd)
    tmp = Path(tmp_name)
    tmp.write_text(text, encoding="utf-8")  # pragma: no mutate
    try:
        entry, deps, managed, secrets = _onboard_python(
            tmp,
            text,
            name=name,
            description=description,
            deps_opt=deps_opt,
            python_opt=python_opt,
            no_input=True,
        )
    except store.StoreError as exc:
        raise _fail(str(exc), 1) from exc
    finally:
        tmp.unlink(missing_ok=True)  # pragma: no mutate
    _print_add_summary(entry, deps, managed, secrets)


def _add_script_from_stdin(kind: str, name: str | None, description: str | None) -> None:
    """`skit add - --kind shell`: the non-python twin of _add_from_stdin. Before this
    lane existed, --kind on stdin was SILENTLY DROPPED and the text became a python
    entry — bash source stored as script.py and fed to `uv run --script` (a corrupted
    entry, in the codebase whose contract is refuse-never-drop)."""
    import tempfile

    from .langs.registry import shebang_program

    if not name:
        err_console.print(
            f"[red]{gettext('Reading the script from stdin needs an explicit --name.')}[/red]"
        )
        raise typer.Exit(EXIT_USAGE)
    text = sys.stdin.read()
    if not text.strip():
        err_console.print(
            f"[red]{gettext('Nothing arrived on stdin, so there is nothing to add.')}[/red]"
        )
        raise typer.Exit(1)
    kind_spec = spec_for(kind)
    suffix = kind_spec.extensions[0] if kind_spec is not None and kind_spec.extensions else ".txt"
    fd, tmp_name = tempfile.mkstemp(suffix=suffix, prefix="skit-stdin-")  # pragma: no mutate
    os.close(fd)
    tmp = Path(tmp_name)
    tmp.write_text(text, encoding="utf-8")  # pragma: no mutate
    try:
        program = shebang_program(tmp)
        interpreter = program if kind_spec is not None and program in kind_spec.shebangs else ""
        entry = store.add_script(
            tmp, kind=kind, name=name, mode="copy", description=description, interpreter=interpreter
        )
        deps: list[str] = []
        if (
            kind_spec is not None
            and kind_spec.deps_flavor == "npm"
            and kind_spec.dep_scanner is not None
        ):
            # Mirror the path lane's non-interactive default: record the script's own
            # imports (they download packages on first run — the summary says so).
            deps = kind_spec.dep_scanner(text)
            if deps:
                entry = store.update_dependencies(entry.slug, deps)
    except store.StoreError as exc:
        raise _fail(str(exc), 1) from exc
    finally:
        tmp.unlink(missing_ok=True)  # pragma: no mutate
    _print_add_summary(entry, deps, [], [])


def _starter_prompt() -> str:
    """The drafted-prompt starter body — user-visible prose in $EDITOR, so it goes
    through gettext like every other UI string (resolved at call time, never import)."""
    return (
        gettext(
            "# New prompt\n"
            "\n"
            "Describe the task for the agent. Mark the parts that change per run as "
            "{{placeholders}} — each becomes a form field. Everything else travels to "
            "the agent exactly as written."
        )
        + "\n"
    )


def _custom_runner_hint() -> str:
    """The one-line "you can define your own agent" teaching, shared by every plain-mode
    runner ask. A function, not a module constant: gettext must run at call time."""
    return gettext("Custom agents: skit runner add NAME COMMAND… (see skit runner --help)")


def _ask_prompt_runner(interactive: bool, runner_opt: str | None) -> str:
    """The runner pin for a new prompt entry: --runner is validated as given; else an
    interactive numbered pick prefilled from the last-picked state; else no pin (the
    run form / --runner asks later — never a guess)."""
    runners = config.load_prompt_runners()
    names = [r.name for r in runners]
    if runner_opt is not None:
        if runner_opt not in names:
            err_console.print(
                "[red]"
                + gettext("Unknown runner: %(runner)s. Configured runners: %(names)s")
                % {"runner": escape(runner_opt), "names": ", ".join(names) or "—"}
                + _RED_CLOSE
            )
            raise typer.Exit(EXIT_USAGE)
        argstate.save_last_runner(runner_opt)
        return runner_opt
    if not interactive or not names:
        return ""
    last = argstate.load_last_runner()
    default = last if last in names else "-"
    console.print(f"[dim]{gettext('- = no pin (the run form asks each time)')}[/dim]")
    console.print(f"[dim]{_custom_runner_hint()}[/dim]")
    picked = Prompt.ask(
        gettext("Run this prompt with which agent?"),
        choices=[*names, "-"],
        default=default,
        console=console,
    )
    if picked == "-":
        return ""
    argstate.save_last_runner(picked)
    return picked


def _onboard_prompt(
    resolved: Path,
    *,
    name: str | None,
    description: str | None,
    ref: bool,
    runner_opt: str | None,
    no_input: bool,
    interpolate: bool = True,
) -> tuple[store.Entry, list[str]]:
    """Prompt onboarding at add time: detected placeholders are candidates (prompts
    contain code snippets, so false positives are expected) — interactive adds pick the
    kept subset (or answer "off" to switch insertion off for the whole entry),
    non-interactive adds manage them all up to the flood cap; unmanaged ones stay
    verbatim in the body. Returns (entry, managed names)."""
    try:
        text = resolved.read_text(encoding="utf-8", errors="replace")
    except OSError as exc:
        raise store.StoreError(
            gettext("Can't read %(path)s: %(error)s")
            % {"path": str(resolved), "error": exc.strerror or str(exc)}
        ) from exc
    from .langs.prompt import analyzer as prompt_analyzer

    detected = prompt_analyzer.placeholder_names(text) if interpolate else []
    interactive = not no_input and _is_interactive()
    managed: list[str] | None = None  # None = keep every detected candidate (capped)
    flooded = len(detected) > prompt_analyzer.AUTO_MANAGE_LIMIT
    if detected and interactive:
        console.print(gettext("Detected placeholders (each becomes a form field):"))
        for i, placeholder in enumerate(detected[: prompt_analyzer.LIST_PREVIEW_LIMIT], start=1):
            mark = gettext(" (secret)") if is_secret_name(placeholder) else ""
            console.print(f"  {i}. {escape(placeholder)}{mark}")
        if len(detected) > prompt_analyzer.LIST_PREVIEW_LIMIT:
            console.print(
                "  "
                + gettext("…and %(count)s more")
                % {"count": len(detected) - prompt_analyzer.LIST_PREVIEW_LIMIT}
            )
        answer = Prompt.ask(
            gettext("Manage which? (all / none / off / numbers like 1,3)"),
            default="none" if flooded else "all",
            console=console,
        )
        normalized = answer.strip().lower()
        if normalized == "off":
            interpolate = False
            managed = None
        elif normalized == "all":
            managed = detected  # an explicit "all" takes everything, preview or not
            flooded = False
        else:
            # Numbers address only the PREVIEWED names — picking an index whose name
            # was never shown would be a blind selection.
            selectable = detected[: prompt_analyzer.LIST_PREVIEW_LIMIT]
            managed = [selectable[i] for i in _parse_selection(answer, len(selectable))]
            flooded = False  # an explicit interactive answer is always honored
    runner = _ask_prompt_runner(interactive, runner_opt)
    entry = store.add_prompt(
        resolved,
        name=name,
        mode="reference" if ref else "copy",
        description=description,
        managed=managed,
        runner=runner,
        interpolate=interpolate,
    )
    if not interpolate:
        console.print(
            f"[dim]{gettext('Variable insertion is off — the body travels to the agent exactly as written (turn it on with: skit params %(name)s --interpolate)') % {'name': escape(entry.meta.name)}}[/dim]"
        )
    elif flooded and managed is None:
        # The auto path tripped the flood cap: nothing was managed, say so honestly.
        console.print(
            f"[dim]{gettext('Detected %(count)s placeholders — too many to manage automatically, so none were. Manage the ones you need with: skit params %(name)s --add NAME, or turn insertion off with --no-interpolate.') % {'count': len(detected), 'name': escape(entry.meta.name)}}[/dim]"
        )
    if runner:
        console.print(
            f"[dim]{gettext('Runs with %(runner)s (change it with: skit params %(name)s --runner NAME)') % {'runner': escape(runner), 'name': escape(entry.meta.name)}}[/dim]"
        )
    return entry, list(entry.meta.params or [])


def _create_prompt_in_editor(
    name: str | None,
    description: str | None,
    runner_opt: str | None,
    *,
    interpolate: bool = True,
) -> None:
    """`skit add --prompt` with no path: draft a new prompt in $EDITOR, then ingest it —
    the prompt twin of `add --edit`. Under a pipe/--no-input there is no editor: the
    body arrives on stdin instead (`skit add --prompt -n review < body.md`)."""
    import tempfile

    if not _is_interactive():
        _add_prompt_from_stdin(name, description, runner_opt, interpolate=interpolate)
        return
    if not name:
        name = Prompt.ask(gettext("Name in skit"), console=console).strip()
        if not name:
            err_console.print(f"[red]{gettext('A name is required.')}[/red]")
            raise typer.Exit(EXIT_USAGE)
    fd, tmp_name = tempfile.mkstemp(suffix=".prompt.md", prefix="skit-new-")  # pragma: no mutate
    os.close(fd)
    tmp = Path(tmp_name)
    starter = _starter_prompt()
    tmp.write_text(starter, encoding="utf-8")  # pragma: no mutate
    try:
        console.print(f"[dim]{gettext('Opening your editor…')}[/dim]")
        editor.open_in_editor(tmp)
        text = tmp.read_text(encoding="utf-8", errors="replace")  # pragma: no mutate — utf-8 equiv
        if text.strip() in ("", starter.strip()):
            console.print(gettext("Nothing was written, so no script was added."))
            return
        entry, managed = _onboard_prompt(
            tmp,
            name=name,
            description=description,
            ref=False,
            runner_opt=runner_opt,
            no_input=False,
            interpolate=interpolate,
        )
    except (editor.EditorError, store.StoreError) as exc:
        raise _fail(str(exc), 1) from exc
    finally:
        tmp.unlink(missing_ok=True)  # pragma: no mutate — the temp file always exists here
    _print_add_summary(entry, [], managed, [n for n in managed if is_secret_name(n)])


def _add_prompt_from_stdin(
    name: str | None,
    description: str | None,
    runner_opt: str | None,
    *,
    interpolate: bool = True,
) -> None:
    """The prompt twin of `skit add -`: the body arrives on stdin, so there is nobody to
    prompt — a name is required up front and every detected placeholder is managed."""
    import tempfile

    if not name:
        err_console.print(
            f"[red]{gettext('Reading the script from stdin needs an explicit --name.')}[/red]"
        )
        raise typer.Exit(EXIT_USAGE)
    text = sys.stdin.read()
    if not text.strip():
        err_console.print(
            f"[red]{gettext('Nothing arrived on stdin, so there is nothing to add.')}[/red]"
        )
        raise typer.Exit(1)
    fd, tmp_name = tempfile.mkstemp(suffix=".prompt.md", prefix="skit-stdin-")  # pragma: no mutate
    os.close(fd)
    tmp = Path(tmp_name)
    tmp.write_text(text, encoding="utf-8")  # pragma: no mutate
    try:
        entry, managed = _onboard_prompt(
            tmp,
            name=name,
            description=description,
            ref=False,
            runner_opt=runner_opt,
            no_input=True,
            interpolate=interpolate,
        )
    except store.StoreError as exc:
        raise _fail(str(exc), 1) from exc
    finally:
        tmp.unlink(missing_ok=True)  # pragma: no mutate
    _print_add_summary(entry, [], managed, [n for n in managed if is_secret_name(n)])


def _infer_add_kind(resolved: Path, exe_flag: bool) -> str:
    """Type inference (v2) — delegated to store.infer_kind so the CLI and the TUI add
    panel share one rule and can't drift apart."""
    return store.infer_kind(resolved, force_exe=exe_flag)


def _forceable_kinds() -> list[str]:
    """The kinds `--kind` may force: every interpreted language plus "exe". Command
    templates are their own path (--cmd), and "unknown" is never a target."""
    interpreted = sorted(
        k for k in KNOWN_KINDS if (spec := spec_for(k)) is not None and spec.family == "interpreted"
    )
    return [*interpreted, "exe"]


def _validate_forced_kind(value: str) -> None:
    """Reject an unknown --kind value with a usage error that lists the valid kinds
    (the non-interactive contract: never guess, fail cleanly)."""
    valid = _forceable_kinds()
    if value not in valid:
        err_console.print(
            f"[red]{gettext('Unknown kind: %(kind)s. Choose from: %(kinds)s') % {'kind': escape(value), 'kinds': ', '.join(valid)}}[/red]"
        )
        raise typer.Exit(EXIT_USAGE)


@app.command(
    help=gettext("Add a script, executable, or command to skit."),
    epilog=gettext(
        "Examples:  skit add tools/resize.py  ·  skit add --cmd 'ffmpeg -i {input}' -n convert  ·  pbpaste | skit add - -n clip"
    ),
)
def add(
    path: str = typer.Argument(
        None, help=gettext("Path to a script or executable, or '-' to read a script from stdin")
    ),
    name: str = typer.Option(
        None, "--name", "-n", help=gettext("Name / alias (defaults to the file name)")
    ),
    description: str = typer.Option(
        None,
        "--description",
        "-d",
        help=gettext("Description (defaults to the first line of the docstring)"),
    ),
    edit_new: bool = typer.Option(
        False, "--edit", "-e", help=gettext("Write a brand-new script in your editor, then add it")
    ),
    ref: bool = typer.Option(
        False,
        "--ref",
        help=gettext("Reference mode: link to the original file instead of copying it"),
    ),
    exe: bool = typer.Option(
        False,
        "--exe",
        help=gettext("Force the executable kind (normally inferred from the file itself)"),
    ),
    kind: str = typer.Option(
        None,
        "--kind",
        help=gettext("Force the language kind (e.g. shell, js) for an extensionless file"),
    ),
    cmd: str = typer.Option(
        None, "--cmd", help=gettext("Register a command template, e.g. --cmd 'ffmpeg -i {input}'")
    ),
    prompt_kind: bool = typer.Option(
        False,
        "--prompt",
        help=gettext(
            "Add the file as a prompt for an AI agent (with no path: draft one in your editor)"
        ),
    ),
    runner: str = typer.Option(
        None,
        "--runner",
        help=gettext("Pin the agent a prompt entry runs with (see skit runner list)"),
    ),
    no_interpolate: bool = typer.Option(
        False,
        "--no-interpolate",
        help=gettext(
            "Prompt only: no variable insertion at all — the body travels exactly as written"
        ),
    ),
    dep: list[str] = typer.Option(
        None,
        "--dep",
        help=gettext("A dependency (repeat for more; skips the interactive question)"),
    ),
    python: str = typer.Option(
        None, "--python", help=gettext('Python version constraint, e.g. ">=3.11"')
    ),
    no_input: bool = typer.Option(
        False, "--no-input", help=gettext("Never prompt; accept the detected suggestions")
    ),
) -> None:
    """Add a script / executable / command to skit."""
    # Both fresh-script lanes (editor buffer / stdin) have no original file for --ref to
    # point at — refused, never dropped (the non-interactive contract).
    if (edit_new or path == "-" or (prompt_kind and not path)) and ref:
        err_console.print(
            f"[red]{gettext('--ref needs an existing file to reference — a script written in the editor or read from stdin has none.')}[/red]"
        )
        raise typer.Exit(EXIT_USAGE)
    if no_interpolate and (edit_new or cmd is not None or exe):
        err_console.print(
            f"[red]{gettext('--no-interpolate only applies to prompt entries — add one with --prompt.')}[/red]"
        )
        raise typer.Exit(EXIT_USAGE)
    if runner is not None and (edit_new or cmd is not None or exe):
        # --runner names the agent a PROMPT runs with; on a lane that can't produce one
        # it would be silently ignored — refused instead (the non-interactive contract).
        # Path-based adds check again after kind inference (a .prompt.md needs no flag).
        err_console.print(
            f"[red]{gettext('--runner only applies to prompt entries — add one with --prompt.')}[/red]"
        )
        raise typer.Exit(EXIT_USAGE)
    if prompt_kind and (edit_new or exe or cmd is not None or kind is not None):
        err_console.print(
            f"[red]{gettext('--prompt names the kind outright — drop --edit/--exe/--kind/--cmd.')}[/red]"
        )
        raise typer.Exit(EXIT_USAGE)
    # The fresh-text lanes can't produce a program on disk, and --edit's starter is
    # python-shaped: an explicit flag the lane can't honor is refused, never dropped
    # (before this check, `cat tool.sh | skit add - --kind shell` silently stored the
    # bash source as a PYTHON entry).
    if exe and (edit_new or path == "-"):
        err_console.print(
            f"[red]{gettext('--exe needs an existing program on disk — stdin and the editor author scripts.')}[/red]"
        )
        raise typer.Exit(EXIT_USAGE)
    if kind is not None and edit_new:
        err_console.print(
            "[red]"
            + gettext(
                "--edit reads the kind from your draft's shebang (write e.g. "
                "#!/usr/bin/env bash) — drop --kind, or pipe it in: "
                "skit add - --kind %(kind)s -n NAME"
            )
            % {"kind": escape(kind)}
            + _RED_CLOSE
        )
        raise typer.Exit(EXIT_USAGE)
    if edit_new:
        if path:
            err_console.print(
                f"[red]{gettext('Use --edit to write a new script, or pass a path to add an existing one — not both.')}[/red]"
            )
            raise typer.Exit(EXIT_USAGE)
        _create_python_in_editor(name, deps_opt=dep, python_opt=python)
        return
    if path == "-":
        if prompt_kind:
            _add_prompt_from_stdin(name, description, runner, interpolate=not no_interpolate)
            return
        if kind is not None:
            _validate_forced_kind(kind)
            if kind == "exe":  # --kind exe: same impossibility as --exe, same refusal
                err_console.print(
                    f"[red]{gettext('--exe needs an existing program on disk — stdin and the editor author scripts.')}[/red]"
                )
                raise typer.Exit(EXIT_USAGE)
            if kind != "python":
                _add_script_from_stdin(kind, name, description)
                return
        _add_from_stdin(name, description, deps_opt=dep, python_opt=python)
        return
    if prompt_kind and not path:
        _create_prompt_in_editor(name, description, runner, interpolate=not no_interpolate)
        return
    summary_deps: list[str] = []
    summary_managed: list[str] = []
    summary_secrets: list[str] = []
    try:
        if cmd is not None:
            if not name:
                err_console.print(f"[red]{gettext('A --cmd entry needs a --name')}[/red]")
                raise typer.Exit(EXIT_USAGE)
            _refuse_unusable_add_flags("command", spec_for("command"), ref, dep, python)
            entry = store.add_command(cmd, name=name, description=description or "")
            if entry.meta.params:
                console.print(
                    gettext(
                        "Detected parameters: %(names)s (the run form asks for them; your last values are remembered)"
                    )
                    % {"names": ", ".join(escape(p) for p in entry.meta.params)}
                )
        else:
            if not path:
                err_console.print(
                    f"[red]{gettext('Provide a script path, or use --cmd to register a command template')}[/red]"
                )
                raise typer.Exit(EXIT_USAGE)
            resolved = Path(path).expanduser().resolve()
            if prompt_kind:
                kind = "prompt"  # --prompt forces the kind outright (mirrors --exe)
            elif kind is not None:
                # --kind forces the language outright (mirrors --exe), for an
                # extensionless file the shebang/extension can't classify.
                _validate_forced_kind(kind)
                if exe and kind != "exe":
                    err_console.print(f"[red]{gettext('Use --kind or --exe, not both.')}[/red]")
                    raise typer.Exit(EXIT_USAGE)
            else:
                kind = _infer_add_kind(resolved, exe)
            # A bare .md is too ambiguous to claim outright, and too likely a prompt
            # to refuse outright: interactively, ask; under --no-input/pipe, an
            # explicit --prompt is required — never a guess.
            if (
                kind == "unknown"
                and resolved.suffix.lower() == ".md"
                and not no_input
                and _is_interactive()
            ):
                if Confirm.ask(
                    gettext("%(file)s looks like a prompt. Add it as one?")
                    % {"file": escape(resolved.name)},
                    default=True,
                    console=console,
                ):
                    kind = "prompt"
                else:
                    console.print(f"[dim]{gettext('Cancelled — nothing was added.')}[/dim]")
                    raise typer.Exit(EXIT_CANCELLED)
            kind_spec = spec_for(kind)
            _refuse_unusable_add_flags(kind, kind_spec, ref, dep, python)
            if runner is not None and kind != "prompt":
                err_console.print(
                    f"[red]{gettext('--runner only applies to prompt entries — add one with --prompt.')}[/red]"
                )
                raise typer.Exit(EXIT_USAGE)
            if no_interpolate and kind != "prompt":
                err_console.print(
                    f"[red]{gettext('--no-interpolate only applies to prompt entries — add one with --prompt.')}[/red]"
                )
                raise typer.Exit(EXIT_USAGE)
            if kind == "exe":
                if not no_input and _is_interactive():
                    # The one add lane that asked NOTHING while every sibling reviews
                    # identity — "nothing to detect inside a binary" justifies no tick
                    # list, not skipping the name and the discovery-surface description.
                    if not name:
                        name = (
                            Prompt.ask(
                                gettext("Name in skit"), default=resolved.stem, console=console
                            ).strip()
                            or None
                        )
                    if description is None:
                        description = Prompt.ask(
                            gettext("Description (optional)"), default="", console=console
                        ).strip()
                entry = store.add_exe(Path(path), name=name, description=description or "")
            elif kind == "unknown":
                err_console.print(
                    f"[red]{gettext("%(file)s isn't a script or an executable — pass --kind <language> for an extensionless script, --prompt for an AI-agent prompt, --exe for a program, or --cmd for a command template.") % {'file': escape(resolved.name)}}[/red]"
                )
                raise typer.Exit(EXIT_USAGE)
            elif kind == "prompt":
                # Interactive + mini-form style: host the SAME review panel the TUI's
                # `a` opens for prompts (flags prefill it) — exact python-lane parity.
                # Pipes/CI/--no-input/form=plain keep the line-prompt path untouched.
                if (
                    not no_input
                    and _is_interactive()
                    and os.environ.get("TERM") != "dumb"
                    and config.load_form() == "tui"
                ):
                    # The panel's __init__ reads the body outright — guard BEFORE it
                    # opens (the python lane's _require_file discipline), so a typo'd
                    # path is a clean StoreError, never a raw FileNotFoundError.
                    _require_file(resolved)
                    if runner is not None and config.find_prompt_runner(runner) is None:
                        # An explicit flag the panel can't honor is refused, never
                        # dropped — same rule _ask_prompt_runner enforces on its lane.
                        runner_names = [r.name for r in config.load_prompt_runners()]
                        err_console.print(
                            "[red]"
                            + gettext("Unknown runner: %(runner)s. Configured runners: %(names)s")
                            % {"runner": escape(runner), "names": ", ".join(runner_names) or "—"}
                            + _RED_CLOSE
                        )
                        raise typer.Exit(EXIT_USAGE)
                    from .tui_add import run_prompt_review

                    slug = run_prompt_review(
                        resolved,
                        name=name,
                        description=description,
                        reference=ref,
                        runner=runner,
                        interpolate=not no_interpolate,
                    )
                    if slug is None:
                        console.print(f"[dim]{gettext('Cancelled — nothing was added.')}[/dim]")
                        raise typer.Exit(EXIT_CANCELLED)
                    entry = store.resolve(slug)
                    summary_managed = list(entry.meta.params or [])
                else:
                    entry, summary_managed = _onboard_prompt(
                        resolved,
                        name=name,
                        description=description,
                        ref=ref,
                        runner_opt=runner,
                        no_input=no_input,
                        interpolate=not no_interpolate,
                    )
                summary_secrets = [n for n in summary_managed if is_secret_name(n)]
            elif kind != "python" and kind_spec is not None and kind_spec.family == "interpreted":
                # Interpreted add (shell/js/ts/fish/ruby/…): the SAME review panel the
                # python lane hosts — identity, storage, deps per flavor, and the tick
                # list from the kind's own analyzer. (The old comment claimed these
                # kinds "have no analyzer to review with"; shell/js/ts/fish all do —
                # the panel just was never wired to them.)
                if (
                    not no_input
                    and _is_interactive()
                    and os.environ.get("TERM") != "dumb"
                    and config.load_form() == "tui"
                ):
                    from .tui_add import run_add_review

                    slug = run_add_review(
                        resolved,
                        kind=kind,
                        name=name,
                        description=description,
                        reference=ref,
                        deps=dep,
                    )
                    if slug is None:
                        console.print(f"[dim]{gettext('Cancelled — nothing was added.')}[/dim]")
                        raise typer.Exit(EXIT_CANCELLED)
                    entry = store.resolve(slug)
                    summary_deps = list(entry.meta.dependencies or [])
                else:
                    from .langs.registry import shebang_program

                    program = shebang_program(resolved)
                    interpreter = program if program in kind_spec.shebangs else ""
                    entry = store.add_script(
                        Path(path),
                        kind=kind,
                        name=name,
                        mode="reference" if ref else "copy",
                        description=description,
                        interpreter=interpreter,
                    )
                    if kind_spec.deps_flavor == "npm" and entry.meta.mode == "copy":
                        summary_deps = _resolve_npm_dependencies(
                            resolved, dep, no_input, kind_spec.dep_scanner
                        )
                        if summary_deps:
                            entry = store.update_dependencies(entry.slug, summary_deps)
                    summary_managed = _onboard_script_params(entry, kind_spec, no_input)
                    summary_secrets = [n for n in summary_managed if is_secret_name(n)]
            else:
                _require_file(resolved)
                try:
                    text = resolved.read_text(encoding="utf-8", errors="replace")
                except OSError as exc:
                    raise store.StoreError(
                        gettext("Can't read %(path)s: %(error)s")
                        % {"path": str(resolved), "error": exc.strerror or str(exc)}
                    ) from exc
                # Interactive + mini-form style: host the SAME review panel the TUI's
                # `a` opens (flags prefill it). Pipes/CI/--no-input/form=plain keep the
                # line-prompt path — the non-interactive contract is untouched.
                if (
                    not no_input
                    and _is_interactive()
                    and os.environ.get("TERM") != "dumb"
                    and config.load_form() == "tui"
                ):
                    from .tui_add import run_add_review

                    slug = run_add_review(
                        Path(path),
                        name=name,
                        description=description,
                        reference=ref,
                        deps=dep,
                        requires_python=python or "",
                    )
                    if slug is None:
                        console.print(f"[dim]{gettext('Cancelled — nothing was added.')}[/dim]")
                        raise typer.Exit(EXIT_CANCELLED)
                    entry = store.resolve(slug)
                else:
                    entry, summary_deps, summary_managed, summary_secrets = _onboard_python(
                        Path(path),
                        text,
                        name=name,
                        description=description,
                        ref=ref,
                        deps_opt=dep,
                        python_opt=python,
                        no_input=no_input,
                    )
    except store.StoreError as exc:
        raise _fail(str(exc), 1) from exc
    _print_add_summary(entry, summary_deps, summary_managed, summary_secrets)


def _print_add_summary(
    entry: store.Entry, deps: list[str], managed: list[str], secrets: list[str]
) -> None:
    """One consolidated block after a successful add."""
    entry_spec = spec_for(entry.meta.kind)
    mode_note = (
        gettext("(%(mode)s mode)") % {"mode": entry.meta.mode}
        if entry_spec is not None and entry_spec.supports_modes
        else ""
    )
    console.print(
        f"[green]{gettext('Added: %(name)s') % {'name': escape(entry.meta.name)}}[/green] {mode_note}"
    )
    if entry.meta.description:
        console.print(
            f"  {gettext('Description: %(desc)s') % {'desc': escape(entry.meta.description)}}"
        )
    if deps:
        console.print(
            f"  {gettext('Dependencies: %(deps)s') % {'deps': ', '.join(escape(d) for d in deps)}}"
        )
    if managed:
        console.print(
            f"  {gettext('Managed parameters: %(names)s') % {'names': ', '.join(escape(n) for n in managed)}}"
        )
    console.print(f"  {gettext('Run it: skit run %(name)s') % {'name': escape(entry.meta.name)}}")
    if secrets:
        console.print(
            f"[dim]{gettext('Secret parameter values are never saved to disk: %(names)s') % {'names': ', '.join(escape(n) for n in secrets)}}[/dim]"
        )


# --------------------------------------------------------------------------
# list / remove / edit
# --------------------------------------------------------------------------


@app.command("list", help=gettext("List every registered script."))
def list_cmd(
    as_json: bool = typer.Option(False, "--json", help=gettext("Output as JSON")),
) -> None:
    """List every registered script."""
    entries = store.list_entries()
    if as_json:
        rows = []
        for e in entries:
            last = argstate.load_state(e.slug)["last_run"]
            rows.append(
                {
                    "name": e.meta.name,
                    "slug": e.slug,
                    "kind": e.meta.kind,
                    "mode": e.meta.mode,
                    "description": e.meta.description,
                    "missing": launcher.target_missing(e),
                    "last_run_at": last.get("at"),
                    "last_exit": last.get("exit"),
                }
            )
        console.print_json(json.dumps(rows, ensure_ascii=False))
        return
    if not entries:
        console.print(gettext("No scripts yet. Add one with: skit add <path>"))
        return
    from rich.table import Table

    table = Table(show_header=True, header_style="bold")
    table.add_column(gettext("Name"))
    table.add_column(gettext("Kind"))
    table.add_column(gettext("Description"))
    for e in entries:
        table.add_row(escape(e.meta.name), e.meta.kind, _list_description(e))
    console.print(table)


def _list_description(e: store.Entry) -> str:
    desc = escape(e.meta.description) if e.meta.description else "—"
    marker = launcher.missing_marker(e)
    if marker is None:
        return desc
    marker = f"[dim]{escape(marker)}[/dim]"
    return marker if desc == "—" else f"{desc}  {marker}"


# --------------------------------------------------------------------------
# show — the full read view of one script (identity + schema + presets)
# --------------------------------------------------------------------------


def _field_secret_cell(f: flows.FormField) -> str:
    """The Secret column for a form field: "—", "yes", or "yes ← $ENVVAR"."""
    if not f.secret:
        return "—"
    if f.env_source:
        return gettext("yes") + f" ← ${escape(f.env_source)}"
    return gettext("yes")


# The stable, machine-facing origin token for the whole form plan (additive to the legacy
# param_source; a value source now includes "declared"/"env" that predates this key).
_PARAM_ORIGIN = {
    "declared": "declared",
    "argparse": "reader",
    "inject": "managed",
    "command": "command",
}


def _param_origin(source: str) -> str:
    return _PARAM_ORIGIN.get(source, "none")


def _field_to_dict(f: flows.FormField) -> dict[str, object]:
    """One form field as a stable-shape JSON object (every key always present).
    `default` is null when the script declares none; a secret's declared default is
    emitted as-is, matching `params --json` (it already lives in the script's own text)."""
    return {
        "key": f.key,
        "label": f.label,
        "type": f.kind,
        "source": f.source,
        "required": f.required,
        "secret": f.secret,
        "multiple": f.multiple,
        "degraded": f.degraded,
        "choices": list(f.choices),
        "default": f.default if f.has_default else None,
        "help": f.help,
        "flag": f.flag,
        "action": f.action,
        "env_source": f.env_source,
    }


def _print_show_human(entry: store.Entry, plan: flows.FormPlan, presets: list[str]) -> None:
    meta = entry.meta
    console.print(f"[bold]{escape(meta.name)}[/bold]  [dim]({meta.kind} · {meta.mode})[/dim]")
    if meta.description:
        console.print(f"  {escape(meta.description)}")
    show_spec = spec_for(meta.kind)
    if show_spec is None or show_spec.has_original_file:
        console.print(f"  {gettext('Source: %(path)s') % {'path': escape(str(meta.source))}}")
    if meta.workdir != "origin":
        console.print(
            f"  {gettext('Working directory: %(dir)s') % {'dir': escape(str(meta.workdir))}}"
        )
    if meta.interpreter:
        console.print(
            f"  {gettext('Interpreter: %(program)s') % {'program': escape(meta.interpreter)}}"
        )
    marker = launcher.missing_marker(entry)
    if marker is not None:
        console.print(f"  [yellow]{escape(marker)}[/yellow]")
    if meta.dependencies:
        console.print(
            f"  {gettext('Dependencies: %(deps)s') % {'deps': ', '.join(escape(d) for d in meta.dependencies)}}"
        )
    if meta.requires_python:
        console.print(
            f"  {gettext('Python constraint: %(python)s') % {'python': escape(meta.requires_python)}}"
        )
    if meta.needs:
        console.print(
            f"  {gettext('Needs: %(needs)s') % {'needs': ', '.join(escape(n) for n in meta.needs)}}"
        )
    if meta.template:
        console.print(
            f"  {gettext('Command template: %(template)s') % {'template': escape(meta.template)}}"
        )
    if meta.kind == "prompt":
        console.print(
            f"  {gettext('Runner: %(runner)s') % {'runner': escape(meta.runner) if meta.runner else gettext('(asks at run time)')}}"
        )
        if not meta.interpolate:
            console.print(f"  {gettext('Variable insertion: off (the body travels as written)')}")
    _print_drift(plan)
    if plan.degraded_reason:
        console.print(
            f"[dim]{gettext("skit could not model this script's own arguments; pass them after -- instead.")}[/dim]"
        )
    if plan.fields:
        _print_show_fields(plan)
    else:
        console.print(
            f"  {gettext('No form fields — arguments after -- pass straight through to the script.')}"
        )
    if presets:
        console.print(
            f"  {gettext('Presets: %(names)s') % {'names': ', '.join(escape(p) for p in presets)}}"
        )
    console.print(f"  {gettext('Run it: skit run %(name)s') % {'name': escape(meta.name)}}")


def _print_show_fields(plan: flows.FormPlan) -> None:
    from rich.table import Table

    table = Table(show_header=True, header_style="bold")  # pragma: no mutate — cosmetic
    table.add_column(gettext("Parameter"))
    table.add_column(gettext("Type"))
    table.add_column(gettext("Required"))
    table.add_column(gettext("Default"))
    table.add_column(gettext("Choices"))
    table.add_column(gettext("Secret"))
    table.add_column(gettext("Help"))
    for f in plan.fields:
        if not f.has_default:
            default_shown = "—"
        elif f.secret:
            default_shown = gettext("•••")
        else:
            default_shown = f.default or "—"
        # Inject fields carry their form prompt in `label`; argparse fields carry help text.
        help_shown = f.help or (f.label if f.label != f.key else "")
        table.add_row(
            escape(f.key),
            escape(f.kind),
            gettext("yes") if f.required else "—",
            escape(default_shown),
            escape(", ".join(f.choices)) if f.choices else "—",
            _field_secret_cell(f),
            escape(help_shown) if help_shown else "—",
        )
    console.print(table)


@app.command(
    help=gettext("Show everything about one script: metadata, dependencies, parameters, presets."),
    epilog=gettext("Examples:  skit show resize  ·  skit show resize --json"),
)
def show(
    name: str = _SCRIPT_ARG,
    as_json: bool = typer.Option(False, "--json", help=gettext("Output as JSON")),
) -> None:
    """The single read view an automation (or a human) needs before running a script:
    identity, dependencies, the unified parameter schema (all three sources), presets."""
    try:
        entry = store.resolve(name)
    except store.NotFoundError as exc:
        raise _fail(str(exc), 1) from exc
    plan = flows.plan_for_entry(entry)
    state = argstate.load_state(entry.slug)
    presets = sorted(state["presets"])
    if not as_json:
        _print_show_human(entry, plan, presets)
        return
    last = state["last_run"]
    payload = {
        "name": entry.meta.name,
        "slug": entry.slug,
        "kind": entry.meta.kind,
        "mode": entry.meta.mode,
        "description": entry.meta.description,
        "source": entry.meta.source,
        "workdir": str(entry.meta.workdir),
        "interpreter": entry.meta.interpreter or None,
        "missing": launcher.target_missing(entry),
        "dependencies": list(entry.meta.dependencies or []),
        "requires_python": entry.meta.requires_python,
        "needs": list(entry.meta.needs or []),
        "template": entry.meta.template or None,
        "param_source": plan.source,
        "param_origin": _param_origin(plan.source),
        "degraded_reason": plan.degraded_reason,
        "drift": bool(plan.drift_lines),
        "fields": [_field_to_dict(f) for f in plan.fields],
        "presets": presets,
        "last_run_at": last.get("at"),
        "last_exit": last.get("exit"),
    }
    if entry.meta.kind == "prompt":
        # Additive, prompt-only: the pin (null = asks/--runner decides), what an agent
        # may pass to --runner, and the insertion master switch. English-only machine
        # contract, like every --json key.
        payload["runner"] = entry.meta.runner or None
        payload["runners_available"] = [r.name for r in config.load_prompt_runners()]
        payload["interpolate"] = entry.meta.interpolate
    console.print_json(json.dumps(payload, ensure_ascii=False))


@app.command(help=gettext("Remove a registered script (the original file is left untouched)."))
def remove(
    name: str = _SCRIPT_ARG,
    yes: bool = typer.Option(False, "--yes", "-y", help=gettext("Skip confirmation")),
) -> None:
    """Remove a script (copy mode deletes the copy in the store; the original is untouched)."""
    try:
        entry = store.resolve(name)
    except store.NotFoundError as exc:
        raise _fail(str(exc), 1) from exc
    if not yes:
        entry_spec = spec_for(entry.meta.kind)
        if entry_spec is not None and not entry_spec.has_original_file:
            question = gettext('Remove "%(name)s"?') % {"name": entry.meta.name}
        else:
            question = gettext('Remove "%(name)s"? Your original file will not be deleted.') % {
                "name": entry.meta.name
            }
        typer.confirm(question, abort=True)
    removed = store.remove(name)
    console.print(f"[green]{gettext('Removed: %(name)s') % {'name': escape(removed)}}[/green]")


@app.command(help=gettext("Rename a script (presets, remembered values and history survive)."))
def rename(
    name: str = _SCRIPT_ARG,
    new_name: str = typer.Argument(..., help=gettext("The new name")),
) -> None:
    """The CLI twin of the Script-settings name field: agents curate the library too,
    and remove + re-add (the only workaround before) destroyed presets and history."""
    try:
        entry = store.rename(name, new_name)
    except (store.NotFoundError, store.StoreError) as exc:
        raise _fail(str(exc), 1) from exc
    console.print(
        f"[green]{gettext('Renamed to %(name)s.') % {'name': escape(entry.meta.name)}}[/green]"
    )


@app.command(help=gettext("Set a script's description (shown in the Library and skit list)."))
def describe(
    name: str = _SCRIPT_ARG,
    text: str = typer.Argument(..., help=gettext("The description (empty text clears it)")),
) -> None:
    """The CLI twin of the Script-settings description field — the discovery surface
    agents are told to maintain must be writable by them."""
    try:
        entry = store.update_description(name, text.strip())
    except store.NotFoundError as exc:
        raise _fail(str(exc), 1) from exc
    if entry.meta.description:
        console.print(
            f"[green]{gettext('Description updated for %(name)s.') % {'name': escape(entry.meta.name)}}[/green]"
        )
    else:
        console.print(
            f"[green]{gettext('Description cleared for %(name)s.') % {'name': escape(entry.meta.name)}}[/green]"
        )


def _offer_create_in_editor(name: str) -> None:
    """`skit edit <unknown>`: offer to create a brand-new script under that name."""
    if not _is_interactive():
        err_console.print(
            f"[red]{gettext('No script named %(name)s.') % {'name': escape(name)}}[/red]"
        )
        raise typer.Exit(1)
    if not Confirm.ask(
        gettext('No script named "%(name)s". Create it now?') % {"name": escape(name)},
        default=True,
        console=console,
    ):
        raise typer.Exit(0)  # pragma: no mutate — Exit(0)/Exit(None) both mean a clean exit
    _create_python_in_editor(name)


@app.command(
    help=gettext("Open a script's source in your editor (offers to create it if the name is new)."),
    epilog=gettext("Example:  skit edit resize"),
)
def edit(name: str = _SCRIPT_ARG) -> None:
    """Open a registered script's source in your editor."""
    try:
        entry = store.resolve(name)
    except store.NotFoundError:
        _offer_create_in_editor(name)
        return
    entry_spec = spec_for(entry.meta.kind)
    if entry_spec is None or not entry_spec.editable:
        raise _fail(
            gettext("%(name)s has no editable source (programs and command templates run as-is).")
            % {"name": entry.meta.name},
            1,
        )
    if entry.meta.mode == "reference":
        source = Path(entry.meta.source)
        if not source.exists():
            raise _fail(
                gettext("%(name)s: the referenced source file is gone: %(path)s")
                % {"name": entry.meta.name, "path": str(source)},
                1,
            )
        console.print(
            f"[dim]{gettext('Editing the original file (reference mode): %(path)s') % {'path': escape(str(source))}}[/dim]"
        )
        target = source
    else:
        target = entry.script_path
        if not target.exists():
            raise _fail(
                gettext("%(name)s has no stored copy to edit.") % {"name": entry.meta.name}, 1
            )
    try:
        editor.open_in_editor(target)
    except editor.EditorError as exc:
        raise _fail(str(exc), 1) from exc
    console.print(
        f"[green]{gettext('Saved %(name)s.') % {'name': escape(entry.meta.name)}}[/green]"
    )
    console.print(
        f"[dim]{gettext('skit reconciles parameter drift at run time; review managed parameters with: skit params %(name)s') % {'name': escape(entry.meta.name)}}[/dim]"
    )


# --------------------------------------------------------------------------
# run
# --------------------------------------------------------------------------


def _validate_preset(entry: store.Entry, preset: str | None) -> None:
    if not preset:
        return
    presets = argstate.load_state(entry.slug)["presets"]
    if preset not in presets:
        err_console.print(
            "[red]"
            + gettext('Unknown preset "%(preset)s". Available: %(presets)s')
            % {
                "preset": escape(preset),
                "presets": ", ".join(escape(p) for p in sorted(presets)) or "—",
            }
            + _RED_CLOSE
        )
        raise typer.Exit(EXIT_USAGE)


def _print_drift(plan: flows.FormPlan) -> None:
    for line in plan.drift_lines:
        err_console.print(f"[yellow]{escape(line)}[/yellow]")


def _parse_set_opts(plan: flows.FormPlan, raw: list[str]) -> dict[str, str]:
    """--set NAME=VALUE overrides: parsed strictly (never guess). A malformed item or an
    unknown name is a usage error (exit 2); a value that fails its field's own validation
    is a skit failure (exit 125), like any other bad value."""
    pairs: dict[str, str] = {}
    bad: list[str] = []
    for item in raw:
        key, sep, value = item.partition("=")
        if sep and key.strip():
            pairs[key.strip()] = value
        else:
            bad.append(item)
    if bad:
        err_console.print(
            "[red]"
            + gettext("Malformed --set (expected NAME=VALUE): %(items)s")
            % {"items": ", ".join(escape(b) for b in bad)}
            + _RED_CLOSE
        )
        raise typer.Exit(EXIT_USAGE)
    fields_by_key = {f.key: f for f in plan.fields}
    unknown = sorted(k for k in pairs if k not in fields_by_key)
    if unknown:
        err_console.print(
            "[red]"
            + gettext("Unknown parameter for --set: %(names)s. This script's parameters: %(valid)s")
            % {
                "names": ", ".join(escape(k) for k in unknown),
                "valid": ", ".join(escape(k) for k in sorted(fields_by_key)) or "—",
            }
            + _RED_CLOSE
        )
        raise typer.Exit(EXIT_USAGE)
    for key, value in pairs.items():
        error = flows.validate_value(fields_by_key[key], value)
        if error:
            raise _fail(error, EXIT_SKIT)
    return pairs


def _collect_values(
    entry: store.Entry,
    plan: flows.FormPlan,
    prefill: dict[str, str],
    *,
    plain: bool,
    runners: list[str] | None = None,
    runner_default: str = "",
) -> tuple[dict[str, str], str | None]:
    """Interactive collection through the configured renderer. The inline mini-form is
    the default; "plain" (--plain / form=plain / TERM=dumb) is the line-prompt fallback.

    Returns (values, picked runner name). The runner element is non-None only when
    `runners` was passed AND the inline form actually rendered its picker row — the
    line-prompt fallback never answers the runner question (the caller line-asks)."""
    style = "plain" if plain or os.environ.get("TERM") == "dumb" else config.load_form()
    if style == "tui":
        import importlib

        try:  # the inline renderer ships with the TUI layer; degrade to plain without it
            inlineform = importlib.import_module("skit.inlineform")
        except ImportError:  # pragma: no cover — transitional
            pass
        else:
            result = inlineform.collect(
                entry, plan, prefill, runners=runners, runner_default=runner_default
            )
            if result is None:
                raise typer.Exit(EXIT_CANCELLED)  # cancelling is not a skit failure
            return result
    console.print(
        gettext("Parameters for %(name)s (press Enter to keep the value shown):")
        % {"name": escape(entry.meta.name)}
    )
    return promptform.collect(plan, prefill, console=console), None


# How a flows.RunOutcome failure maps to skit's exit-code contract (docker convention).
# The numbers live in flows so the TUI's exit-after-run path shares them.
_FAILURE_EXIT = flows.FAILURE_EXIT_CODES


def _resolve_run_runner(
    entry: store.Entry, runner_opt: str | None, no_input: bool
) -> config.PromptRunner | None:
    """The prompt kind's runner resolution (deterministic; the non-interactive contract):
    --runner > the entry's pin > an interactive ask (prefilled from last-picked) > a
    clean exit 126 — never a guess, never a ranking. Non-prompt entries return None
    (and an explicit --runner on one is a usage error, not a silent drop)."""
    if entry.meta.kind != "prompt":
        if runner_opt is not None:
            err_console.print(f"[red]{gettext('--runner only applies to prompt entries.')}[/red]")
            raise typer.Exit(EXIT_USAGE)
        return None
    runners = config.load_prompt_runners()
    names = [r.name for r in runners]
    chosen = runner_opt or entry.meta.runner
    picked = runner_opt is not None
    if not chosen and not no_input and _is_interactive() and names:
        last = argstate.load_last_runner()
        console.print(f"[dim]{_custom_runner_hint()}[/dim]")
        chosen = Prompt.ask(
            gettext("Run this prompt with which agent?"),
            choices=names,
            default=last if last in names else names[0],
            console=console,
        )
        picked = True
    if not chosen:
        raise _fail(
            gettext(
                "No runner selected for %(name)s. Pass --runner NAME, or pin one with: "
                "skit params %(name)s --runner NAME"
            )
            % {"name": entry.meta.name},
            EXIT_NOT_EXECUTABLE,
        )
    found = next((r for r in runners if r.name == chosen), None)
    if found is None:
        raise _fail(
            gettext(
                "The runner %(runner)s isn't configured (known: %(names)s). Manage "
                "runners with: skit runner list"
            )
            % {"runner": chosen, "names": ", ".join(names) or "—"},
            EXIT_NOT_EXECUTABLE,
        )
    if picked:
        argstate.save_last_runner(chosen)
    return found


@app.command(
    help=gettext("Run a registered script or command in the terminal."),
    epilog=gettext(
        "Examples:  skit run stitch  ·  skit run stitch -p web -- extra.png  ·  skit run stitch --set width=800 --no-input  ·  skit run stitch --dry-run"
    ),
)
def run(
    name: str = _SCRIPT_ARG,
    args: list[str] = typer.Argument(
        None, help=gettext("Arguments passed through to the script (after --)")
    ),
    no_input: bool = typer.Option(
        False, "--no-input", help=gettext("Never prompt; reuse last values and defaults")
    ),
    preset: str = typer.Option(
        None,
        "--preset",
        "-p",
        help=gettext("Named preset of parameter values to prefill the form with"),
        autocompletion=_complete_preset,
    ),
    set_opts: list[str] = typer.Option(
        None,
        "--set",
        help=gettext(
            "Set a parameter value by name, as NAME=VALUE (repeatable; values may use tokens like {cwd} or {env:VAR}; the form no longer asks for a field you set)"
        ),
    ),
    save_preset: str = typer.Option(
        None, "--save-preset", help=gettext("Save this run's values as a named preset")
    ),
    plain: bool = typer.Option(
        False, "--plain", help=gettext("Line-by-line prompts instead of the inline form")
    ),
    raw: bool = typer.Option(
        False,
        "--raw",
        help=gettext(
            "Skip the parameter form and injection and run the script as-is (escape hatch)"
        ),
    ),
    dry_run: bool = typer.Option(
        False,
        "--dry-run",
        help=gettext(
            "Print the exact command that would run (tokens and globs expanded), then exit"
        ),
    ),
    runner: str = typer.Option(
        None,
        "--runner",
        help=gettext("Run a prompt entry with this agent (overrides its pin for one run)"),
        autocompletion=_complete_runner,
    ),
    forget_args: bool = typer.Option(
        False,
        "--forget-args",
        help=gettext(
            "Forget the remembered extra arguments before this run (they are otherwise "
            "reused when you pass none)"
        ),
    ),
) -> None:
    """Run a script (straight through the terminal). skit's own failures exit 125/126/127;
    the script's exit code passes through untouched."""
    try:
        entry = store.resolve(name)
    except store.NotFoundError as exc:
        raise _fail(str(exc), EXIT_NOT_FOUND) from exc
    if forget_args:
        # An imperative clear, honored up front regardless of how the run then goes:
        # the remembered argv tail was previously uneraseable from the CLI (an empty
        # `--` is indistinguishable from no `--`).
        argstate.save_last(entry.slug, extra_args=[])
    _validate_preset(entry, preset)
    # Form-shaped flags contradict "as-is": refusing beats silently dropping a preset
    # (or persisting an empty one) the way a bare warning-less run would. Checked before
    # any raw-mode chatter so the refusal is the whole story.
    if raw and (set_opts or preset or save_preset):
        err_console.print(
            f"[red]{gettext('--raw runs the script as-is; --set, --preset, and --save-preset do not apply.')}[/red]"
        )
        raise typer.Exit(EXIT_USAGE)
    run_spec = spec_for(entry.meta.kind)
    if raw and (run_spec is None or run_spec.analyzer is None):
        # Refused BEFORE runner resolution: a refusal must not first ask which agent
        # to use, write last-picked state, or send the caller through a 126 that a
        # later usage error contradicts. Nothing is changed.
        err_console.print(
            "[red]"
            + gettext(
                "--raw only applies to kinds skit injects into; a %(kind)s entry's "
                "parameters are its interface and always apply."
            )
            % {"kind": entry.meta.kind}
            + _RED_CLOSE
        )
        raise typer.Exit(EXIT_USAGE)
    # One interaction paradigm per run, not two glued in sequence: when the inline
    # mini-form is about to open for a prompt entry anyway, the runner question moves
    # INTO the form (the same picker row the TUI workbench shows) instead of a bare
    # line prompt followed by a Textual form. --plain/TERM=dumb/--runner/pipes keep
    # the deterministic line-mode resolution.
    form_style = "plain" if plain or os.environ.get("TERM") == "dumb" else config.load_form()
    run_interactive = not no_input and _is_interactive()
    runner_names = (
        [r.name for r in config.load_prompt_runners()] if entry.meta.kind == "prompt" else []
    )
    form_hosts_runner = (
        bool(runner_names) and runner is None and run_interactive and form_style == "tui"
    )
    runner_obj = None if form_hosts_runner else _resolve_run_runner(entry, runner, no_input)
    if raw and run_spec is not None and run_spec.analyzer is not None:
        console.print(
            f"[dim]{gettext('Raw mode: skipping the parameter form and injection.')}[/dim]"
        )
        plan = flows.FormPlan(source="none")
    else:
        plan = flows.plan_for_entry(entry)
    _print_drift(plan)
    if plan.degraded_reason:
        console.print(
            f"[dim]{gettext("skit could not model this script's own arguments; pass them after -- instead.")}[/dim]"
        )
    prefilled = flows.prefill(plan, entry.slug, preset)
    overrides = _parse_set_opts(plan, set_opts or [])
    prefilled.update(overrides)
    extra = list(args or [])
    # Both ends must be a terminal: `skit run x | tee log` has a tty stdin but would
    # pump the inline form's ANSI straight into the pipe.
    interactive = run_interactive
    # An explicitly --set field is final — the form only asks for the rest (and a secret
    # set this way is actually used; the prompt renderers never echo a secret prefill).
    remaining = [f for f in plan.fields if f.key not in overrides]
    if interactive and (remaining or form_hosts_runner):
        # form_hosts_runner opens the form even field-less: the picker row is the
        # question (exactly the TUI workbench's rule for prompt entries).
        ask_plan = dataclasses.replace(plan, fields=remaining)
        collected, picked_runner = _collect_values(
            entry,
            ask_plan,
            prefilled,
            plain=plain,
            runners=runner_names if form_hosts_runner else None,
            runner_default=entry.meta.runner or argstate.load_last_runner(),
        )
        values = {**prefilled, **collected}
        if form_hosts_runner:
            if picked_runner is None:
                # The inline renderer degraded to line prompts mid-flight — fall back
                # to the deterministic line-mode resolution.
                runner_obj = _resolve_run_runner(entry, runner, no_input)
            else:
                runner_obj = config.find_prompt_runner(picked_runner)
                if runner_obj is None:  # removed while the form was open — honest
                    raise _fail(
                        gettext(
                            "The runner %(runner)s isn't configured (known: %(names)s). "
                            "Manage runners with: skit runner list"
                        )
                        % {
                            "runner": picked_runner,
                            "names": ", ".join(r.name for r in config.load_prompt_runners()) or "—",
                        },
                        EXIT_NOT_EXECUTABLE,
                    )
                if picked_runner != entry.meta.runner:
                    # A PIN left untouched is not a pick — only a real choice
                    # prefills future pickers (argstate's own contract).
                    argstate.save_last_runner(picked_runner)
    else:
        values = prefilled
        errors = flows.validate(plan, values)
        # Passthrough args are the legitimate manual escape (skit run x -- <args>):
        # when the user supplies them, the script's own parser is in charge and an
        # unfilled required *field* is not a hole to refuse over.
        if errors and not extra:
            for msg in errors.values():
                err_console.print(f"[red]{escape(msg)}[/red]")
            raise typer.Exit(EXIT_SKIT)
    # --raw promises "run the script as-is": replaying a previous run's arguments would
    # betray exactly the clean slate it exists to provide (and the Agent Skill documents).
    # takes_argv: a command template's "arguments" are its placeholders, so replaying a
    # remembered argv tail there would be surprising rather than helpful.
    if not extra and not raw and run_spec is not None and run_spec.takes_argv:
        last_extra = argstate.load_state(entry.slug)["extra_args"]
        if last_extra:
            extra = last_extra
            # stderr, like the drift banner: skit chrome must not pollute the script's
            # own stdout, and agents watch stderr for skit-side signals (SKILL.md).
            err_console.print(
                f"[dim]{gettext('Reusing your last arguments: %(args)s') % {'args': ' '.join(escape(a) for a in extra)}}[/dim]"
            )
    try:
        asm = flows.assemble(plan, values, extra, cwd=Path.cwd(), expand_extra=False)
    except flows.FormError as exc:
        raise _fail(str(exc), EXIT_SKIT) from exc
    if save_preset:
        if not plan.fields:
            # Same rule (and sentence) as `skit preset save` — but NOT its exit code:
            # inside `run`, 1-124 belongs to the script (docker convention), so a
            # skit-side refusal must be usage-shaped, never look like the script ran.
            err_console.print(
                "[red]"
                + gettext("%(name)s has no form fields, so there's nothing to save.")
                % {"name": entry.meta.name}
                + _RED_CLOSE
            )
            raise typer.Exit(EXIT_USAGE)
        argstate.save_preset(
            entry.slug,
            save_preset,
            {k: v for k, v in values.items() if v},
            secret_names=plan.secret_names,
        )
        console.print(
            f"[green]{gettext('Preset "%(preset)s" saved for %(name)s.') % {'preset': escape(save_preset), 'name': escape(entry.meta.name)}}[/green]"
        )
    if dry_run:
        # No temp copy is written for a dry run, so the command line shows the original
        # script path — the shape, not a doomed-to-be-deleted temp file.
        for line in flows.transparency_lines(entry, asm, None, runner=runner_obj):
            console.print(f"[dim]{escape(line)}[/dim]")
        raise typer.Exit(0)
    outcome = flows.execute(
        entry,
        plan,
        asm,
        emit=lambda line: console.print(f"[dim]{escape(line)}[/dim]"),
        runner=runner_obj,
    )
    code = outcome.code
    if code is None:
        raise _fail(outcome.message, _FAILURE_EXIT[outcome.failure])
    if raw:
        # The escape hatch leaves no fingerprints: it consulted no form memory, so it
        # must not rewrite it either (values/extra args survive for the next real run).
        # The run stamp still lands — Library sorting and `r` treat it as a run.
        argstate.record_run(entry.slug, code, at=models.now_iso())
    else:
        flows.save_after_run(entry.slug, plan, values, extra, code, at=models.now_iso())
    if code != 0:
        err_console.print(
            f"[yellow]{gettext('Script exited with code %(code)s') % {'code': code}}[/yellow]"
        )
    raise typer.Exit(code)


# --------------------------------------------------------------------------
# runner (the agent CLIs prompt entries run with — [[prompt.runners]] in config)
# --------------------------------------------------------------------------

runner_app = typer.Typer(
    help=gettext("Manage the agents (runners) that prompt entries run with."),
    no_args_is_help=True,
)
app.add_typer(runner_app, name="runner")


@runner_app.command(
    "list", help=gettext("List the configured runners (seeds them into config on first use).")
)
def runner_list(
    as_json: bool = typer.Option(False, "--json", help=gettext("Output as JSON")),
) -> None:
    # The management surface is where the seeds materialize into the user's config —
    # visible and editable, never a hidden built-in list.
    config.ensure_prompt_runners_seeded()
    runners = config.load_prompt_runners()
    if as_json:
        console.print_json(
            json.dumps(
                [{"name": r.name, "argv": list(r.argv)} for r in runners], ensure_ascii=False
            )
        )
        return
    if not runners:
        console.print(gettext("No runners configured. Add one with: skit runner add NAME COMMAND…"))
        return
    from rich.table import Table

    table = Table(show_header=True, header_style="bold")  # pragma: no mutate — cosmetic
    table.add_column(gettext("Runner"))
    table.add_column(gettext("Command"))
    for r in runners:
        table.add_row(escape(r.name), escape(_join_runner_argv(list(r.argv))))
    console.print(table)


def _join_runner_argv(argv: list[str]) -> str:
    """Display-join a runner argv (the same platform-aware join describe uses)."""
    from .langs.launch import join_for_display

    return join_for_display(argv)


def _runner_reason(code: str) -> str:
    """The human wording for a runner-argv validation code. A closed dict of gettext
    LITERALS resolved at call time (never at import — a module-level dict would freeze
    the import-time locale), the same discipline as flows._split_message."""
    return {
        "empty": gettext(
            "A runner needs a command — e.g. skit runner add mycli mycli run {{prompt}}"
        ),
        "prompt-slot-count": gettext(
            "A runner command must contain the {{prompt}} slot exactly once — that's where "
            "the rendered prompt lands."
        ),
        "prompt-in-binary": gettext(
            "{{prompt}} can't be the command itself — the first word must be the program to run."
        ),
        "stray-hole": gettext(
            "Runner commands take only the {{prompt}} slot — single-brace text is literal, "
            "and other {{holes}} aren't supported."
        ),
    }[code]


@runner_app.command(
    "add",
    help=gettext(
        "Register a runner: skit runner add NAME COMMAND… ({{prompt}} marks where the "
        "rendered prompt goes; each shell word becomes one argument, no shell involved)"
    ),
    # Real runner argv contains flags (`--model sonnet`); without this Click would
    # reject them as unknown options instead of collecting them. `--` still works as an
    # explicit guard and is what SKILL.md teaches.
    context_settings={"ignore_unknown_options": True},
)
def runner_add(
    name: str = typer.Argument(..., help=gettext("Runner name (e.g. claude)")),
    argv: list[str] = typer.Argument(
        None, help=gettext("The command, word by word, with one {{prompt}} slot")
    ),
    force: bool = typer.Option(
        False,
        "--force",
        help=gettext("Replace the runner if the name already exists (the edit path)"),
    ),
) -> None:
    config.ensure_prompt_runners_seeded()
    reason = config.validate_prompt_runner_argv(list(argv or []))
    if reason is not None:
        err_console.print(f"[red]{escape(_runner_reason(reason))}[/red]")
        raise typer.Exit(EXIT_USAGE)
    runners = config.load_prompt_runners()
    exists = any(r.name == name for r in runners)
    if exists and not force:
        raise _fail(
            gettext("The runner %(name)s already exists — pass --force to replace its command.")
            % {"name": name},
            1,
        )
    new_row = config.PromptRunner(name, tuple(argv))
    if exists:
        # Replace in place: an edit keeps the row's position in every picker.
        config.save_prompt_runners([new_row if r.name == name else r for r in runners])
    else:
        config.save_prompt_runners([*runners, new_row])
    if exists:
        console.print(
            f"[green]{gettext('Runner %(name)s updated: %(command)s') % {'name': escape(name), 'command': escape(_join_runner_argv(list(argv)))}}[/green]"
        )
    else:
        console.print(
            f"[green]{gettext('Runner %(name)s added: %(command)s') % {'name': escape(name), 'command': escape(_join_runner_argv(list(argv)))}}[/green]"
        )


@runner_app.command("remove", help=gettext("Remove a configured runner."))
def runner_remove(
    name: str = typer.Argument(..., help=gettext("Runner name"), autocompletion=_complete_runner),
    yes: bool = typer.Option(False, "--yes", "-y", help=gettext("Skip confirmation")),
) -> None:
    config.ensure_prompt_runners_seeded()
    runners = config.load_prompt_runners()
    if not any(r.name == name for r in runners):
        raise _fail(
            gettext("Unknown runner: %(runner)s. Configured runners: %(names)s")
            % {"runner": name, "names": ", ".join(r.name for r in runners) or "—"},
            1,
        )
    if not yes:
        # The same ask its two siblings give: skit remove confirms unless -y, and the
        # TUI's agent removal confirms — deleting config rows is not a one-keystroke act.
        typer.confirm(gettext('Remove the agent "%(name)s"?') % {"name": name}, abort=True)
    config.save_prompt_runners([r for r in runners if r.name != name])
    console.print(f"[green]{gettext('Runner %(name)s removed.') % {'name': escape(name)}}[/green]")


# --------------------------------------------------------------------------
# preset
# --------------------------------------------------------------------------

preset_app = typer.Typer(
    help=gettext("Manage named parameter presets for a script."), no_args_is_help=True
)
app.add_typer(preset_app, name="preset")


@preset_app.command("save", help=gettext("Save a set of parameter values as a named preset."))
def preset_save(
    name: str = _SCRIPT_ARG,
    preset_name: str = typer.Argument(..., help=gettext("Preset name")),
    from_last: bool = typer.Option(
        False,
        "--from-last",
        help=gettext("Save the last run's values without asking (automation-friendly)"),
    ),
) -> None:
    """Save a named preset: interactively, or straight from the last run with --from-last.
    Secret values are never persisted (C3)."""
    try:
        entry = store.resolve(name)
    except store.NotFoundError as exc:
        raise _fail(str(exc), 1) from exc
    plan = flows.plan_for_entry(entry)
    if not plan.fields:
        raise _fail(
            gettext("%(name)s has no form fields, so there's nothing to save.")
            % {"name": entry.meta.name},
            1,
        )
    if from_last:
        last = argstate.load_state(entry.slug)["values"]
        keys = {f.key for f in plan.fields}
        values = {k: v for k, v in last.items() if k in keys}
        if not values:
            raise _fail(
                gettext("%(name)s has no remembered values yet — run it once first.")
                % {"name": entry.meta.name},
                1,
            )
    elif sys.stdin.isatty():
        values = promptform.collect(plan, flows.prefill(plan, entry.slug), console=console)
    else:
        # Non-interactive contract: don't prompt — save what the prefill already knows.
        values = flows.prefill(plan, entry.slug)
    secret_overlap = plan.secret_names & values.keys()
    if secret_overlap:
        console.print(
            "[dim]"
            + gettext("Secret values are never stored in presets; skipped: %(names)s")
            % {"names": ", ".join(escape(n) for n in sorted(secret_overlap))}
            + _DIM_CLOSE
        )
    argstate.save_preset(entry.slug, preset_name, values, secret_names=plan.secret_names)
    console.print(
        f"[green]{gettext('Preset "%(preset)s" saved for %(name)s.') % {'preset': escape(preset_name), 'name': escape(entry.meta.name)}}[/green]"
    )


@preset_app.command("list", help=gettext("List a script's saved presets."))
def preset_list(
    name: str = _SCRIPT_ARG,
    as_json: bool = typer.Option(False, "--json", help=gettext("Output as JSON")),
) -> None:
    """List a script's named presets."""
    try:
        entry = store.resolve(name)
    except store.NotFoundError as exc:
        raise _fail(str(exc), 1) from exc
    presets = argstate.load_state(entry.slug)["presets"]
    if as_json:
        console.print_json(json.dumps(presets, ensure_ascii=False))
        return
    if not presets:
        console.print(
            gettext(
                "No presets for %(name)s yet. Create one with: skit run %(name)s --save-preset <preset>"
            )
            % {"name": escape(entry.meta.name)}
        )
        return
    for pname, vals in sorted(presets.items()):
        pairs = ", ".join(f"{escape(k)}={escape(v)}" for k, v in vals.items())
        console.print(f"  [bold]{escape(pname)}[/bold]: {pairs}")


@preset_app.command("delete", help=gettext("Delete a named preset from a script."))
def preset_delete(
    name: str = _SCRIPT_ARG,
    preset_name: str = typer.Argument(..., help=gettext("Preset name")),
) -> None:
    """Delete a named preset."""
    try:
        entry = store.resolve(name)
    except store.NotFoundError as exc:
        raise _fail(str(exc), 1) from exc
    if argstate.delete_preset(entry.slug, preset_name):
        console.print(
            gettext('Preset "%(preset)s" deleted from %(name)s.')
            % {"preset": escape(preset_name), "name": escape(entry.meta.name)}
        )
    else:
        err_console.print(
            "[red]"
            + gettext('Unknown preset "%(preset)s". Available: %(presets)s')
            % {
                "preset": escape(preset_name),
                "presets": ", ".join(
                    escape(p) for p in sorted(argstate.load_state(entry.slug)["presets"])
                )
                or "—",
            }
            + _RED_CLOSE
        )
        raise typer.Exit(1)


# --------------------------------------------------------------------------
# params
# --------------------------------------------------------------------------


def _secret_cell(s: ParamDecl) -> str:
    """The Secret column: "—", "yes", or "yes ← $ENVVAR" when an env source is set."""
    if not s.secret:
        return "—"
    if s.env_source:
        return gettext("yes") + f" ← ${escape(s.env_source)}"
    return gettext("yes")


def _declared_last_value(name: str, secret: bool, last: dict[str, str]) -> str:
    """The Last-value cell for a declared row (a stored secret is masked, never echoed)."""
    if secret:
        return gettext("•••") if name in last else "—"
    return last.get(name, "—")


def _declared_default_cell(d: ParamDecl) -> str:
    if d.default is None:
        return "—"
    if d.secret:
        return gettext("•••")
    return str(d.default)


def _declared_schema_suffix(d: ParamDecl | None) -> str:
    """A dim inline schema marker for a command placeholder / env rider (type · default · flags)."""
    if d is None:
        return ""  # an undeclared placeholder keeps the historical bare `name = value` line
    parts = [escape(d.type)]
    if d.default is not None:
        shown = gettext("•••") if d.secret else str(d.default)
        parts.append(escape(gettext("default %(value)s") % {"value": shown}))
    if not d.required:
        parts.append(gettext("optional"))
    if d.secret:
        parts.append(gettext("secret"))
    return f"  [dim]{' · '.join(parts)}[/dim]"


def _print_declared_table(decls: list[ParamDecl], last: dict[str, str]) -> None:
    """The read table for an exe entry's declared parameters (flag/env delivery)."""
    from rich.table import Table

    table = Table(show_header=True, header_style="bold")  # pragma: no mutate — cosmetic
    table.add_column(gettext("Parameter"))
    table.add_column(gettext("Delivery"))
    table.add_column(gettext("Type"))
    table.add_column(gettext("Default"))
    table.add_column(gettext("Secret"))
    table.add_column(gettext("Last value"))
    for d in decls:
        table.add_row(
            escape(d.name),
            escape(d.delivery),
            escape(d.type),
            escape(_declared_default_cell(d)),
            _secret_cell(d),
            escape(_declared_last_value(d.name, d.secret, last)),
        )
    console.print(table)


def _prompt_body_placeholders(entry: store.Entry) -> list[str]:
    """A prompt entry's placeholders, scanned fresh from the body ("" list when the body
    is unreadable — preflight owns existence errors)."""
    try:
        text = entry.script_path.read_text(encoding="utf-8", errors="replace")
    except OSError:
        return []
    from .langs.prompt import analyzer as prompt_analyzer

    return prompt_analyzer.placeholder_names(text)


def _show_command_params(
    entry: store.Entry, declared: list[ParamDecl], last: dict[str, str]
) -> None:
    """The read view for a placeholder kind (command template / prompt): its managed
    placeholders (enriched with any declared schema) plus declared environment riders.
    A prompt additionally reads its body fresh: candidates the user hasn't managed and
    managed names the body no longer contains both surface HERE — where the user decides
    what to do about them — not only at run time."""
    placeholders = entry.meta.params or []
    by_name = {d.name: d for d in declared}
    env_riders = [d for d in declared if d.delivery == "env" and d.name not in placeholders]
    is_prompt = entry.meta.kind == "prompt"
    if is_prompt and not entry.meta.interpolate:
        console.print(
            gettext(
                "Variable insertion is off — the body travels to the agent exactly as "
                "written. Turn it on with: skit params %(name)s --interpolate"
            )
            % {"name": entry.meta.name}
        )
        return
    fresh = _prompt_body_placeholders(entry) if is_prompt else []
    unmanaged = [n for n in fresh if n not in placeholders]
    gone = [n for n in placeholders if n not in fresh] if is_prompt else []
    if not placeholders and not env_riders and not unmanaged:
        console.print(
            escape(gettext("%(name)s has no managed parameters.") % {"name": entry.meta.name})
        )
        return
    if placeholders:
        console.print(
            gettext("Prompt placeholders (the run form asks for them):")
            if is_prompt
            else gettext("Command template placeholders (the run form asks for them):")
        )
        for p in placeholders:
            d = by_name.get(p)
            shown = _declared_last_value(p, d.secret if d is not None else False, last)
            console.print(f"  {escape(p)} = {escape(shown)}{_declared_schema_suffix(d)}")
    if env_riders:
        console.print(gettext("Declared environment variables (set on the run):"))
        for d in env_riders:
            shown = _declared_last_value(d.name, d.secret, last)
            console.print(f"  {escape(d.name)} = {escape(shown)}{_declared_schema_suffix(d)}")
    if unmanaged:
        from .langs.prompt import analyzer as prompt_analyzer

        console.print(
            gettext("Detected but not yet managed: %(names)s (use --add to manage them)")
            % {"names": escape(prompt_analyzer.preview_names(unmanaged))}
        )
    if gone:
        console.print(
            f"[yellow]{gettext('No longer in the prompt (the value would be ignored): %(names)s — remove with --rm, or edit the body.') % {'names': ', '.join(escape(n) for n in gone)}}[/yellow]"
        )


def _show_params(entry: store.Entry, as_json: bool) -> None:
    """Read view: managed parameters + last values + detected-but-unmanaged candidates."""
    last = argstate.load_state(entry.slug)["values"]
    entry_spec = spec_for(entry.meta.kind)
    specs: list[ParamDecl] = []
    text = ""
    if entry_spec is not None and entry_spec.params_io is not None and entry.script_path.exists():
        text = entry.script_path.read_text(encoding="utf-8", errors="replace")  # pragma: no mutate
        specs = entry_spec.params_io.read(text)
    unmanaged: list[str] = []
    self_locating = False
    if (
        entry_spec is not None
        and entry_spec.analyzer is not None
        and entry.meta.mode == "copy"
        and text
    ):
        report = entry_spec.analyzer.reconcile(text, specs)
        unmanaged = [c.name for c in report.new]
        # $0/BASH_SOURCE: an injected constant runs from a temp copy, so the script would see the
        # temp path instead of its own. Say so HERE — where the user decides whether to manage it —
        # not only in the run-time warning, and point at the fix. Only meaningful for a kind that
        # actually rewrites a copy (an injector); env delivery never moves the file.
        self_locating = (
            entry_spec.injector is not None and entry_spec.analyzer.analyze(text).uses_self_location
        )
    if entry_spec is not None and entry_spec.kind == "prompt" and entry.meta.interpolate:
        # The prompt's "detected but unmanaged" sweep is a fresh body scan, not the
        # analyzer/reconcile machinery (command-kind parity: spec.analyzer is None).
        managed_names = set(entry.meta.params or [])
        unmanaged = [n for n in _prompt_body_placeholders(entry) if n not in managed_names]
    # Declared [[parameters]] rows (empty for a python entry — it manages its schema in-file).
    declared = declared_from_meta(entry.meta.parameters)
    if as_json:
        payload = {
            "params": [s.to_block_dict() for s in specs],
            "unmanaged": unmanaged,
            "placeholders": entry.meta.params or [],
            "declared": [d.to_meta_dict() for d in declared],
        }
        if entry.meta.kind == "prompt":
            payload["runner"] = entry.meta.runner or None
            payload["interpolate"] = entry.meta.interpolate
        console.print_json(json.dumps(payload, ensure_ascii=False))
        return
    if entry_spec is not None and entry_spec.placeholder_params:
        # The trait, not the family: a prompt (family "interpreted") reads exactly like
        # a command template — placeholders are the interface.
        _show_command_params(entry, declared, last)
        return
    if declared and entry_spec is not None and entry_spec.params_io is None:
        # Every meta-schema kind, not just binaries: an interpreted kind whose schema lives in
        # meta.toml (ruby/perl/lua/r/powershell) can hold declared flag/env rows that really do
        # deliver, so denying they exist here (while --json listed them) was a read/write split.
        # Templates already returned above.
        _print_declared_table(declared, last)
        return
    if not specs:
        if entry_spec is None or entry_spec.analyzer is None:
            # No analyzer means --manage can't do anything for this kind — suggesting it
            # would send the user down a dead end (`skit params <exe> --manage X` errors).
            console.print(
                escape(gettext("%(name)s has no managed parameters.") % {"name": entry.meta.name})
            )
            return
        console.print(
            escape(
                gettext(
                    "%(name)s has no managed parameters. Use --manage to bring a detected candidate under management."
                )
                % {"name": entry.meta.name}
            )
        )
    else:
        from rich.table import Table

        table = Table(show_header=True, header_style="bold")  # pragma: no mutate — cosmetic
        table.add_column(gettext("Parameter"))
        table.add_column(gettext("Kind"))
        table.add_column(gettext("Type"))
        table.add_column(gettext("Default"))
        table.add_column(gettext("Secret"))
        table.add_column(gettext("Last value"))
        for s in specs:
            if s.secret:
                last_shown = gettext("•••") if s.name in last else "—"
            else:
                last_shown = last.get(s.name, "—")
            default_shown = (
                gettext("•••")
                if s.secret and s.default is not None
                else ("—" if s.default is None else str(s.default))
            )
            table.add_row(
                escape(s.name),
                escape(s.binding),
                escape(s.type),
                escape(default_shown),
                _secret_cell(s),
                escape(str(last_shown)),
            )
        console.print(table)
    if unmanaged:
        console.print(
            gettext("Detected but not yet managed: %(names)s (use --manage to manage them)")
            % {"names": ", ".join(escape(n) for n in unmanaged)}
        )
    if self_locating:
        console.print(
            f"[dim]{gettext('This script locates itself ($0 / BASH_SOURCE). Injecting a constant runs it from a temporary copy, so it would see that copy path instead. `skit params %(name)s --normalize NAME` converts a constant into the ${NAME:-default} idiom, which is delivered through the environment and leaves the file untouched.') % {'name': escape(entry.meta.name)}}[/dim]"
        )


def _parse_kv_opts(raw: list[str], flag: str) -> tuple[dict[str, str], list[str]]:
    """Parse NAME=value pairs; malformed entries are collected for a warning."""
    pairs: dict[str, str] = {}
    bad: list[str] = []
    for item in raw:
        if "=" in item:
            key, _, value = item.partition("=")
            if key.strip():
                pairs[key.strip()] = value
                continue
        bad.append(f"{flag}: {item}")
    return pairs, bad


@app.command(
    help=gettext("Show or edit a script's managed or declared parameters."),
    epilog=gettext(
        "Examples:  skit params resize --manage WIDTH  ·  skit params conv --add size --type size=int --deliver size=flag --flag size=--size"
    ),
)
def params(
    name: str = _SCRIPT_ARG,
    resync: bool = typer.Option(
        False,
        "--resync",
        help=gettext("Prune definitions that no longer match the script and refresh changed types"),
    ),
    manage: list[str] = typer.Option(
        None,
        "--manage",
        help=gettext("Bring a currently detected candidate under management (repeatable)"),
    ),
    unmanage: list[str] = typer.Option(
        None, "--unmanage", help=gettext("Drop a managed parameter (repeatable)")
    ),
    add: list[str] = typer.Option(
        None,
        "--add",
        help=gettext("Declare a new parameter on an exe/command entry, by name (repeatable)"),
    ),
    rm: list[str] = typer.Option(
        None, "--rm", help=gettext("Remove a declared parameter, by name (repeatable)")
    ),
    type_opt: list[str] = typer.Option(
        None,
        "--type",
        help=gettext("Set a declared parameter's type, as NAME=str|int|float|bool|choice"),
    ),
    default_opt: list[str] = typer.Option(
        None, "--default", help=gettext("Set a declared parameter's default, as NAME=VALUE")
    ),
    choices_opt: list[str] = typer.Option(
        None,
        "--choices",
        help=gettext("Set a declared parameter's choices, as NAME=a,b,c (comma separated)"),
    ),
    deliver_opt: list[str] = typer.Option(
        None,
        "--deliver",
        help=gettext(
            "Set how a declared parameter reaches the program, as NAME=env|flag|placeholder"
        ),
    ),
    flag_opt: list[str] = typer.Option(
        None,
        "--flag",
        help=gettext("Set a declared flag parameter's option, as NAME=--out (empty = positional)"),
    ),
    required_opt: list[str] = typer.Option(
        None, "--required", help=gettext("Mark a declared parameter as required (repeatable)")
    ),
    optional_opt: list[str] = typer.Option(
        None, "--optional", help=gettext("Mark a declared parameter as optional (repeatable)")
    ),
    help_text_opt: list[str] = typer.Option(
        None, "--help-text", help=gettext("Set a declared parameter's help text, as NAME=text")
    ),
    secret: list[str] = typer.Option(
        None, "--secret", help=gettext("Mark a managed parameter as secret (repeatable)")
    ),
    no_secret: list[str] = typer.Option(
        None,
        "--no-secret",
        help=gettext("Remove the secret mark from a managed parameter (repeatable)"),
    ),
    prompt: list[str] = typer.Option(
        None, "--prompt", help=gettext("Set a parameter's form prompt, as NAME=text (repeatable)")
    ),
    env_source: list[str] = typer.Option(
        None,
        "--env-source",
        help=gettext(
            "Read a secret parameter from an environment variable at run time, as NAME=ENVVAR (empty ENVVAR clears it; repeatable)"
        ),
    ),
    normalize_opt: list[str] = typer.Option(
        None,
        "--normalize",
        help=gettext(
            "Shell only: rewrite a constant into the ${NAME:-default} idiom in the stored copy, so its value is delivered as an environment variable instead of a rewritten temporary copy (repeatable)"
        ),
    ),
    runner_pin: str = typer.Option(
        None,
        "--runner",
        help=gettext(
            "Prompt only: pin the agent this prompt runs with (empty value clears the pin)"
        ),
        autocompletion=_complete_runner,
    ),
    interpolate_opt: bool = typer.Option(
        None,
        "--interpolate/--no-interpolate",
        help=gettext(
            "Prompt only: turn variable insertion on/off for this prompt (off = the body "
            "travels exactly as written)"
        ),
    ),
    workdir_opt: str = typer.Option(
        None,
        "--workdir",
        help=gettext(
            "Set where the entry runs: origin (its own folder), store, invoke (where you "
            "run skit from), or an absolute path"
        ),
    ),
    template_opt: str = typer.Option(
        None,
        "--template",
        help=gettext("Command only: rewrite the template ({placeholders} are re-read from it)"),
    ),
    interpreter_opt: str = typer.Option(
        None,
        "--interpreter",
        help=gettext(
            "Pin the interpreter/runtime an interpreted entry runs with (e.g. zsh, bun; "
            "empty value returns to automatic)"
        ),
    ),
    as_json: bool = typer.Option(False, "--json", help=gettext("Output the read view as JSON")),
) -> None:
    """Show a script's parameters, or edit their definitions when any change flag is given.
    Python entries manage constants/inputs from the script itself (--manage/--unmanage);
    exe and command entries carry a declared schema in meta.toml (--add/--rm/--type/…).
    Definitions travel with the entry; values live in central state; secret values are
    never shown or persisted."""
    try:
        entry = store.resolve(name)
    except store.NotFoundError as exc:
        raise _fail(str(exc), 1) from exc
    prompts, bad_prompts = _parse_kv_opts(prompt or [], "--prompt")
    env_sources, bad_env = _parse_kv_opts(env_source or [], "--env-source")
    types, bad_type = _parse_kv_opts(type_opt or [], "--type")
    defaults, bad_default = _parse_kv_opts(default_opt or [], "--default")
    choices_raw, bad_choices = _parse_kv_opts(choices_opt or [], "--choices")
    deliveries, bad_deliver = _parse_kv_opts(deliver_opt or [], "--deliver")
    flags, bad_flag = _parse_kv_opts(flag_opt or [], "--flag")
    help_texts, bad_help = _parse_kv_opts(help_text_opt or [], "--help-text")

    entry_spec = spec_for(entry.meta.kind)
    # One operation per invocation, ENFORCED — not silently first-wins. The policy ops
    # below each used to early-return, dropping any schema flags riding along with a
    # green exit 0 (the exact refuse-never-drop sin this command's own siblings guard
    # against). --workdir/--interpreter/--template stay one combinable launch-policy
    # group; everything else must come alone.
    schema_ops = bool(
        resync
        or manage
        or unmanage
        or add
        or rm
        or secret
        or no_secret
        or required_opt
        or optional_opt
        or prompt
        or env_source
        or type_opt
        or default_opt
        or choices_opt
        or deliver_opt
        or flag_opt
        or help_text_opt
    )
    own_ops = [
        name
        for name, present in (
            ("--interpolate/--no-interpolate", interpolate_opt is not None),
            ("--runner", runner_pin is not None),
            (
                "--workdir/--interpreter/--template",
                workdir_opt is not None or interpreter_opt is not None or template_opt is not None,
            ),
            ("--normalize", bool(normalize_opt)),
        )
        if present
    ]
    if own_ops and (schema_ops or len(own_ops) > 1):
        err_console.print(
            "[red]"
            + gettext(
                "%(op)s is its own operation — run it in a separate skit params call "
                "(nothing was changed)."
            )
            % {"op": own_ops[0]}
            + _RED_CLOSE
        )
        raise typer.Exit(EXIT_USAGE)
    if interpolate_opt is not None:
        # Its own op, like --runner: flipping the master switch changes what the entry
        # IS at run time, so it never mixes into the schema-edit pass.
        _set_prompt_interpolate(entry, interpolate_opt)
        return
    if runner_pin is not None:
        # Its own op, like --normalize: re-pinning changes how the entry LAUNCHES, so
        # mixing it into the schema-edit pass would make the outcome order-dependent.
        _pin_prompt_runner(entry, runner_pin)
        return
    if workdir_opt is not None or interpreter_opt is not None or template_opt is not None:
        # Entry policy, not schema — its own op for the same order-independence reason.
        _edit_entry_policy(entry, workdir_opt, interpreter_opt, template_opt)
        return
    if normalize_opt:
        # A source-idiom rewrite of the user's own stored file — deliberately its own op, not a
        # modifier on the others: it changes what a parameter IS (inject-delivered -> env-delivered),
        # so mixing it into the same pass as --manage/--secret would make the outcome order-dependent.
        _normalize_params(entry, entry_spec, normalize_opt)
        return
    has_params_io = entry_spec is not None and entry_spec.params_io is not None
    declared_kind = entry_spec is not None and entry_spec.params_io is None
    declared_ops = bool(
        add
        or rm
        or types
        or defaults
        or choices_raw
        or deliveries
        or flags
        or required_opt
        or optional_opt
        or help_texts
        or bad_type
        or bad_default
        or bad_choices
        or bad_deliver
        or bad_flag
        or bad_help
    )
    shared_tweaks = bool(secret or no_secret or prompts or env_sources or bad_prompts or bad_env)
    analyzer_ops = bool(resync or manage or unmanage)

    # Python (or any in-file-managed kind): the declared-schema flags belong to exe/command.
    if has_params_io and declared_ops:
        raise _fail(
            gettext(
                "%(name)s manages its parameters from the script itself — use --manage / "
                "--unmanage, or edit the [tool.skit] block."
            )
            % {"name": entry.meta.name},
            1,
        )
    if (
        entry_spec is not None
        and declared_kind
        and (declared_ops or shared_tweaks)
        and not analyzer_ops
    ):
        _edit_declared_params(
            entry,
            entry_spec,
            add=add or [],
            rm=rm or [],
            types=types,
            defaults=defaults,
            choices={n: v.split(",") for n, v in choices_raw.items()},
            deliveries=deliveries,
            flags=flags,
            required=required_opt or [],
            optional=optional_opt or [],
            help_texts=help_texts,
            secret=secret or [],
            no_secret=no_secret or [],
            prompts=prompts,
            env_sources=env_sources,
            malformed=(
                bad_type
                + bad_default
                + bad_choices
                + bad_deliver
                + bad_flag
                + bad_help
                + bad_prompts
                + bad_env
            ),
        )
        if as_json:
            # --json on a write: emit the final state as the machine contract (the
            # deps write path's precedent) — an explicit flag never silently no-ops.
            _show_params(store.resolve(entry.slug), as_json=True)
        return
    # Python edits, and the analyzer-op-on-a-non-python refusal, both go through _edit_params.
    if analyzer_ops or (has_params_io and shared_tweaks):
        _edit_params(
            entry,
            resync=resync,
            manage=manage or [],
            unmanage=unmanage or [],
            secret=secret or [],
            no_secret=no_secret or [],
            prompts=prompts,
            env_sources=env_sources,
            malformed=bad_prompts + bad_env,
        )
        if as_json:
            # Same machine contract as the declared branch above.
            _show_params(store.resolve(entry.slug), as_json=True)
        return
    _show_params(entry, as_json)


def _edit_entry_policy(
    entry: store.Entry,
    workdir_opt: str | None,
    interpreter_opt: str | None,
    template_opt: str | None,
) -> None:
    """params --workdir / --interpreter / --template: the entry policies the product
    previously exposed nowhere outside a hand-edited meta.toml (or not at all — a
    command's template was frozen forever at add time)."""
    try:
        if template_opt is not None:
            entry = store.update_template(entry.slug, template_opt)
            console.print(
                f"[green]{gettext('Template updated. Placeholders: %(names)s') % {'names': ', '.join(escape(p) for p in entry.meta.params or []) or '—'}}[/green]"
            )
        if workdir_opt is not None:
            entry = store.write_workdir(entry.slug, workdir_opt)
            console.print(
                f"[green]{gettext('%(name)s now runs in: %(dir)s') % {'name': escape(entry.meta.name), 'dir': escape(entry.meta.workdir)}}[/green]"
            )
        if interpreter_opt is not None:
            entry = store.write_interpreter(entry.slug, interpreter_opt)
            if entry.meta.interpreter:
                console.print(
                    f"[green]{gettext('%(name)s now runs with: %(program)s') % {'name': escape(entry.meta.name), 'program': escape(entry.meta.interpreter)}}[/green]"
                )
            else:
                console.print(
                    f"[green]{gettext('%(name)s is back to automatic interpreter detection.') % {'name': escape(entry.meta.name)}}[/green]"
                )
    except store.StoreError as exc:
        raise _fail(str(exc), 1) from exc


def _pin_prompt_runner(entry: store.Entry, runner_pin: str) -> None:
    """`skit params <prompt> --runner NAME`: pin (or, with an empty value, clear) the
    agent the entry runs with. Pinning an unknown name would store a run that can only
    exit 126 — validated against the configured list instead."""
    if entry.meta.kind != "prompt":
        raise _fail(gettext("--runner only applies to prompt entries."), 1)
    name = runner_pin.strip()
    if name:
        names = [r.name for r in config.load_prompt_runners()]
        if name not in names:
            raise _fail(
                gettext(
                    "The runner %(runner)s isn't configured (known: %(names)s). Manage "
                    "runners with: skit runner list"
                )
                % {"runner": name, "names": ", ".join(names) or "—"},
                1,
            )
        argstate.save_last_runner(name)
    try:
        store.write_prompt_runner(entry.slug, name)
    except store.StoreError as exc:
        raise _fail(str(exc), 1) from exc
    if name:
        console.print(
            f"[green]{gettext('%(name)s now runs with %(runner)s.') % {'name': escape(entry.meta.name), 'runner': escape(name)}}[/green]"
        )
    else:
        console.print(
            f"[green]{gettext('Cleared the runner pin — %(name)s asks at run time.') % {'name': escape(entry.meta.name)}}[/green]"
        )


def _set_prompt_interpolate(entry: store.Entry, interpolate: bool) -> None:
    """`skit params <prompt> --interpolate/--no-interpolate`: the per-entry insertion
    master switch. The managed list survives an off/on round trip."""
    if entry.meta.kind != "prompt":
        raise _fail(gettext("--interpolate only applies to prompt entries."), 1)
    try:
        store.write_prompt_interpolate(entry.slug, interpolate)
    except store.StoreError as exc:
        raise _fail(str(exc), 1) from exc
    if interpolate:
        console.print(
            f"[green]{gettext('Variable insertion is on — %(name)s fills its managed placeholders again.') % {'name': escape(entry.meta.name)}}[/green]"
        )
    else:
        console.print(
            f"[green]{gettext('Variable insertion is off — %(name)s travels to the agent exactly as written.') % {'name': escape(entry.meta.name)}}[/green]"
        )


def _apply_env_sources(specs: list[ParamDecl], env_sources: dict[str, str]) -> list[str]:
    """Set/clear env_source on secret specs; returns warnings for unusable requests."""
    warnings: list[str] = []
    by_name = {s.name: s for s in specs}
    for pname, envvar in env_sources.items():
        spec = by_name.get(pname)
        if spec is None:
            warnings.append(
                gettext("%(name)s isn't a managed parameter; --env-source skipped.")
                % {"name": pname}
            )
            continue
        if not spec.secret:
            warnings.append(
                gettext(
                    "%(name)s isn't secret; --env-source only applies to secret parameters (mark it with --secret first)."
                )
                % {"name": pname}
            )
            continue
        spec.env_source = envvar.strip()
    return warnings


def _edit_params(
    entry: store.Entry,
    *,
    resync: bool,
    manage: list[str],
    unmanage: list[str],
    secret: list[str],
    no_secret: list[str],
    prompts: dict[str, str],
    env_sources: dict[str, str],
    malformed: list[str],
) -> None:
    """Apply parameter-definition changes to a copy-mode Python entry (rewrites [tool.skit])."""
    entry_spec = spec_for(entry.meta.kind)
    if entry_spec is None or entry_spec.params_io is None or entry_spec.analyzer is None:
        raise _fail(
            gettext(
                "%(name)s has no managed parameters — its kind has no analyzer to read them from."
            )
            % {"name": entry.meta.name},
            1,
        )
    if entry.meta.mode == "reference":
        raise _fail(
            gettext(
                "%(name)s is in reference mode, and skit never writes the original file. "
                "Edit the [tool.skit] block in the source directly."
            )
            % {"name": entry.meta.name},
            1,
        )
    copy_path = entry.script_path
    if not copy_path.exists():
        raise _fail(gettext("%(name)s has no stored copy to edit.") % {"name": entry.meta.name}, 1)
    text = copy_path.read_text(encoding="utf-8", errors="replace")  # pragma: no mutate
    current = entry_spec.params_io.read(text)
    for item in malformed:
        err_console.print(
            f"[yellow]{escape(gettext('Ignored a malformed value: %(item)s (expected NAME=text).') % {'item': item})}[/yellow]"
        )
    result = analysis.edit_specs(
        text,
        current,
        resync=resync,
        add=manage,
        remove=unmanage,
        secret=secret,
        no_secret=no_secret,
        prompts=prompts,
        analyze=entry_spec.analyzer.analyze,
    )
    for w in result.warnings:
        err_console.print(f"[yellow]{escape(analysis.render_warning(w))}[/yellow]")
    for w in _apply_env_sources(result.specs, env_sources):
        err_console.print(f"[yellow]{escape(w)}[/yellow]")
    new_text = entry_spec.params_io.write(text, result.specs)
    copy_path.write_text(new_text, encoding="utf-8")  # pragma: no mutate — utf-8 equivalence
    secret_now = {s.name for s in result.specs if s.secret}
    purged = argstate.purge_secret(entry.slug, secret_now)
    if purged:
        console.print(
            "[dim]"
            + gettext(
                "Removed previously stored plaintext value(s) for now-secret parameter(s): %(names)s"
            )
            % {"names": ", ".join(escape(n) for n in sorted(purged))}
            + _DIM_CLOSE
        )
    remaining = ", ".join(escape(s.name) for s in result.specs) or "—"
    console.print(
        f"[green]{gettext('Updated %(name)s. Managed parameters: %(names)s') % {'name': escape(entry.meta.name), 'names': remaining}}[/green]"
    )


def _render_normalize_warning(warning: str) -> str:
    """Translate a normalizer refusal ("code:name") into a user-facing line. Static lookup (not
    gettext(f"…{code}")) so Babel can extract every string — same discipline as the other two
    warning renderers."""
    code, _, name = warning.partition(":")
    return {
        "not-a-const": gettext(
            "%(name)s isn't a plain constant with a literal value, so there's nothing to normalize; skipped."
        ),
        "multiple-assignments": gettext(
            "%(name)s is assigned more than once at the top level; normalizing it would change which value wins. Skipped."
        ),
        "readonly": gettext(
            "%(name)s is readonly, so the script could never take a value from the environment; skipped."
        ),
        "already-env": gettext("%(name)s already reads from the environment; nothing to do."),
        "unsafe-literal": gettext(
            "%(name)s's value contains a character that can't be moved into ${...:-...} safely "
            '(one of } " ` $ \\ or a newline); skipped — it keeps being injected into a temporary copy.'
        ),
        "syntax-error": gettext(
            "Could not parse the script (syntax error); nothing was normalized."
        ),
    }[code] % {"name": name}


def _reanchor_as_envdefault(spec: ParamDecl, cand: analysis.Candidate) -> ParamDecl:
    """The normalized const's definition, re-anchored onto its new ${NAME:-default} expansion:
    binding/delivery/type/default come from the source (the analyzer just re-read it), while the
    user's own decisions — secret, its env source, a custom prompt — survive the rewrite."""
    decl = ParamDecl.from_candidate(cand)
    decl.secret = spec.secret
    decl.prompt = spec.prompt
    decl.env_source = spec.env_source
    return decl


def _normalize_params(entry: store.Entry, entry_spec: LangSpec | None, names: list[str]) -> None:
    """`skit params <shell> --normalize NAME`: rewrite `NAME=value` into `NAME="${NAME:-value}"` in
    the STORED COPY, then re-read the analyzer so the parameter becomes an env-delivered one — no
    temporary copy is ever written for it again, and $0 keeps pointing at the real file."""
    if (
        entry_spec is None
        or entry_spec.normalizer is None
        or entry_spec.params_io is None
        or entry_spec.analyzer is None
    ):
        raise _fail(
            gettext(
                '%(name)s has no --normalize: it is a shell idiom (VAR=value -> VAR="${VAR:-value}").'
            )
            % {"name": entry.meta.name},
            1,
        )
    if entry.meta.mode == "reference":
        raise _fail(
            gettext(
                "%(name)s is in reference mode, and skit never writes the original file. "
                'Change the line to VAR="${VAR:-value}" in the source directly.'
            )
            % {"name": entry.meta.name},
            1,
        )
    copy_path = entry.script_path
    if not copy_path.exists():
        raise _fail(gettext("%(name)s has no stored copy to edit.") % {"name": entry.meta.name}, 1)
    text = copy_path.read_text(encoding="utf-8", errors="replace")  # pragma: no mutate
    result = entry_spec.normalizer.normalize(text, list(names))
    for warning in result.refused:
        err_console.print(f"[yellow]{escape(_render_normalize_warning(warning))}[/yellow]")
    if not result.normalized:
        return  # every name was refused; the file is untouched (the warnings above said why)
    # Re-read the analyzer: each normalized name is now an ${NAME:-default} expansion, i.e. an
    # envdefault candidate. A name that was MANAGED must follow it — otherwise its stored const
    # definition would go missing on the very next run (loud, and rightly so).
    envdefaults = {
        c.name: c
        for c in entry_spec.analyzer.analyze(result.text).candidates
        if c.binding == "envdefault"
    }
    normalized = set(result.normalized)
    specs = [
        _reanchor_as_envdefault(s, envdefaults[s.name])
        if s.name in normalized and s.name in envdefaults
        else s
        for s in entry_spec.params_io.read(result.text)
    ]
    copy_path.write_text(
        entry_spec.params_io.write(result.text, specs),
        encoding="utf-8",  # pragma: no mutate — utf-8 equivalence
    )
    console.print(
        f"[green]{gettext('Normalized %(names)s in %(name)s: delivered as environment variables from now on (no temporary copy, and $0 stays your real file).') % {'names': ', '.join(escape(n) for n in result.normalized), 'name': escape(entry.meta.name)}}[/green]"
    )


def _render_declared_warning(warning: str) -> str:
    """Translate an edit_declared warning ("code:name") into a user-facing line. The codes are
    the closed set params.edit_declared emits; a static lookup (not gettext(f"...{code}")) keeps
    every string Babel-extractable, mirroring reconcile.render_warning."""
    code, _, name = warning.partition(":")
    return {
        "not-declared": gettext("%(name)s isn't a declared parameter; skipped."),
        "already-declared": gettext("%(name)s is already declared; skipped."),
        "bad-delivery": gettext("%(name)s: that delivery isn't available for this kind; skipped."),
        "not-a-placeholder": gettext(
            "%(name)s isn't a template placeholder, so it can't use placeholder delivery; skipped."
        ),
        "bad-type": gettext(
            "%(name)s: unknown type; skipped (use str, int, float, bool, or choice)."
        ),
        "bad-default": gettext("%(name)s: the default doesn't fit its type; skipped."),
        "choice-without-choices": gettext(
            "%(name)s: a choice parameter needs choices; set --choices %(name)s=a,b,c."
        ),
    }[code] % {"name": name}


def _edit_declared_params(
    entry: store.Entry,
    entry_spec: LangSpec,
    *,
    add: list[str],
    rm: list[str],
    types: dict[str, str],
    defaults: dict[str, str],
    choices: dict[str, list[str]],
    deliveries: dict[str, str],
    flags: dict[str, str],
    required: list[str],
    optional: list[str],
    help_texts: dict[str, str],
    secret: list[str],
    no_secret: list[str],
    prompts: dict[str, str],
    env_sources: dict[str, str],
    malformed: list[str],
) -> None:
    """Apply declared-schema changes to a meta-schema entry (rewrites meta.toml [[parameters]]).

    The allowed deliveries follow the kind: a template's interface is its placeholders, so it takes
    placeholder/env; everything else (a binary, and every interpreted kind that stores its schema in
    meta — ruby/perl/lua/r/powershell) takes flag/env, because they all assemble a real argv.
    Branching on "template" rather than "binary" keeps a declared `--add` on an interpreted kind
    from defaulting to an undeliverable placeholder row.

    allowed[0] is also the DEFAULT for a bare `--add NAME`, which is why "env" leads for a template:
    a name that is not one of the template's {placeholders} cannot be one (the template is the
    truth about which slots exist), so defaulting it to "placeholder" wrote a dead row that the run
    surface then refused. env is the delivery a template can always honour, and it matches what the
    TUI's own add-a-parameter field creates. `edit_declared` still maps a name that IS a placeholder
    onto placeholder delivery, and membership validation is unchanged — only the default moves.

    Prompt entries ride the same path with two differences: the placeholder truth is the BODY
    (scanned fresh, so `--add` of a hole the user typed since adding works), and --add/--rm also
    maintain the MANAGED list (meta `params`) — managing a detected placeholder / unmanaging one
    is exactly the add/remove of its row. An --rm of a managed-but-undeclared name (the common
    synthesized case) is therefore real work for a prompt, not a `not-declared` warning."""
    is_prompt = entry.meta.kind == "prompt"
    if is_prompt and not entry.meta.interpolate:
        # The read view says insertion is off; the edit surface must not silently scan
        # the body and write rows that are inert until the switch flips (coherence with
        # the "off = no scanning" gate).
        raise _fail(
            gettext(
                "Variable insertion is off for %(name)s — turn it on first with: "
                "skit params %(name)s --interpolate"
            )
            % {"name": entry.meta.name},
            1,
        )
    allowed = ("env", "placeholder") if entry_spec.placeholder_params else ("flag", "env")
    managed = list(entry.meta.params or [])
    body_placeholders = _prompt_body_placeholders(entry) if is_prompt else []
    placeholder_truth = (
        sorted(set(body_placeholders) | set(managed)) if is_prompt else entry.meta.params or []
    )
    for item in malformed:
        err_console.print(
            f"[yellow]{escape(gettext('Ignored a malformed value: %(item)s (expected NAME=VALUE).') % {'item': item})}[/yellow]"
        )
    result = edit_declared(
        store.read_parameters(entry.slug),
        add=add,
        rm=rm,
        types=types,
        defaults=defaults,
        choices=choices,
        deliveries=deliveries,
        flags=flags,
        required=required,
        optional=optional,
        help_texts=help_texts,
        secret=secret,
        no_secret=no_secret,
        prompts=prompts,
        env_sources=env_sources,
        allowed_deliveries=allowed,
        placeholder_names=placeholder_truth,
    )
    warnings = list(result.warnings)
    if is_prompt:
        # Maintain the managed list alongside the schema rows: an added body placeholder
        # becomes managed (in body order); a removed name stops being asked for. An --rm
        # that only unmanages (no declared row to drop) is real work here, so its
        # `not-declared` warning is retracted.
        added_managed = [
            n for n in add if n in body_placeholders and n not in managed and n not in rm
        ]
        removed_managed = [n for n in rm if n in managed]
        if added_managed or removed_managed:
            keep = [n for n in managed if n not in removed_managed] + added_managed
            new_managed = [n for n in body_placeholders if n in set(keep)]
            # A managed name the body has already lost isn't in body order — keep it at
            # the tail unless this very call removed it (drift stays visible, never grows).
            new_managed += [n for n in keep if n not in body_placeholders]
            store.write_prompt_managed(entry.slug, new_managed)
            warnings = [
                w for w in warnings if w not in {f"not-declared:{n}" for n in removed_managed}
            ]
    for w in warnings:
        err_console.print(f"[yellow]{escape(_render_declared_warning(w))}[/yellow]")
    store.write_parameters(entry.slug, result.decls)
    purged = argstate.purge_secret(entry.slug, {d.name for d in result.decls if d.secret})
    if purged:
        console.print(
            "[dim]"
            + gettext(
                "Removed previously stored plaintext value(s) for now-secret parameter(s): %(names)s"
            )
            % {"names": ", ".join(escape(n) for n in sorted(purged))}
            + _DIM_CLOSE
        )
    names = ", ".join(escape(d.name) for d in result.decls) or "—"
    console.print(
        f"[green]{gettext('Updated %(name)s. Declared parameters: %(names)s') % {'name': escape(entry.meta.name), 'names': names}}[/green]"
    )


# --------------------------------------------------------------------------
# deps
# --------------------------------------------------------------------------


def _deps_read_view(entry: store.Entry, *, supports_deps: bool, as_json: bool) -> None:
    """The bare `skit deps NAME` view: dependencies + Python constraint (python entries
    only) and the needed external commands (every kind)."""
    needs = list(entry.meta.needs or [])
    if as_json:
        console.print_json(
            json.dumps(
                {
                    "dependencies": list(entry.meta.dependencies or []),
                    "requires_python": entry.meta.requires_python,
                    "needs": needs,
                },
                ensure_ascii=False,
            )
        )
        return
    if supports_deps:
        console.print(
            gettext("Dependencies of %(name)s: %(deps)s")
            % {
                "name": escape(entry.meta.name),
                "deps": ", ".join(escape(d) for d in (entry.meta.dependencies or [])) or "—",
            }
        )
        if entry.meta.requires_python:
            console.print(
                gettext("Python constraint: %(python)s")
                % {"python": escape(entry.meta.requires_python)}
            )
    console.print(
        gettext("External commands needed by %(name)s: %(needs)s")
        % {"name": escape(entry.meta.name), "needs": ", ".join(escape(n) for n in needs) or "—"}
    )


@app.command(
    help=gettext("View or update a script's dependencies, Python constraint, and needed commands."),
    epilog=gettext(
        'Examples:  skit deps tool --dep "requests>=2,<3" --dep rich  ·  skit deps clip --need jq'
    ),
)
def deps(
    name: str = _SCRIPT_ARG,
    dep: list[str] = typer.Option(
        None, "--dep", help=gettext("A dependency (repeat for more; replaces the whole list)")
    ),
    clear: bool = typer.Option(False, "--clear", help=gettext("Remove every dependency")),
    python: str = typer.Option(
        None, "--python", help=gettext('Python version constraint, e.g. ">=3.11"')
    ),
    need: list[str] = typer.Option(
        None,
        "--need",
        help=gettext(
            "An external command the script needs on PATH (repeat; replaces the whole list)"
        ),
    ),
    clear_needs: bool = typer.Option(
        False, "--clear-needs", help=gettext("Remove every needed external command")
    ),
    as_json: bool = typer.Option(False, "--json", help=gettext("Output as JSON")),
) -> None:
    """View or update a script's recorded dependencies and needed external commands.
    Dependencies apply to kinds with package management — python (PEP 723 + uv) and js/ts
    (per-script npm installs); needs (commands that must be on PATH) apply to every kind —
    a shell script may need `ffmpeg` too."""
    try:
        entry = store.resolve(name)
    except store.NotFoundError as exc:
        raise _fail(str(exc), 1) from exc
    deps_spec = spec_for(entry.meta.kind)
    supports_deps = deps_spec is not None and deps_spec.supports_deps
    deps_requested = dep is not None or clear or python is not None
    needs_requested = need is not None or clear_needs
    if deps_requested and not supports_deps:
        # A refused flag, not an operational failure — the usage exit code, so `skit deps`
        # agrees with `skit add` (and its own --dep/--clear conflict below) on what a refusal is.
        raise _fail(
            gettext("%(name)s doesn't take package dependencies — only --need applies here.")
            % {"name": entry.meta.name},
            EXIT_USAGE,
        )
    if dep and clear:
        err_console.print(
            f"[red]{gettext('Use --dep to set the list or --clear to empty it — not both.')}[/red]"
        )
        raise typer.Exit(EXIT_USAGE)
    if need and clear_needs:
        err_console.print(
            f"[red]{gettext('Use --need to set the list or --clear-needs to empty it — not both.')}[/red]"
        )
        raise typer.Exit(EXIT_USAGE)
    if not deps_requested and not needs_requested:
        _deps_read_view(entry, supports_deps=supports_deps, as_json=as_json)
        return
    # Deps BEFORE needs: a --dep/--python refusal raises (StoreUsageError) at the top of
    # update_dependencies, before any write — so processing deps first means a refused request
    # aborts with NOTHING committed. Doing needs first would persist the needs write and then
    # exit 2 on the deps refusal, a partial application a --json/CI caller couldn't detect.
    if deps_requested:
        if clear:
            new_deps: list[str] = []
        elif dep is not None:
            # Drop empty/whitespace values so `--dep ''` clears (and sweeps) rather than
            # recording a junk "" package the --json contract would then carry.
            new_deps = [d.strip() for d in dep if d.strip()]
        else:
            new_deps = list(entry.meta.dependencies or [])
        try:
            entry = store.update_dependencies(entry.slug, new_deps, requires_python=python)
        except store.StoreUsageError as exc:
            raise _fail(str(exc), EXIT_USAGE) from exc
        except store.StoreError as exc:
            raise _fail(str(exc), 1) from exc
        if not as_json:
            console.print(
                f"[green]{gettext('Dependencies of %(name)s updated: %(deps)s') % {'name': escape(entry.meta.name), 'deps': ', '.join(escape(d) for d in new_deps) or '—'}}[/green]"
            )
    if needs_requested:
        # Drop empty/whitespace values, mirroring the --dep path: an empty command name is junk in
        # the --json contract AND bricks the entry — `shutil.which("")` is None, so every run then
        # fails "Missing required command(s):" before the script starts.
        new_needs = [] if clear_needs else [n.strip() for n in (need or []) if n.strip()]
        entry = store.update_needs(entry.slug, new_needs)
        if not as_json:
            console.print(
                f"[green]{gettext('Needs of %(name)s updated: %(needs)s') % {'name': escape(entry.meta.name), 'needs': ', '.join(escape(n) for n in new_needs) or '—'}}[/green]"
            )
    if as_json:
        # --json on a write: emit the final state as the machine contract, same shape as the
        # read view, instead of the human confirmation lines above.
        _deps_read_view(entry, supports_deps=supports_deps, as_json=True)


# --------------------------------------------------------------------------
# agent — connect skit to AI coding agents
# --------------------------------------------------------------------------

agent_app = typer.Typer(
    help=gettext("Connect skit to AI agents: install the official Agent Skill."),
    no_args_is_help=True,
)
app.add_typer(agent_app, name="agent")


def _agent_install_confirmed(skills_dir: Path) -> None:
    # Read the bundled skill BEFORE the write-error wrap: a broken installation (skill
    # missing from the package) must fail loudly, not as "could not write there".
    text = agentskill.skill_text()
    try:
        written = agentskill.install_into(skills_dir, text)
    except OSError as exc:
        # e.g. --to points at an existing file: a clean one-liner, not a traceback.
        raise _fail(
            gettext("Could not write the skill there: %(error)s") % {"error": exc}, 1
        ) from exc
    console.print(
        f"[green]{gettext('Installed the skit Agent Skill: %(path)s') % {'path': escape(str(written))}}[/green]"
    )


def _agent_pick_target(candidates: list[agentskill.Target]) -> agentskill.Target | None:
    """Numbered picker + confirmation; None means the user backed out."""
    scope_names = {"user": gettext("user"), "project": gettext("project")}
    console.print(gettext("Agent directories on this machine:"))
    for i, t in enumerate(candidates, start=1):
        console.print(f"  {i}. {t.name} ({scope_names[t.scope]})  →  {escape(str(t.skills_dir))}")
    choice = Prompt.ask(
        gettext("Install where?"),
        choices=[str(i) for i in range(1, len(candidates) + 1)],
        default="1",
        console=console,
    )
    target = candidates[int(choice) - 1]
    if not Confirm.ask(
        gettext("Write the skill into %(path)s?") % {"path": escape(str(target.skills_dir))},
        default=True,
        console=console,
    ):
        return None
    return target


@agent_app.command(
    "install",
    help=gettext("Install skit's Agent Skill into an AI agent's skills directory."),
    epilog=gettext(
        "Examples:  skit agent install  ·  skit agent install claude  ·  skit agent install codex --project  ·  skit agent install --to ~/.claude/skills"
    ),
)
def agent_install(
    target: str = typer.Argument(
        None,
        help=gettext(
            "Where to install: claude, codex, or agents (the cross-agent ./.agents directory)"
        ),
    ),
    to: Path = typer.Option(
        None,
        "--to",
        help=gettext("Install into this skills directory instead of a named target"),
    ),
    project: bool = typer.Option(
        False,
        "--project",
        help=gettext(
            "Install into the current project (./.claude, ./.codex) instead of your home directory"
        ),
    ),
) -> None:
    """Teach the user's AI agents to use skit. An explicit TARGET or --to is consent by
    itself; bare `skit agent install` detects agent directories and asks (principle #6:
    skit never touches another tool's directory uninvited)."""
    if to is not None and (target or project):
        err_console.print(
            f"[red]{gettext('Use a named target (with optional --project) or --to — not both.')}[/red]"
        )
        raise typer.Exit(EXIT_USAGE)
    if to is not None:
        _agent_install_confirmed(to.expanduser())
        return
    home, cwd = agentskill.default_roots()
    if target:
        resolved = agentskill.named_target(target, project=project, home=home, cwd=cwd)
        if resolved is None:
            err_console.print(
                "[red]"
                + gettext("Unknown target %(name)s. Valid targets: claude, codex, agents.")
                % {"name": escape(target)}
                + _RED_CLOSE
            )
            raise typer.Exit(EXIT_USAGE)
        _agent_install_confirmed(resolved.skills_dir)
        return
    if not _is_interactive():
        err_console.print(
            f"[red]{gettext('Nothing installed: name a target (claude, codex, agents) or pass --to DIR.')}[/red]"
        )
        raise typer.Exit(EXIT_USAGE)
    candidates = agentskill.detect_targets(home=home, cwd=cwd)
    if not candidates:
        raise _fail(
            gettext(
                "No agent directories detected (~/.claude, ~/.codex, ./.agents, …). Pass --to DIR to choose one yourself."
            ),
            1,
        )
    picked = _agent_pick_target(candidates)
    if picked is None:
        console.print(gettext("Cancelled — nothing was written."))
        raise typer.Exit(0)  # pragma: no mutate — Exit(0)/Exit(None) both mean a clean exit
    _agent_install_confirmed(picked.skills_dir)


# --------------------------------------------------------------------------
# doctor
# --------------------------------------------------------------------------


def _uv_required(entries: list[store.Entry]) -> bool:
    """Whether a missing uv should fail doctor's exit code. uv is what runs python
    entries, so it's required when any python entry exists — and also for an EMPTY
    library (a fresh install's doctor must still steer the user toward a working
    setup). A non-empty library made purely of exe/command entries runs fine without
    uv, and exiting 1 there sent automation chasing a phantom problem."""
    if not entries:
        return True
    return any(e.meta.kind == "python" for e in entries)


@app.command(help=gettext("Check that uv is available and the script store is intact."))
def doctor(
    rebuild: bool = typer.Option(
        False, "--rebuild", help=gettext("Rebuild the index from each script's meta.toml")
    ),
    as_json: bool = typer.Option(False, "--json", help=gettext("Output as JSON")),
) -> None:
    """Environment self-check (the CLI face of the TUI health-check screen)."""
    from .paths import scripts_dir

    uv = launcher.find_uv()
    if rebuild:
        count, problems = store.doctor_rebuild()
        console.print(
            f"[green]{ngettext('Index rebuilt: %(count)s entry', 'Index rebuilt: %(count)s entries', count) % {'count': count}}[/green]"
        )
        for p in problems:
            console.print(f"  [yellow]{escape(p)}[/yellow]")
    entries = store.list_entries()
    # One shared collector with the TUI Health screen (healthcheck.collect) — the two
    # faces previously swept separately and disagreed about what "healthy" means.
    report = healthcheck.collect(entries)
    missing = [e.meta.name for e in report.missing]
    drifted = [e.meta.name for e in report.drifted]
    needs_missing = report.needs_missing
    bad_runners = report.invalid_runner_rows
    mirror = config.load_mirror()
    location = scripts_dir()
    size = store.dir_size(location)
    if as_json:
        console.print_json(
            json.dumps(
                {
                    "uv": uv,
                    "entries": len(entries),
                    "missing": missing,
                    "drift": drifted,
                    "needs_missing": needs_missing,
                    "launch_blocked": report.launch_blocked,
                    "runner_rows_invalid": bad_runners,
                    "mirror": mirror.pypi if mirror.enabled else "off",
                    "location": str(location),
                    "size_bytes": size,
                },
                ensure_ascii=False,
            )
        )
        raise typer.Exit(0 if uv or not _uv_required(entries) else 1)
    if uv:
        console.print(f"[green]✓ {gettext('uv: %(path)s') % {'path': escape(uv)}}[/green]")
    else:
        console.print(
            f"[red]✗ {gettext('uv: not found. Install it from https://docs.astral.sh/uv/getting-started/installation/')}[/red]"
        )
    console.print(
        "✓ "
        + ngettext("%(count)s script registered", "%(count)s scripts registered", len(entries))
        % {"count": len(entries)}
    )
    for m in missing:
        console.print(
            f"  [yellow]⚠ {gettext('%(name)s: the launch target is gone from disk') % {'name': escape(m)}}[/yellow]"
        )
    for d in drifted:
        console.print(
            f"  [yellow]⚠ {gettext('%(name)s: form definitions are out of sync — run: skit params %(name)s --resync') % {'name': escape(d)}}[/yellow]"
        )
    for nm_name, tools in needs_missing.items():
        console.print(
            f"  [yellow]⚠ {gettext('%(name)s: missing external command(s): %(tools)s') % {'name': escape(nm_name), 'tools': ', '.join(escape(t) for t in tools)}}[/yellow]"
        )
    for bl_name, reason in report.launch_blocked.items():
        # The runtime sweep the multilang design promised ("doctor warns") but never
        # shipped: an uninstalled interpreter/JS runtime, a gone pinned agent binary.
        console.print(
            f"  [yellow]⚠ {gettext('%(name)s: a run would refuse to start — %(reason)s') % {'name': escape(bl_name), 'reason': escape(reason)}}[/yellow]"
        )
    if bad_runners:
        console.print(
            f"  [yellow]⚠ {gettext('Ignored malformed runner row(s) in config: %(rows)s (fix config.toml or re-add with skit runner add)') % {'rows': ', '.join(escape(r) for r in bad_runners)}}[/yellow]"
        )
    if mirror.enabled:
        console.print("✓ " + gettext("Mirror: on (%(pypi)s)") % {"pypi": escape(mirror.pypi)})
    else:
        console.print("✓ " + gettext("Mirror: off"))
    console.print(
        gettext("Library: %(path)s (%(count)s · %(size)s)")
        % {"path": escape(str(location)), "count": len(entries), "size": store.human_size(size)}
    )
    raise typer.Exit(0 if uv or not _uv_required(entries) else 1)


# --------------------------------------------------------------------------
# config (git-config grammar: bare = list, KEY = read, KEY VALUE = write)
# --------------------------------------------------------------------------

_CONFIG_KEYS = ("lang", "editor", "mirror", "form", "after_run", "shell.bash_path", "js.runner")


def _config_lang_value() -> str:
    override = config.load_config().get("language", "")
    if isinstance(override, str) and override:
        return override
    return gettext("auto (%(locale)s)") % {"locale": i18n.current_locale()}


def _config_mirror_value() -> str:
    m = config.load_mirror()
    return m.pypi if m.enabled else "off"


def _config_value(key: str) -> str:
    # Dispatch table (not an if-chain) keeps the key set open-ended without tripping the
    # too-many-returns lint; _CONFIG_KEYS guards `key` so the lookup can't miss.
    readers = {
        "lang": _config_lang_value,
        "editor": lambda: config.load_editor() or gettext("default ($VISUAL / $EDITOR)"),
        "mirror": _config_mirror_value,
        "form": config.load_form,
        "after_run": config.load_after_run,
        "shell.bash_path": lambda: config.load_bash_path() or gettext("auto (bash on PATH)"),
        "js.runner": lambda: config.load_js_runner() or gettext("auto (deno > bun > node)"),
    }
    return readers[key]()


def _config_set(key: str, value: str) -> None:
    if key == "lang":
        if value.lower() != "auto" and not i18n.is_supported(value):
            err_console.print(
                f"[red]{gettext('Unknown language: %(tag)s. Available: %(locales)s') % {'tag': escape(value), 'locales': ', '.join(i18n.available_locales())}}[/red]"
            )
            raise typer.Exit(EXIT_USAGE)
        i18n.set_language("" if value.lower() == "auto" else value)
    elif key == "editor":
        config.save_editor(value)
    elif key == "mirror":
        if value == "off":
            config.disable()
        elif value in config.PYPI_PRESETS:
            config.save_mirror(config.preset(value))
        else:
            choices = ", ".join([*config.PYPI_PRESETS, "off"])
            err_console.print(
                f"[red]{gettext('Unknown mirror: %(name)s. Choose from: %(names)s') % {'name': escape(value), 'names': choices}}[/red]"
            )
            raise typer.Exit(EXIT_USAGE)
    elif key == "form":
        if value not in config.FORM_STYLES:
            err_console.print(
                f"[red]{gettext('Unknown form style: %(value)s. Choose from: tui, plain') % {'value': escape(value)}}[/red]"
            )
            raise typer.Exit(EXIT_USAGE)
        config.save_form(value)
    elif key == "shell.bash_path":
        # Validate on set (never on clear): an empty value clears the key. A non-empty
        # path must point at a real file — a typo'd bash_path would otherwise surface
        # only later as an opaque "isn't available" refusal on a Windows shell run.
        if value.strip() and not Path(value).expanduser().is_file():
            err_console.print(
                f"[red]{gettext('No such file: %(path)s') % {'path': escape(value)}}[/red]"
            )
            raise typer.Exit(EXIT_USAGE)
        config.save_bash_path(value)
    elif key == "js.runner":
        if value.strip() and value not in config.JS_RUNNERS:
            err_console.print(
                f"[red]{gettext('Unknown JS runner: %(value)s. Choose from: %(names)s') % {'value': escape(value), 'names': ', '.join(config.JS_RUNNERS)}}[/red]"
            )
            raise typer.Exit(EXIT_USAGE)
        config.save_js_runner(value)
    else:  # after_run
        if value not in config.AFTER_RUN_MODES:
            err_console.print(
                f"[red]{gettext('Unknown after-run behavior: %(value)s. Choose from: exit, stay') % {'value': escape(value)}}[/red]"
            )
            raise typer.Exit(EXIT_USAGE)
        config.save_after_run(value)


@app.command(
    "config",
    help=gettext("Read or set skit's settings (language, editor, mirror, form style, after-run)."),
    epilog=gettext(
        "Examples:  skit config  ·  skit config lang zh-TW  ·  skit config after_run stay"
    ),
)
def config_cmd(
    key: str = typer.Argument(
        None,
        help=gettext(
            "Setting name: lang / editor / mirror / form / after_run / shell.bash_path / js.runner"
        ),
    ),
    value: str = typer.Argument(
        None, help=gettext('New value (omit to read; lang also accepts "auto")')
    ),
    as_json: bool = typer.Option(False, "--json", help=gettext("Output as JSON")),
) -> None:
    """git-config grammar: bare `skit config` lists everything; `config KEY` reads one;
    `config KEY VALUE` writes one. The guided experience lives in the TUI (press ,)."""
    if key is None:
        if as_json:
            console.print_json(
                json.dumps({k: _config_value(k) for k in _CONFIG_KEYS}, ensure_ascii=False)
            )
            return
        for k in _CONFIG_KEYS:
            console.print(f"  {k:<16}{escape(_config_value(k))}")
        return
    if key not in _CONFIG_KEYS:
        err_console.print(
            f"[red]{gettext('Unknown setting: %(key)s. Available: %(keys)s') % {'key': escape(key), 'keys': ', '.join(_CONFIG_KEYS)}}[/red]"
        )
        raise typer.Exit(EXIT_USAGE)
    if value is None:
        console.print(escape(_config_value(key)))
        return
    _config_set(key, value)
    console.print(f"[green]{key} = {escape(_config_value(key))}[/green]")


# --------------------------------------------------------------------------
# first run (mirror offer for blocked networks; interactive TTY only)
# --------------------------------------------------------------------------


def _prompt_uv_binary(default: str) -> str:
    """Prompt for the uv-binary mirror URL, insisting on https:// (the binary is
    downloaded, chmod +x'd, and executed — an http:// mirror is a MITM->RCE vector)."""
    while True:
        value = Prompt.ask(gettext("uv binary mirror URL"), default=default, console=console)
        if value.startswith("https://"):
            return value
        err_console.print(
            "[red]"
            + gettext(
                "The uv binary is downloaded and executed, so its mirror URL must use https:// (got: %(url)s)."
            )
            % {"url": escape(value)}
            + _RED_CLOSE
        )


def _mirror_wizard() -> None:
    m = config.load_mirror()
    if not m.enabled:
        default = "off"
    else:
        default = next((k for k, v in config.PYPI_PRESETS.items() if v == m.pypi), "custom")
    choice = Prompt.ask(
        gettext("Mirror for faster installs in mainland China"),
        choices=[*config.PYPI_PRESETS, "custom", "off"],
        default=default,
        console=console,
    )
    if choice == "off":
        config.disable()
    elif choice == "custom":
        config.save_mirror(
            config.MirrorConfig(
                enabled=True,
                pypi=Prompt.ask(
                    gettext("PyPI index URL"),
                    default=m.pypi or config.PYPI_PRESETS["tsinghua"],
                    console=console,
                ),
                python_install=Prompt.ask(
                    gettext("Python-install mirror URL"),
                    default=m.python_install or config.PYTHON_INSTALL_MIRROR,
                    console=console,
                ),
                uv_binary=_prompt_uv_binary(m.uv_binary or config.UV_BINARY_MIRROR),
                npm=Prompt.ask(
                    gettext("npm registry URL"),
                    default=m.npm or config.NPM_REGISTRY_MIRROR,
                    console=console,
                ),
            )
        )
    else:
        config.save_mirror(config.preset(choice))


def _maybe_first_run_setup() -> None:
    """On the first bare `skit` run, offer mirror setup if the network to PyPI/GitHub looks
    blocked. Interactive TTY only; runs once (a [mirror] section marks the offer as done)."""
    if config.mirror_configured() or not _is_interactive():
        return
    if config.looks_blocked():
        console.print(gettext("Network to PyPI / GitHub looks slow or blocked."))
        if Confirm.ask(
            gettext("Configure mirrors for faster installs (mainland China)?"),
            default=True,
            console=console,
        ):
            _mirror_wizard()
    if not config.mirror_configured():
        config.save_mirror(config.load_mirror())  # persist a marker so we don't probe every run


if __name__ == "__main__":
    sys.exit(app())
