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
import uuid
from collections.abc import Callable, Iterator
from dataclasses import replace
from pathlib import Path
from typing import Any

from . import argstate, paths, pep723
from .atomic import (
    advisory_file_lock,
    atomic_write_bytes_keep_mode,
    atomic_write_toml,
    try_advisory_file_lock,
)
from .i18n import gettext
from .langs import registry
from .langs.registry import stored_name
from .models import (
    Entry,
    EntrySummary,
    Kind,
    Mode,
    ScriptMeta,
    ScriptMetaError,
    now_iso,
    slugify,
)
from .params import ParamDecl, declared_from_meta
from .paths import registry_path, scripts_dir
from .rewrite import detect_newline, read_for_block_edit, restore_newline, write_block_edit

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


class NameConflictError(StoreUsageError):
    pass


class AmbiguousNameError(StoreUsageError):
    """Two or more entries carry the requested display name, so resolving by NAME would
    have to guess. Only hand-edited metadata can reach this state — add and rename both
    refuse a taken name — and the remedy is the slug, which never collides. A usage
    refusal like NameConflictError, its write-side twin: the request is answerable,
    just not as asked."""


class NotFoundError(StoreError):
    pass


class CorruptEntryError(StoreError):
    """An indexed entry exists, but its authoritative metadata cannot be read."""


class StaleEntryError(StoreError):
    """The slug still resolves — but to a DIFFERENT entry than the one the caller has
    been holding (its meta id no longer matches). A screen open across a remove +
    same-name re-add reaches this state: the address survived, the identity did not,
    and a write authorized against the old identity must fail closed instead of
    landing on the stranger. The remedy is reopening the screen on the current entry."""


def _hash_file(path: Path) -> str:
    h = hashlib.sha256()
    try:
        with path.open("rb") as f:
            for chunk in iter(lambda: f.read(65536), b""):  # pragma: no mutate
                h.update(chunk)
    except OSError as exc:
        raise StoreError(
            gettext("Can't read %(path)s: %(error)s")
            % {"path": str(path), "error": exc.strerror or str(exc)}
        ) from exc
    return f"sha256:{h.hexdigest()}"


def _hash_bytes(data: bytes) -> str:
    """Hash the exact payload snapshot already used for decoding and analysis."""
    return f"sha256:{hashlib.sha256(data).hexdigest()}"


def _read_meta(entry_dir: Path) -> ScriptMeta:
    with open(entry_dir / "meta.toml", "rb") as f:
        return ScriptMeta.from_toml_dict(tomllib.load(f))


def _write_meta(entry_dir: Path, meta: ScriptMeta) -> None:
    if not meta.id:
        # Identity is minted at the ONE door every meta write passes through: an add
        # reaches disk already stamped, an edit preserves what it read, and a meta from
        # before ids existed heals on its next write (never on a read — reads stay
        # reads). Mutating the caller's object is the point: the Entry handed back must
        # say exactly what disk now says. See models.ScriptMeta.id for what it guards.
        meta.id = uuid.uuid4().hex
    atomic_write_toml(entry_dir / "meta.toml", meta.to_toml_dict())


def _write_meta_and_row(entry_dir: Path, slug: str, meta: ScriptMeta) -> None:
    """Persist meta.toml, then re-project this entry's registry row in the same motion.

    One write path for every mutator (see the projection comment above _ROW_KEYS): the
    row carries the meta's mtime stamp, so any meta write leaves it stale until someone
    re-projects it, and that someone should be the writer — not the next listing's
    self-heal. Caller holds the entry lock; the registry lock nests inside it, the same
    order rename established. A slug missing from the registry (lost or corrupt-renamed
    index) is skipped: the index is rebuildable, and doctor owns rebuilding it.
    """
    _write_meta(entry_dir, meta)
    with _registry_lock():
        entries = _load_registry()
        if slug in entries:
            entries[slug] = _registry_row(meta, entry_dir)
            _save_registry(entries)


def entry_lock_path(slug: str) -> Path:
    """This entry's cross-process RMW lock file — THE serialization point for everything
    that mutates the entry or writes state in its name: every meta mutator holds it
    (_locked_entry), remove() holds it through registry removal, rmtree AND the state
    forget, and flows' post-acceptance persistence doors hold it across their whole
    verify-then-write transaction, so a remove or a secret transition can never slip
    between a door's identity check and its writes. The lock nests OUTSIDE both the
    registry lock and argstate's per-slug values lock (the order remove() set)."""
    # Outside scripts/, never a child of the directory remove() deletes. Keeping the lock in
    # entry.dir would let rmtree unlink a live lock and a waiter acquire a replacement while
    # deletion is still in progress. doctor only scans scripts/ directories, so the
    # persistent lock directory is not an apparent entry either.
    return scripts_dir().parent / ".locks" / f"{slug}.meta.lock"


_entry_lock_path = entry_lock_path  # internal name kept for the existing call sites


def _check_expected_id(entry: Entry, expected_id: str | None) -> None:
    """The write-authorization check (#39): a caller that has been HOLDING an entry
    names the identity it means to mutate, and a mismatch fails closed. None means
    the caller holds nothing (an immediate name-addressed command); an EMPTY string
    is a real expectation — "I hold an unstamped handle" — and meeting a stamped
    entry refuses, because the asymmetry proves the disk changed owners. Never
    soften "" to None (the `id or None` idiom was exactly the hole this closes:
    it switched the guard off for the one handle that most needed it)."""
    if expected_id is not None and entry.meta.id != expected_id:
        raise StaleEntryError(
            gettext("%(name)s changed while this edit was underway — reopen it and try again.")
            % {"name": entry.meta.name}
        )


