"""Mutation-kill tests for src/skit/uvman.py — `_extract_uv`'s tar extraction filter, the staged
temp-file naming, and the cleanup-suppress in the failure path. These pin behaviour that the
existing tests in test_uvman.py leave un-asserted (the exact staged-name shape, the `data`
extraction filter, and the OSError-typed suppress guarding the post-rename cleanup).
"""

from __future__ import annotations

import io
import os
import sys
import tarfile
from pathlib import Path

import pytest

from skit import uvman

EXE = "uv.exe" if sys.platform == "win32" else "uv"


def _tar_gz_with_uv(tmp_path: Path, content: bytes = b"genuine-uv-bytes") -> Path:
    """A real tar.gz holding a single executable member named like the platform uv binary."""
    src_dir = tmp_path / f"src-{os.urandom(4).hex()}"
    src_dir.mkdir()
    member = src_dir / EXE
    member.write_bytes(content)
    archive = tmp_path / f"uv-{os.urandom(4).hex()}.tar.gz"
    with tarfile.open(archive, "w:gz") as tf:
        tf.add(member, arcname=f"uv-1.0/{EXE}")
    return archive


def _tar_gz_with_traversal(tmp_path: Path) -> Path:
    """A real tar.gz holding a legit uv member PLUS a path-traversal member (`../escape.txt`).

    The `data` extraction filter refuses the traversing member (OutsideDestinationError); the
    unfiltered/`None` behaviour (fully-trusted on 3.12) happily writes it outside the destination.
    """
    src_dir = tmp_path / f"src-{os.urandom(4).hex()}"
    src_dir.mkdir()
    uv = src_dir / EXE
    uv.write_bytes(b"binary")
    archive = tmp_path / f"evil-{os.urandom(4).hex()}.tar.gz"
    with tarfile.open(archive, "w:gz") as tf:
        tf.add(uv, arcname=f"pkg/{EXE}")
        info = tarfile.TarInfo(name="../escape.txt")
        payload = b"escaped!"
        info.size = len(payload)
        tf.addfile(info, io.BytesIO(payload))
    return archive


def test_extract_uv_applies_data_filter_rejecting_path_traversal(tmp_path: Path) -> None:
    """`_extract_uv` must extract under the `data` security filter: a member whose name escapes the
    destination via `..` is refused (FilterError), so a hostile release archive can never plant a
    file outside the temp extraction dir. Kills the `filter="data"` -> `filter=None` / dropped
    mutants, which would extract the traversing member instead of rejecting it."""
    archive = _tar_gz_with_traversal(tmp_path)
    dest_dir = tmp_path / "dest"
    with pytest.raises(tarfile.FilterError):
        uvman._extract_uv(archive, dest_dir)
    assert not (dest_dir / EXE).exists()  # nothing installed from a rejected archive


def test_extract_uv_staged_file_is_hidden_and_dot_tmp(monkeypatch, tmp_path: Path) -> None:
    """The staged copy is created via mkstemp with a leading-dot, binary-identifying prefix
    (`.uv.`) and a `.tmp` suffix, so a concurrent reader/globber never mistakes the half-written
    stage for the real binary. Observe the exact staged name (captured at copy2 time, then the real
    copy still runs). Kills every prefix/suffix mutant on the mkstemp call (None, dropped, the
    XX-wrapped suffix, and the case-flipped `.TMP`)."""
    archive = _tar_gz_with_uv(tmp_path)
    dest_dir = tmp_path / "dest"

    staged_names: list[str] = []
    real_copy2 = uvman.shutil.copy2

    def _spy(src, dst, *a, **kw):
        staged_names.append(Path(dst).name)
        return real_copy2(src, dst, *a, **kw)

    monkeypatch.setattr(uvman.shutil, "copy2", _spy)

    dest = uvman._extract_uv(archive, dest_dir)

    assert dest == dest_dir / EXE
    assert len(staged_names) == 1
    name = staged_names[0]
    assert name.startswith(f".{EXE}.")  # leading dot + binary name (kills prefix None/dropped)
    assert name.endswith(".tmp")  # exact suffix (kills suffix None/dropped/XX.tmpXX/.TMP)


def test_extract_uv_cleanup_suppresses_only_oserror_after_rename(
    monkeypatch, tmp_path: Path
) -> None:
    """If a non-OSError is raised AFTER the staged file has already been os.replace'd onto dest (so
    the staged path no longer exists), the failure-cleanup `staged.unlink()` raises
    FileNotFoundError — which the `contextlib.suppress(OSError)` must swallow so the ORIGINAL
    exception propagates unchanged. Kills `suppress(OSError)` -> `suppress(None)`, under which the
    FileNotFoundError is not suppressed and a TypeError leaks out instead."""
    archive = _tar_gz_with_uv(tmp_path)
    dest_dir = tmp_path / "dest"
    real_fsync_path = uvman._fsync_path

    def _selective(path: Path) -> None:
        # staged-file fsync succeeds; the post-replace directory fsync raises a non-OSError,
        # entering the cleanup branch after the staged file is already gone.
        if path == dest_dir:
            raise ValueError("boom after replace, before dir fsync")
        real_fsync_path(path)

    monkeypatch.setattr(uvman, "_fsync_path", _selective)

    with pytest.raises(ValueError, match="boom after replace"):
        uvman._extract_uv(archive, dest_dir)

    # the rename had already committed before the failure, so the finished binary is at dest
    assert (dest_dir / EXE).exists()
    assert (dest_dir / EXE).read_bytes() == b"genuine-uv-bytes"
