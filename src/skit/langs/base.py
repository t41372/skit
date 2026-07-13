"""Language registry core types (see docs/design/multilang.md).

LEAF module: imports stdlib/typing only (language types under TYPE_CHECKING), so every
other module — models, store, launcher, the per-language packages — can import it
without cycles.

The three axes the old ``Kind`` literal conflated are separated here:

- **kind** is an open ``str`` registry key (language identity). Old skit reading a meta
  with an unknown kind lists it fine and fails a run with a clean LaunchError — that
  forward-compat contract is why the type is open.
- **LaunchPayload** is the closed sum type (``ArgvLaunch | ShellLaunch``) where
  exhaustive matching actually pays off (run_entry's process spawn).
- **capabilities** are ``X | None`` fields on a frozen LangSpec: the call-site idiom is
  ``if spec.analyzer is None: <degrade>``, which the type checker narrows structurally.
  A capability object is a frozen record of functions rather than a Protocol — skit owns
  every implementation, so structural abstraction would add nothing but indirection.
"""

from __future__ import annotations

from dataclasses import dataclass, field
from typing import TYPE_CHECKING, Literal, Protocol

if TYPE_CHECKING:
    from collections.abc import Callable
    from pathlib import Path

    from ..analysis import Analysis, Report
    from ..models import Entry
    from ..params import ParamDecl
    from .python.argspec import ArgSpec


class LaunchError(Exception):
    pass


class TargetMissingError(LaunchError):
    """The launch target (script file / executable) is gone from disk.

    A distinct type so `skit run` can map it to exit 127 (command not found, docker
    convention) while other skit-side failures map to 125 — scripts that themselves
    exit 1 stay distinguishable from skit failing to launch them at all."""


class NotExecutableError(LaunchError):
    """The exe target exists but has no execute permission (exit 126, docker convention)."""


# ---------------------------------------------------------------- injection errors
#
# The neutral injection-failure family, shared by every injector (python's shim, shell's
# inject). It lives here — not in a language package — because `flows.execute` maps these
# onto its own failure codes (FAIL_BAD_VALUE / FAIL_DRIFT) for whatever language ran, and a
# second language importing the first one's exception module just to raise a drift error
# would be exactly the coupling the registry exists to prevent. `langs.python.shim` keeps
# re-exporting them under their historical `ShimError` / `ShimValueError` names.


class InjectError(Exception):
    """An injection TARGET could not be located, or two definitions claimed one call site:
    the script has drifted from its definitions. Callers map it to FAIL_DRIFT ("re-add /
    resync"), so nothing that a resync cannot fix may be reported through it directly —
    the two subclasses below carve out exactly those cases."""


class InjectValueError(InjectError):
    """A value couldn't be coerced to its parameter's declared type.

    Distinct from the base InjectError: that one means an injection *target* couldn't be
    located (the script drifted from its [tool.skit] definitions). Here the target was found
    just fine — only the value the user typed doesn't fit the declared int/float/bool type.
    Callers must not conflate the two: telling a user to "re-add" a script because they
    mistyped a number is both wrong and unhelpful (nothing about the source has drifted, so
    re-adding fixes nothing). Carries structured fields (value / type_name / param_name) so a
    caller can build its own value-specific message without re-parsing str(exc) — the str()
    form stays exactly "{value!r} -> {type_name}", matching the plain InjectError message a
    `_coerce` failure has always raised.
    """

    def __init__(self, value: str, type_name: str, param_name: str) -> None:
        self.value = value
        self.type_name = type_name
        self.param_name = param_name
        super().__init__(f"{value!r} -> {type_name}")


class InjectGapError(InjectError):
    """A positional gap in a multi-variable read (`read FIRST LAST`): a value was supplied
    for a later variable of the same call while an earlier one was left empty.

    One `read` consumes one LINE and splits it on IFS, so the injected values of a single
    call must form a contiguous prefix of its variables: there is no way to express "empty
    first field, filled second" in that line — the shell would simply hand the second value
    to the FIRST variable. That is the silent wrong-value binding risk #2 exists to prevent,
    so it is refused explicitly. It is a value-shape problem, not drift (nothing in the
    source changed), so callers map it to FAIL_BAD_VALUE and must not suggest a resync."""

    def __init__(self, empty: str, filled: str) -> None:
        self.empty = empty
        self.filled = filled
        super().__init__(f"{empty} < {filled}")


class InjectSyntaxError(InjectError):
    """The injected copy failed a post-injection syntax gate (offline re-parse, or the
    interpreter's own `-n` check). skit corrupted the script — a resync cannot fix that, so
    callers map it to FAIL_DRIFT's exit code but must NOT print the resync hint. Nothing is
    launched, and the temp copy is removed before it is raised."""


Family = Literal["interpreted", "binary", "template"]


@dataclass(frozen=True)
class ArgvLaunch:
    """An argv vector executed directly (no shell)."""

    argv: list[str]


@dataclass(frozen=True)
class ShellLaunch:
    """A command string executed through the platform shell (command templates)."""

    command: str