@contextlib.contextmanager
def _locked_entry(name_or_slug: str, *, expected_id: str | None = None) -> Iterator[Entry]:
    """Yield fresh metadata while holding this entry's cross-process RMW lock.

    ``atomic_write_toml`` prevents torn TOML, but it cannot stop two setters from
    replacing each other's unrelated fields after both resolved the same old snapshot.
    Resolve once to locate the stable slug directory, acquire its lock, then resolve
    again so every writer mutates the latest committed metadata. ``expected_id`` is the
    holder's write authorization: when given, the LOCKED re-resolve must still carry
    that identity or the mutation fails closed (StaleEntryError) — the slug alone is an
    address, and an address can change hands while a screen sits open."""
    initial = resolve(name_or_slug)
    with advisory_file_lock(entry_lock_path(initial.slug)):
        entry = resolve(initial.slug)
        _check_expected_id(entry, expected_id)
        yield entry


def claim_identity(entry: Entry) -> Entry:
    """Compare-and-claim, the hold-start handshake: verify — under the entry lock —
    that the disk still holds THE ENTRY the caller resolved (exact id; an unstamped
    handle accepts only an unstamped disk), stamp a pre-id meta while the lock is
    held, and return the claimed handle. Every lane that keeps an Entry across
    user-paced time (a run, a form, a settings screen, an editor session, a
    confirmation ask) claims here so its later writes authorize by exact match.

    Never claim-by-address: a claim that merely re-resolved the slug would adopt
    whoever owns it NOW — a remove + same-name re-add between the caller's resolve
    and its claim would be silently blessed, and every guard after it would protect
    the stranger. A changed owner is a refusal (StaleEntryError); a vanished entry
    is honest NotFoundError.

    The stamp alone is best-effort: a library whose data dir cannot be written
    (or even locked) still gets a VERIFIED handle — unstamped, which post-run
    persistence then declines to trust (flows.persistence_target requires a stamped
    match; an unwritable-by-us library is not provably unwritable by others)."""
    try:
        with _locked_entry(entry.slug, expected_id=entry.meta.id) as fresh:
            if not fresh.meta.id:
                try:
                    _write_meta_and_row(fresh.dir, fresh.slug, fresh.meta)  # the door stamps
                except OSError:
                    # The write may have mutated the in-memory id before failing;
                    # answer with what the DISK says (atomic write: unchanged).
                    return resolve(entry.slug)
            return Entry(slug=fresh.slug, meta=fresh.meta, dir=fresh.dir)
    except OSError:
        # Could not even take the lock (a read-only .locks dir). Verify without it:
        # the comparison is still exact, only unserialized — and a lock directory
        # nobody can create is a library this process cannot mutate anyway.
        fresh = resolve(entry.slug)
        _check_expected_id(fresh, entry.meta.id)
        return fresh


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
    entries = doc.get("entries", {})
    if not isinstance(entries, dict):
        return {}
    # Chokepoint normalization: registry.toml is a file a person can edit, so
    # `entries.<slug>` may be a scalar rather than a table. Coercing it to an empty row
    # HERE — not in each consumer — keeps every face on one rule: the slug still
    # resolves (by slug; an empty row matches no name), still lists (the empty row
    # fails validation, so the meta answers), and the self-heal can still repair it.
    # Before this lived here, `skit list` degraded gracefully while `skit run <name>`
    # crashed with a TypeError on the same row.
    return {slug: (row if isinstance(row, dict) else {}) for slug, row in entries.items()}


def _save_registry(entries: dict[str, dict[str, Any]]) -> None:
    atomic_write_toml(registry_path(), {"entries": entries})


# The listing projection of a meta: exactly what a listing renders, and nothing else.
# `kind`, `mode` and the reference target are fixed at add time and never rewritten
# (nothing in this module assigns them after the add_* constructors) — but `mtime_ns`
# is part of the row too, so EVERY meta write invalidates the row wholesale, not just
# one that changes a listed field. Every mutator therefore re-projects the row in its
# own transaction (`_write_meta_and_row`; rename inlines the same steps around its
# uniqueness check): a mutator that skipped it would push a deferred registry rewrite
# onto the next listing via the self-heal, turning a read into a write for work the
# mutator could have absorbed. The self-heal remains for what no mutator can cover —
# hand-edited metas and rows written by older skits.
#
# `mtime_ns` is the meta.toml the row was projected FROM, and it is what makes the row
# trustworthy at all: meta.toml is a file skit's own docstrings acknowledge users hand
# edit, and a projection with no way to notice the original changed would show the old
# name and description forever (list and show disagreeing about the same entry, with
# doctor --rebuild the only cure). The listing already stats each meta to prove the
# entry exists, and the SAME stat carries st_mtime_ns — so freshness costs nothing the
# design was not already paying. A row whose stamp does not match the file is stale,
# whatever it says; a row with no stamp predates this projection. Either way the meta
# answers, and the row is rewritten once.
#
# `target` is omitted for anything with no launch target outside the store, which keeps
# the file small: every `resolve()` — so every run, show, edit, params, completion —
# parses the whole thing to answer one lookup, and carrying an absolute source path on
# all N rows doubled it (133 → 269 KiB at a thousand entries) and made resolve 44%
# slower, a worse trade than the listing win it paid for.
_ROW_KEYS = ("name", "kind", "description")


