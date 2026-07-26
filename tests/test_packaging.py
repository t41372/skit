"""pyproject.toml packaging invariants: no dead extras, wheel excludes catalog sources.

These read pyproject.toml directly with tomllib rather than invoking `uv build` — a full
build is exercised manually (and at release time by .github/workflows/release.yml) but is
too slow to run as a per-mutant/per-CI-matrix-cell pytest case. What's checked here is the
config that a real build reads, so a regression (e.g. someone re-adding the dead `serve`
extra, or dropping the wheel-exclude) is still caught fast and hermetically.
"""

from __future__ import annotations

import re
import tomllib
from pathlib import Path

import pytest

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


def test_mutmut_refreshes_all_runtime_package_data_in_a_reused_worktree() -> None:
    """mutmut only regenerates Python files inside an existing ``mutants/`` source
    tree.  Runtime package data would otherwise remain at the version copied when that
    tree was first created, so updated translations or the bundled skill could fail the
    baseline before any mutant runs.  ``also_copy`` directories are refreshed on every
    run with ``dirs_exist_ok=True``.  Discover the data files instead of naming today's
    two directories here, so future package data cannot silently acquire the same bug."""
    also_copy = PYPROJECT["tool"]["mutmut"]["also_copy"]
    refreshed = [ROOT / path for path in also_copy]
    package = ROOT / "src" / "skit"
    data_files = [
        path
        for path in package.rglob("*")
        if path.is_file()
        and path.suffix not in {".meta", ".py", ".pyc"}
        and "__pycache__" not in path.parts
    ]
    stale = [
        str(path.relative_to(ROOT))
        for path in data_files
        if not any(path.is_relative_to(root) for root in refreshed)
    ]
    assert data_files
    assert not stale, f"mutmut does not refresh runtime package data: {stale}"


def test_binary_spec_pins_every_tree_sitter_grammar() -> None:
    """The frozen binary's spec must name every grammar the analyzers import.

    Each ``import tree_sitter_<lang>`` in langs/ sits inside a try/except guard, so a
    grammar missing from the bundle does not crash the binary — it silently turns that
    language's analyzer into ``None`` and ships a build with parameter analysis quietly
    disabled. Discover the imports instead of naming today's three, so a new language's
    grammar cannot be added without the spec learning about it."""
    spec = (ROOT / "packaging" / "skit.spec").read_text(encoding="utf-8")
    grammars = {
        match.group(1)
        for path in (ROOT / "src" / "skit" / "langs").rglob("*.py")
        for match in re.finditer(
            r"^import (tree_sitter_\w+)", path.read_text(encoding="utf-8"), re.MULTILINE
        )
    }
    assert grammars
    missing = sorted(name for name in grammars if name not in spec)
    assert not missing, f"packaging/skit.spec does not collect: {missing}"


def test_release_workflow_and_packaging_files_agree() -> None:
    """The release workflow builds the binary from files in this repo; a rename that
    updates only one side would fail at tag time, when a release is already half-made."""
    workflow = (ROOT / ".github" / "workflows" / "release.yml").read_text(encoding="utf-8")
    for referenced in ("packaging/skit.spec", "packaging/smoke.py"):
        assert referenced in workflow
        assert (ROOT / referenced).is_file()
    # The build installs the project plus this group; a rename would leave the release
    # jobs installing nothing and failing on the first `pyinstaller` call.
    assert "--group packaging" in workflow
    assert "pyinstaller" in " ".join(PYPROJECT["dependency-groups"]["packaging"])


def test_version_is_single_sourced_from_the_distribution() -> None:
    """skit.__version__ mirrors the installed skit-cli metadata, which in turn comes
    from pyproject.toml at build time — one source, no drift (the old hand-synced
    literal in __init__.py once shipped a release with mismatched versions)."""
    from importlib.metadata import version

    import skit

    assert skit.__version__ == version("skit-cli")


def test_version_falls_back_when_no_distribution_is_installed(monkeypatch) -> None:
    """A bare checkout without an installed dist still imports (and says so)."""
    import importlib.metadata

    import skit

    def missing(_name: str) -> str:
        raise importlib.metadata.PackageNotFoundError

    skit.__version__  # noqa: B018 — resolve first, so delattr below has a value to restore
    monkeypatch.setattr(importlib.metadata, "version", missing)
    # Drop the memoized value so the next attribute access runs the resolver again;
    # monkeypatch restores the real one at teardown.
    monkeypatch.delattr(skit, "__version__")
    assert skit.__version__ == "0.0.0+unknown"


def test_version_is_resolved_once_and_then_memoized(monkeypatch) -> None:
    """`importlib.metadata` costs ~85 modules — the largest single import on skit's
    startup path — so the version resolves lazily, and at most once per interpreter.
    A second access must not reach the metadata layer again."""
    import importlib.metadata

    import skit

    calls: list[str] = []

    def counting(name: str) -> str:
        calls.append(name)
        return "9.9.9"

    skit.__version__  # noqa: B018 — as above: give delattr something to restore
    monkeypatch.setattr(importlib.metadata, "version", counting)
    monkeypatch.delattr(skit, "__version__")
    assert skit.__version__ == "9.9.9"
    assert skit.__version__ == "9.9.9"
    assert calls == ["skit-cli"]


def test_module_getattr_refuses_anything_but_the_version() -> None:
    """The lazy hook answers exactly one name; everything else must still raise the
    normal AttributeError, or a typo anywhere in skit would resolve to a version
    string instead of failing."""
    import skit

    with pytest.raises(AttributeError, match="module 'skit' has no attribute 'nope'"):
        skit.nope  # noqa: B018 — the attribute access IS the assertion
