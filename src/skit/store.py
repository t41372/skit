"""Store + Registry (Layer 0).

- Each script directory scripts/<slug>/ carries its own meta.toml (self-describing, C7).
- registry.toml is only an index; doctor_rebuild() can fully reconstruct it from the metas.
- All writes go through atomic replace.
- This module is fully headless and imports no CLI/TUI dependency.
"""

from __future__ import annotations

import contextlib
import hashlib
import os
import shutil
import time
import tomllib
from collections.abc import Callable, Iterator
from pathlib import Path
from typing import Any

from . import argstate, pep723
from .atomic import atomic_write_toml
from .i18n import gettext
from .langs import registry
from .langs.registry import stored_name
from .models import Entry, Kind, Mode, ScriptMeta, ScriptMetaError, now_iso, slugify
from .params import ParamDecl, declared_from_meta
from .paths import registry_path, scripts_dir

# Corruption/error types every meta.toml reader must treat the same way: valid-but-unreadable file,
# invalid TOML, or valid TOML missing a required key are all "this entry is corrupt" — never a bare
# KeyError/OSError escaping to a caller that only handles store errors (models.py:64, store.py:210).
_META_CORRUPTION = (OSError, tomllib.TOMLDecodeError, ScriptMetaError)

# Registry read-modify-write lock (concurrency, store.py:181): a portable advisory lockfile via
# O_CREAT|O_EXCL, with retry + a stale-lock timeout so a crashed holder can't wedge the store
# forever. skit is a single-user CLI/TUI tool, so contention is rare and short-lived; this closes
# the remaining race left after the filesystem-truth fix below (_fs_truth) already prevents the
# worst case (a silent overwrite) even without a lock.
_LOCK_STALE_SECONDS = 30
_LOCK_POLL_SECONDS = 0.05


class StoreError(Exception):
    pass


class StoreUsageError(StoreError):
    """A refused request — an inapplicable flag or an operation the entry's kind/mode can't
    honor — as opposed to an operational failure (a locked file, a bad disk). The CLI maps it to
    the usage exit code so `skit deps` agrees with `skit add` on what a refusal looks like."""


class NameConflictError(StoreError):
    pass


class NotFoundError(StoreError):
    pass


def _hash_file(path: Path) -> str:
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(65536), b""):  # pragma: no mutate
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
    try:
        with open(path, "rb") as f:
            doc = tomllib.load(f)
    except (OSError, tomllib.TOMLDecodeError):
        # registry.toml is only a rebuildable index (module docstring), so degrade the same way a
        # missing file already does: an empty registry that `doctor --rebuild` can reconstruct from
        # the untouched scripts/<slug> metas. Preserve the bad bytes instead of discarding them
        # outright — rename (not copy) so a corrupt file can't keep re-triggering this branch (and
        # spawning a fresh backup) on every subsequent read before the next successful write.
        with contextlib.suppress(OSError):
            os.replace(path, path.with_name(f"{path.name}.corrupt"))
        return {}
    return doc.get("entries", {})


def _save_registry(entries: dict[str, dict[str, Any]]) -> None:
    atomic_write_toml(registry_path(), {"entries": entries})


@contextlib.contextmanager
def _registry_lock() -> Iterator[None]:
    """Serialize the registry read-modify-write + slug allocation across processes.

    A portable advisory lock (no fcntl/msvcrt split needed): an exclusive lockfile created with
    O_CREAT|O_EXCL. A holder that never releases it (crashed mid-operation) is reclaimed once the
    lockfile is older than _LOCK_STALE_SECONDS, so a dead process can't wedge the store forever.
    """
    lock_path = registry_path().with_suffix(".lock")
    lock_path.parent.mkdir(parents=True, exist_ok=True)
    while True:
        try:
            fd = os.open(lock_path, os.O_CREAT | os.O_EXCL | os.O_WRONLY)
        except FileExistsError:
            try:
                age = time.time() - lock_path.stat().st_mtime
            except OSError:
                age = _LOCK_STALE_SECONDS + 1  # vanished mid-check; treat as reclaimable
            if age > _LOCK_STALE_SECONDS:
                with contextlib.suppress(OSError):
                    lock_path.unlink()
            else:
                time.sleep(_LOCK_POLL_SECONDS)
            continue
        else:
            os.close(fd)
            break
    try:
        yield
    finally:
        with contextlib.suppress(OSError):
            lock_path.unlink()


