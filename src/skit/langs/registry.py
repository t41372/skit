"""The language registry: kind -> LangSpec resolution and add-time kind inference.

Registration is explicit aggregation (a builder table), not import-time side effects —
deterministic, statically analyzable, no import-order magic. Specs are built lazily and
cached: resolving "python" never pays for a future language's parser import, and a
language whose optional parser fails to import degrades its capabilities to None
instead of crashing (the `spec.analyzer is None` idiom downstream handles the rest).
"""

from __future__ import annotations

import os
import re
import sys
from dataclasses import replace
from functools import cache
from pathlib import Path
from typing import TYPE_CHECKING

from . import launch
from .base import (
    Analyzer,
    CliReader,
    CommentSyntax,
    Injector,
    LangSpec,
    LaunchStrategy,
    Normalizer,
    ParamsIO,
)

if TYPE_CHECKING:
    from collections.abc import Callable


def _python_spec() -> LangSpec:
    from .python import analyzer, argspec, metawriter, reconcile, shim

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
        injector=Injector(inject=shim.inject_entry),
        supports_modes=True,
        deps_flavor="uv",
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
    from .python import metawriter  # the '#'-comment block engine is language-blind (PEP-723 regex)

    spec = _interpreted(
        "shell",
        "#",
        "bash",
        (".sh", ".bash", ".zsh"),
        ("bash", "sh", "zsh", "dash", "ash", "ksh"),
        "#",
    )
    # The in-file [tool.skit] block works verbatim on shell text (same '#' comment prefix), so shell
    # gets the full manage/reconcile experience through the same metawriter as Python.
    analyzer_cap: Analyzer | None
    injector_cap: Injector | None
    normalizer_cap: Normalizer | None
    cli_reader_cap: CliReader | None
    try:
        # One guard for all four: every one of them is a tree-sitter consumer (inject/normalize and
        # the getopts reader import the analyzer's grammar handle and node helpers), so they stand or
        # fall together — a broken grammar wheel must not leave a half-capable shell kind behind.
        from .shell import analyzer, cli_reader, inject, normalize
    except ImportError:  # pragma: no cover — a broken/absent tree-sitter grammar wheel
        # Degrade the capabilities to None (the `if spec.analyzer is None` idiom downstream handles
        # the rest); everything else about the shell kind still works. With no analyzer there are no
        # detected params at all, so an inject-delivery value can never reach the missing injector.
        analyzer_cap = None
        injector_cap = None
        normalizer_cap = None
        cli_reader_cap = None
    else:
        analyzer_cap = Analyzer(analyze=analyzer.analyze, reconcile=analyzer.reconcile)
        injector_cap = Injector(inject=inject.inject)
        normalizer_cap = Normalizer(normalize=normalize.normalize_idiom)
        cli_reader_cap = CliReader(read_cli=cli_reader.read_cli)
    return replace(
        spec,
        params_io=ParamsIO(read=metawriter.read_params, write=metawriter.write_params),
        analyzer=analyzer_cap,
        cli_reader=cli_reader_cap,
        injector=injector_cap,
        normalizer=normalizer_cap,
    )


def _fish_spec() -> LangSpec:
    from .fish import analyzer, cli_reader
    from .python import metawriter  # the '#'-comment block engine is language-blind (PEP-723 regex)

    spec = _interpreted("fish", "∿", "fish", (".fish",), ("fish",), "#")
    # fish v1: env-idiom detection (analyzer, envdefault candidates only — env delivery needs no
    # injector) + argparse reading (cli_reader), plus in-file [tool.skit] via the '#' block engine.
    # No injector: const/read injection is a later increment, so analyze() emits ONLY env-default
    # candidates (env delivery reaches the child through the launcher's env overlay, never the
    # injector — flows.execute only calls an injector for inject-delivery values, of which fish
    # has none). Pure stdlib scanner, so no import guard is needed.
    return replace(
        spec,
        params_io=ParamsIO(read=metawriter.read_params, write=metawriter.write_params),
        analyzer=Analyzer(analyze=analyzer.analyze, reconcile=analyzer.reconcile),
        cli_reader=CliReader(read_cli=cli_reader.read_cli),
    )


def _js_analysis(
    lang: str,
) -> tuple[Analyzer | None, CliReader | None, Injector | None, Callable[[str], list[str]] | None]:
    """The four tree-sitter-backed JS/TS capabilities for a `lang` ("js"/"ts"), behind ONE import
    guard: analyzer, parseArgs reader, injector and dep scanner are all grammar consumers, so they
    stand or fall together — a broken grammar wheel must not leave a half-capable kind behind. Each
    is bound to the kind's `lang` so ts parses under the TypeScript grammar and js under
    JavaScript."""
    try:
        from .javascript import analyzer, cli_reader, inject
    except ImportError:  # pragma: no cover — a broken/absent tree-sitter grammar wheel
        return None, None, None, None
    return (
        Analyzer(
            analyze=lambda text: analyzer.analyze(text, lang=lang),
            reconcile=lambda text, specs: analyzer.reconcile(text, specs, lang=lang),
        ),
        CliReader(read_cli=lambda text: cli_reader.read_cli(text, lang=lang)),
        Injector(inject=lambda request: inject.inject(request, lang=lang)),
        lambda text: analyzer.external_imports(text, lang=lang),
    )


