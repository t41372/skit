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

from dataclasses import dataclass
from typing import TYPE_CHECKING, Literal, Protocol

if TYPE_CHECKING:
    from collections.abc import Callable
    from pathlib import Path

    from ..models import Entry
    from .python.analyzer import Analysis
    from .python.argspec import ArgSpec
    from .python.metawriter import ParamSpec
    from .python.reconcile import Report


class LaunchError(Exception):
    pass


class TargetMissingError(LaunchError):
    """The launch target (script file / executable) is gone from disk.

    A distinct type so `skit run` can map it to exit 127 (command not found, docker
    convention) while other skit-side failures map to 125 — scripts that themselves
    exit 1 stay distinguishable from skit failing to launch them at all."""


class NotExecutableError(LaunchError):
    """The exe target exists but has no execute permission (exit 126, docker convention)."""


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
    reconcile: Callable[[str, list[ParamSpec]], Report]


@dataclass(frozen=True)
class CliReader:
    """Static reader for the script's OWN argument parser (argparse tier)."""

    read_cli: Callable[[str], ArgSpec | None]


@dataclass(frozen=True)
class ParamsIO:
    """Read/write declared parameter definitions carried in the script text."""

    read: Callable[[str], list[ParamSpec]]
    write: Callable[[str, list[ParamSpec]], str]


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