def _unique_slug(base: str, existing: set[str]) -> str:
    slug = base
    i = 2
    while slug in existing:
        slug = f"{base}-{i}"
        i += 1
    return slug


def _fs_truth(entries: dict[str, dict[str, Any]]) -> tuple[set[str], set[str]]:
    """(taken slugs, taken names), cross-checked against the on-disk scripts/ directory.

    registry.toml is only a rebuildable index (module docstring) — trusting it alone for slug
    uniqueness / name-conflict checks means a lost or corrupt registry lets a name/slug collision
    silently overwrite an existing stored script (store.py:187). A directory is only counted as
    "taken" if it actually holds something (has any content): an empty leftover directory (e.g. from
    a process that mkdir'd but crashed before writing anything) claims no slug and stays reusable.
    """
    slugs = set(entries)
    names = {e["name"] for e in entries.values()}
    root = scripts_dir()
    if not root.is_dir():
        return slugs, names
    for entry_dir in root.iterdir():
        if not entry_dir.is_dir():
            continue
        in_registry = entry_dir.name in entries
        if not in_registry and not any(entry_dir.iterdir()):
            continue  # empty, unregistered leftover — nothing to protect, safe to reuse
        slugs.add(entry_dir.name)
        if in_registry:
            continue  # its name is already accounted for via the registry row
        try:
            names.add(_read_meta(entry_dir).name)
        except _META_CORRUPTION:
            continue  # unreadable; doctor --rebuild will report it, but it can't claim a name here
    return slugs, names


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


def infer_kind(path: Path, force_exe: bool = False) -> str:
    """What kind of entry a path should become. Delegates to the language registry
    (langs.registry.infer_kind) — kept as a store-level name because the CLI and the
    TUI add panel both resolve inference through the store, so the two paths can't
    drift apart."""
    return registry.infer_kind(path, force_exe=force_exe)


def suggest_description(script_text: str) -> str:
    """Public: the description skit would auto-derive from a script (its docstring's first line, or
    empty). Used by the interactive `add` prompt to prefill a suggested description."""
    return _extract_description(script_text)


