"""Atomic writes: every state/metadata file goes through tmp + os.replace (C7)."""

from __future__ import annotations

import contextlib
import os
import shutil
import sys
import tempfile
import tomllib
from dataclasses import dataclass
from pathlib import Path
from typing import Any

import tomli_w


def _fsync_dir(dir_path: Path) -> None:
    """Fsync a directory fd so a prior os.replace()'s rename entry is durable on stable storage.

    POSIX only: directories can never be opened for writing, but a read-only fd is enough --
    fsync() flushes the underlying inode's dirty metadata regardless of which fd requested it.
    Callers must guard with `sys.platform != "win32"` (os.open can't open a directory there) and
    wrap this in contextlib.suppress(OSError): not every filesystem supports fsync on a
    directory, and that failure must never undo or fail a write os.replace() already committed.
    """
    fd = os.open(dir_path, os.O_RDONLY)
    try:
        os.fsync(fd)
    finally:
        os.close(fd)


def atomic_write_bytes(path: Path, data: bytes) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    fd, tmp = tempfile.mkstemp(dir=path.parent, prefix=f".{path.name}.", suffix=".tmp")
    try:
        with os.fdopen(fd, "wb") as f:
            f.write(data)
            f.flush()
            os.fsync(f.fileno())  # durable on disk BEFORE the rename, not just before this returns
        os.replace(tmp, path)
        if sys.platform != "win32":  # os.open can't open a directory on Windows
            with contextlib.suppress(OSError):
                _fsync_dir(path.parent)  # best-effort: persist the rename's directory entry too
    except BaseException:
        with contextlib.suppress(OSError):
            os.unlink(tmp)
        raise


def atomic_write_text(path: Path, text: str) -> None:
    atomic_write_bytes(path, text.encode("utf-8"))  # pragma: no mutate — utf-8/UTF-8 alias


def atomic_write_toml(path: Path, doc: dict[str, Any]) -> None:
    atomic_write_bytes(path, tomli_w.dumps(doc).encode("utf-8"))  # pragma: no mutate — alias


@dataclass(frozen=True)
class TomlRecovery:
    """Result of load_toml_recoverable().

    `doc` is the parsed table when the file is absent or parses fine (and `corrupt` is False,
    `backup_path` is None). When the file exists but fails to parse, `doc` is always {} and
    `corrupt` is True; `backup_path` then names where the original got copied to before the
    caller treats it as empty and overwrites it — or is None when even the backup attempt failed
    (e.g. a read-only config dir), so the caller can still warn the user which case it hit.
    """

    doc: dict[str, Any]
    corrupt: bool
    backup_path: Path | None


def load_toml_recoverable(path: Path) -> TomlRecovery:
    """Read a TOML file for a read-modify-write save, without silently discarding a corrupt one.

    A plain best-effort load (catch-and-return-{}) is fine for read-only callers, but a
    read-modify-write saver that started from that {} would then overwrite the file with just the
    one key it just set — silently destroying every other saved setting with no trace. So here: if
    the file exists but fails to parse, it is first copied to `<path>.bak` (best-effort) before
    being treated as empty, so the caller can proceed *and* tell the user where to recover from.

    Headless: this module has no gettext dependency, so it never prints anything itself — callers
    own the warning (worded for their own context) based on `corrupt` / `backup_path`. This is what
    lets both config.py and i18n.py (which config.py imports, so the reverse would be a cycle)
    share the exact same backup mechanics.
    """
    if not path.is_file():
        return TomlRecovery(doc={}, corrupt=False, backup_path=None)
    try:
        with open(path, "rb") as f:
            return TomlRecovery(doc=tomllib.load(f), corrupt=False, backup_path=None)
    except (OSError, tomllib.TOMLDecodeError):
        backup = path.with_name(path.name + ".bak")
        try:
            shutil.copy2(path, backup)
        except OSError:
            return TomlRecovery(doc={}, corrupt=True, backup_path=None)
        return TomlRecovery(doc={}, corrupt=True, backup_path=backup)