def _javascript_spec(kind: str, glyph: str, extensions: tuple[str, ...], lang: str) -> LangSpec:
    """A js/ts LangSpec: the Tier-0 interpreted base (RunnerLaunch, `//`-comment) plus the full
    analysis stack (in-file [tool.skit] via the `//` block engine, analyzer, parseArgs reader,
    const injector) — everything but params_io behind the tree-sitter import guard."""
    from .javascript import io  # the '//' block engine is grammar-free (metawriter), always present

    spec = _interpreted(
        kind,
        glyph,
        "",
        extensions,
        ("node", "deno", "bun") if lang == "js" else (),
        "//",
        launch_strategy=launch.RunnerLaunch(),
    )
    analyzer_cap, cli_reader_cap, injector_cap, dep_scanner = _js_analysis(lang)
    return replace(
        spec,
        params_io=ParamsIO(read=io.read_params, write=io.write_params),
        analyzer=analyzer_cap,
        cli_reader=cli_reader_cap,
        injector=injector_cap,
        # Per-script npm deps (package.json + node_modules materialized next to the stored copy —
        # the PEP 723 analogue); the scanner suggests the list from the script's own imports.
        deps_flavor="npm",
        dep_scanner=dep_scanner,
    )


def _js_spec() -> LangSpec:
    return _javascript_spec("js", "✦", (".js", ".mjs", ".cjs"), "js")


def _ts_spec() -> LangSpec:
    return _javascript_spec("ts", "✧", (".ts", ".mts", ".cts"), "ts")


def _powershell_spec() -> LangSpec:
    # `pwsh -File` (explicit file semantics); powershell.exe users set
    # meta.interpreter — the strategy resolves whatever name is recorded.
    from .powershell import cli_reader

    spec = _interpreted(
        "powershell",
        "»",
        "pwsh",
        (".ps1",),
        ("pwsh", "powershell"),
        "#",
        prefix=("-File",),
    )
    # The `param()` block IS the CLI surface, read through PowerShell's own parser (a static
    # subprocess, stdlib only — no import guard needed; the reader degrades at run time when
    # no pwsh/powershell.exe is on PATH). No analyzer/injector: injection is out of scope for
    # PowerShell in v1 — the reader assembles real `-Name value` flags instead.
    return replace(spec, cli_reader=CliReader(read_cli=cli_reader.read_cli))


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
        placeholder_params=True,
    )


