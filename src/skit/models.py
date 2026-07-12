"""Data models: meta.toml (per-script, self-describing) and registry index entries.

meta.toml is the source of truth; registry.toml is only an index (rebuildable via doctor --rebuild).
"""

from __future__ import annotations

from dataclasses import dataclass
from datetime import UTC, datetime
from pathlib import Path
from typing import Any, Literal

from .i18n import gettext

SCHEMA_VERSION = 1

# Kind is an OPEN string — a registry key resolved via langs.registry.spec_for. Keeping the
# type open is the forward-compat contract: a meta written by a newer skit (an unknown kind)
# must still list/show/remove cleanly and fail a run with a clean LaunchError, never a parse
# error (see docs/design/multilang.md).
Kind = str
Mode = Literal["copy", "reference"]
Workdir = str  # "origin" | "store" | "invoke" | absolute path


class ScriptMetaError(ValueError):
    """meta.toml parsed as valid TOML but is missing a key ScriptMeta requires.

    Distinguished from a bare KeyError so store.py's corruption handling (which already treats
    malformed/unreadable meta.toml as "corrupt, skip and let doctor --rebuild report it") can catch
    this alongside tomllib.TOMLDecodeError instead of a crash escaping list/resolve/doctor.
    """


@dataclass
class ScriptMeta:
    """Maps to scripts/<slug>/meta.toml."""

    name: str
    kind: Kind
    mode: Mode = "copy"
    source: str = ""  # provenance: original path at add time (also for exe/command)
    source_hash: str = ""  # "sha256:…"; empty for command entries
    added_at: str = ""
    workdir: Workdir = "origin"
    description: str = ""
    template: str = ""  # command template when kind=command
    dependencies: list[str] | None = None  # recorded deps when reference mode can't inject PEP 723
    requires_python: str = ""  # same; in copy mode both are written into the copy's PEP 723 block
    params: list[str] | None = (
        None  # named placeholders in a command template (prompted before run)
    )
    # Declared parameter schema rows ([[parameters]]), for kinds whose declarations
    # can't live in a text body (exe, command). Raw TOML tables here — the model
    # boundary is params.ParamDecl.from_meta_dict, which is total on hand-edited
    # garbage. For template kinds, `params` above stays a write-through cache of the
    # placeholder names so an OLDER skit still prompts instead of running a template
    # with literal {holes} (downgrade safety; see docs/design/multilang.md).
    parameters: list[dict[str, Any]] | None = None
    schema: int = SCHEMA_VERSION

    def to_toml_dict(self) -> dict[str, Any]:
        d: dict[str, Any] = {
            "schema": self.schema,
            "name": self.name,
            "kind": self.kind,
            "mode": self.mode,
            "source": self.source,
            "source_hash": self.source_hash,
            "added_at": self.added_at,
            "workdir": self.workdir,
            "description": self.description,
        }
        if self.template:
            d["template"] = self.template
        if self.dependencies:
            d["dependencies"] = self.dependencies
        if self.requires_python:
            d["requires_python"] = self.requires_python
        if self.params:
            d["params"] = self.params
        if self.parameters:
            d["parameters"] = self.parameters
        return d

    @classmethod
    def from_toml_dict(cls, d: dict[str, Any]) -> ScriptMeta:
        """Validate at the model boundary: a meta.toml can be well-formed TOML yet structurally
        invalid (missing keys, or a scalar where a list is required). Every such case must raise
        ScriptMetaError — never a raw KeyError/TypeError — so store.py's corruption handling
        (_META_CORRUPTION) can catch it uniformly instead of it crashing list/resolve/doctor."""
        missing = [key for key in ("name", "kind") if key not in d]
        if missing:
            raise ScriptMetaError(
                gettext("meta.toml is missing required key(s): %(keys)s")
                % {"keys": ", ".join(missing)}
            )
        invalid = [key for key in ("name", "kind") if not isinstance(d[key], str)]
        invalid += [
            key
            for key in ("dependencies", "params", "parameters")
            if d.get(key) is not None and not isinstance(d[key], list)
        ]
        if invalid:
            raise ScriptMetaError(
                gettext("meta.toml has invalid type for key(s): %(keys)s")
                % {"keys": ", ".join(invalid)}
            )
        return cls(
            name=d["name"],
            kind=d["kind"],
            mode=d.get("mode", "copy"),
            source=d.get("source", ""),
            source_hash=d.get("source_hash", ""),
            added_at=d.get("added_at", ""),
            workdir=d.get("workdir", "origin"),
            description=d.get("description", ""),
            template=d.get("template", ""),
            dependencies=list(d["dependencies"]) if d.get("dependencies") else None,
            requires_python=d.get("requires_python", ""),
            params=list(d["params"]) if d.get("params") else None,
            # Non-dict rows (a hand-edited scalar in the array) are dropped here so every
            # downstream reader gets real tables; ParamDecl.from_meta_dict handles the rest.
            parameters=(
                [row for row in d["parameters"] if isinstance(row, dict)]
                if d.get("parameters")
                else None
            ),
            schema=d.get("schema", SCHEMA_VERSION),
        )


@dataclass
class RegistryEntry:
    """A single index row in registry.toml."""

    name: str
    slug: str
    kind: Kind
    description: str = ""


@dataclass
class Entry:
    """Combined view: index + full meta + directory path. The primary object of the Core API."""

    slug: str
    meta: ScriptMeta
    dir: Path

    @property
    def script_path(self) -> Path:
        """The in-store script in copy mode; the original path in reference mode."""
        if self.meta.mode == "reference":
            return Path(self.meta.source)
        # Lazy import: models is the data layer everything imports, so it must not pull the
        # registry (and through it the launch strategies) in at module load.
        from .langs.registry import stored_name

        return self.dir / stored_name(self.meta.kind)


def now_iso() -> str:
    return datetime.now(UTC).replace(microsecond=0).isoformat()


def slugify(name: str) -> str:
    out: list[str] = []
    prev_dash = False  # pragma: no mutate — falsy-equivalent init; `and out` guard hides it while out is empty
    for ch in name.strip().lower():
        if ch.isalnum():
            out.append(ch)
            prev_dash = False  # pragma: no mutate — falsy-equivalent; only read via truthiness
        elif not prev_dash and out:
            out.append("-")
            prev_dash = True
    slug = "".join(out).strip("-")  # pragma: no mutate — text already lowercased
    return slug or "script"
