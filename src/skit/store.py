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
import stat
import tomllib
from collections.abc import Callable, Iterator
from dataclasses import replace
from pathlib import Path
from typing import Any

from . import argstate, paths, pep723
from .atomic import advisory_file_lock, atomic_write_text_keep_mode, atomic_write_toml
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


def _hash_bytes(data: bytes) -> str:
    """Hash the exact payload snapshot already used for decoding and analysis."""
    return f"sha256:{hashlib.sha256(data).hexdigest()}"


def _read_meta(entry_dir: Path) -> ScriptMeta:
    with open(entry_dir / "meta.toml", "rb") as f:
        return ScriptMeta.from_toml_dict(tomllib.load(f))


def _write_meta(entry_dir: Path, meta: ScriptMeta) -> None:
    atomic_write_toml(entry_dir / "meta.toml", meta.to_toml_dict())


def _entry_lock_path(slug: str) -> Path:
    # Outside scripts/, never a child of the directory remove() deletes. Keeping the lock in
    # entry.dir would let rmtree unlink a live lock and a waiter acquire a replacement while
    # deletion is still in progress. doctor only scans scripts/ directories, so the
    # persistent lock directory is not an apparent entry either.
    return scripts_dir().parent / ".locks" / f"{slug}.meta.lock"


@contextlib.contextmanager
def _locked_entry(name_or_slug: str) -> Iterator[Entry]:
    """Yield fresh metadata while holding this entry's cross-process RMW lock.

    ``atomic_write_toml`` prevents torn TOML, but it cannot stop two setters from
    replacing each other's unrelated fields after both resolved the same old snapshot.
    Resolve once to locate the stable slug directory, acquire its lock, then resolve
    again so every writer mutates the latest committed metadata.
    """
    initial = resolve(name_or_slug)
    with advisory_file_lock(_entry_lock_path(initial.slug)):
        yield resolve(initial.slug)


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

    The persistent OS-backed lock is shared with config, per-entry metadata and JS
    installs; a crashed process releases it in the kernel without unlink races.
    """
    # Version the protocol path: released skit builds used registry.lock as an
    # O_EXCL lease and would stall for 30s then unlink a persistent native inode.
    # Different protocol versions cannot safely synchronize, but they must not
    # sabotage or impose a guaranteed delay on each other during a downgrade.
    with advisory_file_lock(registry_path().with_suffix(".native.lock")):
        yield


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
    drift apart.

    Skit's own kept drafts are the one exception: their suffix is mkstemp's artifact
    (a bash draft is still named skit-new-*.py), so a resumed draft is classified
    shebang-first (registry.kind_for_draft) — otherwise the SAME bytes were shell
    when authored and python when resumed, and the kept-draft advice ("add it with:
    skit add <path>") was itself the corrupting command."""
    if not force_exe and paths.is_draft(path):
        return registry.kind_for_draft(path)
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
    # The chokepoint belt (update_dependencies' rule, applied to the add-time writer
    # too): strip-and-drop empty entries, then refuse anything unparseable before a
    # block is built. Every shipped intake validates earlier — this line is what a
    # future caller can't forget.
    dependencies = [d.strip() for d in dependencies if d.strip()] if dependencies else None
    _validate_uv_metadata(registry.spec_for("python"), dependencies or [], requires_python)
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
    # deps were injected into the stored copy exactly when after_copy was set to write them;
    # derive the flag from that instead of tracking a redundant parallel boolean.
    deps_injected = after_copy is not None
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
    # Prompts are stored files, but their onboarding is not the generic interpreted-
    # script contract: it strictly decodes UTF-8, derives placeholder schema, pins the
    # prompt workdir and records runner/interpolation policy.  Keep that distinction at
    # the store chokepoint so a future CLI/TUI lane cannot silently create a malformed
    # prompt entry by calling the superficially compatible API.
    if kind == "prompt":
        raise StoreUsageError(gettext("Prompt entries must be added with add_prompt()."))
    spec = registry.spec_for(kind)
    # The or→and mutation of the next line is equivalent: no registered kind is non-interpreted
    # with a truthy stored_name (nor interpreted with a falsy one), so the three disjuncts can
    # never disagree between `or` and `and`.
    if spec is None or spec.family != "interpreted" or not spec.stored_name:  # pragma: no mutate
        raise StoreError(gettext("Unknown entry kind: %(kind)s") % {"kind": kind})
    source = source.expanduser().resolve()
    if not source.is_file():
        raise StoreError(gettext("File not found: %(path)s") % {"path": str(source)})
    text = source.read_text(encoding="utf-8", errors="replace")
    # The else literal is dead code: every interpreted kind reaching this line carries a
    # CommentSyntax, so `spec.comment is not None` is always true here.
    prefix = spec.comment.prefix if spec.comment is not None else "#"  # pragma: no mutate
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