def _prompt_spec() -> LangSpec:
    # family "interpreted", NOT "template": a prompt has an original file, copy/reference
    # modes, and an editable stored body — has_original_file (family != "template") must
    # stay True or reference-mode removal/drift messaging lies. The placeholder form
    # surface comes from the placeholder_params trait instead. No analyzer capability, on
    # purpose (command-kind parity): detection is langs/prompt/analyzer.placeholder_names,
    # consumed directly by the add/params/plan surfaces, and `run --raw` then keeps the
    # form exactly as it does for command templates.
    return LangSpec(
        kind="prompt",
        family="interpreted",
        glyph="✎",
        launch=launch.PromptLaunch(),
        extensions=(".prompt.md", ".prompt"),
        stored_name="prompt.md",
        supports_modes=True,
        takes_argv=False,
        placeholder_params=True,
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
    "prompt": _prompt_spec,
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
    # Compound registered extensions first (".prompt.md"): Path.suffix sees only the last
    # dot component, so `review.prompt.md` reads as ".md" — unreachable through the plain
    # suffix map. Longest-first endswith on the whole filename keeps a future ".d.ts"-style
    # registration unambiguous, and single-suffix behavior below is untouched.
    lowered = path.name.lower()
    for ext, kind in _compound_extensions():
        if lowered.endswith(ext):
            return kind
    by_ext = _extension_map().get(path.suffix.lower())
    if by_ext is not None:
        return by_ext
    if path.is_file():
        program = shebang_program(path)
        if program is not None:
            by_shebang = _kind_for_program(program)
            if by_shebang is not None:
                return by_shebang
        if _is_executable_file(path):
            return "exe"
    return "unknown"


def kind_for_shebang(path: Path) -> str | None:
    """The kind a file's #! line names, or None. The one shebang→kind mapping, shared
    by every draft/authoring lane — a changed shebang is an explicit user signal, and
    the TUI and CLI faces must read it by the same rule."""
    program = shebang_program(path)
    if program is None:
        return None
    return _kind_for_program(program)


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
    return shebang_program_from_line(first.decode("utf-8", errors="replace"))


def shebang_program_from_line(line: str) -> str | None:
    """The path-less half of shebang_program, for text that isn't on disk yet (the
    stdin add lane) — SAME parsing, one rule."""
    if not line.startswith("#!"):
        return None
    tokens = line[2:].split()
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


def kind_for_shebang_text(text: str) -> str | None:
    """The kind a text blob's first line names (None = no/unregistered shebang) — the
    stdin twin of kind_for_shebang."""
    first = text.split("\n", 1)[0]
    program = shebang_program_from_line(first)
    if program is None:
        return None
    return _kind_for_program(program)


# Versioned interpreter names: `#!/usr/bin/env python3.12` is the same explicit signal
# as `python3` — uv/pyenv-provisioned scripts carry exact versions routinely. ONE rule
# here, so the stdin, editor and TUI draft lanes can't split into per-lane carve-outs
# (they did: two honored the spelling, one refused it). python2 is deliberately OUT:
# skit launches through `uv run --script` on python3 — claiming a python2 shebang would
# fabricate exactly the entry that can only die at run time (the --kind escape applies).
_VERSIONED_PYTHON = re.compile(r"python(3(\.\d+)*)?")
# The version half of that signal ("3.12" of python3.12) — honored as a requires-python
# default, never silently dropped (half-honoring an explicit signal is still a drop —
# which is why a micro version, python3.12.1, keeps its .1 in the lower bound too).
_PYTHON_MINOR = re.compile(r"python(3)\.(\d+)((?:\.\d+)*)")


def _kind_for_program(program: str) -> str | None:
    """The kind a shebang program name maps to — the registered table first, then the
    versioned-python rule. Every shebang→kind consumer goes through here."""
    kind = _shebang_map().get(program)
    if kind is not None:
        return kind
    if _VERSIONED_PYTHON.fullmatch(program):
        return "python"
    return None


def kind_for_draft(path: Path) -> str:
    """Kind inference for skit's OWN kept authoring drafts (paths.is_draft): the
    shebang the user wrote outranks the draft's suffix, because mkstemp picked the
    suffix BEFORE the user decided what to write — on a script draft the suffix is
    skit's artifact, not a user signal. The one exception is keyed on the rationale,
    not on suffix shape: an extension registered to a placeholder-bodied kind
    (.prompt.md / .prompt — the body is content fired at an agent, never fed to an
    interpreter) outranks the shebang, because that suffix encodes the user's own
    lane choice and a prompt body may legitimately open with a #! line it describes
    or transforms — the prompt authoring lanes never read the shebang, so neither
    may the resume seam. Then: a registered shebang names its kind; an unregistered
    one is "unknown" (the caller's --kind / kind-picker escape — never a fabricated
    python entry); no shebang at all falls back to plain inference, where the
    suffix is all there is. This is the same verdict the authoring lanes reached
    before the draft was kept, so a draft crossing the keep/resume seam can never
    change kind."""
    by_ext: str | None = None
    lowered = path.name.lower()
    for ext, kind in _compound_extensions():
        if lowered.endswith(ext):
            by_ext = kind
            break
    if by_ext is None:
        by_ext = _extension_map().get(path.suffix.lower())
    if by_ext is not None:
        spec = spec_for(by_ext)
        if spec is not None and spec.placeholder_params:
            return by_ext
    program = shebang_program(path)
    if program is None:
        return infer_kind(path)
    return _kind_for_program(program) or "unknown"


def python_version_pin(program: str | None) -> str:
    """The requires-python constraint a versioned python shebang implies
    ("python3.12" → ">=3.12,<3.13"), or "" for unversioned/absent/non-python
    programs. The kind half of `#!/usr/bin/env python3.12` and the version half are
    ONE explicit signal — honoring the first while dropping the second would run the
    script on whatever python uv happens to pick."""
    if program is None:
        return ""
    m = _PYTHON_MINOR.fullmatch(program)
    if m is None:
        return ""
    major, minor, micro = int(m.group(1)), int(m.group(2)), m.group(3)
    return f">={major}.{minor}{micro},<{major}.{minor + 1}"


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
def _compound_extensions() -> tuple[tuple[str, str], ...]:
    """Registered extensions with more than one dot component, longest first — the ones
    Path.suffix can't see (matched by filename endswith in infer_kind)."""
    out = [
        (ext, kind)
        for ext, kind in _extension_map().items()
        if ext.count(".") > 1  # pragma: no mutate — ">= 2" is the same predicate
    ]
    return tuple(sorted(out, key=lambda pair: len(pair[0]), reverse=True))


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
        # The fallback's exact case is unobservable — the extensions are compared case-insensitively
        # below (both sides lowercased) — so mutating only its case is an equivalent no-op; its
        # identity (spelling) stays pinned by test_win_pathext_fallback_recognizes_*.
        default_pathext = ".COM;.EXE;.BAT;.CMD"  # pragma: no mutate — case washed out by .lower()
        pathext = os.environ.get("PATHEXT") or default_pathext
        runnable = {ext.lower() for ext in pathext.split(";") if ext}
        return path.suffix.lower() in runnable
    return os.access(path, os.X_OK)