def add_python(
    source: Path,
    *,
    name: str | None = None,
    mode: Mode = "copy",
    description: str | None = None,
    workdir: str | None = None,
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
    # compliant), so the copy is portable — but only when the source is strict-UTF-8: re-encoding a
    # lossy `errors="replace"` decode back to disk would corrupt any non-UTF-8 byte in the copy
    # (store.py:130). A source that doesn't decode cleanly falls back to recording the deps in meta
    # instead (same as reference mode) and leaves the copy byte-exact.
    try:
        strict_text: str | None = source.read_bytes().decode("utf-8")
    except UnicodeDecodeError:
        strict_text = None
    # reference mode: never touch the original; record in meta, and launcher passes it via
    # --with/--python.
    after_copy: Callable[[Path], None] | None = None
    deps_injected = False
    if (
        mode == "copy"
        and (dependencies or requires_python)
        and strict_text is not None
        and not pep723.has_block(strict_text)
    ):
        injected_text = pep723.inject_block(strict_text, dependencies or [], requires_python)

        def _write_injected(entry_dir: Path) -> None:
            (entry_dir / stored_name("python")).write_text(injected_text, encoding="utf-8")

        after_copy = _write_injected
        deps_injected = True
    if mode == "reference":
        resolved_workdir = "origin"
    elif workdir is not None:
        resolved_workdir = workdir
    else:
        # Copy mode exists specifically to decouple the entry from its original location, so its
        # default workdir must not depend on that location either (the gap: a copy-mode script
        # could never run again once its source directory was gone, even though the store copy was
        # intact). "invoke" (the caller's cwd at run time) always exists and mirrors add_command's
        # existing default for the same reason (store.py add_command); "store" (entry.dir) holds
        # only script.py + meta.toml, with no reason to assume a script's relative file operations
        # target it.
        resolved_workdir = "invoke"
    meta = ScriptMeta(
        name=final_name,
        kind="python",
        mode=mode,
        source=str(source),
        source_hash=_hash_file(source),
        added_at=now_iso(),
        workdir=resolved_workdir,
        description=desc,
        dependencies=None if deps_injected else (dependencies or None),
        requires_python="" if deps_injected else requires_python,
    )
    return _add_entry(meta, payload=source if mode == "copy" else None, after_copy=after_copy)


def extract_comment_description(text: str, prefix: str) -> str:
    """The first line of a leading comment block — the docstring analogue for comment
    languages. Skips the shebang and blank lines; stops at the first code line. A
    metadata-block opener (`# /// script`) is skipped rather than surfaced (it is
    machinery, not a description)."""
    for i, line in enumerate(text.splitlines()):
        stripped = line.strip()
        if i == 0 and stripped.startswith("#!"):
            continue
        if not stripped:
            continue
        if not stripped.startswith(prefix):
            return ""
        content = stripped[len(prefix) :].strip()
        if content.startswith("///"):
            continue  # a metadata fence, not prose
        if content:
            return content
    return ""


def add_script(
    source: Path,
    *,
    kind: str,
    name: str | None = None,
    mode: Mode = "copy",
    description: str | None = None,
    workdir: str | None = None,
    interpreter: str = "",
) -> Entry:
    """Add an interpreted (non-python) script: shell/fish/js/ts/powershell/ruby/….

    Mirrors add_python's copy/reference semantics: copy mode decouples the entry from
    its origin (verbatim byte copy, workdir defaults to "invoke"), reference mode never
    touches the original. The interpreter is recorded from the argument (usually the
    shebang's program via registry.shebang_program) so a #!/bin/zsh script keeps
    running under zsh even though the kind's default is bash."""
    spec = registry.spec_for(kind)
    if spec is None or spec.family != "interpreted" or not spec.stored_name:
        raise StoreError(gettext("Unknown entry kind: %(kind)s") % {"kind": kind})
    source = source.expanduser().resolve()
    if not source.is_file():
        raise StoreError(gettext("File not found: %(path)s") % {"path": str(source)})
    text = source.read_text(encoding="utf-8", errors="replace")
    prefix = spec.comment.prefix if spec.comment is not None else "#"
    desc = description if description is not None else extract_comment_description(text, prefix)
    # An EXPLICIT workdir wins in both modes (the docs/design/prompt.md amendment): the
    # prompt add path must pin "invoke" even for a reference-mode entry, or the agent
    # would launch in the prompt file's directory. No existing caller passes workdir at
    # all, so the reference default below is byte-for-byte preserved for them.
    if workdir is not None:
        resolved_workdir = workdir
    elif mode == "reference":
        resolved_workdir = "origin"
    else:
        resolved_workdir = "invoke"  # same decoupling rationale as add_python's copy mode
    meta = ScriptMeta(
        name=name or source.stem,
        kind=kind,
        mode=mode,
        source=str(source),
        source_hash=_hash_file(source),
        added_at=now_iso(),
        workdir=resolved_workdir,
        description=desc,
        interpreter=interpreter,
    )
    return _add_entry(meta, payload=source if mode == "copy" else None)


def prompt_description(text: str) -> str:
    """A prompt body's suggested description: its first non-empty line, minus markdown
    heading markers — the docstring analogue for markdown."""
    for line in text.splitlines():
        stripped = line.strip().lstrip("#").strip()
        if stripped:
            return stripped
    return ""


def add_prompt(
    source: Path,
    *,
    name: str | None = None,
    mode: Mode = "copy",
    description: str | None = None,
    managed: list[str] | None = None,
    runner: str = "",
) -> Entry:
    """Add a prompt entry (docs/design/prompt.md). Mirrors add_script's copy/reference
    semantics with the prompt kind's own defaults: workdir is PINNED to "invoke" in both
    modes (agents work on the repo the user is standing in, never the prompt file's
    directory), `managed` is the placeholder names the form asks for (None = every
    detected candidate — the CLI's tick step passes the kept subset), and `runner` is
    the optional pinned PromptRunner name."""
    source = source.expanduser().resolve()
    if not source.is_file():
        raise StoreError(gettext("File not found: %(path)s") % {"path": str(source)})
    text = source.read_text(encoding="utf-8", errors="replace")
    from .langs.prompt import analyzer as prompt_analyzer

    detected = prompt_analyzer.placeholder_names(text)
    if managed is None:
        resolved_managed = detected
    else:
        unknown = [n for n in managed if n not in detected]
        if unknown:
            raise StoreError(
                gettext("Not a placeholder in this prompt: %(names)s")
                % {"names": ", ".join(unknown)}
            )
        resolved_managed = [n for n in detected if n in set(managed)]  # body order, always
    desc = description if description is not None else prompt_description(text)
    meta = ScriptMeta(
        name=name or source.stem.removesuffix(".prompt"),
        kind="prompt",
        mode=mode,
        source=str(source),
        source_hash=_hash_file(source),
        added_at=now_iso(),
        workdir="invoke",
        description=desc,
        params=resolved_managed or None,
        runner=runner,
    )
    return _add_entry(meta, payload=source if mode == "copy" else None)


def write_prompt_managed(name_or_slug: str, managed: list[str]) -> Entry:
    """Persist a prompt entry's MANAGED placeholder list (meta `params`) — the names the
    run form asks for and the renderer fills; everything else in the body stays verbatim.
    Prompt-only: a command template's placeholder list comes from the template itself and
    is never written through here."""
    entry = resolve(name_or_slug)
    if entry.meta.kind != "prompt":
        raise StoreUsageError(gettext("%(name)s isn't a prompt entry.") % {"name": entry.meta.name})
    meta = entry.meta
    meta.params = managed or None
    _write_meta(entry.dir, meta)
    return Entry(slug=entry.slug, meta=meta, dir=entry.dir)


def write_prompt_runner(name_or_slug: str, runner: str) -> Entry:
    """Persist (or clear, when empty) a prompt entry's pinned runner name."""
    entry = resolve(name_or_slug)
    if entry.meta.kind != "prompt":
        raise StoreUsageError(gettext("%(name)s isn't a prompt entry.") % {"name": entry.meta.name})
    meta = entry.meta
    meta.runner = runner
    _write_meta(entry.dir, meta)
    return Entry(slug=entry.slug, meta=meta, dir=entry.dir)


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
        description=description,
    )
    meta.workdir = "origin"  # pragma: no mutate — explicit default, self-describing call site
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
        added_at=now_iso(),
        workdir="invoke",
        description=description,
        template=template,
        params=placeholders or None,
    )
    meta.source = ""  # pragma: no mutate — explicit default, self-describing call site
    return _add_entry(meta, payload=None)


