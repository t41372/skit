"""Mutation-kill tests for langs/javascript/deps.py — the stdlib-only npm/bun/deno resolver.

These pin behaviours the broader suite in test_js_deps.py leaves under-specified: that clean()
reports the OS strerror (not the raw exception repr) when node_modules won't go, that the
module-type manifest is 2-space pretty JSON, and that needs_install's staleness stamp folds in
BOTH the runner's own installer and the module flavor. No installer is ever spawned — the
subprocess seam is patched, exactly as the sibling suite does it.
"""

from __future__ import annotations

import subprocess
from pathlib import Path

import pytest

from skit.langs.base import NotExecutableError
from skit.langs.javascript import deps as js_deps


@pytest.fixture(autouse=True)
def _english(monkeypatch: pytest.MonkeyPatch):
    monkeypatch.setenv("SKIT_LANG", "en")  # message assertions read the English catalog


def _install_ok(calls: list[tuple[list[str], dict[str, object]]]):
    """A fake subprocess.run recording (argv, kwargs) and reporting a clean install."""

    def run(argv: list[str], **kwargs: object) -> subprocess.CompletedProcess[bytes]:
        calls.append((argv, dict(kwargs)))
        return subprocess.CompletedProcess(argv, 0, stdout=b"", stderr=b"")

    return run


# ==========================================================================
# clean(): loud node_modules failure carries the strerror, not the repr
# ==========================================================================


def test_clean_node_modules_failure_names_the_strerror_not_the_repr(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
):
    """When node_modules can't be torn down, the loud NotExecutableError's detail is the OS
    strerror ("Permission denied") — not the exception's full repr ("[Errno 13] ...: path"),
    which would leak an absolute path into a user-facing line and bury the actual reason."""
    (tmp_path / "node_modules").mkdir()

    def boom(path: object, onexc: object = None, **kwargs: object) -> None:
        raise OSError(13, "Permission denied", str(path))

    monkeypatch.setattr(js_deps.shutil, "rmtree", boom)
    with pytest.raises(NotExecutableError) as exc:
        js_deps.clean(tmp_path)
    detail = str(exc.value)
    assert "node_modules: Permission denied" in detail  # strerror, joined to the name
    assert "[Errno 13]" not in detail  # the mutant surfaces the exception repr instead


# ==========================================================================
# ensure_module_manifest(): the module-type manifest is 2-space pretty JSON
# ==========================================================================


def test_ensure_module_manifest_writes_two_space_pretty_json(tmp_path: Path):
    """A flavor-only package.json is pinned as 2-space-indented, key-ordered pretty JSON — the
    exact text node reads to learn a flattened script.js is really ESM. A different indent (3,
    or compact) is a different file that would rewrite on every run and drift from the marker."""
    js_deps.ensure_module_manifest(tmp_path, "module")
    written = (tmp_path / "package.json").read_text(encoding="utf-8")
    assert written == '{\n  "private": true,\n  "type": "module"\n}\n'


# ==========================================================================
# needs_install(): the staleness stamp folds in installer AND module flavor
# ==========================================================================


def test_needs_install_stamp_is_specific_to_the_runners_installer(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
):
    """needs_install reuses ensure_installed's own stamp: a marker written by a bun install reads
    as up-to-date for bun, but STALE for node — switching installers must trigger a reinstall, so
    the runner has to reach _resolve_manifest's stamp, not a fixed npm one."""
    monkeypatch.setattr(js_deps.shutil, "which", lambda name: f"/bin/{name}")
    monkeypatch.setattr(js_deps.subprocess, "run", _install_ok([]))
    js_deps.ensure_installed(tmp_path, ["chalk@^5"], "bun", {})  # stamps for bun's installer
    assert js_deps.needs_install(tmp_path, ["chalk@^5"], "bun") is False  # same installer: fresh
    assert js_deps.needs_install(tmp_path, ["chalk@^5"], "node") is True  # npm ≠ bun: stale


def test_needs_install_stamp_includes_the_module_type(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
):
    """The module flavor is part of the manifest and therefore the stamp: a marker stamped for a
    "module" manifest is stale the moment the type is dropped, so needs_install must carry
    module_type through to _resolve_manifest rather than pinning it to the default."""
    monkeypatch.setattr(js_deps.shutil, "which", lambda name: f"/bin/{name}")
    monkeypatch.setattr(js_deps.subprocess, "run", _install_ok([]))
    js_deps.ensure_installed(tmp_path, ["chalk"], "node", {}, module_type="module")
    assert js_deps.needs_install(tmp_path, ["chalk"], "node", module_type="module") is False
    assert js_deps.needs_install(tmp_path, ["chalk"], "node") is True  # flavor dropped ⇒ stale
