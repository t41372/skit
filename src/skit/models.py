"""Data models: meta.toml (per-script, self-describing) and registry index entries.

meta.toml is the source of truth; registry.toml is only an index (rebuildable via doctor --rebuild).
"""

from __future__ import annotations

from dataclasses import dataclass
from datetime import UTC, datetime
from pathlib import Path
from typing import Any, Literal

SCHEMA_VERSION = 1

Kind = Literal["python", "exe", "command"]
Mode = Literal["copy", "reference"]
Workdir = str  # "origin" | "store" | "invoke" | absolute path


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
        return d

    @classmethod
    def from_toml_dict(cls, d: dict[str, Any]) -> ScriptMeta:
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
        return self.dir / ("script.py" if self.meta.kind == "python" else "payload")


def now_iso() -> str:
    return datetime.now(UTC).replace(microsecond=0).isoformat()


def slugify(name: str) -> str:
    out: list[str] = []
    prev_dash = False
    for ch in name.strip().lower():
        if ch.isalnum():
            out.append(ch)
            prev_dash = False
        elif not prev_dash and out:
            out.append("-")
            prev_dash = True
    slug = "".join(out).strip("-")
    return slug or "script"
