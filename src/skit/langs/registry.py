"""The language registry: kind -> LangSpec resolution and add-time kind inference.

Registration is explicit aggregation (a builder table), not import-time side effects —
deterministic, statically analyzable, no import-order magic. Specs are built lazily and
cached: resolving "python" never pays for a future language's parser import, and a
language whose optional parser fails to import degrades its capabilities to None
instead of crashing (the `spec.analyzer is None` idiom downstream handles the rest).
"""

from __future__ import annotations

import os
import sys
from functools import cache
from pathlib import Path
from typing import TYPE_CHECKING

from . import launch
from .base import Analyzer, CliReader, CommentSyntax, LangSpec, LaunchStrategy, ParamsIO

if TYPE_CHECKING:
    from collections.abc import Callable


def _python_spec() -> LangSpec:
    from .python import analyzer, argspec, metawriter, reconcile

    return LangSpec(
        kind="python",
        family="interpreted",
        glyph="⬡",
        launch=launch.UvLaunch(),
        extensions=(".py",),
        shebangs=("python", "python3"),
        stored_name="script.py",  # PINNED: existing stores carry this name on disk
        comment=CommentSyntax(prefix="#"),
        params_io=ParamsIO(read=metawriter.read_params, write=metawriter.write_params),
        analyzer=Analyzer(analyze=analyzer.analyze, reconcile=reconcile.reconcile),
        cli_reader=CliReader(read_cli=argspec.read_cli),
        supports_modes=True,
        supports_deps=True,
    )


def _interpreted(
    kind: str,
    glyph: str,
    interpreter: str,
    extensions: tuple[str, ...],
    shebangs: tuple[str, ...],
    comment_prefix: str,
    *,
    launch_strategy: LaunchStrategy | None = None,
    prefix: tuple[str, ...] = (),
) -> LangSpec:
    """A Tier-0 interpreted kind: launchable, copyable, editable, comment-described,
    declared-params-capable (via meta [[parameters]]) — analyzer/cli_reader arrive as
    later, purely additive capabilities. One data row per language."""
    return LangSpec(
        kind=kind,
        family="interpreted",
        glyph=glyph,
        launch=launch_strategy or launch.InterpreterLaunch(interpreter, prefix=prefix),
        extensions=extensions,
        shebangs=shebangs,
        default_interpreter=interpreter,
        stored_name=f"script{extensions[0]}",
        comment=CommentSyntax(prefix=comment_prefix),
        supports_modes=True,
    )


def _shell_spec() -> LangSpec:
    return _interpreted(
        "shell",
        "#",
        "bash",
        (".sh", ".bash", ".zsh"),
        ("bash", "sh", "zsh", "dash", "ash", "ksh"),
        "#",
    )


def _fish_spec() -> LangSpec:
    return _interpreted("fish", "∿", "fish", (".fish",), ("fish",), "#")


def _js_spec() -> LangSpec:
    return _interpreted(
        "js",
        "✦",
        "",
        (".js", ".mjs", ".cjs"),
        ("node", "deno", "bun"),
        "//",
        launch_strategy=launch.RunnerLaunch(),
    )


def _ts_spec() -> LangSpec:
    return _interpreted(
        "ts",
        "✧",
        "",
        (".ts", ".mts", ".cts"),
        (),
        "//",
        launch_strategy=launch.RunnerLaunch(),
    )


def _powershell_spec() -> LangSpec:
    # `pwsh -File` (explicit file semantics); powershell.exe users set
    # meta.interpreter — the strategy resolves whatever name is recorded.
    return _interpreted(
        "powershell",
        "»",
        "pwsh",
        (".ps1",),
        ("pwsh", "powershell"),
        "#",
        prefix=("-File",),
    )


def _ruby_spec() -> LangSpec:
    return _interpreted("ruby", "◆", "ruby", (".rb",), ("ruby",), "#")


def _perl_spec() -> LangSpec:
    return _interpreted("perl", "◈", "perl", (".pl",), ("perl",), "#")


def _lua_spec() -> LangSpec:
    return _interpreted("lua", "○", "lua", (".lua",), ("lua", "luajit"), "--")


def _r_spec() -> LangSpec:
    return _interpreted("r", "◇", "Rscript", (".r",), ("Rscript",), "#")


def _exe_spec() -> LangSpec:
    return LangSpec(kind="exe", family="binary", glyph="▶", launch=launch.DirectLaunch())


def _command_spec() -> LangSpec:
    # takes_argv=False: a command's "arguments" are its placeholders; silently reusing a
    # remembered argv tail on a template is more surprising than helpful (cli run's
    # reuse-last-args affordance keys off this).
    return LangSpec(
        kind="command",
        family="template",
        glyph="$",
        launch=launch.TemplateLaunch(),
        takes_argv=False,
    )


