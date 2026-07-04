"""Store + Registry (Layer 0).

- Each script directory scripts/<slug>/ carries its own meta.toml (self-describing, C7).
- registry.toml is only an index; doctor_rebuild() can fully reconstruct it from the metas.
- All writes go through atomic replace.
- This module is fully headless and imports no CLI/TUI dependency.
"""

from __future__ import annotations

import hashlib
import shutil
import tomllib
from pathlib import Path
from typing import Any

from . import argstate, pep723
from .atomic import atomic_write_toml
from .i18n import gettext
from .models import Entry, Kind, Mode, ScriptMeta, now_iso, slugify
from .paths import registry_path, scripts_dir


class StoreError(Exception):
    pass


class NameConflictError(StoreError):
    pass


class NotFoundError(StoreError):
    pass


def _hash_file(path: Path) -> str:
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(65536), b""):
            h.update(chunk)
    return f"sha256:{h.hexdigest()}"


def _read_meta(entry_dir: Path) -> ScriptMeta:
    with open(entry_dir / "meta.toml", "rb") as f:
        return ScriptMeta.from_toml_dict(tomllib.load(f))


def _write_meta(entry_dir: Path, meta: ScriptMeta) -> None:
    atomic_write_toml(entry_dir / "meta.toml", meta.to_toml_dict())


def _load_registry() -> dict[str, dict[str, Any]]:
    path = registry_path()
    if not path.exists():
        return {}
    with open(path, "rb") as f:
        doc = tomllib.load(f)
    return doc.get("entries", {})


def _save_registry(entries: dict[str, dict[str, Any]]) -> None:
    atomic_write_toml(registry_path(), {"entries": entries})


def _unique_slug(base: str, existing: set[str]) -> str:
    slug = base
    i = 2
    while slug in existing:
        slug = f"{base}-{i}"
        i += 1
    return slug


def _extract_description(script_text: str) -> str:
    """Take the first line of the module docstring as a suggested description (empty on failure)."""
    import ast

    try:
        doc = ast.get_docstring(ast.parse(script_text))
    except SyntaxError:
        return ""
    if not doc:
        return ""
    return doc.strip().splitlines()[0].strip()


def add_python(
    source: Path,
    *,
    name: str | None = None,
    mode: Mode = "copy",
    description: str | None = None,
    workdir: str = "origin",
    dependencies: list[str] | None = None,
    requires_python: str = "",
) -> Entry:
    source = source.expanduser().resolve()
    if not source.is_file():
        raise StoreError(gettext("File not found: %(path)s") % {"path": str(source)})
    text = source.read_text(encoding="utf-8", errors="replace")
    final_name = name or source.stem
    desc = description if description is not None else _extract_description(text)
    # copy mode: dependency completion is written into the copy's PEP 723 block (comment-only, A5
    # compliant), so the copy is portable.
    # reference mode: never touch the original; record in meta, and launcher passes it via
    # --with/--python.
    inject = mode == "copy" and (dependencies or requires_python) and not pep723.has_block(text)
    meta = ScriptMeta(
        name=final_name,
        kind="python",
        mode=mode,
        source=str(source),
        source_hash=_hash_file(source),
        added_at=now_iso(),
        workdir="origin" if mode == "reference" else workdir,
        description=desc,
        dependencies=None if inject else (dependencies or None),
        requires_python="" if inject else requires_python,
    )
    entry = _add_entry(meta, payload=source if mode == "copy" else None)
    if inject:
        new_text = pep723.inject_block(text, dependencies or [], requires_python)
        (entry.dir / "script.py").write_text(new_text, encoding="utf-8")
    return entry


def add_exe(source: Path, *, name: str | None = None, description: str = "") -> Entry:
    source = source.expanduser().resolve()
    if not source.exists():
        raise StoreError(gettext("File not found: %(path)s") % {"path": str(source)})
    meta = ScriptMeta(
        name=name or source.stem,
        kind="exe",
        mode="reference",  # exe is always reference; we never copy the binary
        source=str(source),
        source_hash=_hash_file(source) if source.is_file() else "",
        added_at=now_iso(),
        workdir="origin",
        description=description,
    )
    return _add_entry(meta, payload=None)


def extract_placeholders(template: str) -> list[str]:
    """Extract {name} placeholders (deduped by order of appearance; {{ }} is an escape, ignored)."""
    import re

    seen: list[str] = []
    for m in re.finditer(r"(?<!\{)\{([a-zA-Z_][a-zA-Z0-9_]*)\}(?!\})", template):
        if m.group(1) not in seen:
            seen.append(m.group(1))
    return seen


def add_command(template: str, *, name: str, description: str = "") -> Entry:
    if not template.strip():
        raise StoreError(gettext("Command template must not be empty"))
    placeholders = extract_placeholders(template)
    meta = ScriptMeta(
        name=name,
        kind="command",
        mode="reference",
        source="",
        added_at=now_iso(),
        workdir="invoke",
        description=description,
        template=template,
        params=placeholders or None,
    )
    return _add_entry(meta, payload=None)