def _registry_row(
    meta: ScriptMeta, entry_dir: Path, *, mtime_ns: int | None = None
) -> dict[str, Any]:
    row: dict[str, Any] = {
        "name": meta.name,
        "kind": meta.kind,
        "mode": meta.mode,
        "description": meta.description,
        # The stamp has two modes, split by who is projecting:
        # - A WRITER omits mtime_ns and stats now: it holds the entry lock across
        #   write-then-project, so content and stamp cannot disagree. (The residual is
        #   a hand edit landing inside that locked microsecond window — stale until
        #   the next edit, the same residual as a forged timestamp.)
        # - A RE-DERIVING path (the listing fallback, repair, doctor --rebuild) passes
        #   the stamp it statted BEFORE reading the meta: a write landing mid-
        #   derivation then dates the stamp before the content, so the row can only
        #   read as stale and be re-derived — never trusted-but-wrong.
        "mtime_ns": (
            os.stat(entry_dir / "meta.toml").st_mtime_ns if mtime_ns is None else mtime_ns
        ),
    }
    if meta.mode == "reference":
        # Only a reference entry HAS a launch target outside the store; for a copied one
        # this would be pure provenance no listing reads. Written even when EMPTY (a
        # command template has no file target at all), because the key's presence is
        # what a reader checks: a reference row without it lost the one field that says
        # where the script is, and must fall back to the meta rather than resolve to
        # Path("") — which is the current directory, and exists.
        row["target"] = meta.source
    return row


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
    # .get, not [] — the chokepoint (_load_registry) coerces a hand-edited scalar row to
    # {}, and an empty or mangled row claims no name (its slug is already counted above).
    names = {name for e in entries.values() if isinstance(name := e.get("name"), str)}
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
        if in_registry and isinstance(entries[entry_dir.name].get("name"), str):
            continue  # its name is already accounted for via the registry row
        # No row, or a row the chokepoint emptied: the meta answers — a mangled row
        # must not strip its entry's display name of collision protection.
        try:
            names.add(_read_meta(entry_dir).name)
        except _META_CORRUPTION:
            continue  # unreadable; doctor --rebuild will report it, but it can't claim a name here
    return slugs, names


def unindexed_slugs() -> list[str]:
    """Stored entries that exist under scripts/ but are absent from the index.

    The READ side of the cross-check _fs_truth already performs for writes. The index is
    only a rebuildable projection (module docstring), and _fs_truth trusts disk over it so
    a lost registry can't let `add` overwrite a stored script — but nothing ever told the
    USER about the same divergence. A missing or corrupt registry.toml made every entry
    vanish from `list`, `run` and `show` while `doctor` cheerfully reported a healthy
    library of zero entries, and the one command that repairs it (`doctor --rebuild`) was
    named nowhere. A promise the code makes to itself is not a promise to the user.

    Only directories that hold a meta.toml count: that is precisely what doctor_rebuild can
    recover. A directory without one is a crashed-mid-add leftover, not a lost entry.
    """
    entries = _load_registry()
    root = scripts_dir()
    if not root.is_dir():
        return []
    return sorted(
        p.name
        for p in root.iterdir()
        if p.is_dir() and p.name not in entries and (p / "meta.toml").exists()
    )


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
    if not source.exists():
        raise NotFoundError(gettext("File not found: %(path)s") % {"path": str(source)})
    if not source.is_file():
        raise StoreUsageError(gettext("Not a file: %(path)s") % {"path": str(source)})
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
    # Fold to LF for the block engine (its fences match on "\n" only) and remember the
    # source's own style for the write-back: a CRLF script whose text was handed to the
    # engine raw looked blockless, so `has_block` said False and skit injected a SECOND
    # `# /// script` block on top of the existing one.
    source_raw = source.read_bytes()
    source_newline = detect_newline(source_raw)
    try:
        strict_text: str | None = source_raw.decode("utf-8")
    except UnicodeDecodeError:
        strict_text = None
    else:
        strict_text = strict_text.replace("\r\n", "\n").replace("\r", "\n")
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
            # write_bytes with the style restored, not write_text: the latter re-expands
            # every "\n" to os.linesep, which on Windows rewrote an LF script's whole file
            # to CRLF for the sake of a comment block.
            (entry_dir / stored_name("python")).write_bytes(
                restore_newline(injected_text, source_newline).encode("utf-8")
            )

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
        raise StoreUsageError(gettext("Unknown entry kind: %(kind)s") % {"kind": kind})
    source = source.expanduser().resolve()
    if not source.exists():
        raise NotFoundError(gettext("File not found: %(path)s") % {"path": str(source)})
    if not source.is_file():
        raise StoreUsageError(gettext("Not a file: %(path)s") % {"path": str(source)})
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
    if not source.exists():
        raise NotFoundError(gettext("File not found: %(path)s") % {"path": str(source)})
    if not source.is_file():
        raise StoreUsageError(gettext("Not a file: %(path)s") % {"path": str(source)})
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
        raise StoreUsageError(str(exc)) from exc
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