_BUILDERS: dict[str, Callable[[], LangSpec]] = {
    "python": _python_spec,
    "shell": _shell_spec,
    "fish": _fish_spec,
    "js": _js_spec,
    "ts": _ts_spec,
    "powershell": _powershell_spec,
    "ruby": _ruby_spec,
    "perl": _perl_spec,
    "lua": _lua_spec,
    "r": _r_spec,
    "exe": _exe_spec,
    "command": _command_spec,
}

KNOWN_KINDS = frozenset(_BUILDERS)


@cache
def spec_for(kind: str) -> LangSpec | None:
    """Resolve a kind to its LangSpec; None for a kind this skit version doesn't know
    (a meta written by a newer skit — every consumer degrades: launcher raises a clean
    LaunchError, forms fall back to the extra-args escape, the TUI shows a plain badge)."""
    builder = _BUILDERS.get(kind)
    return builder() if builder is not None else None


def stored_name(kind: str) -> str:
    """The in-store copy filename for a kind ("" when the kind is never copied).
    Unknown kinds fall back to the historical "payload" so a newer store's copy-mode
    entry still resolves to *some* path instead of crashing."""
    spec = spec_for(kind)
    if spec is None:
        return "payload"
    return spec.stored_name


def infer_kind(path: Path, force_exe: bool = False) -> str:
    """What kind of entry a path should become — the tool can see the file type, so
    don't demand a flag: a registered extension names its kind, an executable file is
    "exe", anything else is "unknown" (callers point at --exe / --cmd). Shared by the
    CLI and the TUI add panel so the two paths can't drift apart."""
    if force_exe:
        return "exe"
    by_ext = _extension_map().get(path.suffix.lower())
    if by_ext is not None:
        return by_ext
    if path.is_file():
        program = shebang_program(path)
        if program is not None:
            by_shebang = _shebang_map().get(program)
            if by_shebang is not None:
                return by_shebang
        if _is_executable_file(path):
            return "exe"
    return "unknown"


def shebang_program(path: Path) -> str | None:
    """The program basename a #! line names, or None (no shebang / unreadable).

    Handles the `#!/usr/bin/env [-S...] program` indirection: env flags (-S and the
    split-string payload's leading dashes) are skipped so `#!/usr/bin/env -S deno run
    --allow-net` still names deno. Only the FIRST line is ever read — this runs inside
    add-time inference, so it must stay cheap and total (any I/O or decode error is
    simply "no shebang")."""
    try:
        with open(path, "rb") as f:
            first = f.readline(512)
    except OSError:
        return None
    if not first.startswith(b"#!"):
        return None
    tokens = first[2:].decode("utf-8", errors="replace").split()
    if not tokens:
        return None
    program = Path(tokens[0]).name
    if program == "env":
        for tok in tokens[1:]:
            if not tok.startswith("-"):
                program = Path(tok).name
                break
        else:
            return None
    return program


@cache
def _shebang_map() -> dict[str, str]:
    out: dict[str, str] = {}
    for kind in _BUILDERS:
        spec = spec_for(kind)
        if spec is None:  # pragma: no cover — every _BUILDERS key resolves by construction
            continue
        for program in spec.shebangs:
            out[program] = kind
    return out


@cache
def _extension_map() -> dict[str, str]:
    out: dict[str, str] = {}
    for kind in _BUILDERS:
        spec = spec_for(kind)
        if spec is None:  # pragma: no cover — every _BUILDERS key resolves by construction
            continue
        for ext in spec.extensions:
            out[ext] = kind
    return out


def _is_executable_file(path: Path) -> bool:
    """Whether `path` is a program this platform would run directly.

    POSIX has an execute bit, so os.access(X_OK) is the right question there. Windows has none —
    os.access(X_OK) is True for *every* readable file, which would misclassify a plain `notes.txt`
    as an executable — so on Windows a file counts as executable only when its extension is one the
    OS itself treats as runnable, i.e. a member of PATHEXT (.exe/.bat/.cmd/…)."""
    if sys.platform == "win32":
        # PATHEXT is a Windows env var, always ';'-delimited (independent of os.pathsep), listing
        # the extensions the shell will execute; fall back to the conventional default when unset.
        pathext = os.environ.get("PATHEXT") or ".COM;.EXE;.BAT;.CMD"
        runnable = {ext.lower() for ext in pathext.split(";") if ext}
        return path.suffix.lower() in runnable
    return os.access(path, os.X_OK)
