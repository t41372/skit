"""Atomic writes: every state/metadata file goes through tmp + os.replace (C7)."""

from __future__ import annotations

import contextlib
import errno
import os
import shutil
import stat
import sys
import tempfile
import threading
import time
import tomllib
from collections.abc import Callable, Iterator
from dataclasses import dataclass
from pathlib import Path
from typing import Any

import tomli_w

# Windows can't replace a file that another handle has open (sharing violation →
# PermissionError); concurrent readers of registry.toml hold it for microseconds, so a
# bounded exponential backoff is the standard idiom (total worst-case wait ≈ 1.3 s).
# POSIX replaces open files freely, so this path never fires there.
_REPLACE_RETRIES = 7  # sleeps: 0.01 · 0.02 · 0.04 · 0.08 · 0.16 · 0.32 · 0.64 s
_REPLACE_BACKOFF_START = 0.01

_LOCK_POLL_SECONDS = 0.05
_WINDOWS = sys.platform == "win32"
_THREAD_LOCKS: dict[str, threading.Lock] = {}
_THREAD_LOCKS_GUARD = threading.Lock()


def _thread_lock_for(lock_path: Path) -> threading.Lock:
    """One in-process mutex per absolute path; OS locks provide cross-process exclusion."""
    key = os.path.abspath(os.fspath(lock_path))
    with _THREAD_LOCKS_GUARD:
        return _THREAD_LOCKS.setdefault(key, threading.Lock())


def _windows_lock_module() -> Any:
    import msvcrt

    return msvcrt


def _posix_lock_module() -> Any:
    import fcntl

    return fcntl


def _try_native_lock(fd: int) -> bool:
    if _WINDOWS:
        msvcrt = _windows_lock_module()
        os.lseek(fd, 0, os.SEEK_SET)
        try:
            msvcrt.locking(fd, msvcrt.LK_NBLCK, 1)
        except OSError as exc:
            if exc.errno in {errno.EACCES, errno.EAGAIN, errno.EDEADLK}:
                return False
            raise
        return True
    fcntl = _posix_lock_module()
    exclusive_nonblocking = fcntl.LOCK_EX | fcntl.LOCK_NB
    try:
        fcntl.flock(fd, exclusive_nonblocking)
    except OSError as exc:
        if exc.errno in {errno.EACCES, errno.EAGAIN}:
            return False
        raise
    return True


def _unlock_native(fd: int) -> None:
    if _WINDOWS:
        msvcrt = _windows_lock_module()
        os.lseek(fd, 0, os.SEEK_SET)
        msvcrt.locking(fd, msvcrt.LK_UNLCK, 1)
        return
    fcntl = _posix_lock_module()
    fcntl.flock(fd, fcntl.LOCK_UN)


@contextlib.contextmanager
def advisory_file_lock(
    lock_path: Path,
    *,
    poll_seconds: float = _LOCK_POLL_SECONDS,
) -> Iterator[None]:
    """Serialize a filesystem transaction across processes and threads.

    Atomic replacement protects a single write from torn contents; callers still need
    this lock around the whole read-modify-write transaction to prevent last-writer-wins
    data loss. The lockfile is persistent and never unlinked: POSIX ``flock`` and
    Windows' one-byte ``msvcrt.locking`` are released by the kernel when a process
    exits, so crash recovery needs no racy age/stat/unlink lease. A per-path thread lock
    supplies the same exclusion within one process on every supported OS.
    """
    process_lock = _thread_lock_for(lock_path)
    process_lock.acquire()
    fd = -1
    native_locked = False
    try:
        lock_path.parent.mkdir(parents=True, exist_ok=True)
        fd = os.open(lock_path, os.O_CREAT | os.O_RDWR, 0o600)
        if os.fstat(fd).st_size == 0:
            os.write(fd, b"\0")  # Windows locks one real byte; concurrent initialization is benign.
        while not _try_native_lock(fd):
            time.sleep(poll_seconds)
        native_locked = True
        yield
    finally:
        try:
            if fd >= 0:
                if native_locked:
                    with contextlib.suppress(OSError):
                        _unlock_native(fd)
                os.close(fd)
        finally:
            process_lock.release()


