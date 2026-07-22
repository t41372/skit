"""atomic.py: atomic writes plus the shared corrupt-TOML backup helper (load_toml_recoverable).

config.py and i18n.py both do read-modify-write saves against config.toml and both need "a
present-but-corrupt file gets backed up instead of silently wiped" — but i18n can't import
config's version of that logic (config already imports gettext from i18n, so the reverse would
cycle). This module is the neutral home both safely import; these tests exercise it directly,
independent of either caller.
"""

from __future__ import annotations

import errno
import os
import re
import stat
import subprocess
import sys
import threading
import time
from pathlib import Path

import pytest

from skit import atomic


def test_load_toml_recoverable_missing_file_returns_empty_no_backup(tmp_path: Path) -> None:
    result = atomic.load_toml_recoverable(tmp_path / "config.toml")
    assert result.doc == {}
    assert result.corrupt is False
    assert result.backup_path is None


def test_load_toml_recoverable_valid_file_returns_doc_no_backup(tmp_path: Path) -> None:
    path = tmp_path / "config.toml"
    path.write_text('language = "zh-CN"\n[mirror]\nenabled = true\n', encoding="utf-8")
    result = atomic.load_toml_recoverable(path)
    assert result.doc == {"language": "zh-CN", "mirror": {"enabled": True}}
    assert result.corrupt is False
    assert result.backup_path is None
    assert not path.with_name("config.toml.bak").exists()


def test_load_toml_recoverable_corrupt_file_backs_up_and_returns_empty(tmp_path: Path) -> None:
    path = tmp_path / "config.toml"
    corrupt = 'language = "zh-CN"\nthis is = = not valid toml'
    path.write_text(corrupt, encoding="utf-8")
    result = atomic.load_toml_recoverable(path)
    assert result.doc == {}
    assert result.corrupt is True
    assert result.backup_path == path.with_name("config.toml.bak")
    assert result.backup_path is not None
    assert result.backup_path.read_text(encoding="utf-8") == corrupt
    # the corrupt original is untouched — the caller decides when/whether to overwrite it
    assert path.read_text(encoding="utf-8") == corrupt