def write_prompt_managed(
    name_or_slug: str, managed: list[str], *, expected_id: str | None = None
) -> Entry:
    """Persist a prompt entry's MANAGED placeholder list (meta `params`) — the names the
    run form asks for and the renderer fills; everything else in the body stays verbatim.
    Prompt-only: a command template's placeholder list comes from the template itself and
    is never written through here."""
    with _locked_entry(name_or_slug, expected_id=expected_id) as entry:
        if entry.meta.kind != "prompt":
            raise StoreUsageError(
                gettext("%(name)s isn't a prompt entry.") % {"name": entry.meta.name}
            )
        meta = entry.meta
        meta.params = managed or None
        _write_meta_and_row(entry.dir, entry.slug, meta)
        return Entry(slug=entry.slug, meta=meta, dir=entry.dir)


def write_prompt_interpolate(
    name_or_slug: str, interpolate: bool, *, expected_id: str | None = None
) -> Entry:
    """Flip a prompt entry's insertion master switch. The managed list is deliberately
    NOT cleared on off — switching back on restores exactly what was managed before."""
    with _locked_entry(name_or_slug, expected_id=expected_id) as entry:
        if entry.meta.kind != "prompt":
            raise StoreUsageError(
                gettext("%(name)s isn't a prompt entry.") % {"name": entry.meta.name}
            )
        meta = entry.meta
        meta.interpolate = interpolate
        _write_meta_and_row(entry.dir, entry.slug, meta)
        return Entry(slug=entry.slug, meta=meta, dir=entry.dir)


def write_prompt_runner(name_or_slug: str, runner: str, *, expected_id: str | None = None) -> Entry:
    """Persist (or clear, when empty) a prompt entry's pinned runner name."""
    with _locked_entry(name_or_slug, expected_id=expected_id) as entry:
        if entry.meta.kind != "prompt":
            raise StoreUsageError(
                gettext("%(name)s isn't a prompt entry.") % {"name": entry.meta.name}
            )
        meta = entry.meta
        meta.runner = runner
        _write_meta_and_row(entry.dir, entry.slug, meta)
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
    expected_id: str | None = None,
) -> Entry:
    """Validate every supplied launch-policy axis, then persist them in one meta write.

    The CLI deliberately permits these axes in one invocation. Treating them as one
    transaction prevents a later inapplicable value from leaving earlier values applied
    even though the command reports failure.
    """
    with _locked_entry(name_or_slug, expected_id=expected_id) as entry:
        template_value = entry.meta.template
        params_value = entry.meta.params
        if template is not None:
            if entry.meta.kind != "command":
                raise StoreUsageError(
                    gettext("%(name)s isn't a command entry.") % {"name": entry.meta.name}
                )
            if not template.strip():
                raise StoreUsageError(gettext("Command template must not be empty"))
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
        _write_meta_and_row(entry.dir, entry.slug, meta)
        return Entry(slug=entry.slug, meta=meta, dir=entry.dir)


def write_workdir(name_or_slug: str, workdir: str, *, expected_id: str | None = None) -> Entry:
    """Persist an entry's working-directory policy: origin | store | invoke | an
    absolute path — the launch policy every kind honors (launcher._resolve_workdir),
    previously writable only by hand-editing meta.toml."""
    return update_launch_policy(name_or_slug, workdir=workdir, expected_id=expected_id)


def write_interpreter(
    name_or_slug: str, interpreter: str, *, expected_id: str | None = None
) -> Entry:
    """Persist (or clear, when empty) an interpreted entry's interpreter/runtime pin
    (shell → the binary, js/ts → deno/bun/node). Refused for kinds that launch some
    other way — a pin must never be recorded where nothing reads it."""
    return update_launch_policy(name_or_slug, interpreter=interpreter, expected_id=expected_id)


def add_exe(source: Path, *, name: str | None = None, description: str = "") -> Entry:
    source = source.expanduser().resolve()
    if not source.exists():
        raise NotFoundError(gettext("File not found: %(path)s") % {"path": str(source)})
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
        raise StoreUsageError(gettext("Command template must not be empty"))
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


def update_template(name_or_slug: str, template: str, *, expected_id: str | None = None) -> Entry:
    """Rewrite a command entry's template — the actual program at the center of the
    kind, previously frozen forever at add time (the only fix was remove + re-add,
    destroying presets and history). Placeholders are re-extracted exactly like
    add_command; declared [[parameters]] rows for names that survive are kept."""
    return update_launch_policy(name_or_slug, template=template, expected_id=expected_id)


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
        entries[slug] = _registry_row(meta, entry_dir)
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