def _add_entry(
    meta: ScriptMeta,
    *,
    payload: Path | None,
    after_copy: Callable[[Path], None] | None = None,
) -> Entry:
    with _registry_lock():
        entries = _load_registry()
        existing_slugs, existing_names = _fs_truth(entries)
        if meta.name in existing_names:
            raise NameConflictError(
                gettext("The name %(name)s is already taken (use --name to pick another)")
                % {"name": meta.name}
            )
        slug = _unique_slug(slugify(meta.name), existing_slugs)
        entry_dir = scripts_dir() / slug
        if entry_dir.exists() and any(entry_dir.iterdir()):
            # Defense in depth: _fs_truth already excludes any non-empty existing directory from
            # the slug candidates above, so this should be unreachable — but never silently reuse
            # (and overwrite) a directory that actually holds a stored script (store.py:187).
            raise StoreError(
                gettext("Refusing to reuse the existing, non-empty entry directory: %(path)s")
                % {"path": str(entry_dir)}
            )
        entry_dir.mkdir(parents=True, exist_ok=True)
        try:
            if payload is not None:
                # copy mode: copy the original verbatim (A5: never land a processed script)
                shutil.copy2(payload, entry_dir / stored_name(meta.kind))
            _write_meta(entry_dir, meta)
            if after_copy is not None:
                after_copy(entry_dir)
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
        except _META_CORRUPTION:
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
    try:
        meta = _read_meta(entry_dir)
    except _META_CORRUPTION as exc:
        raise NotFoundError(
            gettext("%(name)s: metadata is corrupt (%(error)s); run skit doctor --rebuild")
            % {"name": name_or_slug, "error": str(exc)}
        ) from exc
    return Entry(slug=slug, meta=meta, dir=entry_dir)