_PROMPT_DESCRIPTION_LIMIT = 120


def prompt_description(text: str) -> str:
    """A prompt body's suggested description: its first non-empty line, minus markdown
    heading markers — the docstring analogue for markdown. Descriptions are discovery
    metadata, not a second copy of the prompt body, so cap an unusually long first line
    before it can flood add/list/Library surfaces."""
    for line in text.splitlines():
        stripped = line.strip().lstrip("#").strip()
        if stripped:
            if len(stripped) <= _PROMPT_DESCRIPTION_LIMIT:
                return stripped
            return stripped[: _PROMPT_DESCRIPTION_LIMIT - 1].rstrip() + "…"
    return ""


def add_prompt(
    source: Path,
    *,
    name: str | None = None,
    mode: Mode = "copy",
    description: str | None = None,
    managed: list[str] | None = None,
    runner: str = "",
    interpolate: bool = True,
) -> Entry:
    """Add a prompt entry (docs/design/prompt.md). Mirrors add_script's copy/reference
    semantics with the prompt kind's own defaults: workdir is PINNED to "invoke" in both
    modes (agents work on the repo the user is standing in, never the prompt file's
    directory), `managed` is the placeholder names the form asks for (None = every
    detected candidate — the CLI's tick step passes the kept subset), `runner` is the
    optional pinned PromptRunner name, and `interpolate=False` turns variable insertion
    off outright (nothing scanned, nothing managed, the body travels verbatim).

    Flood guard: `managed=None` (the auto path — `--no-input`, the TUI direct lane) caps
    at AUTO_MANAGE_LIMIT detections. A long prompt that trips more was clearly not
    written for insertion, and auto-managing hundreds of required fields would make the
    entry unrunnable; nothing is managed instead (an EXPLICIT `managed` list is always
    honored — the user asked)."""
    source = source.expanduser().resolve()
    if not source.is_file():
        raise StoreError(gettext("File not found: %(path)s") % {"path": str(source)})
    from .langs.prompt import analyzer as prompt_analyzer
    from .langs.prompt import text as prompt_text

    try:
        # Bytes and permissions belong to one open-file snapshot.  Reopening the path
        # for copy/stat would let an editor replacement change either fact between the
        # strict decode/hash and storage.
        with source.open("rb") as stream:
            raw = stream.read()
            source_mode = stat.S_IMODE(os.fstat(stream.fileno()).st_mode) & 0o777
        text = prompt_text.decode(raw, source)
    except prompt_text.PromptEncodingError as exc:
        # Validate before hashing, allocating an entry directory, or touching the
        # registry: invalid payload bytes are a clean all-or-nothing add refusal.
        raise StoreError(str(exc)) from exc
    except OSError as exc:
        raise StoreError(
            gettext("Can't read %(path)s: %(error)s")
            % {"path": str(source), "error": exc.strerror or str(exc)}
        ) from exc

    detected = prompt_analyzer.placeholder_names(text) if interpolate else []
    if not interpolate or (managed is None and len(detected) > prompt_analyzer.AUTO_MANAGE_LIMIT):
        resolved_managed: list[str] = []
    elif managed is None:
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
        source_hash=_hash_bytes(raw),
        added_at=now_iso(),
        workdir="invoke",
        description=desc,
        params=resolved_managed or None,
        runner=runner,
        interpolate=interpolate,
    )
    return _add_entry(
        meta,
        payload=None,
        payload_bytes=raw if mode == "copy" else None,
        payload_mode=source_mode if mode == "copy" else None,
    )


def write_prompt_managed(name_or_slug: str, managed: list[str]) -> Entry:
    """Persist a prompt entry's MANAGED placeholder list (meta `params`) — the names the
    run form asks for and the renderer fills; everything else in the body stays verbatim.
    Prompt-only: a command template's placeholder list comes from the template itself and
    is never written through here."""
    with _locked_entry(name_or_slug) as entry:
        if entry.meta.kind != "prompt":
            raise StoreUsageError(
                gettext("%(name)s isn't a prompt entry.") % {"name": entry.meta.name}
            )
        meta = entry.meta
        meta.params = managed or None
        _write_meta(entry.dir, meta)
        return Entry(slug=entry.slug, meta=meta, dir=entry.dir)