def _summary_from_row(
    slug: str, row: object, entry_dir: Path, meta_mtime_ns: int
) -> EntrySummary | None:
    """An EntrySummary from an index row, or None when the row can't supply one.

    None is not an error — it means "this row cannot be trusted": its stamp does not
    match the meta on disk (a hand edit, an older skit, or a projection this code has
    since stopped writing), or a hand edit broke its shape. The caller re-reads that
    entry's meta.toml, which is always the truth. Rows are only ever written by
    `_registry_row`, so in an untouched store this skit wrote the answer is never None.

    `row` is typed `object` as defense in depth behind `_load_registry`'s chokepoint
    normalization: registry.toml is a file a person can edit.
    """
    if not isinstance(row, dict):
        return None
    if row.get("mtime_ns") != meta_mtime_ns:
        # The meta changed after this row was projected from it (or the row predates
        # the stamp). Serving it anyway would show a hand-edited name or description
        # forever — `list` and `show` disagreeing about the same entry — and would
        # trust a projection of content that no longer exists. This is also the
        # corruption filter: breaking a meta changes its mtime, so the fallback
        # re-reads it, fails to parse, and skips the entry exactly as list_entries
        # does. (An edit that deliberately preserves mtime defeats both; that is a
        # forged timestamp, not a failure mode this index arbitrates.)
        return None
    name, kind, description = (row.get(key) for key in _ROW_KEYS)
    if not (isinstance(name, str) and isinstance(kind, str) and isinstance(description, str)):
        return None
    stored_mode = row.get("mode")
    mode: Mode
    if stored_mode == "copy":
        mode = "copy"
    elif stored_mode == "reference":
        mode = "reference"
    else:
        return None
    # Presence, not truthiness: a reference row must SAY where its script is, even when
    # the answer is "nowhere on disk" (a command template). Defaulting a missing key to
    # "" would resolve the entry to Path(""), which is the current directory and exists,
    # so a hand-broken row would report a deleted original as healthy instead of falling
    # back to the meta. A copy-mode row carries no target and needs none — its script is
    # in the store, at a path derived from the kind.
    target = row.get("target", "")
    if not isinstance(target, str) or (mode == "reference" and "target" not in row):
        return None
    if mode == "reference" and not target:
        # An EMPTY target is a real answer only for a kind with no file to launch (a
        # command template). For a file kind it is the same Path("") trap as a missing
        # key — a hand-emptied value would report a deleted original as healthy — so
        # the meta answers. Unknown kinds (a newer skit's meta) fall back too: this
        # version cannot know whether "" is honest for them.
        spec = registry.spec_for(kind)
        if spec is None or spec.has_original_file:
            return None
    return EntrySummary(
        slug=slug,
        name=name,
        kind=kind,
        mode=mode,
        description=description,
        dir=entry_dir,
        target=target,
    )


def _summary_from_meta(slug: str, meta: ScriptMeta, entry_dir: Path) -> EntrySummary:
    return EntrySummary(
        slug=slug,
        name=meta.name,
        kind=meta.kind,
        mode=meta.mode,
        description=meta.description,
        dir=entry_dir,
        target=meta.source if meta.mode == "reference" else "",
    )


def list_summaries() -> list[EntrySummary]:
    """Every entry, listing-shaped, served from registry.toml.

    The point is what it does NOT do: `list_entries` opens and parses one meta.toml per
    entry, which at a thousand entries is a thousand file reads on a path an agent calls
    to see what exists. Every field a listing renders is already in the index — fixed at
    add time (kind/mode/target) or refreshed by the mutator that changes it
    (name/description) — so the common case reads one file total.

    A row this skit could not have written falls back to that entry's meta, which is
    the truth: a row from an older store, one whose stamp says the meta changed under
    it (a hand edit — meta.toml is a file users edit, and the pre-index listing always
    reflected that), or one a hand edit broke. Either way the row is then repaired, so
    the fallback is paid once per change rather than forever.

    An entry with NO meta.toml is not listed, whatever the index says; a corrupt meta
    is skipped when its row forces the fallback (and breaking a meta changes its mtime,
    so it does). One stat per entry buys all of that, and it is the same stat the
    freshness check needs — without it the CLI would list entries the TUI, doctor and
    `run` (which read the metas) all refuse, three faces disagreeing about what the
    library contains. `doctor --rebuild` still reconstructs the whole index.
    """
    entries = _load_registry()
    root = scripts_dir()
    out: list[EntrySummary] = []
    stale: list[str] = []
    for slug in sorted(entries):
        entry_dir = root / slug
        try:
            meta_mtime_ns = os.stat(entry_dir / "meta.toml").st_mtime_ns
        except OSError:
            continue  # storage is gone; the index is stale and doctor owns that
        summary = _summary_from_row(slug, entries[slug], entry_dir, meta_mtime_ns)
        if summary is None:
            try:
                meta = _read_meta(entry_dir)
            except _META_CORRUPTION:
                continue  # leave corrupt entries for doctor to handle
            # Serve exactly what the repaired row will serve next time — one
            # construction, no chance of the two projections disagreeing. A meta the
            # row cannot represent (a hand-edited mode, say) is served from the meta
            # directly and NOT staged: repairing it is impossible, and restaging it
            # would rewrite the index on every listing without ever converging.
            # The stamp is the stat taken BEFORE the read (stamp contract in
            # _registry_row), which also makes this projection pure — an entry removed
            # between the read and here is served from the snapshot, never crashed on.
            row = _registry_row(meta, entry_dir, mtime_ns=meta_mtime_ns)
            summary = _summary_from_row(slug, row, entry_dir, row["mtime_ns"])
            if summary is not None:
                stale.append(slug)
            else:
                summary = _summary_from_meta(slug, meta, entry_dir)
        out.append(summary)
    if stale:
        _repair_rows(stale)
    return out


