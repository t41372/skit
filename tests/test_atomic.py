"""atomic.py: atomic writes plus the shared corrupt-TOML backup helper (load_toml_recoverable).

config.py and i18n.py both do read-modify-write saves against config.toml and both need "a
present-but-corrupt file gets backed up instead of silently wiped" — but i18n can't import
config's version of that logic (config already imports gettext from i18n, so the reverse would
cycle). This module is the neutral home both safely import; these tests exercise it directly,
independent of either caller.
"""

from __future__ import annotations

import os
import sys
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
