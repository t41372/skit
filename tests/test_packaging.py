"""pyproject.toml packaging invariants: no dead extras, wheel excludes catalog sources.

These read pyproject.toml directly with tomllib rather than invoking `uv build` — a full
build is exercised manually (and at release time by .github/workflows/release.yml) but is
too slow to run as a per-mutant/per-CI-matrix-cell pytest case. What's checked here is the
config that a real build reads, so a regression (e.g. someone re-adding the dead `serve`
extra, or dropping the wheel-exclude) is still caught fast and hermetically.
"""

from __future__ import annotations

import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
PYPROJECT = tomllib.loads((ROOT / "pyproject.toml").read_text(encoding="utf-8"))


def test_no_dead_optional_dependencies() -> None:
    # scripts/serve_preview.py (the only textual-serve consumer) is dev-only and not shipped
    # in the wheel; textual-serve belongs solely to [dependency-groups].dev. A public extra
    # here would let a user `pip install skit-cli[serve]` into installing a dependency that no
    # shipped code imports.
    assert "optional-dependencies" not in PYPROJECT["project"]


def test_wheel_excludes_catalog_sources() -> None:
    # The runtime (src/skit/i18n.py) loads only compiled .mo via stdlib gettext; .po/.pot
    # are maintainer inputs to scripts/i18n.py and should stay in the sdist, not the wheel.
    excludes = PYPROJECT["tool"]["uv"]["build-backend"]["wheel-exclude"]
    assert any(pattern.endswith("*.po") for pattern in excludes)
    assert any(pattern.endswith("*.pot") for pattern in excludes)