def write_prompt_interpolate(name_or_slug: str, interpolate: bool) -> Entry:
    """Flip a prompt entry's insertion master switch. The managed list is deliberately
    NOT cleared on off — switching back on restores exactly what was managed before."""
    with _locked_entry(name_or_slug) as entry:
        if entry.meta.kind != "prompt":
            raise StoreUsageError(
                gettext("%(name)s isn't a prompt entry.") % {"name": entry.meta.name}
            )
        meta = entry.meta
        meta.interpolate = interpolate
        _write_meta(entry.dir, meta)
        return Entry(slug=entry.slug, meta=meta, dir=entry.dir)


def write_prompt_runner(name_or_slug: str, runner: str) -> Entry:
    """Persist (or clear, when empty) a prompt entry's pinned runner name."""
    with _locked_entry(name_or_slug) as entry:
        if entry.meta.kind != "prompt":
            raise StoreUsageError(
                gettext("%(name)s isn't a prompt entry.") % {"name": entry.meta.name}
            )
        meta = entry.meta
        meta.runner = runner
        _write_meta(entry.dir, meta)
        return Entry(slug=entry.slug, meta=meta, dir=entry.dir)


_WORKDIR_LITERALS = ("origin", "store", "invoke")


def _normalized_workdir(entry: Entry, workdir: str) -> str:
    """Validate and normalize a workdir without mutating the entry."""
    value = workdir.strip()
    spec = registry.spec_for(entry.meta.kind)
    # Kind-aware, same rule as the settings radio: a command template has no "origin"
    # (no file) and a reference-only kind has no stored copy — confirming a policy
    # that silently resolves as something else is a label that lies.
    if value == "origin" and spec is not None and not spec.has_original_file:
        raise StoreUsageError(
            gettext("%(name)s has no original file — origin doesn't apply to its kind.")
            % {"name": entry.meta.name}
        )
    if value == "store" and spec is not None and not spec.stored_name:
        raise StoreUsageError(
            gettext("%(name)s has no stored copy — store doesn't apply to its kind.")
            % {"name": entry.meta.name}
        )
    if value not in _WORKDIR_LITERALS:
        expanded = Path(value).expanduser()
        if not value or not expanded.is_absolute():
            raise StoreUsageError(
                gettext("The working directory must be origin, store, invoke, or an absolute path.")
            )
        value = str(expanded)
    return value


def _normalized_interpreter(entry: Entry, interpreter: str) -> str:
    """Validate and normalize an interpreter pin without mutating the entry."""
    from .langs.registry import spec_for

    spec = spec_for(entry.meta.kind)
    if (
        spec is None
        or spec.family != "interpreted"
        # Kinds whose launch never reads meta.interpreter: python goes through uv's
        # PEP 723 machinery, prompts through a PromptRunner — a pin must not be
        # recorded where nothing reads it.
        or entry.meta.kind in ("python", "prompt")
    ):
        raise StoreUsageError(
            gettext("%(name)s doesn't run through a pinnable interpreter.")
            % {"name": entry.meta.name}
        )
    return interpreter.strip()


def update_launch_policy(
    name_or_slug: str,
    *,
    workdir: str | None = None,
    interpreter: str | None = None,
    template: str | None = None,
) -> Entry:
    """Validate every supplied launch-policy axis, then persist them in one meta write.

    The CLI deliberately permits these axes in one invocation. Treating them as one
    transaction prevents a later inapplicable value from leaving earlier values applied
    even though the command reports failure.
    """
    with _locked_entry(name_or_slug) as entry:
        template_value = entry.meta.template
        params_value = entry.meta.params
        if template is not None:
            if entry.meta.kind != "command":
                raise StoreUsageError(
                    gettext("%(name)s isn't a command entry.") % {"name": entry.meta.name}
                )
            if not template.strip():
                raise StoreError(gettext("Command template must not be empty"))
            template_value = template
            params_value = extract_placeholders(template) or None

        workdir_value = entry.meta.workdir
        if workdir is not None:
            workdir_value = _normalized_workdir(entry, workdir)

        interpreter_value = entry.meta.interpreter
        if interpreter is not None:
            interpreter_value = _normalized_interpreter(entry, interpreter)

        meta = replace(
            entry.meta,
            template=template_value,
            params=params_value,
            workdir=workdir_value,
            interpreter=interpreter_value,
        )
        _write_meta(entry.dir, meta)
        return Entry(slug=entry.slug, meta=meta, dir=entry.dir)