def _repair_rows(slugs: list[str]) -> None:
    """Re-project the named slugs' index rows from their metas, in place.

    Without this, a library added by an older skit — or one whose metas were hand
    edited — would fall back to reading those metas on every listing forever; the index
    only refreshes on add/rename/describe otherwise. Repairing on first listing makes
    it self-healing instead of something a user must know to run `doctor --rebuild`
    for.

    Two properties, both load-bearing:

    - **The lock is TRY-ONLY.** This runs on read paths — `skit list`, shell TAB
      completion — and the blocking lock polls forever, so a listing that used it
      would freeze the user's shell behind any process holding the lock (a large add,
      a hung skit). If the lock is busy, the repair simply doesn't happen this time;
      the next listing tries again. A read stays a read.
    - **Rows are re-derived from their metas UNDER the lock, never written from the
      listing's snapshot.** Anything can commit while the listing reads metas: a
      rename, a set-description, a remove-then-add that reuses the slug — including by
      an OLDER skit whose fresh legacy row is indistinguishable from the stale one the
      listing saw. A snapshot write would revert those; re-deriving from the meta as
      it is NOW makes the newest state win no matter who wrote it. The stamp is
      statted before the meta is read (stamp contract in `_registry_row`), so a change
      landing mid-derivation only makes the row stale again — never trusted-but-wrong.

    Best effort throughout: a slug whose meta vanished or broke meanwhile is skipped
    (doctor owns it), a row that would not validate is not written (see
    `list_summaries`), and nothing is saved unless something actually changed.
    """
    with (
        contextlib.suppress(OSError),
        try_advisory_file_lock(registry_path().with_suffix(".native.lock")) as acquired,
    ):
        if not acquired:
            return
        entries = _load_registry()
        changed = False
        for slug in slugs:
            if slug not in entries:
                continue  # removed since the listing read the index
            entry_dir = scripts_dir() / slug
            try:
                # Stat BEFORE the read — the ordering the docstring's second property
                # rests on (stamp contract in _registry_row).
                stamp = os.stat(entry_dir / "meta.toml").st_mtime_ns
                meta = _read_meta(entry_dir)
                row = _registry_row(meta, entry_dir, mtime_ns=stamp)
            except _META_CORRUPTION:
                continue
            if _summary_from_row(slug, row, entry_dir, row["mtime_ns"]) is None:
                continue  # unrepresentable meta: writing this row would repair nothing
            if entries[slug] != row:
                entries[slug] = row
                changed = True
        if changed:
            _save_registry(entries)


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


def _entry_at(slug: str, requested: str) -> Entry:
    """The stored entry under one slug, read from its meta (the truth). `requested` is
    only for the corrupt-meta message: it names the entry the way the user asked for it."""
    entry_dir = scripts_dir() / slug
    try:
        meta = _read_meta(entry_dir)
    except FileNotFoundError as exc:
        raise NotFoundError(gettext("Script not found: %(name)s") % {"name": requested}) from exc
    except _META_CORRUPTION as exc:
        raise CorruptEntryError(
            gettext("%(name)s: metadata is corrupt (%(error)s); run skit doctor --rebuild")
            % {"name": requested, "error": str(exc)}
        ) from exc
    return Entry(slug=slug, meta=meta, dir=entry_dir)


def resolve(name_or_slug: str) -> Entry:
    """The entry a NAME or SLUG refers to.

    A slug is the directory name — there is no projection of it that can go stale, so a
    slug hit is served straight from the index. A NAME is a projection, and this function
    already reads the meta a line later, so verifying it costs nothing: if the meta no
    longer carries that name, the row is stale and the match was a lie.

    The MISS path then pays for the truth once, via list_summaries — which stats every
    meta, falls back to it when the stamp says the row is stale, and repairs the row. That
    is the freshness proof _summary_from_row was built for, and list_summaries' own
    docstring gives the reason it must live here too: without it "the CLI would list
    entries the TUI, doctor and `run` all refuse, three faces disagreeing about what the
    library contains" — `run` reaches the store through THIS function, and it used to be
    the one door that never checked. A hand-edited meta name made `skit list` show an
    entry that `skit show`/`skit run` called not-found, until some unrelated listing
    happened to heal the index.

    The hit path is unchanged (one registry read, one meta read). Only a miss — which is
    about to raise, or about to be a lie — pays for the sweep.
    """
    entries = _load_registry()
    if name_or_slug in entries:
        return _entry_at(name_or_slug, name_or_slug)
    matches = [s for s, e in entries.items() if e.get("name") == name_or_slug]
    if len(matches) == 1:
        entry = _entry_at(matches[0], name_or_slug)
        if entry.meta.name == name_or_slug:
            return entry
    # The sweep serves hand-edited metas, so it must also survive what hand edits can
    # break: two metas edited to one name. Collect every claimant before answering —
    # returning the first hit would silently run whichever entry sorts lowest, and an
    # entry picked by sort order is a guess, which resolution is never allowed to be.
    candidates = [s.slug for s in list_summaries() if s.name == name_or_slug]
    if len(candidates) > 1:
        raise AmbiguousNameError(
            gettext("The name %(name)s belongs to more than one entry (%(slugs)s) — use a slug.")
            % {"name": name_or_slug, "slugs": ", ".join(sorted(candidates))}
        )
    if candidates:
        entry = _entry_at(candidates[0], name_or_slug)
        # Same freshness proof the registry hit pays a few lines up: the meta is the
        # truth, and a rename between the sweep and this read makes the match a lie.
        if entry.meta.name == name_or_slug:
            return entry
    raise NotFoundError(gettext("Script not found: %(name)s") % {"name": name_or_slug})


