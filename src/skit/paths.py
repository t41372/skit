"""Directory resolution. Everything goes through platformdirs; overridable via env vars (for tests).

SKIT_DATA_DIR / SKIT_STATE_DIR / SKIT_CONFIG_DIR override the matching root directory.
"""

from __future__ import annotations

import os
from pathlib import Path

from platformdirs import user_config_dir, user_data_dir, user_state_dir

_APP = "skit"


def data_dir() -> Path:
    override = os.environ.get("SKIT_DATA_DIR")
    return Path(override) if override else Path(user_data_dir(_APP))


def state_dir() -> Path:
    override = os.environ.get("SKIT_STATE_DIR")
    return Path(override) if override else Path(user_state_dir(_APP))


def config_dir() -> Path:
    override = os.environ.get("SKIT_CONFIG_DIR")
    return Path(override) if override else Path(user_config_dir(_APP))


def scripts_dir() -> Path:
    return data_dir() / "scripts"


def registry_path() -> Path:
    return data_dir() / "registry.toml"


def private_bin_dir() -> Path:
    return data_dir() / "bin"


def values_dir() -> Path:
    return state_dir() / "values"


def drafts_dir() -> Path:
    """Where authoring drafts live — skit's OWN data dir, not $TMPDIR: "your draft was
    kept" must be a promise the OS can't break (macOS reaps temp files in days), and an
    accumulation the user can see and manage beats invisible litter."""
    return data_dir() / "drafts"


def is_draft(path: Path) -> bool:
    """Whether `path` is one of skit's OWN kept drafts — the single definition every
    draft-aware behavior keys on (the TUI's resumable list, the consume-on-success
    unlink, and draft-aware kind inference). Both halves matter: the drafts directory
    scopes it, and the mkstemp `skit-` prefix keeps a user file merely parked in that
    directory from being treated (and consumed) as skit's artifact."""
    return path.name.startswith("skit-") and path.resolve().parent == drafts_dir().resolve()
