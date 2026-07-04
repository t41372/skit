"""Atomic writes: every state/metadata file goes through tmp + os.replace (C7)."""

from __future__ import annotations

import contextlib
import os
import tempfile
from pathlib import Path
from typing import Any

import tomli_w


def atomic_write_bytes(path: Path, data: bytes) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    fd, tmp = tempfile.mkstemp(dir=path.parent, prefix=f".{path.name}.", suffix=".tmp")
    try:
        with os.fdopen(fd, "wb") as f:
            f.write(data)
            f.flush()
            os.fsync(f.fileno())
        os.replace(tmp, path)
    except BaseException:
        with contextlib.suppress(OSError):
            os.unlink(tmp)
        raise


def atomic_write_text(path: Path, text: str) -> None:
    atomic_write_bytes(path, text.encode("utf-8"))


def atomic_write_toml(path: Path, doc: dict[str, Any]) -> None:
    atomic_write_bytes(path, tomli_w.dumps(doc).encode("utf-8"))