def remove(name_or_slug: str, *, expected_id: str | None = None) -> str:
    """Delete an entry — the most destructive slug-addressed write there is, so a
    caller that held the entry across a confirmation ask authorizes it like any other
    mutation: expected_id, checked under the entry lock, refuses to delete whoever
    owns the slug by the time the user answered."""
    with _locked_entry(name_or_slug, expected_id=expected_id) as entry:
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
    try:
        argstate.forget(entry.slug)  # drop the last-used values too
    except argstate.StateWriteError as exc:
        # The entry is gone (registry and directory both); only its state rider
        # survived. Honest partial success, the rmtree branch's shape: say what was
        # done, what was not, and the recovery — never a raw traceback for cleanup.
        raise StoreError(
            gettext(
                "%(name)s was removed from the library, but its remembered values "
                "couldn't be deleted (%(error)s) — delete this file by hand: %(path)s"
            )
            % {
                "name": entry.meta.name,
                "error": exc.strerror or str(exc),
                "path": str(paths.values_dir() / f"{entry.slug}.toml"),
            }
        ) from exc
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
    *,
    expected_id: str | None = None,
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
    with _locked_entry(name_or_slug, expected_id=expected_id) as entry:
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
    _write_meta_and_row(entry.dir, entry.slug, meta)
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
        # The shared read half, strict (add_python's encoding rule, applied to the sync
        # path too): re-encoding a lossy errors="replace" decode would swap every
        # non-UTF-8 byte in the copy for U+FFFD, so a copy that doesn't decode bails out
        # whole — the edit is already in meta, which the launcher passes via
        # --with/--python exactly like a reference-mode entry. The case where meta CAN'T
        # stand in — a copy that carries its own block — never reaches here:
        # _refuse_unsyncable_block turned it away before meta was written.
        text, newline = read_for_block_edit(script, errors="strict")
    except (OSError, UnicodeDecodeError):
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
    # The shared write half (rewrite.write_block_edit): atomic + mode-preserving. The strict
    # read above stays deliberately different from read_for_block_edit — a copy that doesn't
    # decode bails out whole and meta stands in (the add_python encoding rule).
    write_block_edit(
        script, pep723.set_dependencies(text, block_deps, requires_python=constraint), newline
    )


def update_needs(name_or_slug: str, needs: list[str], *, expected_id: str | None = None) -> Entry:
    """Update an entry's `needs` list (external commands checked on PATH before launch).
    Mirrors update_dependencies' meta write, but applies to every kind — a shell script
    or a command template can need `ffmpeg` just as a python script can. An empty list
    clears the key (stored as None so the meta stays minimal)."""
    with _locked_entry(name_or_slug, expected_id=expected_id) as entry:
        meta = entry.meta
        meta.needs = needs or None
        _write_meta_and_row(entry.dir, entry.slug, meta)
        return Entry(slug=entry.slug, meta=meta, dir=entry.dir)


def write_parameters(
    name_or_slug: str,
    decls: list[ParamDecl],
    *,
    managed: list[str] | None = None,
    expected_id: str | None = None,
) -> tuple[Entry, set[str]]:
    """Persist declared parameter rows to meta.toml [[parameters]] (the schema home for
    kinds without a text body — exe/command). The legacy `params` placeholder-name list
    is deliberately NOT derived from decls: the template is the source of truth for
    WHICH placeholders exist (extract_placeholders at add time), and keeping it
    untouched is what lets an older skit still prompt for every placeholder
    (downgrade safety) even when only some carry declared schema.

    `managed` folds a prompt's managed-list update into the SAME meta write: for a
    prompt schema the two are one logical unit — the run form asks by the list, types
    by the rows — and committing them as two transactions left a half-new schema when
    the second write failed. None means don't touch `meta.params`; a list (empty
    included) replaces it, under write_prompt_managed's prompt-only rule.

    The C3 scrub travels INSIDE the same transaction: committing a row as secret purges
    that name's stored plaintext first, under the entry lock, so no post-run persistence
    (which holds the same lock) can interleave between the scrub and the commit — and a
    schema that says secret can never coexist with plaintext the scrub missed. Returns
    the entry plus the names the scrub actually removed (for the caller's notice)."""
    with _locked_entry(name_or_slug, expected_id=expected_id) as entry:
        if managed is not None and entry.meta.kind != "prompt":
            raise StoreUsageError(
                gettext("%(name)s isn't a prompt entry.") % {"name": entry.meta.name}
            )
        # Purge BEFORE the schema commits (F2's rule, now under the entry lock): every
        # interruption lands on public+value, public+no-value or secret+no-value.
        purged = argstate.purge_secret(entry.slug, {d.name for d in decls if d.secret})
        meta = entry.meta
        if managed is not None:
            meta.params = managed or None
        meta.parameters = [d.to_meta_dict() for d in decls] or None
        _write_meta_and_row(entry.dir, entry.slug, meta)
        return Entry(slug=entry.slug, meta=meta, dir=entry.dir), purged