def remove(name_or_slug: str) -> str:
    entry = resolve(name_or_slug)
    with _registry_lock():
        entries = _load_registry()
        entries.pop(entry.slug, None)  # pragma: no mutate — TOCTOU defense, kept deliberately
        _save_registry(entries)
    shutil.rmtree(entry.dir, ignore_errors=True)
    argstate.forget(entry.slug)  # drop the last-used values too
    return entry.meta.name


def update_dependencies(
    name_or_slug: str,
    dependencies: list[str],
    requires_python: str | None = None,
) -> Entry:
    """Update an entry's dependency record (meta.toml). Python copy mode also syncs the copy's
    PEP 723 block; python reference mode only touches meta (the original can't be written, A7)
    and passes it via --with at run time. An npm-flavor entry (js/ts) is copy-mode only — the
    engine materializes node_modules next to the stored copy, and a reference entry's script
    lives in its own project, whose node_modules already serves it — and a Python constraint
    is meaningless there, so both are refused loudly rather than recorded and ignored."""
    entry = resolve(name_or_slug)
    meta = entry.meta
    spec = registry.spec_for(meta.kind)
    if spec is not None and spec.deps_flavor == "npm":
        if requires_python:
            raise StoreUsageError(
                gettext("A Python constraint doesn't apply to %(kind)s scripts.")
                % {"kind": meta.kind}
            )
        if dependencies and meta.mode != "copy":
            raise StoreUsageError(
                gettext(
                    "%(name)s is a reference-mode entry: it runs from its own project, which "
                    "already provides its packages. Dependency management applies to copies."
                )
                % {"name": meta.name}
            )
    if spec is not None and spec.deps_flavor == "npm" and not dependencies:
        # Sweep node_modules BEFORE writing meta. The disk cleanup is the step that can fail (a
        # locked file), so doing it first means a failure leaves BOTH the record and the tree
        # untouched — genuinely retryable, the "leave the entry unchanged" contract the TUI
        # relies on. (Adding/changing deps has no clear step; the launch path installs.) clear()
        # takes the entry's install lock so a concurrent run's installer can't have the tree
        # ripped out from under it and then stamp over the wreckage.
        from .langs.base import NotExecutableError
        from .langs.javascript import deps as js_deps

        try:
            js_deps.clear(entry.dir)
        except NotExecutableError as exc:
            raise StoreError(str(exc)) from exc
    meta.dependencies = dependencies or None
    if requires_python is not None:
        # Strip: a whitespace-only constraint ("   ") is truthy but an unparseable version
        # specifier that bricks every run — store "" (omitted) instead.
        meta.requires_python = (requires_python or "").strip()
    _write_meta(entry.dir, meta)
    if meta.kind == "python" and meta.mode == "copy":  # pragma: no mutate — and/or equivalent
        from . import pep723

        script = entry.script_path
        if script.exists():
            text = script.read_text(encoding="utf-8", errors="replace")
            script.write_text(
                pep723.set_dependencies(
                    text, dependencies, requires_python=meta.requires_python or ""
                ),
                encoding="utf-8",
            )
    return Entry(slug=entry.slug, meta=meta, dir=entry.dir)


def update_needs(name_or_slug: str, needs: list[str]) -> Entry:
    """Update an entry's `needs` list (external commands checked on PATH before launch).
    Mirrors update_dependencies' meta write, but applies to every kind — a shell script
    or a command template can need `ffmpeg` just as a python script can. An empty list
    clears the key (stored as None so the meta stays minimal)."""
    entry = resolve(name_or_slug)
    meta = entry.meta
    meta.needs = needs or None
    _write_meta(entry.dir, meta)
    return Entry(slug=entry.slug, meta=meta, dir=entry.dir)