def _replace_with_retry(src: str, dst: Path) -> None:
    """os.replace that rides out transient Windows sharing violations. After the
    retries are exhausted, the final attempt's PermissionError propagates — a target
    held open indefinitely (antivirus, an actual leak) must stay loud."""
    delay = _REPLACE_BACKOFF_START
    for _ in range(_REPLACE_RETRIES):
        try:
            os.replace(src, dst)
        except PermissionError:
            time.sleep(delay)
            delay *= 2
        else:
            return
    os.replace(src, dst)


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


# os.fchmod exists on POSIX and not on Windows. Looked up once, here, so the mode-carrying
# write and its post-rename fallback below key off ONE predicate — "can this platform set the
# mode through an open fd?" — which is the actual question, and which the type checker (it
# checks every platform at once) can follow without a suppression.
_FCHMOD: Callable[[int, int], None] | None = getattr(os, "fchmod", None)


def atomic_write_bytes(path: Path, data: bytes, *, mode: int | None = None) -> None:
    """`mode`, when given, is applied to the temp file BEFORE the rename, so the permission
    bits are published in the same atomic swap as the content. Chmod'ing afterwards leaves a
    window where a crash strands the file at mkstemp's 0600 — for a stored copy that means
    losing the execute bit permanently. POSIX only: os.fchmod does not exist on Windows,
    where the mode is a read-only bit that callers restore after the replace instead."""
    path.parent.mkdir(parents=True, exist_ok=True)
    fd, tmp = tempfile.mkstemp(dir=path.parent, prefix=f".{path.name}.", suffix=".tmp")
    try:
        with os.fdopen(fd, "wb") as f:
            f.write(data)
            if mode is not None and _FCHMOD is not None:
                with contextlib.suppress(OSError):
                    _FCHMOD(f.fileno(), mode)
            f.flush()
            os.fsync(f.fileno())  # durable on disk BEFORE the rename, not just before this returns
        _replace_with_retry(tmp, path)
        if sys.platform != "win32":  # os.open can't open a directory on Windows
            with contextlib.suppress(OSError):
                _fsync_dir(path.parent)  # best-effort: persist the rename's directory entry too
    except BaseException:
        with contextlib.suppress(OSError):
            os.unlink(tmp)
        raise


def atomic_write_text(path: Path, text: str, *, mode: int | None = None) -> None:
    atomic_write_bytes(path, text.encode("utf-8"), mode=mode)  # pragma: no mutate — utf-8 alias


def atomic_write_bytes_keep_mode(path: Path, data: bytes) -> None:
    """atomic_write_text_keep_mode for callers that already hold exact bytes — a write-back of
    a script decoded with surrogateescape, where re-encoding through the strict-UTF-8 text
    helper would raise (or, decoded with errors="replace", would silently swap every
    non-UTF-8 byte for U+FFFD)."""
    _write_keeping_mode(path, lambda mode: atomic_write_bytes(path, data, mode=mode))


def atomic_write_text_keep_mode(path: Path, text: str) -> None:
    """Atomic replacement for an existing file that keeps its permission bits: mkstemp's
    tmp is always 0600, so a plain atomic_write_text would silently re-mode a stored
    script copy (added via copy2, which preserved the original's bits). A target that
    vanished since the caller read it just gets the fresh write (nothing to preserve).

    The bits ride along with the content (see atomic_write_bytes) rather than being
    restored afterwards, so there is no window in which the file exists with the wrong
    mode. Windows has no fchmod, so the post-replace chmod stays the path there."""
    _write_keeping_mode(path, lambda mode: atomic_write_text(path, text, mode=mode))


def _write_keeping_mode(path: Path, write: Callable[[int | None], None]) -> None:
    """Capture the target's mode, hand it to `write` so it rides along with the content, and
    restore it afterwards only where that is the only option (Windows has no os.fchmod)."""
    try:
        mode = stat.S_IMODE(path.stat().st_mode)
    except OSError:
        mode = None
    write(mode)
    if mode is not None and _FCHMOD is None:
        # Windows: no fchmod, so the bits can only go on after the rename. That window is
        # exactly what the POSIX path exists to avoid, and it is the best available there.
        with contextlib.suppress(OSError):
            os.chmod(path, mode)


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