def test_load_toml_recoverable_reports_none_when_backup_itself_fails(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    path = tmp_path / "config.toml"
    path.write_text("this is = = not valid toml", encoding="utf-8")

    def boom(*_a: object, **_k: object) -> None:
        raise OSError("disk full")

    monkeypatch.setattr(atomic.shutil, "copy2", boom)
    result = atomic.load_toml_recoverable(path)
    assert result.doc == {}
    assert result.corrupt is True
    assert result.backup_path is None
    assert not path.with_name("config.toml.bak").exists()


# ---- advisory_file_lock: persistent kernel-backed transaction serialization ----


def test_advisory_file_lock_keeps_a_persistent_one_byte_inode(tmp_path: Path) -> None:
    lock = tmp_path / "config.lock"
    with atomic.advisory_file_lock(lock):
        assert lock.is_file()
        assert lock.stat().st_size >= 1
    # Never unlink a lock inode: path replacement is what made the old lease design
    # admit two owners. Kernel ownership, not path existence, is the lock state.
    assert lock.is_file()


def test_advisory_file_lock_serializes_two_waiting_threads(tmp_path: Path) -> None:
    lock = tmp_path / "config.lock"
    entered: list[str] = []
    active = 0
    state_lock = threading.Lock()
    start = threading.Event()

    def waiter(name: str) -> None:
        nonlocal active
        start.wait(timeout=1)
        with atomic.advisory_file_lock(lock, poll_seconds=0.005):
            with state_lock:
                assert active == 0
                active += 1
                entered.append(name)
            time.sleep(0.02)
            with state_lock:
                active -= 1

    with atomic.advisory_file_lock(lock):
        threads = [threading.Thread(target=waiter, args=(name,)) for name in ("a", "b")]
        for thread in threads:
            thread.start()
        start.set()
        time.sleep(0.03)
        assert entered == []
    for thread in threads:
        thread.join(timeout=1)
    assert sorted(entered) == ["a", "b"]
    assert active == 0
    assert not any(thread.is_alive() for thread in threads)


@pytest.mark.skipif(sys.platform == "win32", reason="POSIX subprocess exercises flock")
def test_advisory_file_lock_is_released_by_kernel_after_process_crash(tmp_path: Path) -> None:
    lock = tmp_path / "crash.lock"
    code = (
        "import os, sys; from pathlib import Path; from skit.atomic import advisory_file_lock; "
        "lock=Path(sys.argv[1]); "
        "ctx=advisory_file_lock(lock); ctx.__enter__(); "
        "print('locked', flush=True); os._exit(23)"
    )
    proc = subprocess.Popen(
        [sys.executable, "-c", code, str(lock)],
        stdout=subprocess.PIPE,
        text=True,
    )
    assert proc.stdout is not None
    assert proc.stdout.readline().strip() == "locked"
    assert proc.wait(timeout=3) == 23

    # No stale timeout/reclaim: close-on-crash released flock immediately.
    with atomic.advisory_file_lock(lock, poll_seconds=0.005):
        assert lock.is_file()


def test_windows_locking_uses_one_byte_seek_retry_and_unlock(tmp_path: Path, monkeypatch) -> None:
    lock = tmp_path / "windows.lock"

    class FakeMsvcrt:
        LK_NBLCK = 10
        LK_UNLCK = 20

        def __init__(self) -> None:
            self.calls: list[tuple[int, int, int]] = []
            self.busy_once = True

        def locking(self, fd: int, mode: int, length: int) -> None:
            self.calls.append((os.lseek(fd, 0, os.SEEK_CUR), mode, length))
            if mode == self.LK_NBLCK and self.busy_once:
                self.busy_once = False
                raise OSError(errno.EACCES, "busy")

    fake = FakeMsvcrt()
    sleeps: list[float] = []
    monkeypatch.setattr(atomic, "_WINDOWS", True)
    # Exercise the deferred platform import too: importing atomic on POSIX must not
    # require msvcrt, while the Windows path must consume the real module seam.
    monkeypatch.setitem(sys.modules, "msvcrt", fake)
    monkeypatch.setattr(atomic.time, "sleep", sleeps.append)

    with atomic.advisory_file_lock(lock, poll_seconds=0.007):
        assert lock.stat().st_size >= 1

    assert fake.calls == [(0, fake.LK_NBLCK, 1), (0, fake.LK_NBLCK, 1), (0, fake.LK_UNLCK, 1)]
    assert sleeps == [0.007]
    assert lock.is_file()


def test_native_lock_distinguishes_contention_from_unexpected_os_errors(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """Only the documented busy errno values are retryable; disk/fd failures stay loud."""

    class FakeFcntl:
        LOCK_EX = 1
        LOCK_NB = 2

        def __init__(self, error: int) -> None:
            self.error = error

        def flock(self, _fd: int, _flags: int) -> None:
            raise OSError(self.error, os.strerror(self.error))

    monkeypatch.setattr(atomic, "_WINDOWS", False)
    monkeypatch.setattr(atomic, "_posix_lock_module", lambda: FakeFcntl(errno.EAGAIN))
    assert atomic._try_native_lock(0) is False

    monkeypatch.setattr(atomic, "_posix_lock_module", lambda: FakeFcntl(errno.EBADF))
    with pytest.raises(OSError, match=re.escape(os.strerror(errno.EBADF))) as exc_info:
        atomic._try_native_lock(0)
    assert exc_info.value.errno == errno.EBADF

    class FakeMsvcrt:
        LK_NBLCK = 1

        @staticmethod
        def locking(_fd: int, _mode: int, _length: int) -> None:
            raise OSError(errno.ENOSPC, "disk full")

    fd = os.open(tmp_path / "windows.lock", os.O_CREAT | os.O_RDWR, 0o600)
    try:
        monkeypatch.setattr(atomic, "_WINDOWS", True)
        monkeypatch.setitem(sys.modules, "msvcrt", FakeMsvcrt())
        with pytest.raises(OSError, match="disk full") as exc_info:
            atomic._try_native_lock(fd)
        assert exc_info.value.errno == errno.ENOSPC
    finally:
        os.close(fd)


def test_advisory_lock_open_failure_releases_its_thread_mutex(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """A failed lockfile open must not permanently deadlock later callers in-process."""
    lock = tmp_path / "locks" / "entry.lock"
    real_open = atomic.os.open

    def denied(*_args, **_kwargs):
        raise PermissionError(errno.EACCES, "permission denied", str(lock))

    monkeypatch.setattr(atomic.os, "open", denied)
    with pytest.raises(PermissionError):
        with atomic.advisory_file_lock(lock):
            pytest.fail("an unopened lockfile cannot be acquired")

    monkeypatch.setattr(atomic.os, "open", real_open)
    with atomic.advisory_file_lock(lock):
        assert lock.is_file()


def test_advisory_lock_native_failure_closes_fd_and_releases_mutex(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """An unexpected native-lock failure cleans up both resources before propagating."""
    lock = tmp_path / "entry.lock"
    attempted_fd = -1

    def broken_native(fd: int) -> bool:
        nonlocal attempted_fd
        attempted_fd = fd
        raise OSError(errno.EBADF, "bad lock descriptor")

    real_try = atomic._try_native_lock
    monkeypatch.setattr(atomic, "_try_native_lock", broken_native)
    with pytest.raises(OSError, match="bad lock descriptor"):
        with atomic.advisory_file_lock(lock):
            pytest.fail("a failed native lock cannot be acquired")

    assert attempted_fd >= 0
    # A closed fd's strerror text is OS-specific ("Bad file descriptor" on POSIX,
    # "The handle is invalid" on Windows); the errno below is the portable assertion.
    with pytest.raises(OSError) as exc_info:  # noqa: PT011 — asserted on errno, not the OS message
        os.fstat(attempted_fd)
    assert exc_info.value.errno == errno.EBADF

    monkeypatch.setattr(atomic, "_try_native_lock", real_try)
    with atomic.advisory_file_lock(lock):
        assert lock.is_file()


# ---- atomic_write_*: durability (fsync temp file before replace, fsync dir after replace) ----


def test_atomic_write_bytes_fsyncs_before_replace(
    monkeypatch: pytest.MonkeyPatch, tmp_path: Path
) -> None:
    """os.replace() alone only guarantees atomicity wrt concurrent readers -- it says nothing about
    whether the temp file's bytes reached stable storage. The temp file must be fsync'd BEFORE the
    rename commits, so a crash between the rename and the page-cache writeback can't leave `path`
    looking written but holding zero-length/garbage bytes. Spy on the order of os.fsync vs
    os.replace (real fsync/replace still run; only the order is observed)."""
    calls: list[str] = []
    real_fsync = os.fsync
    real_replace = os.replace

    def _spy_fsync(fd: int) -> None:
        calls.append("fsync")
        real_fsync(fd)

    def _spy_replace(src: str, dst: str) -> None:
        calls.append("replace")
        real_replace(src, dst)

    monkeypatch.setattr(atomic.os, "fsync", _spy_fsync)
    monkeypatch.setattr(atomic.os, "replace", _spy_replace)

    path = tmp_path / "out.bin"
    atomic.atomic_write_bytes(path, b"payload")

    assert path.read_bytes() == b"payload"
    assert "fsync" in calls
    assert "replace" in calls
    assert calls.index("fsync") < calls.index("replace")  # temp file synced before the rename


def test_atomic_write_text_fsyncs_before_replace(
    monkeypatch: pytest.MonkeyPatch, tmp_path: Path
) -> None:
    calls: list[str] = []
    real_fsync = os.fsync
    real_replace = os.replace

    def _spy_fsync(fd: int) -> None:
        calls.append("fsync")
        real_fsync(fd)

    def _spy_replace(src: str, dst: str) -> None:
        calls.append("replace")
        real_replace(src, dst)

    monkeypatch.setattr(atomic.os, "fsync", _spy_fsync)
    monkeypatch.setattr(atomic.os, "replace", _spy_replace)

    path = tmp_path / "out.txt"
    atomic.atomic_write_text(path, "hello")

    assert path.read_text(encoding="utf-8") == "hello"
    assert calls.index("fsync") < calls.index("replace")


def test_atomic_write_toml_fsyncs_before_replace(
    monkeypatch: pytest.MonkeyPatch, tmp_path: Path
) -> None:
    calls: list[str] = []
    real_fsync = os.fsync
    real_replace = os.replace

    def _spy_fsync(fd: int) -> None:
        calls.append("fsync")
        real_fsync(fd)

    def _spy_replace(src: str, dst: str) -> None:
        calls.append("replace")
        real_replace(src, dst)

    monkeypatch.setattr(atomic.os, "fsync", _spy_fsync)
    monkeypatch.setattr(atomic.os, "replace", _spy_replace)

    path = tmp_path / "out.toml"
    atomic.atomic_write_toml(path, {"language": "en"})

    assert path.read_bytes() == b'language = "en"\n'
    assert calls.index("fsync") < calls.index("replace")


def test_atomic_write_bytes_fsyncs_parent_dir_after_replace(
    monkeypatch: pytest.MonkeyPatch, tmp_path: Path
) -> None:
    """On POSIX, the rename's directory entry is also fsync'd (best-effort) once os.replace has
    landed, so the write survives not just a torn file but a lost/rolled-back directory entry
    across a power loss."""
    if sys.platform == "win32":
        pytest.skip("directory fsync is POSIX-only")

    calls: list[Path] = []
    real_fsync_dir = atomic._fsync_dir

    def _spy(dir_path: Path) -> None:
        calls.append(dir_path)
        real_fsync_dir(dir_path)

    monkeypatch.setattr(atomic, "_fsync_dir", _spy)

    path = tmp_path / "out.bin"
    atomic.atomic_write_bytes(path, b"payload")

    assert path.read_bytes() == b"payload"
    assert calls == [tmp_path]


def test_atomic_write_bytes_dir_fsync_failure_is_swallowed(
    monkeypatch: pytest.MonkeyPatch, tmp_path: Path
) -> None:
    """The post-replace directory fsync is best-effort (not every filesystem supports fsync on a
    directory fd): a failure there must not fail the write or leak past the
    contextlib.suppress guarding it, since the durability guarantee for the file's *contents* was
    already secured by the temp-file fsync before the rename."""

    def _boom(_dir_path: Path) -> None:
        raise OSError("simulated: fsync not supported for directories on this filesystem")

    monkeypatch.setattr(atomic, "_fsync_dir", _boom)

    path = tmp_path / "out.bin"
    atomic.atomic_write_bytes(path, b"payload")

    assert path.read_bytes() == b"payload"  # write still succeeded despite the dir-fsync failure


def test_atomic_write_bytes_skips_dir_fsync_on_windows(
    monkeypatch: pytest.MonkeyPatch, tmp_path: Path
) -> None:
    """Directories can't be opened via os.open on Windows, so the best-effort directory fsync must
    not even be attempted there -- only the temp-file fsync (which works fine on Windows) runs."""
    monkeypatch.setattr(atomic.sys, "platform", "win32")

    calls: list[Path] = []

    def _record(dir_path: Path) -> None:
        calls.append(dir_path)

    monkeypatch.setattr(atomic, "_fsync_dir", _record)

    path = tmp_path / "out.bin"
    atomic.atomic_write_bytes(path, b"payload")

    assert path.read_bytes() == b"payload"
    assert calls == []  # _fsync_dir was never called


def test_atomic_write_bytes_temp_fsync_failure_still_cleans_up_tmp_file(
    monkeypatch: pytest.MonkeyPatch, tmp_path: Path
) -> None:
    """Unlike the best-effort directory fsync, a failure fsync'ing the temp file's *data* is not
    swallowed: it must propagate, and it must compose with the pre-existing cleanup-on-failure
    (except BaseException: unlink the temp file) exactly like a write failure does -- the
    destination directory is left exactly as if the write never started."""

    def _boom(_fd: int) -> None:
        raise OSError("simulated: fsync EIO")

    monkeypatch.setattr(atomic.os, "fsync", _boom)

    path = tmp_path / "out.bin"
    with pytest.raises(OSError, match="simulated: fsync EIO"):
        atomic.atomic_write_bytes(path, b"payload")

    assert not path.exists()
    assert list(tmp_path.iterdir()) == []  # the temp file was cleaned up, not left behind


# --------------------------------------------------------------------------
# atomic_write_text_keep_mode — preserve an existing file's permission bits
# --------------------------------------------------------------------------


def test_atomic_write_text_keep_mode_preserves_existing_mode(tmp_path: Path) -> None:
    """An existing file's permission bits survive the atomic replace: mkstemp's tmp is always
    0600, so a plain atomic_write_text would silently re-mode a 0755 stored script copy (added
    via copy2, which preserved the original's bits). keep_mode restores the exact captured mode
    after the replace, and the new content lands."""
    target = tmp_path / "script.sh"
    target.write_text("old\n", encoding="utf-8")
    target.chmod(0o755)

    atomic.atomic_write_text_keep_mode(target, "new content\n")

    assert target.read_text(encoding="utf-8") == "new content\n"
    assert stat.S_IMODE(target.stat().st_mode) == 0o755  # bits preserved exactly


def test_atomic_write_text_keep_mode_missing_target_skips_chmod(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """A target that vanished since the caller read it: the pre-stat raises OSError so mode is
    None, and NO chmod is attempted (nothing to preserve). The fresh write still lands, without
    crashing."""
    target = tmp_path / "gone.txt"
    assert not target.exists()
    chmod_calls: list[object] = []
    monkeypatch.setattr(atomic.os, "chmod", lambda *a, **_k: chmod_calls.append(a))

    atomic.atomic_write_text_keep_mode(target, "created\n")

    assert target.read_text(encoding="utf-8") == "created\n"
    assert chmod_calls == []  # mode was None → the chmod branch was skipped entirely


def test_atomic_write_text_keep_mode_suppresses_chmod_failure(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """The mode-restoring chmod is best-effort (contextlib.suppress(OSError)): if os.chmod
    raises, the write still succeeds — the new content was already committed by the atomic
    replace before the chmod ran."""
    target = tmp_path / "script.sh"
    target.write_text("old\n", encoding="utf-8")
    target.chmod(0o644)

    def boom(*_a: object, **_k: object) -> None:
        raise OSError("chmod not permitted")

    monkeypatch.setattr(atomic.os, "chmod", boom)

    atomic.atomic_write_text_keep_mode(target, "new\n")  # must not raise

    assert target.read_text(encoding="utf-8") == "new\n"


# --------------------------------------------------------------------------
# _replace_with_retry — Windows sharing-violation backoff (issue #4)
# --------------------------------------------------------------------------


def test_replace_retries_through_transient_permission_error(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """Two sharing violations then success: the write lands, with exact backoff."""
    real_replace = os.replace
    attempts: list[str] = []
    sleeps: list[float] = []

    def flaky_replace(src, dst):
        attempts.append("call")
        if len(attempts) <= 2:
            raise PermissionError(13, "Permission denied")
        real_replace(src, dst)

    monkeypatch.setattr(atomic.os, "replace", flaky_replace)
    monkeypatch.setattr(atomic.time, "sleep", sleeps.append)
    target = tmp_path / "registry.toml"
    atomic.atomic_write_bytes(target, b"payload")
    assert target.read_bytes() == b"payload"
    assert len(attempts) == 3
    assert sleeps == [0.01, 0.02]  # exponential, starting at the documented base


def test_replace_gives_up_loudly_after_bounded_attempts(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """A target held open forever (antivirus, a leak) must surface, not spin."""
    attempts: list[str] = []
    sleeps: list[float] = []

    def stuck_replace(src, dst):
        attempts.append("call")
        raise PermissionError(13, "Permission denied")

    monkeypatch.setattr(atomic.os, "replace", stuck_replace)
    monkeypatch.setattr(atomic.time, "sleep", sleeps.append)
    with pytest.raises(PermissionError):
        atomic.atomic_write_bytes(tmp_path / "registry.toml", b"payload")
    assert len(attempts) == 8  # 7 retried + 1 final loud attempt
    assert sleeps == [0.01, 0.02, 0.04, 0.08, 0.16, 0.32, 0.64]


def test_replace_other_oserrors_are_not_retried(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """Only the Windows sharing violation is transient; anything else stays immediate."""
    attempts: list[str] = []

    def broken_replace(src, dst):
        attempts.append("call")
        raise IsADirectoryError(21, "Is a directory")

    monkeypatch.setattr(atomic.os, "replace", broken_replace)
    monkeypatch.setattr(atomic.time, "sleep", lambda _s: pytest.fail("must not sleep"))
    with pytest.raises(IsADirectoryError):
        atomic.atomic_write_bytes(tmp_path / "registry.toml", b"payload")
    assert len(attempts) == 1