def _add_entry(meta: ScriptMeta, *, payload: Path | None) -> Entry:
    entries = _load_registry()
    if any(e["name"] == meta.name for e in entries.values()):
        raise NameConflictError(
            gettext("The name %(name)s is already taken (use --name to pick another)")
            % {"name": meta.name}
        )
    slug = _unique_slug(slugify(meta.name), set(entries))
    entry_dir = scripts_dir() / slug
    entry_dir.mkdir(parents=True, exist_ok=True)
    try:
        if payload is not None:
            # copy mode: copy the original verbatim (A5: never land a processed script)
            shutil.copy2(payload, entry_dir / "script.py")
        _write_meta(entry_dir, meta)
    except BaseException:
        shutil.rmtree(entry_dir, ignore_errors=True)
        raise
    entries[slug] = {"name": meta.name, "kind": meta.kind, "description": meta.description}
    _save_registry(entries)
    return Entry(slug=slug, meta=meta, dir=entry_dir)


def list_entries() -> list[Entry]:
    entries = _load_registry()
    out: list[Entry] = []
    for slug in sorted(entries):
        entry_dir = scripts_dir() / slug
        try:
            meta = _read_meta(entry_dir)
        except (OSError, tomllib.TOMLDecodeError):
            continue  # leave corrupt entries for doctor to handle
        out.append(Entry(slug=slug, meta=meta, dir=entry_dir))
    return out


def resolve(name_or_slug: str) -> Entry:
    entries = _load_registry()
    slug = None
    if name_or_slug in entries:
        slug = name_or_slug
    else:
        matches = [s for s, e in entries.items() if e["name"] == name_or_slug]
        if len(matches) == 1:
            slug = matches[0]
    if slug is None:
        raise NotFoundError(gettext("Script not found: %(name)s") % {"name": name_or_slug})
    entry_dir = scripts_dir() / slug
    return Entry(slug=slug, meta=_read_meta(entry_dir), dir=entry_dir)


def remove(name_or_slug: str) -> str:
    entry = resolve(name_or_slug)
    entries = _load_registry()
    entries.pop(entry.slug, None)
    _save_registry(entries)
    shutil.rmtree(entry.dir, ignore_errors=True)
    argstate.forget(entry.slug)  # drop the last-used values too
    return entry.meta.name


def update_dependencies(
    name_or_slug: str,
    dependencies: list[str],
    requires_python: str | None = None,
) -> Entry:
    """Update an entry's dependency record (meta.toml). In copy mode, also sync the copy's PEP 723
    block; in reference mode, only touch meta (the original can't be written, A7) and pass it via
    --with at run time."""
    entry = resolve(name_or_slug)
    meta = entry.meta
    meta.dependencies = dependencies or None
    if requires_python is not None:
        meta.requires_python = requires_python or ""
    _write_meta(entry.dir, meta)
    if meta.kind == "python" and meta.mode == "copy":
        from . import pep723

        script = entry.dir / "script.py"
        if script.exists():
            text = script.read_text(encoding="utf-8", errors="replace")
            script.write_text(
                pep723.set_dependencies(
                    text, dependencies, requires_python=meta.requires_python or ""
                ),
                encoding="utf-8",
            )
    return Entry(slug=entry.slug, meta=meta, dir=entry.dir)


def doctor_rebuild() -> tuple[int, list[str]]:
    """Rebuild the registry from each scripts/<slug>/meta.toml.

    Returns (count rebuilt, problems).
    """
    problems: list[str] = []
    entries: dict[str, dict[str, Any]] = {}
    root = scripts_dir()
    if root.exists():
        for entry_dir in sorted(p for p in root.iterdir() if p.is_dir()):
            try:
                meta = _read_meta(entry_dir)
            except FileNotFoundError:
                problems.append(
                    gettext("%(slug)s: meta.toml is missing; skipped") % {"slug": entry_dir.name}
                )
                continue
            except (OSError, tomllib.TOMLDecodeError) as exc:
                problems.append(
                    gettext("%(slug)s: meta.toml is corrupt (%(error)s); skipped")
                    % {"slug": entry_dir.name, "error": str(exc)}
                )
                continue
            if meta.mode == "reference" and meta.source and not Path(meta.source).exists():
                problems.append(
                    gettext("%(slug)s: the referenced source file is gone: %(path)s")
                    % {"slug": entry_dir.name, "path": meta.source}
                )
            entries[entry_dir.name] = {
                "name": meta.name,
                "kind": meta.kind,
                "description": meta.description,
            }
    _save_registry(entries)
    return len(entries), problems


# Type re-exports, so callers upstream only need to import store.
__all__ = [
    "Entry",
    "Kind",
    "NameConflictError",
    "NotFoundError",
    "ScriptMeta",
    "StoreError",
    "add_command",
    "add_exe",
    "add_python",
    "doctor_rebuild",
    "list_entries",
    "remove",
    "resolve",
]