LaunchPayload = ArgvLaunch | ShellLaunch


class LaunchStrategy(Protocol):
    """How a kind turns an Entry into a running process. Required on every LangSpec."""

    def build(
        self,
        entry: Entry,
        extra: list[str],
        values: dict[str, str] | None,
        script_override: Path | None,
    ) -> LaunchPayload:
        """Assemble the launch payload; raises LaunchError family on failure."""
        ...

    def describe(
        self,
        entry: Entry,
        extra: list[str],
        values: dict[str, str] | None,
        script_override: Path | None,
    ) -> str:
        """A purely descriptive command line: no lookups, no checks, no side effects."""
        ...

    def target(self, entry: Entry) -> Path | None:
        """The launch target on disk, or None when the kind has no file target."""
        ...

    def preflight(self, entry: Entry) -> None:
        """Existence/executability checks that can run before values are collected."""
        ...


@dataclass(frozen=True)
class Analyzer:
    """Static candidate detection + drift reconciliation (the A2 shared decision set)."""

    analyze: Callable[[str], Analysis]
    reconcile: Callable[[str, list[ParamDecl]], Report]


@dataclass(frozen=True)
class CliReader:
    """Static reader for the script's OWN argument parser (argparse tier)."""

    read_cli: Callable[[str], ArgSpec | None]


@dataclass(frozen=True)
class InjectRequest:
    """Everything an injector needs, and nothing about the caller: the script text, the
    definitions, this run's values, the temp-file fallback directory, and the interpreter
    NAME the entry will actually run under ("" when the kind has none). Deliberately not an
    Entry — an injector is a pure function of the source and the values."""

    text: str
    specs: list[ParamDecl]
    values: dict[str, str]
    entry_dir: Path
    interpreter: str = ""


@dataclass(frozen=True)
class InjectResult:
    """The three delivery channels an injector may use, in one record:

    - ``path``: the injected temp copy to run instead of the stored script, or None when the
      language delivered every value without rewriting a single byte (shell's env channel).
    - ``env``: variables to overlay onto the child process environment.
    - ``warnings``: already-localized lines the caller emits (e.g. "$0 will be the temp copy").
    """

    path: Path | None = None
    env: dict[str, str] = field(default_factory=dict)
    warnings: list[str] = field(default_factory=list)


@dataclass(frozen=True)
class Injector:
    """Run-time value delivery for a kind: rewrite a temp copy, overlay the environment, or
    both. The caller (flows.execute) owns the temp file's lifetime and unlinks it."""

    inject: Callable[[InjectRequest], InjectResult]


@dataclass(frozen=True)
class Normalizer:
    """Opt-in source-idiom rewriting (A5 amendment): convert an inject-delivered parameter
    into an env-delivered one *permanently*, in the stored copy. Separate from Injector
    because it writes the user's file (a deliberate, explicit act) rather than a temp copy."""

    normalize: Callable[[str, list[str]], Normalization]


@dataclass(frozen=True)
class Normalization:
    """A normalizer's result: the new text, the names actually rewritten, and coded
    ``reason:name`` refusals the caller renders (same shape as analysis.EditResult warnings —
    the UI owns the human wording)."""

    text: str
    normalized: list[str] = field(default_factory=list)
    refused: list[str] = field(default_factory=list)


@dataclass(frozen=True)
class ParamsIO:
    """Read/write declared parameter definitions carried in the script text."""

    read: Callable[[str], list[ParamDecl]]
    write: Callable[[str, list[ParamDecl]], str]


@dataclass(frozen=True)
class CommentSyntax:
    """The line-comment shape that carries in-file metadata blocks."""

    prefix: str  # "#" or "//"


@dataclass(frozen=True)
class LangSpec:
    """One registered kind. Data + strategy + optional capabilities; frozen."""

    kind: str
    family: Family
    glyph: str
    launch: LaunchStrategy
    extensions: tuple[str, ...] = ()
    shebangs: tuple[str, ...] = ()  # program basenames recognized in a #! line
    default_interpreter: str = ""
    stored_name: str = ""  # in-store copy filename; "" = this kind is never copied
    comment: CommentSyntax | None = None
    params_io: ParamsIO | None = None
    analyzer: Analyzer | None = None
    cli_reader: CliReader | None = None
    injector: Injector | None = None  # run-time value delivery (temp-copy rewrite / env)
    normalizer: Normalizer | None = None  # opt-in source-idiom rewrite (const -> envdefault)
    supports_modes: bool = False  # copy/reference choice offered at add time
    supports_deps: bool = False  # package-dependency management (PEP 723 + uv)
    takes_argv: bool = True  # False: appended args are not this kind's interface

    @property
    def editable(self) -> bool:
        """Whether `skit edit` can open a stored text source for this kind."""
        return bool(self.stored_name)

    @property
    def has_original_file(self) -> bool:
        """Whether removal leaves an original file behind (everything but templates)."""
        return self.family != "template"