def write_workdir(name_or_slug: str, workdir: str) -> Entry:
    """Persist an entry's working-directory policy: origin | store | invoke | an
    absolute path — the launch policy every kind honors (launcher._resolve_workdir),
    previously writable only by hand-editing meta.toml."""
    return update_launch_policy(name_or_slug, workdir=workdir)


def write_interpreter(name_or_slug: str, interpreter: str) -> Entry:
    """Persist (or clear, when empty) an interpreted entry's interpreter/runtime pin
    (shell → the binary, js/ts → deno/bun/node). Refused for kinds that launch some
    other way — a pin must never be recorded where nothing reads it."""
    return update_launch_policy(name_or_slug, interpreter=interpreter)


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


def update_template(name_or_slug: str, template: str) -> Entry:
    """Rewrite a command entry's template — the actual program at the center of the
    kind, previously frozen forever at add time (the only fix was remove + re-add,
    destroying presets and history). Placeholders are re-extracted exactly like
    add_command; declared [[parameters]] rows for names that survive are kept."""
    return update_launch_policy(name_or_slug, template=template)


def _add_entry(
    meta: ScriptMeta,
    *,
    payload: Path | None,
    payload_bytes: bytes | None = None,
    payload_mode: int | None = None,
    after_copy: Callable[[Path], None] | None = None,
) -> Entry:
    if payload is not None and payload_bytes is not None:
        raise ValueError("payload and payload_bytes are mutually exclusive")
    if payload_mode is not None and payload_bytes is None:
        raise ValueError("payload_mode requires payload_bytes")
    with _registry_lock():
        entries = _load_registry()
        existing_slugs, existing_names = _fs_truth(entries)
        if meta.name in existing_names:
            raise NameConflictError(
                gettext("The name %(name)s is already taken — pick another name.")
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
            if payload_bytes is not None:
                # Prompt copy mode writes the same snapshot that was strictly decoded,
                # analyzed and hashed.  Create it no broader than the source snapshot:
                # os.open applies umask (which can only narrow), then chmod restores the
                # exact ordinary permission bits.  At no point does a private 0600 body
                # become the Path.write_bytes default 0666/0644.
                target = entry_dir / stored_name(meta.kind)
                if payload_mode is None:
                    target.write_bytes(payload_bytes)
                else:
                    fd = os.open(target, os.O_WRONLY | os.O_CREAT | os.O_EXCL, payload_mode)
                    with os.fdopen(fd, "wb") as stream:
                        stream.write(payload_bytes)
                    os.chmod(target, payload_mode)
            elif payload is not None:
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


def prompt_entries_pinned_to(runner: str) -> list[Entry]:
    """Prompt entries whose durable runner pin names ``runner``.

    Runner removal deliberately does not clear these references: a temporarily removed
    config row can be restored without losing the user's choice. Management surfaces use
    this query to warn about the launches the removal will block.
    """
    return [
        entry
        for entry in list_entries()
        if entry.meta.kind == "prompt" and entry.meta.runner == runner
    ]


def unmanaged_prompt_placeholders(entry: Entry) -> list[str]:
    """A prompt body's detected ``{{placeholders}}`` that are not yet managed, in order
    of first appearance. This is the ONE rule the surfaces agree on for "you typed a
    variable that isn't a field yet": `skit params` and Script settings already show it;
    the edit path uses it so a placeholder added by editing the body is offered for
    management, not silently dropped into the body as literal text.

    Empty for non-prompt kinds, an insertion-off prompt (its body travels verbatim, so
    nothing is a candidate), and an unreadable or missing body (existence/decoding
    refusals belong to preflight, never to a schema invented from replacement bytes)."""
    if entry.meta.kind != "prompt" or not entry.meta.interpolate:
        return []
    if not entry.script_path.exists():
        return []
    from .langs.prompt import analyzer as prompt_analyzer
    from .langs.prompt import text as prompt_text

    try:
        text = prompt_text.read(entry.script_path)
    except (OSError, prompt_text.PromptEncodingError):
        return []
    managed = set(entry.meta.params or [])
    return [name for name in prompt_analyzer.placeholder_names(text) if name not in managed]


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
    with _locked_entry(name_or_slug) as entry:
        spec = registry.spec_for(entry.meta.kind)
        if spec is not None and spec.deps_flavor == "npm":
            from .langs.base import NotExecutableError
            from .langs.javascript import deps as js_deps

            try:
                with js_deps._install_lock(entry.dir):
                    return _remove_locked_entry(entry)
            except NotExecutableError as exc:
                raise StoreError(str(exc)) from exc
        return _remove_locked_entry(entry)


def _remove_locked_entry(entry: Entry) -> str:
    # Lock order is entry → registry, matching rename/update_description. The
    # durable registry removal happens before rmtree, so a later waiter re-resolves
    # to NotFound instead of resurrecting an orphan meta.toml. npm entries additionally
    # hold their install lock before reaching this helper.
    with _registry_lock():
        entries = _load_registry()
        entries.pop(entry.slug, None)  # pragma: no mutate — TOCTOU defense, kept deliberately
        _save_registry(entries)
    shutil.rmtree(entry.dir, ignore_errors=True)
    if entry.dir.exists():
        # A held-open file (Windows) can make the best-effort rmtree a silent no-op —
        # and a later `doctor --rebuild` would then re-index the surviving meta.toml,
        # resurrecting the "removed" entry. Say so instead of reporting success; the
        # values file is deliberately kept so a doctor-restored entry keeps its state.
        raise StoreError(
            gettext(
                "%(name)s was removed from the library, but its files couldn't be fully "
                "deleted: %(path)s — close any program using them, then delete the folder "
                "(or run `skit doctor --rebuild` to restore the entry and retry)."
            )
            % {"name": entry.meta.name, "path": str(entry.dir)}
        )
    argstate.forget(entry.slug)  # drop the last-used values too
    return entry.meta.name


def effective_uv_metadata(entry: Entry) -> tuple[list[str], str]:
    """The dependencies and requires-python that actually govern a run: meta when it
    carries them, else — copy-mode python only — the stored copy's own PEP 723 block
    (the add-time deps_injected path deliberately leaves meta blank and makes the
    block the source of truth). Every surface that DISPLAYS or BASELINES the record
    must read this, never raw meta: showing "—" for a pin uv enforces is a lie, and
    treating a blank-reflected-from-meta field as user-cleared executes unpins and
    dependency wipes nobody asked for."""
    deps = list(entry.meta.dependencies or [])
    constraint = entry.meta.requires_python
    if (
        entry.meta.kind == "python"
        and entry.meta.mode == "copy"
        and (not deps or not constraint)
        and entry.script_path.exists()
    ):
        text = entry.script_path.read_text(encoding="utf-8", errors="replace")
        block = pep723.parse_block(text) or {}
        if not deps:
            deps = [str(d) for d in (block.get("dependencies") or [])]
        if not constraint:
            constraint = str(block.get("requires-python", "") or "")
    return deps, constraint


def _validate_uv_metadata(
    spec: registry.LangSpec | None, dependencies: list[str], requires_python: str | None
) -> None:
    """Validate-then-write at the ONE chokepoint every editing surface calls (`skit
    add`'s intakes validate earlier for their own refusal timing; `skit deps` and the
    settings screen land here): an unparseable requirement or constraint written into
    meta / the PEP 723 block bricks every subsequent run with uv's raw error. npm
    grammar belongs to the npm installer, so npm-flavor entries are not routed here."""
    if spec is not None and spec.deps_flavor == "npm":
        return
    for d in dependencies:
        if (error := pep723.requirement_error(d)) is not None:
            raise StoreUsageError(error)
    if requires_python and (error := pep723.requires_python_error(requires_python)) is not None:
        raise StoreUsageError(error)


def update_dependencies(
    name_or_slug: str,
    dependencies: list[str] | None,
    requires_python: str | None = None,
) -> Entry:
    """Update an entry's dependency record (meta.toml). Python copy mode also syncs the copy's
    PEP 723 block; python reference mode only touches meta (the original can't be written, A7)
    and passes it via --with at run time. An npm-flavor entry (js/ts) is copy-mode only — the
    engine materializes node_modules next to the stored copy, and a reference entry's script
    lives in its own project, whose node_modules already serves it — and a Python constraint
    is meaningless there, so both are refused loudly rather than recorded and ignored.

    BOTH axes distinguish untouched from cleared: None = don't touch (a python-only
    edit must not wipe deps; a deps-only edit must not unpin), [] / "" = explicitly
    clear. One rule, stated twice — the constraint axis learned it first, and leaving
    the deps axis on always-replace let `skit deps x --python …` erase block-only
    add-time dependencies under a green line."""
    with _locked_entry(name_or_slug) as entry:
        return _update_dependencies_entry(entry, dependencies, requires_python)


def _update_dependencies_entry(
    entry: Entry,
    dependencies: list[str] | None,
    requires_python: str | None,
) -> Entry:
    meta = entry.meta
    spec = registry.spec_for(meta.kind)
    if dependencies is not None:
        # Strip-and-drop empty entries BEFORE validating or writing: a whitespace-only
        # requirement is "nothing", not an error — and written verbatim it would brick
        # every run with uv's raw "Empty field" error (every shipped caller filters
        # already; the chokepoint must not rely on that).
        dependencies = [d.strip() for d in dependencies if d.strip()]
    uv_flavor = spec is None or spec.deps_flavor != "npm"
    if (
        uv_flavor
        and requires_python is not None
        and requires_python.strip().lower() in ("-", "none")
    ):
        # The add ask's own token for "automatic" — but only where a constraint can
        # exist at all: on an npm entry EVERY --python spelling is inapplicable, and
        # normalizing '-' first would make acceptance value-dependent (the refusal
        # says the flag "doesn't apply"; it must not apply for some spellings only).
        requires_python = ""
    _validate_uv_metadata(spec, dependencies or [], requires_python)
    if spec is not None and spec.deps_flavor == "npm":
        if requires_python is not None:
            # `is not None`, not truthiness (_refuse_unusable_add_flags' own
            # predicate): `--python ''` is a spelling too, and a flag the kind's
            # doctrine calls inapplicable must not apply for the empty spelling only.
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
    if (
        spec is not None
        and spec.deps_flavor == "npm"
        and dependencies is not None
        and not dependencies
    ):
        # Sweep node_modules only on an EXPLICIT clear ([]), never on None (untouched).
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
    if meta.kind == "python" and meta.mode == "copy":
        _refuse_unsyncable_block(entry, dependencies, requires_python)
    if dependencies is not None:
        meta.dependencies = dependencies or None
    if requires_python is not None:
        # Strip: a whitespace-only constraint ("   ") is truthy but an unparseable version
        # specifier that bricks every run — store "" (omitted) instead.
        meta.requires_python = (requires_python or "").strip()
    _write_meta(entry.dir, meta)
    if meta.kind == "python" and meta.mode == "copy":  # pragma: no mutate — and/or equivalent
        _sync_python_block(entry.script_path, meta, dependencies, requires_python)
    return Entry(slug=entry.slug, meta=meta, dir=entry.dir)


def _refuse_unsyncable_block(
    entry: Entry,
    dependencies: list[str] | None,
    requires_python: str | None,
) -> None:
    """Refuse an edit whose result skit could not actually deliver — BEFORE meta is written.

    A stored copy that isn't valid UTF-8 can't have its PEP 723 block rewritten (re-encoding
    an errors="replace" decode would swap every non-UTF-8 byte for U+FFFD, so add_python's
    rule is to leave the copy byte-exact). If that copy also HAS a block, the block is what
    uv reads, and meta cannot override it: an empty meta value means "untouched, defer to the
    block" everywhere, so there is no way to record a clear or an unpin at all. Letting the
    write through printed "Dependencies of x updated: —" while `skit show` and `uv run` both
    kept the old list — the exact false statement _sync_python_block's docstring forbids.
    Validate-then-write instead: nothing is committed, and the edit stays retryable."""
    if dependencies is None and requires_python is None:
        return  # nothing explicitly edited; the sync path has nothing to deliver either
    try:
        raw = entry.script_path.read_bytes()
    except OSError:
        return  # a missing/unreadable copy is the sync path's own no-op case
    try:
        raw.decode("utf-8")  # pragma: no mutate — utf-8/UTF-8 alias, and utf-8 is the default
    except UnicodeDecodeError:
        # The block fence and keys are ASCII, so a lossy decode is sound for DETECTION even
        # though it is not sound for rewriting.
        if pep723.has_block(raw.decode("utf-8", errors="replace")):  # pragma: no mutate — alias
            raise StoreUsageError(
                gettext(
                    "%(name)s's stored copy isn't valid UTF-8, so skit can't rewrite the "
                    "script's own dependency block — and that block is what uv reads. "
                    "Edit it in the script itself: skit edit %(name)s"
                )
                % {"name": entry.meta.name}
            ) from None


def _sync_python_block(
    script: Path,
    meta: ScriptMeta,
    dependencies: list[str] | None,
    requires_python: str | None,
) -> None:
    """Sync a copy-mode python entry's PEP 723 block after a metadata edit. BOTH axes
    share one derive rule: an untouched axis (None) whose meta carries nothing keeps
    the block's own value — the block is the source of truth for the add-time
    deps_injected split state (meta deliberately blank). An explicitly edited axis
    reaches the block uv actually reads: an unpin ("" via the '-' token) that left
    the block pinned was "updated: —" as a specific false statement on three
    surfaces at once, and a deps clear that left the block's list would be its twin."""
    if not script.exists():
        return
    try:
        # pragma: the surviving mutant is decode("UTF-8"), a genuine equivalent (codec
        # names are case-insensitive — codecs.lookup normalizes them); mirrors atomic.py's
        # utf-8/UTF-8 alias pragma. The co-generated decode("XXutf-8XX") is killable
        # (LookupError) and dies by coverage on the identical unpragma'd add_python:239.
        text = script.read_bytes().decode("utf-8")  # pragma: no mutate — utf-8/UTF-8 alias
    except (OSError, UnicodeDecodeError):
        # add_python's encoding rule, applied to the sync path too: re-encoding a lossy
        # errors="replace" decode would swap every non-UTF-8 byte in the copy for U+FFFD.
        # Leave the copy byte-exact; the edit is already in meta, which the launcher
        # passes via --with/--python exactly like a reference-mode entry. The case where
        # meta CAN'T stand in — a copy that carries its own block — never reaches here:
        # _refuse_unsyncable_block turned it away before meta was written.
        return
    block = pep723.parse_block(text) or {}
    constraint = meta.requires_python
    if not constraint and requires_python is None:
        constraint = str(block.get("requires-python", "") or "")
    block_deps = dependencies
    if block_deps is None:
        block_deps = list(meta.dependencies or []) or [
            str(d) for d in (block.get("dependencies") or [])
        ]
    # Atomic, mode-preserving: a plain write_text can tear the stored copy on a crash,
    # and a tmp+replace without the chmod would drop the bits copy2 preserved at add.
    atomic_write_text_keep_mode(
        script, pep723.set_dependencies(text, block_deps, requires_python=constraint)
    )


def update_needs(name_or_slug: str, needs: list[str]) -> Entry:
    """Update an entry's `needs` list (external commands checked on PATH before launch).
    Mirrors update_dependencies' meta write, but applies to every kind — a shell script
    or a command template can need `ffmpeg` just as a python script can. An empty list
    clears the key (stored as None so the meta stays minimal)."""
    with _locked_entry(name_or_slug) as entry:
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
    with _locked_entry(name_or_slug) as entry:
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
    new_name = new_name.strip()
    if not new_name:
        raise StoreError(gettext("A name is required."))
    with _locked_entry(name_or_slug) as entry:
        meta = entry.meta
        with _registry_lock():
            # The uniqueness decision sits INSIDE the registry lock: two entries renaming
            # to the same name concurrently each hold only their own entry lock, so a
            # pre-lock resolve() check lets both pass and both write — two entries, one
            # display name. The predicate restates resolve()'s matching (another slug
            # key, or another row's display name) against the locked snapshot.
            entries = _load_registry()
            taken = (new_name in entries and new_name != entry.slug) or any(
                s != entry.slug and e.get("name") == new_name for s, e in entries.items()
            )
            if taken:
                raise StoreError(
                    gettext("The name %(name)s is already taken.") % {"name": new_name}
                )
            meta.name = new_name
            _write_meta(entry.dir, meta)
            row = entries.get(entry.slug)
            if row is not None:
                row["name"] = new_name
                _save_registry(entries)
        return Entry(slug=entry.slug, meta=meta, dir=entry.dir)


def update_description(name_or_slug: str, description: str) -> Entry:
    """Update an entry's description (meta.toml is the truth; the registry index row is
    refreshed too so `list` doesn't need a rebuild to show it)."""
    with _locked_entry(name_or_slug) as entry:
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
