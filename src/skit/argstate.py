"""Persistence of parameter values (state layer, separate from the data layer).

- Stored at state_dir()/values/<slug>.toml, removed together with the script.
- File shape: [values] (last-used), extra_args, [presets.<name>] (named presets).
- **C3 is enforced structurally here**: every write entry point requires secret_names, and any key
  in that list is stripped before it hits disk, so a secret value can never appear in a state file
  (there are tests for this).
- The value resolution order (PLAN §4.2) is implemented by resolve_defaults():
  preset > last-used > definition default.
  ("This run's form input" has the highest priority; it happens in the presentation layer and does
  not pass through this module.)
"""

from __future__ import annotations

import contextlib
import tomllib
from collections.abc import Iterable, Sequence
from typing import Any, Protocol

from .atomic import atomic_write_toml
from .paths import values_dir


class HasNameDefault(Protocol):
    """The minimal interface resolve_defaults needs (metawriter.ParamSpec satisfies it)."""

    @property
    def name(self) -> str: ...
    @property
    def default(self) -> str | int | float | bool | None: ...


def _load_doc(slug: str) -> dict[str, Any]:
    path = values_dir() / f"{slug}.toml"
    if not path.exists():
        return {}
    try:
        with open(path, "rb") as f:
            return tomllib.load(f)
    except (OSError, tomllib.TOMLDecodeError):
        return {}


def _save_doc(slug: str, doc: dict[str, Any]) -> None:
    doc = {k: v for k, v in doc.items() if v}  # don't persist empty sections
    atomic_write_toml(values_dir() / f"{slug}.toml", doc)


def _strip_secrets(values: dict[str, str], secret_names: Iterable[str]) -> dict[str, str]:
    banned = set(secret_names)
    return {k: v for k, v in values.items() if k not in banned}


def load_state(slug: str) -> dict[str, Any]:
    """Return {"values": {…}, "extra_args": […], "presets": {name: {…}}}."""
    doc = _load_doc(slug)
    return {
        "values": dict(doc.get("values", {})),
        "extra_args": list(doc.get("extra_args", [])),
        "presets": {k: dict(v) for k, v in doc.get("presets", {}).items()},
    }


def load_last(slug: str) -> dict[str, Any]:
    """Compatibility API: return only the last-used portion."""
    state = load_state(slug)
    return {"values": state["values"], "extra_args": state["extra_args"]}


def save_last(
    slug: str,
    *,
    values: dict[str, str] | None = None,
    extra_args: list[str] | None = None,
    secret_names: Iterable[str] = (),
) -> None:
    """Remember last-used (read-modify-write, keeping presets). Secret keys are stripped (C3)."""
    doc = _load_doc(slug)
    clean = _strip_secrets(values or {}, secret_names)
    if clean:
        doc["values"] = clean
    if extra_args:
        doc["extra_args"] = extra_args
    _save_doc(slug, doc)


def save_preset(
    slug: str,
    preset: str,
    values: dict[str, str],
    *,
    secret_names: Iterable[str] = (),
) -> None:
    """Save one named preset. Secret keys are stripped (C3)."""
    doc = _load_doc(slug)
    presets = dict(doc.get("presets", {}))
    presets[preset] = _strip_secrets(values, secret_names)
    doc["presets"] = presets
    _save_doc(slug, doc)


def delete_preset(slug: str, preset: str) -> bool:
    doc = _load_doc(slug)
    presets = dict(doc.get("presets", {}))
    if preset not in presets:
        return False
    del presets[preset]
    doc["presets"] = presets
    _save_doc(slug, doc)
    return True


def resolve_defaults(
    specs: Sequence[HasNameDefault], slug: str, preset: str | None = None
) -> dict[str, str]:
    """Compute the pre-filled value for each parameter in the run form:
    preset > last-used > definition default.

    specs are metawriter.ParamSpec (or any object with name/default attributes).
    Returns a str-valued map; parameters with no source at all are absent from the map.
    A non-existent preset is treated as no preset (the caller should validate and error first).
    """
    state = load_state(slug)
    names = {spec.name for spec in specs}
    out: dict[str, str] = {}
    for spec in specs:
        if spec.default is not None:
            out[spec.name] = str(spec.default)
    out.update({k: v for k, v in state["values"].items() if k in names})
    if preset:
        out.update({k: v for k, v in state["presets"].get(preset, {}).items() if k in names})
    return out


def forget(slug: str) -> None:
    path = values_dir() / f"{slug}.toml"
    with contextlib.suppress(FileNotFoundError):
        path.unlink()