def write_parameters(name_or_slug: str, decls: list[ParamDecl]) -> Entry:
    """Persist declared parameter rows to meta.toml [[parameters]] (the schema home for
    kinds without a text body — exe/command). The legacy `params` placeholder-name list
    is deliberately NOT derived from decls: the template is the source of truth for
    WHICH placeholders exist (extract_placeholders at add time), and keeping it
    untouched is what lets an older skit still prompt for every placeholder
    (downgrade safety) even when only some carry declared schema."""
    entry = resolve(name_or_slug)
    meta = entry.meta
    meta.parameters = [d.to_meta_dict() for d in decls] or None
    _write_meta(entry.dir, meta)
    return Entry(slug=entry.slug, meta=meta, dir=entry.dir)


def read_parameters(name_or_slug: str) -> list[ParamDecl]:
    """The declared [[parameters]] rows of an entry, as decls (nameless rows dropped)."""
    entry = resolve(name_or_slug)
    return declared_from_meta(entry.meta.parameters)


def rename(name_or_slug: str, new_name: str) -> Entry:
    """Rename an entry's display name. The slug is immutable after add — it keys the
    entry directory and the argstate values file, so keeping it means nothing moves on
    disk and remembered values/presets survive the rename."""
    entry = resolve(name_or_slug)
    new_name = new_name.strip()
    if not new_name:
        raise StoreError(gettext("A name is required."))
    try:
        other = resolve(new_name)
    except NotFoundError:
        other = None
    if other is not None and other.slug != entry.slug:
        raise StoreError(gettext("The name %(name)s is already taken.") % {"name": new_name})
    meta = entry.meta
    meta.name = new_name
    _write_meta(entry.dir, meta)
    with _registry_lock():
        entries = _load_registry()
        row = entries.get(entry.slug)
        if row is not None:
            row["name"] = new_name
            _save_registry(entries)
    return Entry(slug=entry.slug, meta=meta, dir=entry.dir)


def update_description(name_or_slug: str, description: str) -> Entry:
    """Update an entry's description (meta.toml is the truth; the registry index row is
    refreshed too so `list` doesn't need a rebuild to show it)."""
    entry = resolve(name_or_slug)
    meta = entry.meta
    meta.description = description
    _write_meta(entry.dir, meta)
    with _registry_lock():
        entries = _load_registry()
        row = entries.get(entry.slug)
        if row is not None:
            row["description"] = description
            _save_registry(entries)
    return Entry(slug=entry.slug, meta=meta, dir=entry.dir)


def doctor_rebuild() -> tuple[int, list[str]]:
    """Rebuild the registry from each scripts/<slug>/meta.toml.

    Returns (count rebuilt, problems).
    """
    problems: list[str] = []
    entries: dict[str, dict[str, Any]] = {}
    with _registry_lock():
        root = scripts_dir()
        if root.exists():
            for entry_dir in sorted(p for p in root.iterdir() if p.is_dir()):
                try:
                    meta = _read_meta(entry_dir)
                except FileNotFoundError:
                    problems.append(
                        gettext("%(slug)s: meta.toml is missing; skipped")
                        % {"slug": entry_dir.name}
                    )
                    continue
                except _META_CORRUPTION as exc:
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
def dir_size(path: Path) -> int:
    """Total bytes of the files under a directory (0 if it doesn't exist). The library
    disk-usage figure the health check shows — shared by `skit doctor` and the TUI."""
    total = 0
    if path.is_dir():
        for p in path.rglob("*"):
            if p.is_file():
                total += p.stat().st_size
    return total


def human_size(size: int) -> str:
    """Bytes as a compact human string (B/KB/MB/GB)."""
    value = float(size)
    for unit in ("B", "KB", "MB", "GB"):
        if value < 1024 or unit == "GB":
            return f"{value:.1f} {unit}" if unit != "B" else f"{int(value)} B"
        value /= 1024
    return f"{value:.1f} GB"  # pragma: no cover — loop always returns


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
    "add_script",
    "dir_size",
    "doctor_rebuild",
    "human_size",
    "list_entries",
    "read_parameters",
    "remove",
    "resolve",
    "update_needs",
    "write_parameters",
]