def write_source_params(
    name_or_slug: str, specs: list[ParamDecl], *, expected_id: str | None = None
) -> set[str]:
    """Commit an analyzable copy-mode entry's [tool.skit] schema into its STORED COPY —
    the spec-lane twin of write_parameters, sharing its transaction shape: the C3 scrub
    runs first, the block edit second, both under the entry lock, so a post-run
    persistence door (same lock) can never interleave between scrub and commit, and no
    interruption leaves "schema says secret, old plaintext still on disk". The write
    half is the shared byte-lossless pair (rewrite.py): only the comment block changes,
    unrelated bytes and the copy's own line endings survive. Returns the names the
    scrub actually removed (for the caller's notice).

    Copy-mode only, kept as a chokepoint guard (A5: skit edits its stored copy, never
    the user's original) — callers pre-check to give their own richer refusals."""
    with _locked_entry(name_or_slug, expected_id=expected_id) as entry:
        spec = registry.spec_for(entry.meta.kind)
        if spec is None or spec.params_io is None:
            raise StoreUsageError(
                gettext("%(name)s doesn't carry an editable [tool.skit] block.")
                % {"name": entry.meta.name}
            )
        if entry.meta.mode != "copy":
            raise StoreUsageError(
                gettext(
                    "%(name)s is in reference mode, and skit never writes the original file. "
                    "Edit the [tool.skit] block in the source directly."
                )
                % {"name": entry.meta.name}
            )
        purged = argstate.purge_secret(entry.slug, {s.name for s in specs if s.secret})
        text, newline = read_for_block_edit(entry.script_path)
        write_block_edit(entry.script_path, spec.params_io.write(text, specs), newline)
        return purged


def rewrite_source(
    name_or_slug: str,
    transform: Callable[[str], str | None],
    *,
    expected_id: str | None = None,
) -> None:
    """The A5 lane's transaction: apply a semantic rewrite (--normalize) to the STORED
    COPY under the entry lock, with the identity check first. The transform re-derives
    from the FRESH text — never from bytes read before a consent prompt or an analysis
    pass, which is exactly the window a concurrent edit (or a reincarnated slug) slips
    through. Strict UTF-8, this lane's own policy: a copy that doesn't decode is
    refused whole (UnicodeDecodeError propagates for the caller's refusal). None from
    the transform means write nothing; otherwise the shared byte-discipline write half
    lands it atomically with the copy's own newline style."""
    with _locked_entry(name_or_slug, expected_id=expected_id) as entry:
        try:
            text, newline = read_for_block_edit(entry.script_path, errors="strict")
        except FileNotFoundError as exc:
            raise NotFoundError(
                gettext("%(name)s has no stored copy to edit.") % {"name": entry.meta.name}
            ) from exc
        new = transform(text)
        if new is not None:
            write_block_edit(entry.script_path, new, newline)


def commit_copy_edit(name_or_slug: str, payload: bytes, *, expected_id: str | None = None) -> Entry:
    """Land an edited STORED COPY from a staged draft, atomically, under the entry
    lock with the identity check first — the editor-session twin of rewrite_source.
    The editor never touches the stored path itself (an editor session is the longest
    user-paced hold there is, and its save must not land on a reincarnated slug); the
    draft's bytes arrive here verbatim, and the stored copy keeps its own permission
    bits (a staged draft carries the umask default, not the copy's mode)."""
    with _locked_entry(name_or_slug, expected_id=expected_id) as entry:
        target = entry.script_path
        if not target.exists():
            raise NotFoundError(
                gettext("%(name)s has no stored copy to edit.") % {"name": entry.meta.name}
            )
        atomic_write_bytes_keep_mode(target, payload)
        return Entry(slug=entry.slug, meta=entry.meta, dir=entry.dir)


def read_parameters(name_or_slug: str) -> list[ParamDecl]:
    """The declared [[parameters]] rows of an entry, as decls (nameless rows dropped)."""
    entry = resolve(name_or_slug)
    return declared_from_meta(entry.meta.parameters)


def rename(name_or_slug: str, new_name: str, *, expected_id: str | None = None) -> Entry:
    """Rename an entry's display name. The slug is immutable after add — it keys the
    entry directory and the argstate values file, so keeping it means nothing moves on
    disk and remembered values/presets survive the rename."""
    new_name = new_name.strip()
    if not new_name:
        raise StoreUsageError(gettext("A name is required."))
    with _locked_entry(name_or_slug, expected_id=expected_id) as entry:
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
                raise NameConflictError(
                    gettext("The name %(name)s is already taken.") % {"name": new_name}
                )
            meta.name = new_name
            _write_meta(entry.dir, meta)
            if entry.slug in entries:
                # The whole projection, not just the changed key: the meta is in hand,
                # and patching one field of a row written by an older skit would leave
                # it legacy-shaped forever.
                entries[entry.slug] = _registry_row(meta, entry.dir)
                _save_registry(entries)
        return Entry(slug=entry.slug, meta=meta, dir=entry.dir)


def update_description(
    name_or_slug: str, description: str, *, expected_id: str | None = None
) -> Entry:
    """Update an entry's description (meta.toml is the truth; the registry index row is
    refreshed too so `list` doesn't need a rebuild to show it)."""
    with _locked_entry(name_or_slug, expected_id=expected_id) as entry:
        meta = entry.meta
        meta.description = description
        _write_meta_and_row(entry.dir, entry.slug, meta)
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
                    # Stat before read (stamp contract in _registry_row): a write
                    # landing mid-rebuild leaves a stale row, never a wrong-but-fresh one.
                    stamp = os.stat(entry_dir / "meta.toml").st_mtime_ns
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
                entries[entry_dir.name] = _registry_row(meta, entry_dir, mtime_ns=stamp)
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
    "EntrySummary",
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
    "list_summaries",
    "read_parameters",
    "remove",
    "resolve",
    "update_needs",
    "write_parameters",
]
