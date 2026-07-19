"""Per-script npm dependencies for JS/TS entries (langs/javascript/deps.py and its seams).

Covers the whole feature: the requirement→manifest translation, the staleness marker, the
clean-rebuild rule, the installer-per-runner table, RunnerLaunch's build/preflight hookup, the
import scanner behind the dependency suggestions, the store-level guards (reference mode, Python
constraints), the CLI add/deps surfaces, the mirror plumbing, and the injected-copy adjacency
(prefer_entry_dir). No test spawns a real installer — the subprocess seam is patched and its
argv/cwd/env asserted, mirroring how launch tests patch `_which`.
"""

from __future__ import annotations

import json
import subprocess
from datetime import datetime
from pathlib import Path
from typing import Any

import pytest
from typer.testing import CliRunner

from conftest import full_mirror
from skit import cli, config, store
from skit.langs import launch
from skit.langs.base import ArgvLaunch, InjectRequest, NotExecutableError
from skit.langs.javascript import deps as js_deps
from skit.langs.javascript.analyzer import external_imports
from skit.langs.registry import spec_for
from skit.models import Entry, Mode, ScriptMeta
from skit.params import ParamDecl

runner = CliRunner()


@pytest.fixture(autouse=True)
def tmp_store(tmp_path: Path, monkeypatch: pytest.MonkeyPatch):
    monkeypatch.setenv("SKIT_DATA_DIR", str(tmp_path / "data"))
    monkeypatch.setenv("SKIT_STATE_DIR", str(tmp_path / "state"))
    monkeypatch.setenv("SKIT_CONFIG_DIR", str(tmp_path / "config"))
    monkeypatch.setenv("SKIT_LANG", "en")


def _entry(
    tmp_path: Path,
    *,
    mode: Mode = "copy",
    dependencies: list[str] | None = None,
    body: str = "console.log(1)\n",
    source: str = "",
) -> Entry:
    d = tmp_path / "e"
    d.mkdir(exist_ok=True)
    if mode == "reference":
        original = tmp_path / "original.js"
        original.write_text(body, encoding="utf-8")
        source = str(original)
    meta = ScriptMeta(name="e", kind="js", mode=mode, dependencies=dependencies, source=source)
    entry = Entry(slug="e", meta=meta, dir=d)
    if mode == "copy":
        entry.script_path.write_text(body, encoding="utf-8")
    return entry


def _js_file(tmp_path: Path, body: str = 'import chalk from "chalk";\n', name: str = "t.mjs"):
    p = tmp_path / name
    p.write_text(body, encoding="utf-8")
    return p


# ==========================================================================
# split_requirement / manifest_text
# ==========================================================================


@pytest.mark.parametrize(
    ("req", "expected"),
    [
        ("chalk", ("chalk", "*")),
        ("chalk@^5", ("chalk", "^5")),
        ("chalk@5.6.2", ("chalk", "5.6.2")),
        ("chalk@", ("chalk", "*")),  # a trailing @ declares no range
        ("@scope/pkg", ("@scope/pkg", "*")),  # the scope's @ is not a range separator
        ("@scope/pkg@>=1,<2", ("@scope/pkg", ">=1,<2")),
        ("@scope", ("@scope", "*")),  # degenerate scope: whole text is the name
    ],
)
def test_split_requirement(req: str, expected: tuple[str, str]):
    assert js_deps.split_requirement(req) == expected


def test_manifest_text_is_deterministic_and_private():
    text = js_deps.manifest_text(["chalk@^5", " zod ", ""])
    assert text == js_deps.manifest_text(["chalk@^5", " zod ", ""])  # staleness-hash input
    assert '"private": true' in text
    assert '"chalk": "^5"' in text
    assert '"zod": "*"' in text  # whitespace stripped, bare name ranges to *
    assert text.endswith("\n")


def test_manifest_text_skips_an_empty_requirement():
    # A stray empty string (a doubled comma survives some splitters) records nothing — an empty
    # name key would be garbage npm rejects cryptically.
    assert '"dependencies": {}' in js_deps.manifest_text(["", "  "])


# ==========================================================================
# clean
# ==========================================================================


def test_clean_removes_manifest_lockfiles_and_node_modules(tmp_path: Path):
    for name in ("package.json", "package-lock.json", "bun.lock", "bun.lockb", "deno.lock"):
        (tmp_path / name).write_text("{}", encoding="utf-8")
    (tmp_path / "node_modules" / "chalk").mkdir(parents=True)
    (tmp_path / "meta.toml").write_text("", encoding="utf-8")
    js_deps.clean(tmp_path)
    assert sorted(p.name for p in tmp_path.iterdir()) == ["meta.toml"]


def test_clean_on_an_already_clean_dir_is_a_no_op(tmp_path: Path):
    js_deps.clean(tmp_path)  # nothing to remove, nothing raised
    assert list(tmp_path.iterdir()) == []


# ==========================================================================
# require_installer
# ==========================================================================


@pytest.mark.parametrize(
    ("runner_name", "installer"),
    [("node", "npm"), ("bun", "bun"), ("deno", "deno"), ("weird", "npm")],
)
def test_require_installer_maps_runner_to_its_own_installer(
    monkeypatch: pytest.MonkeyPatch, runner_name: str, installer: str
):
    seen: list[str] = []

    def fake_which(name: str) -> str:
        seen.append(name)
        return f"/bin/{name}"

    monkeypatch.setattr(js_deps.shutil, "which", fake_which)
    assert js_deps.require_installer(runner_name) == f"/bin/{installer}"
    assert seen == [installer]


def test_require_installer_missing_raises_126_family(monkeypatch: pytest.MonkeyPatch):
    monkeypatch.setattr(js_deps.shutil, "which", lambda name: None)
    with pytest.raises(NotExecutableError) as exc:
        js_deps.require_installer("node")
    assert "npm" in str(exc.value)


# ==========================================================================
# ensure_installed
# ==========================================================================


def _install_ok(calls: list[tuple[list[str], dict[str, object]]]):
    """A fake subprocess.run recording (argv, kwargs) and reporting success."""

    def run(argv: list[str], **kwargs: object) -> subprocess.CompletedProcess[bytes]:
        calls.append((argv, dict(kwargs)))
        return subprocess.CompletedProcess(argv, 0, stdout=b"", stderr=b"")

    return run


def test_ensure_installed_writes_manifest_runs_installer_and_stamps(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
):
    calls: list[tuple[list[str], dict[str, object]]] = []
    monkeypatch.setattr(js_deps.shutil, "which", lambda name: f"/bin/{name}")
    monkeypatch.setattr(js_deps.subprocess, "run", _install_ok(calls))
    js_deps.ensure_installed(tmp_path, ["chalk@^5"], "node", {"PATH": "/bin", "X": "y"})
    assert len(calls) == 1
    argv, kwargs = calls[0]
    assert argv == ["/bin/npm", "install", "--no-audit", "--no-fund", "--ignore-scripts"]
    assert kwargs["cwd"] == tmp_path
    assert kwargs["env"] == {"PATH": "/bin", "X": "y"}
    assert (tmp_path / "package.json").read_text(encoding="utf-8") == js_deps.manifest_text(
        ["chalk@^5"]
    )
    assert (tmp_path / "node_modules" / ".skit-deps-ok").is_file()


@pytest.mark.parametrize(
    ("runner_name", "argv_tail"),
    # bun gets --ignore-scripts too: its default is to run a built-in allow-list of packages'
    # postinstall scripts, so the flag is what keeps a declared dep from executing code at
    # install time (deno skips lifecycle scripts unasked, so it needs none).
    [("bun", ["install", "--ignore-scripts"]), ("deno", ["install"])],
)
def test_ensure_installed_uses_the_runners_own_installer(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, runner_name: str, argv_tail: list[str]
):
    calls: list[tuple[list[str], dict[str, object]]] = []
    monkeypatch.setattr(js_deps.shutil, "which", lambda name: f"/bin/{name}")
    monkeypatch.setattr(js_deps.subprocess, "run", _install_ok(calls))
    js_deps.ensure_installed(tmp_path, ["zod"], runner_name, {})
    assert calls[0][0] == [f"/bin/{runner_name}", *argv_tail]


def test_ensure_installed_fresh_marker_short_circuits(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
):
    calls: list[tuple[list[str], dict[str, object]]] = []
    monkeypatch.setattr(js_deps.shutil, "which", lambda name: f"/bin/{name}")
    monkeypatch.setattr(js_deps.subprocess, "run", _install_ok(calls))
    js_deps.ensure_installed(tmp_path, ["chalk"], "node", {})
    monkeypatch.setattr(
        js_deps.shutil, "which", lambda name: pytest.fail("fresh marker must not re-resolve")
    )
    monkeypatch.setattr(
        js_deps.subprocess, "run", lambda *a, **k: pytest.fail("fresh marker must not reinstall")
    )
    js_deps.ensure_installed(tmp_path, ["chalk"], "node", {})  # same deps, same installer


@pytest.mark.parametrize(
    ("change", "new_deps", "new_runner"),
    [("deps edited", ["chalk", "zod"], "node"), ("installer switched", ["chalk"], "bun")],
)
def test_ensure_installed_stale_marker_rebuilds_from_scratch(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
    change: str,
    new_deps: list[str],
    new_runner: str,
):
    """Staleness means REBUILD: the old tree and every lockfile go first (npm layered over a
    deno-layout tree installs the store's devDependencies; a removed dep would linger), then the
    fresh install runs."""
    calls: list[tuple[list[str], dict[str, object]]] = []
    monkeypatch.setattr(js_deps.shutil, "which", lambda name: f"/bin/{name}")
    monkeypatch.setattr(js_deps.subprocess, "run", _install_ok(calls))
    js_deps.ensure_installed(tmp_path, ["chalk"], "node", {})
    stray = tmp_path / "node_modules" / "leftover"
    stray.mkdir()
    (tmp_path / "deno.lock").write_text("{}", encoding="utf-8")
    js_deps.ensure_installed(tmp_path, new_deps, new_runner, {})
    assert len(calls) == 2, change
    assert not stray.exists()  # the foreign/orphaned tree was wiped before reinstalling
    assert not (tmp_path / "deno.lock").exists()
    assert (tmp_path / "package.json").read_text(encoding="utf-8") == js_deps.manifest_text(
        new_deps
    )


def test_ensure_installed_installer_failure_carries_its_stderr(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
):
    real_npm_stderr = (
        b"npm error code E404\n"
        b"npm error 404 Not Found - GET https://registry.npmjs.org/no-such-pkg\n"
        b"npm error 404 'no-such-pkg@*' is not in this registry.\n"
        b"\n"
        b"npm error A complete log of this run can be found in: /home/u/.npm/_logs/x.log\n"
    )
    monkeypatch.setattr(js_deps.shutil, "which", lambda name: f"/bin/{name}")
    monkeypatch.setattr(
        js_deps.subprocess,
        "run",
        lambda argv, **k: subprocess.CompletedProcess(argv, 1, stdout=b"", stderr=real_npm_stderr),
    )
    with pytest.raises(NotExecutableError) as exc:
        js_deps.ensure_installed(tmp_path, ["no-such-pkg"], "node", {})
    # The log-pointer line npm ENDS with is skipped; the cause line names the package.
    assert "Not Found - GET" in str(exc.value)
    assert "no-such-pkg" in str(exc.value)
    assert "A complete log" not in str(exc.value)
    assert not (tmp_path / "node_modules" / ".skit-deps-ok").exists()  # failure never stamps


def test_ensure_installed_failure_without_stderr_still_reports(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
):
    monkeypatch.setattr(js_deps.shutil, "which", lambda name: f"/bin/{name}")
    monkeypatch.setattr(
        js_deps.subprocess,
        "run",
        lambda argv, **k: subprocess.CompletedProcess(argv, 1, stdout=b"", stderr=b""),
    )
    with pytest.raises(NotExecutableError) as exc:
        js_deps.ensure_installed(tmp_path, ["x"], "node", {})
    assert "npm" in str(exc.value)


def test_ensure_installed_spawn_oserror_is_wrapped(tmp_path: Path, monkeypatch: pytest.MonkeyPatch):
    def boom(*a, **k):
        raise OSError("exec format error")

    monkeypatch.setattr(js_deps.shutil, "which", lambda name: f"/bin/{name}")
    monkeypatch.setattr(js_deps.subprocess, "run", boom)
    with pytest.raises(NotExecutableError) as exc:
        js_deps.ensure_installed(tmp_path, ["x"], "node", {})
    assert "exec format error" in str(exc.value)


def test_ensure_installed_missing_installer_raises_before_touching_the_dir(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
):
    monkeypatch.setattr(js_deps.shutil, "which", lambda name: None)
    with pytest.raises(NotExecutableError):
        js_deps.ensure_installed(tmp_path, ["chalk"], "node", {})
    assert not (tmp_path / "package.json").exists()


def test_ensure_installed_stamps_even_when_installer_creates_no_node_modules(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
):
    # A dep-less manifest (every requirement nameless) may install nothing at all; the marker
    # directory is created so the stamp still lands and the next run short-circuits.
    monkeypatch.setattr(js_deps.shutil, "which", lambda name: f"/bin/{name}")
    monkeypatch.setattr(js_deps.subprocess, "run", _install_ok([]))
    js_deps.ensure_installed(tmp_path, ["@5"], "node", {})
    assert (tmp_path / "node_modules" / ".skit-deps-ok").is_file()


# ==========================================================================
# external_imports (the dependency scanner)
# ==========================================================================


def test_external_imports_covers_all_import_forms():
    text = (
        'import chalk from "chalk";\n'
        'import { z } from "zod";\n'
        'export { x } from "commander";\n'
        'const dyn = await import("execa");\n'
        'const cjs = require("rimraf");\n'
        'import chalk2 from "chalk";\n'  # duplicate: reported once
    )
    assert external_imports(text) == ["chalk", "zod", "commander", "execa", "rimraf"]


def test_external_imports_excludes_non_packages():
    text = (
        'import fs from "node:fs";\n'
        'import path from "path";\n'  # bare builtin
        'import local from "./util.mjs";\n'
        'import abs from "/opt/x.js";\n'
        'import n from "npm:chalk@5";\n'  # deno-native — declaring it would double-manage
        'import j from "jsr:@std/fs";\n'
        'import remote from "https://esm.sh/preact";\n'
        'import d from "data:text/javascript,export default 1";\n'
        'import log from "#internal/log";\n'  # Node subpath import (package.json "imports" field)
        'import cfg from "#config";\n'  # bare subpath import
    )
    assert external_imports(text) == []


@pytest.mark.parametrize(
    "specifier",
    ["@scope/", "@scope//pkg", "@/pkg", "@only-a-scope"],
)
def test_external_imports_rejects_malformed_scoped_specifiers(specifier: str):
    # A scoped specifier needs both "@scope" and "/name"; a bare scope, an empty scope, or an
    # empty name resolves to no installable package — recording it would only make npm choke.
    assert external_imports(f'import x from "{specifier}";') == []


def test_external_imports_maps_deep_imports_to_the_package_root():
    text = (
        'import fp from "lodash/fp";\n'
        'import cmd from "@aws-sdk/client-s3/commands";\n'
        'import a from "@a/b";\n'  # a single-char scope is still a real package
    )
    assert external_imports(text) == ["lodash", "@aws-sdk/client-s3", "@a/b"]


def test_external_imports_skips_unreadable_specifiers():
    text = (
        "const a = require(name);\n"  # not a string literal
        'const b = require("a", "b");\n'  # not the single-argument shape
        'const c = notrequire("pkg");\n'  # not require/import
        "const d = require();\n"  # no argument at all
        "const e = require(`tpl`);\n"  # template string: may interpolate
    )
    assert external_imports(text) == []


def test_external_imports_reads_typescript_under_the_ts_grammar():
    text = 'import type { X } from "type-fest";\nimport { t } from "@trpc/server";\n'
    assert external_imports(text, lang="ts") == ["type-fest", "@trpc/server"]


def test_external_imports_degrades_to_empty_on_a_parse_error():
    assert external_imports("import broken from ;") == []


def test_external_imports_ignores_an_import_statement_without_a_string_source():
    # `import x = require(...)` (TS import-equals) has no plain string `source` field under the
    # js grammar; the walk must skip it rather than crash.
    assert external_imports("import x from 1;") == []


# ==========================================================================
# RunnerLaunch: build installs, preflight checks, sweep
# ==========================================================================


def _runner_env(monkeypatch: pytest.MonkeyPatch):
    monkeypatch.setattr("skit.langs.launch._which", {"node": "/bin/node"}.get)


def test_build_installs_declared_deps_with_the_resolved_runner(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
):
    _runner_env(monkeypatch)
    seen: dict[str, Any] = {}

    def fake_ensure(entry_dir, dependencies, runner_name, env, *, module_type=""):
        seen.update(
            dir=entry_dir, deps=dependencies, runner=runner_name, env=env, mtype=module_type
        )

    monkeypatch.setattr(js_deps, "ensure_installed", fake_ensure)
    monkeypatch.setenv("NPM_CONFIG_REGISTRY", "")  # defer rule: empty means unset
    config.save_mirror(full_mirror())
    entry = _entry(tmp_path, dependencies=["chalk"])
    launch.RunnerLaunch().build(entry, [], None, None)
    assert seen["dir"] == entry.dir
    assert seen["deps"] == ["chalk"]
    assert seen["runner"] == "node"
    assert seen["env"]["NPM_CONFIG_REGISTRY"] == config.NPM_REGISTRY_MIRROR  # mirror overlaid


@pytest.mark.parametrize(
    ("mode", "dependencies"),
    [("copy", None), ("reference", ["chalk"])],
)
def test_build_skips_the_engine_without_copy_mode_deps(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, mode: Mode, dependencies: list[str] | None
):
    _runner_env(monkeypatch)
    monkeypatch.setattr(
        js_deps, "ensure_installed", lambda *a, **k: pytest.fail("engine must not run")
    )
    entry = _entry(tmp_path, mode=mode, dependencies=dependencies)
    payload = launch.RunnerLaunch().build(entry, [], None, None)
    assert isinstance(payload, ArgvLaunch)
    assert entry.script_path.name in " ".join(payload.argv)


def test_preflight_requires_the_installer_when_deps_are_declared(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
):
    _runner_env(monkeypatch)
    monkeypatch.setattr(js_deps.shutil, "which", lambda name: None)  # npm missing
    entry = _entry(tmp_path, dependencies=["chalk"])
    with pytest.raises(NotExecutableError) as exc:
        launch.RunnerLaunch().preflight(entry)
    assert "npm" in str(exc.value)


def test_preflight_without_deps_does_not_ask_for_an_installer(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
):
    _runner_env(monkeypatch)
    monkeypatch.setattr(
        js_deps, "require_installer", lambda name: pytest.fail("no deps, no installer check")
    )
    launch.RunnerLaunch().preflight(_entry(tmp_path))


def test_build_sweeps_aged_injected_leftovers_but_not_fresh_ones(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
):
    import os

    _runner_env(monkeypatch)
    monkeypatch.setattr(js_deps, "ensure_installed", lambda *a, **k: None)
    entry = _entry(tmp_path)  # NO deps: the sweep must not depend on the declaration surviving
    aged = entry.dir / ".injected-dead.js"
    aged.write_text("x", encoding="utf-8")
    os.utime(aged, (1, 1))  # crashed eons ago
    fresh = entry.dir / ".injected-live.js"
    fresh.write_text("x", encoding="utf-8")  # a concurrent run's live copy
    launch.RunnerLaunch().build(entry, [], None, None)
    assert not aged.exists()
    assert fresh.exists()


# ==========================================================================
# write_injected adjacency (prefer_entry_dir) and the JS injector's use of it
# ==========================================================================


def test_write_injected_prefers_entry_dir_when_asked(tmp_path: Path):
    from skit import rewrite

    path = rewrite.write_injected(tmp_path, "console.log(1)\n", suffix=".js", prefer_entry_dir=True)
    try:
        assert path.parent == tmp_path
    finally:
        path.unlink()


def test_write_injected_prefer_entry_dir_falls_back_to_os_temp(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
):
    import tempfile

    real_mkstemp = tempfile.mkstemp

    def flaky_mkstemp(*args, **kwargs):
        if kwargs.get("dir") is not None:  # the entry_dir attempt
            raise OSError("entry dir on a read-only volume")
        return real_mkstemp(*args, **kwargs)

    from skit import rewrite

    monkeypatch.setattr(tempfile, "mkstemp", flaky_mkstemp)
    path = rewrite.write_injected(tmp_path, "console.log(1)\n", suffix=".js", prefer_entry_dir=True)
    try:
        assert path.parent != tmp_path
    finally:
        path.unlink()


def test_js_injector_honors_prefer_entry_dir(tmp_path: Path):
    from skit.langs.javascript import inject as js_inject

    request = InjectRequest(
        text='const MESSAGE = "x";\nconsole.log(MESSAGE);\n',
        specs=[ParamDecl(name="MESSAGE", binding="const", type="str", default="x")],
        values={"MESSAGE": "y"},
        entry_dir=tmp_path,
        prefer_entry_dir=True,
    )
    result = js_inject.inject(request)
    assert result.path is not None
    try:
        assert result.path.parent == tmp_path
    finally:
        result.path.unlink()


def test_flows_marks_prefer_entry_dir_only_for_deps_managed_npm_copies(tmp_path: Path):
    """The flows-side computation: npm flavor + copy mode + declared deps ⇒ True; drop any one
    leg and the injected copy stays in the OS temp dir (the historical location)."""
    from skit import flows
    from skit.langs.javascript import inject as js_inject

    seen: list[bool] = []
    real = js_inject.inject

    def spying(request: InjectRequest, *, lang: str = "js"):
        seen.append(request.prefer_entry_dir)
        return real(request, lang=lang)

    from skit.langs.javascript import io as js_io

    managed = js_io.write_params(
        'const M = "x";\nconsole.log(M);\n',
        [ParamDecl(name="M", binding="const", type="str", default="x")],
    )
    src = _js_file(tmp_path, managed, name="m.mjs")
    entry = store.add_script(src, kind="js", name="m")

    for deps, expected in ((["chalk"], True), (None, False)):
        seen.clear()
        entry.meta.dependencies = deps
        plan = flows.plan_for_entry(entry)
        asm = flows.assemble(plan, {"M": "y"}, [], cwd=tmp_path, env={}, now=datetime(2026, 7, 15))
        with pytest.MonkeyPatch.context() as mp:
            mp.setattr(js_inject, "inject", spying)
            mp.setattr(
                "skit.launcher.run_entry",
                lambda *a, **k: 0,
            )
            flows.execute(entry, plan, asm, emit=lambda line: None)
        assert seen == [expected], f"dependencies={deps}"


# ==========================================================================
# store.update_dependencies guards + cleanup
# ==========================================================================


def _added_js(tmp_path: Path, *, ref: bool = False) -> Entry:
    src = _js_file(tmp_path)
    return store.add_script(src, kind="js", name="t", mode="reference" if ref else "copy")


def test_update_dependencies_js_copy_records_meta_without_touching_the_script(tmp_path: Path):
    entry = _added_js(tmp_path)
    before = entry.script_path.read_text(encoding="utf-8")
    updated = store.update_dependencies("t", ["chalk@^5"])
    assert updated.meta.dependencies == ["chalk@^5"]
    assert entry.script_path.read_text(encoding="utf-8") == before  # no PEP 723 sync for js


def test_update_dependencies_js_reference_is_refused(tmp_path: Path):
    _added_js(tmp_path, ref=True)
    # A refusal is a StoreUsageError (→ the CLI's usage exit code), not a plain operational error.
    with pytest.raises(store.StoreUsageError) as exc:
        store.update_dependencies("t", ["chalk"])
    assert "reference-mode" in str(exc.value)


def test_update_dependencies_js_python_constraint_is_refused(tmp_path: Path):
    _added_js(tmp_path)
    with pytest.raises(store.StoreUsageError) as exc:
        store.update_dependencies("t", ["chalk"], requires_python=">=3.11")
    assert "Python constraint" in str(exc.value)


def test_update_dependencies_js_clearing_sweeps_the_materialized_env(tmp_path: Path):
    entry = _added_js(tmp_path)
    store.update_dependencies("t", ["chalk"])
    (entry.dir / "package.json").write_text("{}", encoding="utf-8")
    (entry.dir / "node_modules").mkdir()
    store.update_dependencies("t", [])
    assert not (entry.dir / "package.json").exists()
    assert not (entry.dir / "node_modules").exists()
    assert store.resolve("t").meta.dependencies is None


def test_update_dependencies_js_reference_clearing_is_allowed(tmp_path: Path):
    # Only *declaring* deps is copy-only; clearing (a no-op or a cleanup) must never be refused,
    # so a mode switch can't strand an undeletable record.
    _added_js(tmp_path, ref=True)
    assert store.update_dependencies("t", []).meta.dependencies is None


# ==========================================================================
# CLI: add-time suggestion and the deps command
# ==========================================================================


def test_add_js_no_input_records_scanned_imports(tmp_path: Path):
    src = _js_file(tmp_path, 'import chalk from "chalk";\nimport { z } from "zod";\n')
    result = runner.invoke(cli.app, ["add", str(src), "--no-input"])
    assert result.exit_code == 0
    assert store.resolve("t").meta.dependencies == ["chalk", "zod"]
    assert "chalk, zod" in result.output  # the add summary names them


def test_add_js_explicit_dep_flags_win_without_scanning(tmp_path: Path):
    src = _js_file(tmp_path)
    result = runner.invoke(
        cli.app, ["add", str(src), "--dep", "zod@3", "--dep", "execa", "--no-input"]
    )
    assert result.exit_code == 0
    assert store.resolve("t").meta.dependencies == ["zod@3", "execa"]


def test_add_js_without_external_imports_records_nothing(tmp_path: Path):
    src = _js_file(tmp_path, 'import fs from "node:fs";\nconsole.log(1);\n')
    result = runner.invoke(cli.app, ["add", str(src), "--no-input"])
    assert result.exit_code == 0
    assert store.resolve("t").meta.dependencies is None


def test_add_js_reference_mode_asks_no_deps_question(tmp_path: Path):
    src = _js_file(tmp_path)
    result = runner.invoke(cli.app, ["add", str(src), "--ref", "--no-input"])
    assert result.exit_code == 0
    assert store.resolve("t").meta.dependencies is None


def test_resolve_npm_dependencies_interactive_accepts_the_suggestion(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
):
    src = _js_file(tmp_path)
    monkeypatch.setattr("sys.stdin.isatty", lambda: True, raising=False)
    monkeypatch.setattr("rich.prompt.Prompt.ask", lambda *a, **k: k.get("default", ""))
    spec = spec_for("js")
    assert spec is not None
    scanner = spec.dep_scanner
    assert scanner is not None
    assert cli._resolve_npm_dependencies(src, None, False, scanner) == ["chalk"]


def test_resolve_npm_dependencies_interactive_dash_declines(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
):
    src = _js_file(tmp_path)
    monkeypatch.setattr("sys.stdin.isatty", lambda: True, raising=False)
    monkeypatch.setattr("rich.prompt.Prompt.ask", lambda *a, **k: " - ")
    spec = spec_for("js")
    assert spec is not None
    scanner = spec.dep_scanner
    assert scanner is not None
    assert cli._resolve_npm_dependencies(src, None, False, scanner) == []


def test_resolve_npm_dependencies_interactive_edit_splits_requirements(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
):
    src = _js_file(tmp_path)
    monkeypatch.setattr("sys.stdin.isatty", lambda: True, raising=False)
    monkeypatch.setattr("rich.prompt.Prompt.ask", lambda *a, **k: "chalk@^5, zod")
    spec = spec_for("js")
    assert spec is not None
    scanner = spec.dep_scanner
    assert scanner is not None
    assert cli._resolve_npm_dependencies(src, None, False, scanner) == ["chalk@^5", "zod"]


def test_resolve_npm_dependencies_without_scanner_suggests_nothing(tmp_path: Path):
    src = _js_file(tmp_path)
    assert cli._resolve_npm_dependencies(src, None, True, None) == []


def test_resolve_npm_dependencies_unreadable_file_suggests_nothing(tmp_path: Path):
    spec = spec_for("js")
    assert spec is not None
    scanner = spec.dep_scanner
    assert scanner is not None
    assert cli._resolve_npm_dependencies(tmp_path / "gone.mjs", None, True, scanner) == []


def test_deps_command_sets_and_shows_js_dependencies(tmp_path: Path):
    _added_js(tmp_path)
    result = runner.invoke(cli.app, ["deps", "t", "--dep", "chalk@^5"])
    assert result.exit_code == 0
    view = runner.invoke(cli.app, ["deps", "t"])
    assert "chalk@^5" in view.output
    as_json = runner.invoke(cli.app, ["deps", "t", "--json"])
    assert '"chalk@^5"' in as_json.output


def test_deps_command_python_flag_on_js_is_refused(tmp_path: Path):
    _added_js(tmp_path)
    result = runner.invoke(cli.app, ["deps", "t", "--python", ">=3.11"])
    # A refused flag is a usage error (exit 2), the same code `skit add` gives — not the
    # generic 1 that used to make the two commands disagree.
    assert result.exit_code == 2
    assert "Python constraint" in result.output


def test_deps_command_dep_on_js_reference_is_refused(tmp_path: Path):
    _added_js(tmp_path, ref=True)
    result = runner.invoke(cli.app, ["deps", "t", "--dep", "chalk"])
    assert result.exit_code == 2
    assert "reference-mode" in result.output


# ==========================================================================
# TUI: the direct add lane records scanned deps; settings gates the fields
# ==========================================================================


async def test_tui_direct_add_records_scanned_js_dependencies(tmp_path: Path):
    from textual.widgets import Input as TInput

    from skit import tui
    from skit.tui_add import AddReviewScreen, AddSourceScreen

    src = _js_file(tmp_path, 'import chalk from "chalk";\nimport fs from "node:fs";\n')
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        screen = AddSourceScreen()
        app.push_screen(screen)
        await pilot.pause()
        screen.query_one("#add-path", TInput).value = str(src)
        screen.action_continue_add()
        await pilot.pause()
        review = app.screen
        assert isinstance(review, AddReviewScreen)  # js gets the panel too now
        assert review.query_one("#rv-deps", TInput).value == "chalk"  # builtin excluded
        review.action_accept()
        await pilot.pause()
    (entry,) = store.list_entries()
    assert entry.meta.kind == "js"
    assert entry.meta.dependencies == ["chalk"]  # the builtin was not suggested


async def test_tui_direct_add_js_without_imports_records_none(tmp_path: Path):
    from textual.widgets import Input as TInput

    from skit import tui
    from skit.tui_add import AddReviewScreen, AddSourceScreen

    src = _js_file(tmp_path, "console.log(1);\n")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        screen = AddSourceScreen()
        app.push_screen(screen)
        await pilot.pause()
        screen.query_one("#add-path", TInput).value = str(src)
        screen.action_continue_add()
        await pilot.pause()
        review = app.screen
        assert isinstance(review, AddReviewScreen)
        assert review.query_one("#rv-deps", TInput).value == ""
        review.action_accept()
        await pilot.pause()
    (entry,) = store.list_entries()
    assert entry.meta.dependencies is None


async def test_tui_direct_add_survives_the_source_vanishing_after_the_copy(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
):
    """The panel scans the text ONCE at open; a source vanishing right after the copy
    landed no longer loses the suggestions (the old direct lane re-read and raced)."""
    from textual.widgets import Input as TInput

    from skit import tui
    from skit.tui_add import AddReviewScreen, AddSourceScreen

    real_add_script = store.add_script

    def adding_and_vanishing(path, **kwargs):
        entry = real_add_script(path, **kwargs)
        path.unlink()  # the source disappears right after the copy landed
        return entry

    monkeypatch.setattr(store, "add_script", adding_and_vanishing)
    src = _js_file(tmp_path, 'import chalk from "chalk";\n')
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        screen = AddSourceScreen()
        app.push_screen(screen)
        await pilot.pause()
        screen.query_one("#add-path", TInput).value = str(src)
        screen.action_continue_add()
        await pilot.pause()
        review = app.screen
        assert isinstance(review, AddReviewScreen)
        review.action_accept()  # the panel scanned at open — no re-read can race
        await pilot.pause()
    (entry,) = store.list_entries()
    assert entry.meta.dependencies == ["chalk"]  # the suggestions survived the vanish


async def test_settings_js_copy_offers_deps_without_python_constraint(tmp_path: Path):
    from textual.widgets import Input as TInput

    from skit import tui
    from skit.tui_settings import ScriptSettingsScreen

    entry = _added_js(tmp_path)
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        screen = ScriptSettingsScreen(entry)
        app.push_screen(screen)
        await pilot.pause()
        assert screen.query("#st-deps")  # npm deps are editable...
        assert not screen.query("#st-python")  # ...but a Python pin is never offered
        screen.query_one("#st-deps", TInput).value = "chalk@^5, zod"
        screen.action_save()
        await pilot.pause()
    assert store.resolve("t").meta.dependencies == ["chalk@^5", "zod"]


async def test_settings_js_reference_hides_the_deps_section(tmp_path: Path):
    from skit import tui
    from skit.tui_settings import ScriptSettingsScreen

    entry = _added_js(tmp_path, ref=True)
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        screen = ScriptSettingsScreen(entry)
        app.push_screen(screen)
        await pilot.pause()
        assert not screen.query("#st-deps")  # reference js: nothing skit could install for it
        screen.action_save()  # saving without the widgets must not crash on a missing query
        await pilot.pause()
    assert store.resolve("t").meta.dependencies is None


async def test_prefs_custom_mirror_saves_the_npm_registry(tmp_path: Path):
    from textual.widgets import Input as TInput
    from textual.widgets import RadioButton, RadioSet

    from skit import tui
    from skit.tui_prefs import PreferencesScreen

    app = tui.MenuApp()
    async with app.run_test() as pilot:
        screen = PreferencesScreen()
        app.push_screen(screen)
        await pilot.pause()
        radio = screen.query_one("#pf-mirror-npm", RadioSet)
        buttons = list(radio.query(RadioButton))
        custom_index = next(i for i, b in enumerate(buttons) if str(b.label) == "custom")
        buttons[custom_index].value = True  # click the npm axis's "custom"
        await pilot.pause()
        assert screen.query_one("#pf-npm", TInput).display is True  # its own axis reveals it
        screen.query_one("#pf-npm", TInput).value = "https://npm.example"
        screen.action_save()
        await pilot.pause()
    mirror = config.load_mirror()
    assert mirror.enabled
    assert mirror.npm == "https://npm.example"
    assert mirror.pypi == ""  # the npm axis alone enables nothing PyPI-side


# ==========================================================================
# mirror plumbing
# ==========================================================================


def test_npm_axis_is_independent_of_the_pypi_axis():
    """The npm registry is its own mirror axis: setting only the PyPI axis must NOT drag a
    (differently-vendored) npm registry along, and the npm axis works alone."""
    config.save_mirror(config.compose(pypi=config.PYPI_PRESETS["tsinghua"]))
    assert config.load_mirror().npm == ""
    config.save_mirror(config.compose(npm=config.NPM_PRESETS["npmmirror"]))
    m = config.load_mirror()
    assert m.enabled
    assert m.npm == config.NPM_REGISTRY_MIRROR
    assert m.pypi == ""


def test_mirror_npm_round_trips_through_save_and_load():
    config.save_mirror(
        config.MirrorConfig(enabled=True, pypi="https://p", npm="https://my.registry")
    )
    assert config.load_mirror().npm == "https://my.registry"


def test_mirror_env_sets_npm_registry_and_defers_to_the_user(monkeypatch: pytest.MonkeyPatch):
    config.save_mirror(full_mirror())
    assert config.mirror_env({})["NPM_CONFIG_REGISTRY"] == config.NPM_REGISTRY_MIRROR
    for var in ("NPM_CONFIG_REGISTRY", "npm_config_registry"):
        overlay = config.mirror_env({var: "https://user.registry"})
        assert "NPM_CONFIG_REGISTRY" not in overlay, var  # a truthy user value wins
    # An empty value means "unset", so the mirror still applies.
    assert "NPM_CONFIG_REGISTRY" in config.mirror_env({"NPM_CONFIG_REGISTRY": ""})


def test_mirror_env_without_npm_url_sets_nothing_npm(monkeypatch: pytest.MonkeyPatch):
    config.save_mirror(config.MirrorConfig(enabled=True, pypi="https://p"))
    assert "NPM_CONFIG_REGISTRY" not in config.mirror_env({})


def test_load_mirror_type_hardens_a_hand_edited_npm_value(tmp_path: Path):
    config.save_config({"mirror": {"enabled": True, "npm": 123}})
    assert config.load_mirror().npm == ""


# ==========================================================================
# review fixes: npm splitter, module type, lock, loud failures, refusals
# ==========================================================================


def test_split_requirements_keeps_scoped_packages_apart():
    # The PEP 508 splitter merges ", @scope/pkg" into its neighbor (names must start with a
    # letter/digit there) — the exact bug that corrupted an accepted suggestion. The npm
    # splitter must keep them apart; npm ranges never contain commas, so a plain split is safe.
    assert js_deps.split_requirements("chalk, @aws-sdk/client-s3") == [
        "chalk",
        "@aws-sdk/client-s3",
    ]
    assert js_deps.split_requirements(" zod@^3 ,, @trpc/server@10 , ") == [
        "zod@^3",
        "@trpc/server@10",
    ]
    assert js_deps.split_requirements("") == []


def test_interactive_accept_of_a_scoped_suggestion_round_trips(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
):
    """The reviewer's repro: pressing Enter to accept a scanned suggestion that contains a
    scoped package must record the same list the scanner produced — not a merged blob."""
    src = _js_file(
        tmp_path,
        'import chalk from "chalk";\nimport { S3Client } from "@aws-sdk/client-s3";\n',
    )
    monkeypatch.setattr("sys.stdin.isatty", lambda: True, raising=False)
    monkeypatch.setattr("rich.prompt.Prompt.ask", lambda *a, **k: k.get("default", ""))
    spec = spec_for("js")
    assert spec is not None
    assert cli._resolve_npm_dependencies(src, None, False, spec.dep_scanner) == [
        "chalk",
        "@aws-sdk/client-s3",
    ]


async def test_settings_save_keeps_scoped_packages_apart(tmp_path: Path):
    from textual.widgets import Input as TInput

    from skit import tui
    from skit.tui_settings import ScriptSettingsScreen

    entry = _added_js(tmp_path)
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        screen = ScriptSettingsScreen(entry)
        app.push_screen(screen)
        await pilot.pause()
        screen.query_one("#st-deps", TInput).value = "chalk, @aws-sdk/client-s3"
        screen.action_save()
        await pilot.pause()
    assert store.resolve("t").meta.dependencies == ["chalk", "@aws-sdk/client-s3"]


@pytest.mark.parametrize(
    ("source", "expected"),
    [
        ("/home/u/tool.mjs", "module"),
        ("/home/u/tool.MJS", "module"),
        ("C:\\u\\tool.cjs", "commonjs"),
        ("/home/u/tool.mts", "module"),
        ("/home/u/tool.cts", "commonjs"),
        ("/home/u/tool.js", ""),  # no explicit signal: omit and let node decide
        ("noext", ""),
        ("", ""),
    ],
)
def test_module_type_for(source: str, expected: str):
    assert js_deps.module_type_for(source) == expected


def test_manifest_text_carries_the_module_type():
    text = js_deps.manifest_text(["chalk"], module_type="module")
    assert '"type": "module"' in text
    assert '"type"' not in js_deps.manifest_text(["chalk"])


def test_build_passes_the_original_extensions_module_type(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
):
    """A .mjs source stored as script.js keeps its explicit ESM signal: the manifest gets
    "type": "module", so node runs the copy without the MODULE_TYPELESS reparse warning."""
    _runner_env(monkeypatch)
    seen: dict[str, Any] = {}

    def fake_ensure(entry_dir, dependencies, runner_name, env, *, module_type=""):
        seen["mtype"] = module_type

    monkeypatch.setattr(js_deps, "ensure_installed", fake_ensure)
    src = _js_file(tmp_path, 'import chalk from "chalk";\n', name="orig.mjs")
    entry = store.add_script(src, kind="js", name="m")
    entry = store.update_dependencies("m", ["chalk"])
    launch.RunnerLaunch().build(entry, [], None, None)
    assert seen["mtype"] == "module"


def test_install_lock_reclaims_a_stale_holder_and_releases(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
):
    import os

    lock = tmp_path / ".skit-deps.lock"
    lock.write_text("", encoding="utf-8")
    os.utime(lock, (1, 1))  # a crashed install from eons ago must not wedge the entry
    with js_deps._install_lock(tmp_path):
        assert lock.exists()  # (re)taken by us
    assert not lock.exists()  # released on exit


def test_install_lock_waits_for_a_live_holder(tmp_path: Path, monkeypatch: pytest.MonkeyPatch):
    lock = tmp_path / ".skit-deps.lock"
    lock.write_text("", encoding="utf-8")  # fresh mtime: a live install holds it
    polls: list[float] = []

    def fake_sleep(seconds: float) -> None:
        polls.append(seconds)
        lock.unlink()  # the holder finishes during our wait

    monkeypatch.setattr(js_deps.time, "sleep", fake_sleep)
    with js_deps._install_lock(tmp_path):
        pass
    assert polls  # we waited instead of stealing the live holder's lock


def test_ensure_installed_serializes_under_the_entry_lock(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
):
    lock_states: list[bool] = []

    def run(argv: list[str], **kwargs: object) -> subprocess.CompletedProcess[bytes]:
        lock_states.append((tmp_path / ".skit-deps.lock").exists())
        return subprocess.CompletedProcess(argv, 0, stdout=b"", stderr=b"")

    monkeypatch.setattr(js_deps.shutil, "which", lambda name: f"/bin/{name}")
    monkeypatch.setattr(js_deps.subprocess, "run", run)
    js_deps.ensure_installed(tmp_path, ["chalk"], "node", {})
    assert lock_states == [True]  # the installer ran while the entry lock was held
    assert not (tmp_path / ".skit-deps.lock").exists()  # and it was released after


def test_clean_failure_is_loud_not_silent(tmp_path: Path, monkeypatch: pytest.MonkeyPatch):
    (tmp_path / "package.json").write_text("{}", encoding="utf-8")

    def stubborn_unlink(self: Path, missing_ok: bool = False) -> None:
        raise PermissionError(13, "held by another process", str(self))

    monkeypatch.setattr(Path, "unlink", stubborn_unlink)
    with pytest.raises(NotExecutableError) as exc:
        js_deps.clean(tmp_path)
    assert "package.json" in str(exc.value)


def test_clean_rmtree_failure_is_loud(tmp_path: Path, monkeypatch: pytest.MonkeyPatch):
    # A silently half-deleted node_modules would be stamped as good by the next install —
    # the exact Windows read-only-file failure mode rmtree(ignore_errors=True) used to hide.
    (tmp_path / "node_modules").mkdir()

    def failing_rmtree(path, onexc=None, **kwargs):
        assert onexc is not None
        onexc(None, str(path / "stuck"), OSError("read-only"))

    monkeypatch.setattr(js_deps.shutil, "rmtree", failing_rmtree)
    with pytest.raises(NotExecutableError) as exc:
        js_deps.clean(tmp_path)
    assert "stuck" in str(exc.value)


def test_update_dependencies_surfaces_clean_failure_as_store_error(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
):
    _added_js(tmp_path)
    store.update_dependencies("t", ["chalk"])

    def boom(entry_dir: Path) -> None:
        raise NotExecutableError("node_modules/stuck: held open")

    monkeypatch.setattr("skit.langs.javascript.deps.clean", boom)
    with pytest.raises(store.StoreError) as exc:
        store.update_dependencies("t", [])
    assert "stuck" in str(exc.value)
    # The sweep runs BEFORE the meta write, so a failed clear leaves the record untouched —
    # genuinely retryable, not a half-applied "dependencies=None with node_modules still there".
    assert store.resolve("t").meta.dependencies == ["chalk"]


def test_clean_sweeps_aged_injected_leftovers(tmp_path: Path):
    import os

    stranded = tmp_path / ".injected-crash.js"
    stranded.write_text("secret", encoding="utf-8")
    os.utime(stranded, (1, 1))
    js_deps.clean(tmp_path)  # a deps --clear must not strand a secret-bearing leftover
    assert not stranded.exists()


def test_add_js_ref_with_dep_is_refused_loudly(tmp_path: Path):
    src = _js_file(tmp_path)
    result = runner.invoke(cli.app, ["add", str(src), "--ref", "--dep", "chalk", "--no-input"])
    assert result.exit_code == 2  # usage error, and nothing was added
    assert "Reference-mode" in result.output
    assert not store.list_entries()


def test_add_js_with_python_flag_is_refused_loudly(tmp_path: Path):
    src = _js_file(tmp_path)
    result = runner.invoke(cli.app, ["add", str(src), "--python", ">=3.11", "--no-input"])
    assert result.exit_code == 2
    assert "Python constraint" in result.output
    assert not store.list_entries()


def test_write_injected_default_stays_in_the_os_temp_dir(tmp_path: Path):
    """The DEFAULT location is the OS temp directory — the secrets-never-persist-in-the-store
    property every python/shell injection relies on. Only an explicit prefer_entry_dir=True
    (deps-managed js copies) may move a copy into the store."""
    from skit import rewrite

    path = rewrite.write_injected(tmp_path, "print(1)\n", suffix=".py")
    try:
        assert path.parent != tmp_path
    finally:
        path.unlink()


def test_js_and_ts_specs_declare_the_npm_flavor():
    for kind in ("js", "ts"):
        spec = spec_for(kind)
        assert spec is not None
        assert spec.deps_flavor == "npm"
        assert spec.supports_deps
        assert spec.dep_scanner is not None
    python_spec = spec_for("python")
    assert python_spec is not None
    assert python_spec.deps_flavor == "uv"
    assert python_spec.dep_scanner is None


def test_install_lock_handles_the_holder_vanishing_mid_check(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
):
    """The reclaim race: os.open loses to a holder whose lockfile is gone by the time we stat
    it (it released between our two syscalls). A vanished holder is reclaimable — the retry
    must acquire, not crash on the failed stat."""
    calls = {"n": 0}
    real_open = js_deps.os.open

    def racing_open(path: str, flags: int) -> int:
        calls["n"] += 1
        if calls["n"] == 1:
            raise FileExistsError(17, "raced", path)
        return real_open(path, flags)

    monkeypatch.setattr(js_deps.os, "open", racing_open)
    with js_deps._install_lock(tmp_path):
        assert (tmp_path / ".skit-deps.lock").exists()
    assert calls["n"] == 2  # first attempt lost the race; the retry acquired


# ==========================================================================
# round-2 review fixes: real stderr, ANSI, locks on clear, TUI resilience
# ==========================================================================

# Captured verbatim from `npm install --no-audit --no-fund --ignore-scripts` (npm 11.x) against
# a nonexistent package — including the content-free "npm error 404" separators and the
# trailing "Note that…" + log-pointer boilerplate a naive last-line pick would surface.
_REAL_NPM_E404 = (
    b"npm error code E404\n"
    b"npm error 404 Not Found - GET https://registry.npmjs.org/skit-no-such-pkg-e2e-xyz - Not found\n"
    b"npm error 404\n"
    b"npm error 404  The requested resource 'skit-no-such-pkg-e2e-xyz@*' could not be found or you do not have permission to access it.\n"
    b"npm error 404\n"
    b"npm error 404 Note that you can also install from a\n"
    b"npm error 404 tarball, folder, http url, or git url.\n"
    b"npm error A complete log of this run can be found in: /Users/u/.npm/_logs/2026-07-16T00_36_04_636Z-debug-0.log\n"
)

# Captured verbatim from `deno install` (deno 2.x) — SGR color escapes survive piping.
_REAL_DENO_MISSING = (
    b"\x1b[0m\x1b[32mDownload\x1b[0m https://registry.npmjs.org/skit-no-such-pkg-e2e-xyz\n"
    b"\x1b[0m\x1b[1m\x1b[31merror\x1b[0m: npm package 'skit-no-such-pkg-e2e-xyz' does not exist.\n"
)

# Captured verbatim from `bun install` (bun 1.x).
_REAL_BUN_MISSING = (
    b"Resolving dependencies\n"
    b"Resolved, downloaded and extracted [1]\n"
    b"error: GET https://registry.npmjs.org/skit-no-such-pkg-e2e-xyz - 404\n"
    b"error: skit-no-such-pkg-e2e-xyz@* failed to resolve\n"
)


# Captured verbatim from npm 11.x: a peer-dependency conflict (react 18 vs react-dom 17).
# The cause is buried mid-output; the tail is a report-path line and a log pointer that a
# last-line pick would surface as "npm error /Users/…/eresolve-report.txt".
_REAL_NPM_ERESOLVE = (
    b"npm error code ERESOLVE\n"
    b"npm error ERESOLVE unable to resolve dependency tree\n"
    b"npm error\n"
    b"npm error While resolving: undefined@undefined\n"
    b"npm error Found: react@18.2.0\n"
    b"npm error node_modules/react\n"
    b'npm error   react@"18.2.0" from the root project\n'
    b"npm error\n"
    b"npm error Could not resolve dependency:\n"
    b'npm error peer react@"17.0.2" from react-dom@17.0.2\n'
    b"npm error node_modules/react-dom\n"
    b'npm error   react-dom@"17.0.2" from the root project\n'
    b"npm error\n"
    b"npm error Fix the upstream dependency conflict, or retry this command with --force or --legacy-peer-deps to accept an incorrect (and potentially broken) dependency resolution.\n"
    b"npm error\n"
    b"npm error\n"
    b"npm error For a full report see:\n"
    b"npm error /Users/u/.npm/_logs/2026-07-16T01_01_09_210Z-eresolve-report.txt\n"
    b"npm error A complete log of this run can be found in: /Users/u/.npm/_logs/2026-07-16T01_01_09_210Z-debug-0.log\n"
)

# Captured verbatim from npm 11.x against an unreachable registry (the misconfigured-mirror /
# offline case): a stack trace, an error-object dump, and a proxy hint follow the cause.
_REAL_NPM_ECONNREFUSED = (
    b"npm error code ECONNREFUSED\n"
    b"npm error syscall connect\n"
    b"npm error errno ECONNREFUSED\n"
    b"npm error FetchError: request to http://127.0.0.1:9/chalk failed, reason: connect ECONNREFUSED 127.0.0.1:9\n"
    b"npm error     at ClientRequest.<anonymous> (/opt/homebrew/lib/node_modules/npm/node_modules/minipass-fetch/lib/index.js:130:14)\n"
    b"npm error     at ClientRequest.emit (node:events:509:20)\n"
    b"npm error     at process.processTicksAndRejections (node:internal/process/task_queues:91:21) {\n"
    b"npm error   code: 'ECONNREFUSED',\n"
    b"npm error   errno: 'ECONNREFUSED',\n"
    b"npm error   syscall: 'connect',\n"
    b"npm error   requiredBy: '.'\n"
    b"npm error }\n"
    b"npm error\n"
    b"npm error If you are behind a proxy, please make sure that the 'proxy' config is set properly.  See: 'npm help config'\n"
    b"npm error A complete log of this run can be found in: /Users/u/.npm/_logs/2026-07-16T01_01_09_771Z-debug-0.log\n"
)


@pytest.mark.parametrize(
    ("stderr", "expected_fragment", "label"),
    [
        (_REAL_NPM_E404, "Not Found - GET", "npm's headline names the resource URL"),
        (_REAL_DENO_MISSING, "does not exist", "deno's cause line, ANSI stripped"),
        (_REAL_BUN_MISSING, "failed to resolve", "bun's cause line"),
        (
            _REAL_NPM_ERESOLVE,
            "dependency conflict",
            "ERESOLVE surfaces the conflict, not the report path",
        ),
        (
            _REAL_NPM_ECONNREFUSED,
            "connect ECONNREFUSED",
            "network failure surfaces the FetchError, not the proxy hint",
        ),
    ],
)
def test_failure_detail_against_real_installer_output(
    stderr: bytes, expected_fragment: str, label: str
):
    detail = js_deps._failure_detail(stderr)
    assert expected_fragment in detail, label
    assert "\x1b" not in detail  # raw escape bytes must never reach the user or rich markup
    assert "A complete log" not in detail
    assert "tarball, folder" not in detail
    assert "_logs/" not in detail  # never a bare report/log path
    assert "behind a proxy" not in detail


@pytest.mark.parametrize("stderr", [_REAL_NPM_E404, _REAL_DENO_MISSING, _REAL_BUN_MISSING])
def test_failure_detail_names_the_missing_package(stderr: bytes):
    assert "skit-no-such-pkg-e2e-xyz" in js_deps._failure_detail(stderr)


def test_failure_detail_empty_stderr_degrades():
    assert js_deps._failure_detail(b"") == "?"
    assert js_deps._failure_detail(b"npm error 404\n\n") == "?"  # only content-free lines


def test_clear_takes_the_install_lock(tmp_path: Path, monkeypatch: pytest.MonkeyPatch):
    """`deps --clear` must serialize against a concurrent run's installer: clear() wraps
    clean() in the same per-entry lock ensure_installed holds."""
    held: list[bool] = []
    real_clean = js_deps.clean
    monkeypatch.setattr(
        js_deps,
        "clean",
        lambda entry_dir: (
            held.append((entry_dir / ".skit-deps.lock").exists()),
            real_clean(entry_dir),
        )[1],
    )
    (tmp_path / "package.json").write_text("{}", encoding="utf-8")
    js_deps.clear(tmp_path)
    assert held == [True]  # clean ran while the entry lock was held
    assert not (tmp_path / ".skit-deps.lock").exists()  # and it was released after
    assert not (tmp_path / "package.json").exists()


def test_store_clear_goes_through_the_locked_entry_point(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
):
    _added_js(tmp_path)
    store.update_dependencies("t", ["chalk"])
    calls: list[str] = []
    monkeypatch.setattr("skit.langs.javascript.deps.clear", lambda entry_dir: calls.append("clear"))
    monkeypatch.setattr(
        "skit.langs.javascript.deps.clean",
        lambda entry_dir: calls.append("unlocked clean (wrong entry point)"),
    )
    store.update_dependencies("t", [])
    assert calls == ["clear"]


async def test_settings_save_survives_a_failed_deps_clear(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
):
    """A held-open node_modules on clear must be a toast, not an app crash: the save reports
    and stays on the screen, exactly like a refused rename."""
    from textual.widgets import Input as TInput

    from skit import tui
    from skit.tui_settings import ScriptSettingsScreen

    _added_js(tmp_path)
    store.update_dependencies("t", ["chalk"])

    def stuck_clear(entry_dir: Path) -> None:
        raise NotExecutableError("node_modules/stuck: held by another process")

    monkeypatch.setattr("skit.langs.javascript.deps.clear", stuck_clear)
    notes: list[str] = []
    monkeypatch.setattr(
        ScriptSettingsScreen, "notify", lambda self, message, **kw: notes.append(str(message))
    )
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        screen = ScriptSettingsScreen(store.resolve("t"))
        app.push_screen(screen)
        await pilot.pause()
        screen.query_one("#st-deps", TInput).value = ""  # clear the list -> disk sweep -> boom
        screen.action_save()
        await pilot.pause()
        assert isinstance(app.screen, ScriptSettingsScreen)  # stayed on screen, app alive
    assert any("stuck" in n for n in notes)


@pytest.mark.parametrize(
    ("args", "fragment"),
    [
        (["--dep", "requests"], "don't take package dependencies"),
        (["--python", ">=3.11"], "Python constraint"),
    ],
)
def test_add_shell_refuses_unusable_flags_loudly(tmp_path: Path, args: list[str], fragment: str):
    sh = tmp_path / "d.sh"
    sh.write_text("#!/bin/sh\necho hi\n", encoding="utf-8")
    result = runner.invoke(cli.app, ["add", str(sh), *args, "--no-input"])
    assert result.exit_code == 2
    assert fragment in result.output
    assert not store.list_entries()


def test_add_cmd_refuses_dep_flag_loudly(tmp_path: Path):
    result = runner.invoke(
        cli.app, ["add", "--cmd", "echo {x}", "--name", "e", "--dep", "requests"]
    )
    assert result.exit_code == 2
    # Either the lane matrix's "can't apply here" (a --cmd template takes only
    # --name/--description) or the older per-kind wording — both exit 2, nothing added.
    assert (
        "don't take package dependencies" in result.output
        or "--dep can't apply here" in result.output
    )
    assert not store.list_entries()


def test_add_python_still_honors_both_flags(tmp_path: Path):
    py = tmp_path / "j.py"
    py.write_text("print(1)\n", encoding="utf-8")
    result = runner.invoke(
        cli.app,
        ["add", str(py), "--dep", "requests", "--python", ">=3.11", "--no-input"],
        env={"SKIT_FORM": "plain"},
    )
    assert result.exit_code == 0
    # Copy-mode python records deps in the stored copy's PEP 723 block (meta stays None —
    # the block is the source of truth); the flags were consumed, not refused.
    copy_text = store.resolve("j").script_path.read_text(encoding="utf-8")
    assert '"requests"' in copy_text
    assert 'requires-python = ">=3.11"' in copy_text


# ==========================================================================
# round-3 review fixes: stdin/--edit lanes honor flags; wizard covers npm
# ==========================================================================


def test_add_stdin_honors_explicit_dep_and_python_flags(tmp_path: Path):
    result = runner.invoke(
        cli.app,
        ["add", "-", "--name", "clip", "--dep", "requests>=2,<3", "--python", ">=3.11"],
        input='print("hi")\n',
    )
    assert result.exit_code == 0
    copy_text = store.resolve("clip").script_path.read_text(encoding="utf-8")
    assert '"requests>=2,<3"' in copy_text  # honored into the copy's PEP 723 block
    assert 'requires-python = ">=3.11"' in copy_text


def test_add_stdin_refuses_ref_loudly(tmp_path: Path):
    result = runner.invoke(cli.app, ["add", "-", "--name", "clip", "--ref"], input='print("hi")\n')
    assert result.exit_code == 2
    # The lane matrix refuses --ref on stdin ("can't apply here"); the older wording named
    # the missing existing file. Either voice is honest — exit 2, nothing added.
    assert "existing file" in result.output or "--ref can't apply here" in result.output
    assert not store.list_entries()


def test_add_edit_refuses_ref_loudly(tmp_path: Path, monkeypatch: pytest.MonkeyPatch):
    monkeypatch.setattr(cli, "_is_interactive", lambda: True)
    result = runner.invoke(cli.app, ["add", "--edit", "--name", "n", "--ref"])
    assert result.exit_code == 2
    assert "existing file" in result.output or "--ref can't apply here" in result.output


def test_add_edit_honors_explicit_dep_and_python_flags(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
):
    monkeypatch.setattr(cli, "_is_interactive", lambda: True)
    monkeypatch.setattr(
        cli.editor,
        "open_in_editor",
        lambda path: path.write_text('print("made in editor")\n', encoding="utf-8"),
    )
    monkeypatch.setattr("sys.stdin.isatty", lambda: False, raising=False)  # no param prompts
    result = runner.invoke(
        cli.app, ["add", "--edit", "--name", "n", "--dep", "rich", "--python", ">=3.12"]
    )
    assert result.exit_code == 0
    copy_text = store.resolve("n").script_path.read_text(encoding="utf-8")
    assert '"rich"' in copy_text
    assert 'requires-python = ">=3.12"' in copy_text


# ==========================================================================
# round-4 review fixes: lock OSError taxonomy, catalog syntax gate
# ==========================================================================


def test_install_lock_unwritable_dir_raises_126_family_not_a_traceback(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
):
    """A read-only entry dir must surface as the 126 prerequisite family (one clean line),
    never a raw PermissionError at exit 1 — an agent could not tell that from the script
    itself failing."""

    def denied(path: str, flags: int) -> int:
        raise PermissionError(13, "Read-only file system", path)

    monkeypatch.setattr(js_deps.os, "open", denied)
    with pytest.raises(NotExecutableError) as exc:
        with js_deps._install_lock(tmp_path):
            pytest.fail("the lock must not be acquired")
    assert "Read-only file system" in str(exc.value)


def test_run_on_unwritable_entry_dir_exits_126_not_1(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
):
    _runner_env(monkeypatch)
    monkeypatch.setattr(js_deps.shutil, "which", lambda name: f"/bin/{name}")

    def denied(path: str, flags: int) -> int:
        raise PermissionError(13, "Read-only file system", path)

    monkeypatch.setattr(js_deps.os, "open", denied)
    entry = _entry(tmp_path, dependencies=["chalk"])
    with pytest.raises(NotExecutableError):
        launch.RunnerLaunch().build(entry, [], None, None)


def test_i18n_gate_catches_an_unquoted_msgstr(tmp_path: Path, monkeypatch: pytest.MonkeyPatch):
    """The exact corruption class a lenient pybabel parse mis-recovers (a lost opening quote)
    must fail the coverage gate instead of shipping a mangled runtime string."""
    import importlib.util

    spec = importlib.util.spec_from_file_location(
        "i18n_coverage", Path(__file__).parent.parent / "scripts" / "i18n_coverage.py"
    )
    assert spec is not None
    assert spec.loader is not None
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)

    catalog = tmp_path / "zz" / "LC_MESSAGES"
    catalog.mkdir(parents=True)
    (catalog / "skit.po").write_text(
        'msgid ""\nmsgstr ""\n\nmsgid "good"\nmsgstr "好"\n\nmsgid "broken"\nmsgstr 壞掉了"\n',
        encoding="utf-8",
    )
    monkeypatch.setattr(mod, "LOCALES", tmp_path)
    problems = mod.check_po_syntax()
    assert len(problems) == 1
    assert "unquoted msgstr" in problems[0]
    assert "skit.po:8" in problems[0]


def test_i18n_gate_passes_the_shipped_catalogs():
    import importlib.util

    spec = importlib.util.spec_from_file_location(
        "i18n_coverage", Path(__file__).parent.parent / "scripts" / "i18n_coverage.py"
    )
    assert spec is not None
    assert spec.loader is not None
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    assert mod.check_po_syntax() == []


# ==========================================================================
# round-5 review fixes: stale-lock starvation, continuation-line gate
# ==========================================================================


def test_install_lock_unremovable_stale_lock_fails_loud_instead_of_spinning(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
):
    """A stale lock in a dir where the unlink is denied must raise the 126 family, not
    busy-loop forever at 100% CPU (the round-4 read-only scenario, lock-present variant)."""
    import os

    lock = tmp_path / ".skit-deps.lock"
    lock.write_text("", encoding="utf-8")
    os.utime(lock, (1, 1))  # stale: eligible for reclaim

    def denied(self: Path, missing_ok: bool = False) -> None:
        raise PermissionError(13, "Read-only file system", str(self))

    monkeypatch.setattr(Path, "unlink", denied)
    with pytest.raises(NotExecutableError) as exc:
        with js_deps._install_lock(tmp_path):
            pytest.fail("the lock must not be acquired")
    assert "Read-only file system" in str(exc.value)


def test_install_lock_stale_reclaim_lost_to_another_waiter_retries(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
):
    """Two waiters race to reclaim the same stale lock: the loser's unlink hits
    FileNotFoundError and must simply retry the acquisition, not crash."""
    import os

    lock = tmp_path / ".skit-deps.lock"
    lock.write_text("", encoding="utf-8")
    os.utime(lock, (1, 1))
    real_unlink = Path.unlink

    def racing_unlink(self: Path, missing_ok: bool = False) -> None:
        real_unlink(self)  # the other waiter got there first...
        raise FileNotFoundError(2, "No such file or directory", str(self))  # ...we lose

    monkeypatch.setattr(Path, "unlink", racing_unlink)
    with js_deps._install_lock(tmp_path):
        monkeypatch.setattr(Path, "unlink", real_unlink)
        assert (tmp_path / ".skit-deps.lock").exists()


def test_i18n_gate_catches_an_unquoted_continuation_line(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
):
    """babel silently truncates a wrapped msgstr at an unquoted continuation line — the exact
    round-4 corruption class one line lower. The gate must flag it, and must NOT flag the
    header, comments, #~ obsolete entries, or well-quoted wrapped strings."""
    import importlib.util

    spec = importlib.util.spec_from_file_location(
        "i18n_coverage", Path(__file__).parent.parent / "scripts" / "i18n_coverage.py"
    )
    assert spec is not None
    assert spec.loader is not None
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)

    catalog = tmp_path / "zz" / "LC_MESSAGES"
    catalog.mkdir(parents=True)
    (catalog / "skit.po").write_text(
        'msgid ""\n'
        'msgstr ""\n'
        '"Project-Id-Version: skit\\n"\n'
        "\n"
        "# a translator comment\n"
        '#~ msgid "obsolete"\n'
        '#~ msgstr "舊"\n'
        'msgid "wrapped"\n'
        'msgstr ""\n'
        '"good start "\n'
        'broken end"\n',
        encoding="utf-8",
    )
    monkeypatch.setattr(mod, "LOCALES", tmp_path)
    problems = mod.check_po_syntax()
    assert len(problems) == 1
    assert "unquoted continuation line" in problems[0]
    assert "skit.po:11" in problems[0]


def test_install_announces_itself_but_a_fresh_marker_stays_silent(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
):
    """A captured install can take minutes — one stderr line separates "working" from "hung".
    The short-circuit path prints nothing (nothing is happening)."""
    monkeypatch.setattr(js_deps.shutil, "which", lambda name: f"/bin/{name}")
    monkeypatch.setattr(js_deps.subprocess, "run", _install_ok([]))
    js_deps.ensure_installed(tmp_path, ["chalk"], "node", {})
    captured = capsys.readouterr()
    assert "npm" in captured.err
    assert captured.out == ""  # a script's piped stdout stays clean
    js_deps.ensure_installed(tmp_path, ["chalk"], "node", {})  # marker fresh: no reinstall
    assert capsys.readouterr().err == ""


def test_corrupted_marker_triggers_reinstall_not_a_persistent_crash(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
):
    """A tampered marker with invalid UTF-8 must mean "stale — rebuild", exactly like any
    other unreadable/garbage marker — not a UnicodeDecodeError traceback on every run."""
    calls: list[tuple[list[str], dict[str, object]]] = []
    monkeypatch.setattr(js_deps.shutil, "which", lambda name: f"/bin/{name}")
    monkeypatch.setattr(js_deps.subprocess, "run", _install_ok(calls))
    (tmp_path / "node_modules").mkdir()
    (tmp_path / "node_modules" / ".skit-deps-ok").write_bytes(b"\xff\xfe garbage")
    js_deps.ensure_installed(tmp_path, ["chalk"], "node", {})
    assert len(calls) == 1  # reinstalled
    marker = (tmp_path / "node_modules" / ".skit-deps-ok").read_text(encoding="utf-8")
    assert len(marker) == 64  # a fresh, valid hex stamp replaced the garbage


# ==========================================================================
# mutation-hardening: inputs the behavioral mutants proved untested
# ==========================================================================


@pytest.mark.parametrize(
    ("req", "expected"),
    [
        ("a@5", ("a", "5")),  # single-char name: the rfind boundary is exactly 1
        ("foo/@2", ("foo/@2", "*")),  # name ending in '/': the @ is not a range separator
    ],
)
def test_split_requirement_boundary_shapes(req: str, expected: tuple[str, str]):
    assert js_deps.split_requirement(req) == expected


@pytest.mark.parametrize(
    ("source", "expected"),
    [
        ("/home/u.name/tool.v2.mjs", "module"),  # multiple dots: only the LAST names the ext
        ("archive.tar.cjs", "commonjs"),
    ],
)
def test_module_type_for_multi_dot_sources(source: str, expected: str):
    assert js_deps.module_type_for(source) == expected


def test_manifest_text_exact_layout():
    # The manifest text is the staleness-hash input AND a file npm reads: pin the exact
    # indented layout so a formatting drift (which would force a spurious reinstall for
    # every existing entry) can't slip through silently.
    assert js_deps.manifest_text(["chalk@^5"], module_type="module") == (
        '{\n  "private": true,\n  "type": "module",\n  "dependencies": {\n'
        '    "chalk": "^5"\n  }\n}\n'
    )


def test_sweep_keeps_a_file_exactly_at_the_cutoff(tmp_path: Path, monkeypatch: pytest.MonkeyPatch):
    """The age gate is strictly `older than`: a copy exactly AT the cutoff is kept (the gate
    protects live copies, so ties break toward keeping)."""
    import os

    now = 2_000_000_000.0
    monkeypatch.setattr(js_deps.time, "time", lambda: now)
    at_cutoff = tmp_path / ".injected-edge.js"
    at_cutoff.write_text("x", encoding="utf-8")
    os.utime(at_cutoff, (now - 3600.0, now - 3600.0))
    js_deps.sweep_stale_injected(tmp_path)
    assert at_cutoff.exists()


def test_ensure_installed_unknown_runner_falls_back_to_npm_argv(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
):
    calls: list[tuple[list[str], dict[str, object]]] = []
    monkeypatch.setattr(js_deps.shutil, "which", lambda name: f"/bin/{name}")
    monkeypatch.setattr(js_deps.subprocess, "run", _install_ok(calls))
    js_deps.ensure_installed(tmp_path, ["chalk"], "some-future-runner", {})
    assert calls[0][0] == ["/bin/npm", "install", "--no-audit", "--no-fund", "--ignore-scripts"]


def test_ensure_installed_writes_the_module_type_into_the_manifest(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
):
    monkeypatch.setattr(js_deps.shutil, "which", lambda name: f"/bin/{name}")
    monkeypatch.setattr(js_deps.subprocess, "run", _install_ok([]))
    js_deps.ensure_installed(tmp_path, ["chalk"], "node", {}, module_type="module")
    assert '"type": "module"' in (tmp_path / "package.json").read_text(encoding="utf-8")


def test_failure_detail_drops_bare_paths_even_without_a_cause_line():
    # No cause-keyword line anywhere: the fallback is the last informative line, which must
    # never be a bare report/log path (unix or windows shaped).
    stderr = (
        b"npm error something odd happened\n"
        b"npm error /var/log/npm/report-123.txt\n"
        b"npm error C:\\Users\\u\\AppData\\npm-report.txt\n"
    )
    assert js_deps._failure_detail(stderr) == "npm error something odd happened"


def test_module_type_for_a_bare_dotfile_name():
    # A source whose only dot is at index 0 (".mjs" — degenerate but legal): the suffix IS
    # the whole name and still pins the flavor.
    assert js_deps.module_type_for(".mjs") == "module"


def test_sweep_survives_one_failed_unlink_and_still_sweeps_the_rest(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
):
    """One unremovable leftover must not abort the sweep — the remaining aged copies (which
    may carry secrets) still go."""
    import os

    first = tmp_path / ".injected-a.js"
    second = tmp_path / ".injected-b.js"
    for f in (first, second):
        f.write_text("x", encoding="utf-8")
        os.utime(f, (1, 1))
    real_unlink = Path.unlink
    failed_once = {"done": False}

    def flaky_unlink(self: Path, missing_ok: bool = False) -> None:
        if not failed_once["done"]:
            failed_once["done"] = True
            raise PermissionError(13, "held", str(self))
        real_unlink(self)

    monkeypatch.setattr(Path, "unlink", flaky_unlink)
    js_deps.sweep_stale_injected(tmp_path)
    survivors = sorted(p.name for p in tmp_path.glob(".injected-*"))
    assert len(survivors) == 1  # exactly one kept (the failed unlink), the other swept


@pytest.mark.parametrize(
    "marker_line",
    [
        "npm error 404 failed: A complete log of this run can be found in: /x.log",
        "npm error 404 failed: Note that you can also install from a",
        "npm error 404 failed: tarball, folder, http url, or git url.",
        "npm error failed: For a full report see:",
        "npm error failed: If you are behind a proxy, check 'npm help config'",
    ],
)
def test_failure_detail_filters_each_noise_marker(marker_line: str):
    # Each marker line here carries a cause word ("failed") — if the marker filter loses a
    # member, the line would WIN the cause preference. The real cause must win instead.
    stderr = f"npm error install failed for pkg\n{marker_line}\n".encode()
    assert js_deps._failure_detail(stderr) == "npm error install failed for pkg"


def test_failure_detail_noise_before_the_cause_still_finds_the_cause():
    # A noise line mid-stream must be SKIPPED (continue), not end the scan (break).
    stderr = (
        b"npm error A complete log of this run can be found in: /x.log\n"
        b"npm error something odd happened\n"
    )
    assert js_deps._failure_detail(stderr) == "npm error something odd happened"


def test_failure_detail_drops_every_npm_prefix_noise_shape():
    # Unindented stack frame, lone braces, lowercase Windows drive — each would win the
    # last-informative fallback if its noise rule loses a member.
    stderr = (
        b"npm error something odd happened\n"
        b"npm error at Object.fn (/x/y.js:1:1)\n"
        b"npm error {\n"
        b"npm error }\n"
        b"npm error c:\\Users\\u\\report.txt\n"
    )
    assert js_deps._failure_detail(stderr) == "npm error something odd happened"


def test_failure_detail_deno_line_is_reproduced_exactly():
    # Exact equality: ANSI stripping must REMOVE the escapes (replace with nothing), not
    # substitute placeholder text into the line.
    assert (
        js_deps._failure_detail(_REAL_DENO_MISSING)
        == "error: npm package 'skit-no-such-pkg-e2e-xyz' does not exist."
    )


def test_install_subprocess_contract_and_marker_dir_reuse(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
):
    """The installer subprocess runs captured (its noise is summarized, not streamed), with
    check=False (WE map the exit code); the marker lands inside the node_modules the installer
    itself just created."""
    seen: dict[str, object] = {}

    def realistic_run(argv: list[str], **kwargs: object) -> subprocess.CompletedProcess[bytes]:
        seen.update(kwargs)
        (tmp_path / "node_modules" / "chalk").mkdir(parents=True)  # a real installer does this
        return subprocess.CompletedProcess(argv, 0, stdout=b"", stderr=b"")

    monkeypatch.setattr(js_deps.shutil, "which", lambda name: f"/bin/{name}")
    monkeypatch.setattr(js_deps.subprocess, "run", realistic_run)
    js_deps.ensure_installed(tmp_path, ["chalk"], "node", {})
    assert seen["capture_output"] is True
    assert seen["check"] is False
    assert (tmp_path / "node_modules" / ".skit-deps-ok").is_file()


def test_dependency_failure_messages_verbatim(tmp_path: Path, monkeypatch: pytest.MonkeyPatch):
    """The exact user-facing sentences (msgid == English source): the message is the whole
    interface on these paths, so it is pinned verbatim like the wizard prompts are."""
    monkeypatch.setattr(js_deps.shutil, "which", lambda name: None)
    with pytest.raises(NotExecutableError) as missing:
        js_deps.require_installer("node")
    assert (
        str(missing.value)
        == "npm is needed to install this script's dependencies, but it isn't on your PATH."
    )

    monkeypatch.setattr(js_deps.shutil, "which", lambda name: f"/bin/{name}")

    def boom(*a: object, **k: object) -> subprocess.CompletedProcess[bytes]:
        raise OSError(8, "Exec format error")

    monkeypatch.setattr(js_deps.subprocess, "run", boom)
    with pytest.raises(NotExecutableError) as spawn:
        js_deps.ensure_installed(tmp_path, ["chalk"], "node", {})
    assert str(spawn.value) == "Couldn't run npm: [Errno 8] Exec format error"

    monkeypatch.setattr(
        js_deps.subprocess,
        "run",
        lambda argv, **k: subprocess.CompletedProcess(
            argv, 1, stdout=b"", stderr=b"npm error it failed\n"
        ),
    )
    with pytest.raises(NotExecutableError) as failed:
        js_deps.ensure_installed(tmp_path, ["chalk"], "node", {})
    assert str(failed.value) == "Installing dependencies failed (npm): npm error it failed"


def test_install_announce_line_verbatim(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
):
    monkeypatch.setattr(js_deps.shutil, "which", lambda name: f"/bin/{name}")
    monkeypatch.setattr(js_deps.subprocess, "run", _install_ok([]))
    js_deps.ensure_installed(tmp_path, ["chalk"], "node", {})
    assert capsys.readouterr().err == "Installing dependencies (npm)…\n"


def test_clean_failure_message_verbatim(tmp_path: Path, monkeypatch: pytest.MonkeyPatch):
    (tmp_path / "package.json").write_text("{}", encoding="utf-8")

    def stubborn_unlink(self: Path, missing_ok: bool = False) -> None:
        raise PermissionError(13, "held by another process", str(self))

    monkeypatch.setattr(Path, "unlink", stubborn_unlink)
    with pytest.raises(NotExecutableError) as exc:
        js_deps.clean(tmp_path)
    assert str(exc.value) == (
        "Couldn't clear the old dependency environment: package.json: held by another process"
    )


def test_failure_detail_survives_invalid_utf8_bytes():
    # Installer stderr is not guaranteed to be UTF-8; undecodable bytes are replaced, never
    # raised — and the error-handler spelling matters (handlers are case-sensitive).
    detail = js_deps._failure_detail(b"npm error caf\xe9 install failed\n")
    assert "install failed" in detail
    assert "�" in detail  # the bad byte became the replacement character


# ==========================================================================
# review round: lock ownership token, needs_install/preflight parity,
# clean() already-gone + symlink handling, empty --dep, --json on write,
# and the placeholder-parity i18n gate
# ==========================================================================


def test_install_lock_release_leaves_a_successors_lock_alone(tmp_path: Path):
    """A holder whose over-running install was stale-reclaimed must NOT, on its late release,
    delete the successor's fresh lockfile — that would admit a third concurrent installer over
    one directory. The per-acquisition token makes release remove only our own lock."""
    lock = tmp_path / ".skit-deps.lock"
    successor = b"successor-11111"
    with js_deps._install_lock(tmp_path):
        # Simulate a stale-reclaim replacing our lock with a successor's fresh one.
        lock.write_bytes(successor)
    assert lock.read_bytes() == successor  # release saw a foreign token and left it in place


def test_needs_install_true_without_a_marker(tmp_path: Path):
    assert js_deps.needs_install(tmp_path, ["chalk"], "node") is True


def test_needs_install_false_when_the_marker_matches(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
):
    monkeypatch.setattr(js_deps.shutil, "which", lambda name: f"/bin/{name}")
    monkeypatch.setattr(js_deps.subprocess, "run", _install_ok([]))
    js_deps.ensure_installed(tmp_path, ["chalk"], "node", {})
    assert js_deps.needs_install(tmp_path, ["chalk"], "node") is False


def test_needs_install_true_when_the_declared_deps_changed(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
):
    monkeypatch.setattr(js_deps.shutil, "which", lambda name: f"/bin/{name}")
    monkeypatch.setattr(js_deps.subprocess, "run", _install_ok([]))
    js_deps.ensure_installed(tmp_path, ["chalk"], "node", {})
    assert js_deps.needs_install(tmp_path, ["chalk", "zod"], "node") is True


def test_preflight_skips_the_installer_when_the_marker_is_already_fresh(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
):
    """A fresh marker means build short-circuits without the installer, so preflight must not
    refuse the run just because the installer is gone (node-without-npm on Debian, say). Without
    this, the TUI would block a run the CLI completes fine."""
    _runner_env(monkeypatch)
    entry = _entry(tmp_path, dependencies=["chalk"])
    stamp = js_deps._resolve_manifest(["chalk"], "node", "")[2]
    marker = entry.dir / "node_modules" / js_deps._MARKER
    marker.parent.mkdir(parents=True)
    marker.write_text(stamp, encoding="utf-8")
    monkeypatch.setattr(
        js_deps, "require_installer", lambda name: pytest.fail("marker fresh; no installer needed")
    )
    launch.RunnerLaunch().preflight(entry)  # must not raise


def test_clean_unlinks_a_symlinked_node_modules_but_keeps_the_target(tmp_path: Path):
    target = tmp_path / "shared"
    (target / "chalk").mkdir(parents=True)
    (tmp_path / "node_modules").symlink_to(target, target_is_directory=True)
    js_deps.clean(tmp_path)
    assert not (tmp_path / "node_modules").exists()  # the link is gone
    assert (target / "chalk").exists()  # the target's contents are not ours to delete


def test_clean_tolerates_a_node_modules_symlink_vanishing(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
):
    (tmp_path / "shared").mkdir()
    (tmp_path / "node_modules").symlink_to(tmp_path / "shared", target_is_directory=True)
    real_unlink = Path.unlink

    def racing_unlink(self: Path, missing_ok: bool = False) -> None:
        if self.name == "node_modules":
            raise FileNotFoundError(2, "No such file or directory", str(self))
        return real_unlink(self, missing_ok=missing_ok)

    monkeypatch.setattr(Path, "unlink", racing_unlink)
    js_deps.clean(tmp_path)  # already-gone is success, not a loud failure


def test_clean_records_a_stuck_symlinked_node_modules(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
):
    (tmp_path / "shared").mkdir()
    (tmp_path / "node_modules").symlink_to(tmp_path / "shared", target_is_directory=True)
    real_unlink = Path.unlink

    def stuck_unlink(self: Path, missing_ok: bool = False) -> None:
        if self.name == "node_modules":
            raise PermissionError(13, "held by another process", str(self))
        return real_unlink(self, missing_ok=missing_ok)

    monkeypatch.setattr(Path, "unlink", stuck_unlink)
    with pytest.raises(NotExecutableError) as exc:
        js_deps.clean(tmp_path)
    assert "node_modules" in str(exc.value)


def test_clean_onexc_treats_an_already_gone_tree_as_success(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
):
    # rmtree can race an external remover and land in onexc with FileNotFoundError — that's the
    # desired end state (tree gone), not a failure to shout about.
    (tmp_path / "node_modules").mkdir()

    def rmtree_racing(path, onexc=None, **kwargs):
        assert onexc is not None
        onexc(None, str(path / "gone"), FileNotFoundError(2, "No such file", str(path / "gone")))

    monkeypatch.setattr(js_deps.shutil, "rmtree", rmtree_racing)
    js_deps.clean(tmp_path)  # FileNotFoundError in onexc is not a failure


def test_add_js_empty_dep_records_nothing(tmp_path: Path):
    src = _js_file(tmp_path, body="console.log(1)\n", name="hello.js")
    result = runner.invoke(cli.app, ["add", str(src), "-n", "j", "--dep", "  ", "--no-input"])
    assert result.exit_code == 0
    assert store.resolve("j").meta.dependencies is None  # "" is junk, not a package


def test_deps_command_empty_dep_clears_and_sweeps(tmp_path: Path):
    entry = _added_js(tmp_path)
    store.update_dependencies("t", ["chalk"])
    (entry.dir / "node_modules").mkdir(exist_ok=True)
    result = runner.invoke(cli.app, ["deps", "t", "--dep", ""])
    assert result.exit_code == 0
    assert store.resolve("t").meta.dependencies is None  # cleared, not recorded as [""]
    assert not (entry.dir / "node_modules").exists()  # and swept, like --clear


def test_deps_command_write_emits_json_when_asked(tmp_path: Path):
    _added_js(tmp_path)
    result = runner.invoke(cli.app, ["deps", "t", "--dep", "chalk@^5", "--json"])
    assert result.exit_code == 0
    payload = json.loads(result.output)
    assert payload["dependencies"] == ["chalk@^5"]


def test_deps_command_needs_write_emits_json_and_skips_the_human_line(tmp_path: Path):
    # --json on a needs write emits the machine view too, not the green confirmation line.
    _added_js(tmp_path)
    result = runner.invoke(cli.app, ["deps", "t", "--need", "jq", "--json"])
    assert result.exit_code == 0
    payload = json.loads(result.output)
    assert payload["needs"] == ["jq"]
    assert "updated" not in result.output  # the human confirmation is suppressed under --json


def test_deps_command_applies_both_deps_and_needs(tmp_path: Path):
    _added_js(tmp_path)
    result = runner.invoke(cli.app, ["deps", "t", "--dep", "chalk", "--need", "jq"])
    assert result.exit_code == 0
    entry = store.resolve("t")
    assert entry.meta.dependencies == ["chalk"]
    assert entry.meta.needs == ["jq"]


def test_deps_command_refused_dep_does_not_commit_a_concurrent_need(tmp_path: Path):
    """A --dep/--python refusal must abort BEFORE the needs write. Deps are processed first
    precisely so a refused request (exit 2) commits nothing — otherwise the --need in the same
    command would silently persist, a partial write a --json/CI caller couldn't detect."""
    _added_js(tmp_path)
    result = runner.invoke(cli.app, ["deps", "t", "--need", "jq", "--python", ">=3.11"])
    assert result.exit_code == 2  # the Python-constraint-on-npm refusal
    assert store.resolve("t").meta.needs is None  # the needs write never happened


def test_deps_command_drops_empty_and_whitespace_needs(tmp_path: Path):
    # Mirrors the --dep filter: an empty/whitespace command name is junk in --json and would brick
    # the entry (shutil.which("") is None → "Missing required command" on every run).
    _added_js(tmp_path)
    result = runner.invoke(cli.app, ["deps", "t", "--need", "  ", "--need", " jq ", "--need", ""])
    assert result.exit_code == 0
    assert store.resolve("t").meta.needs == ["jq"]  # stripped, empties dropped


def _load_i18n_coverage():
    import importlib.util

    spec = importlib.util.spec_from_file_location(
        "i18n_coverage", Path(__file__).parent.parent / "scripts" / "i18n_coverage.py"
    )
    assert spec is not None
    assert spec.loader is not None
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


def _write_po(tmp_path: Path, body: str, locale: str = "zz") -> None:
    catalog = tmp_path / locale / "LC_MESSAGES"
    catalog.mkdir(parents=True)
    (catalog / "skit.po").write_text(body, encoding="utf-8")


def test_placeholder_parity_flags_a_swapped_named_placeholder(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
):
    """The AGENTS.md trap: a fuzzy-grafted msgstr references a key the msgid never supplies, so
    %-format raises KeyError at runtime. Every other gate waves it through; this one catches it."""
    mod = _load_i18n_coverage()
    _write_po(
        tmp_path,
        'msgid ""\nmsgstr ""\n\nmsgid "needs %(error)s"\nmsgstr "需要 %(detail)s"\n',
    )
    monkeypatch.setattr(mod, "LOCALES", tmp_path)
    problems = mod.check_placeholder_parity()
    assert len(problems) == 1
    assert "placeholder mismatch" in problems[0]


def test_placeholder_parity_flags_a_positional_count_mismatch(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
):
    mod = _load_i18n_coverage()
    _write_po(tmp_path, 'msgid ""\nmsgstr ""\n\nmsgid "a %s b"\nmsgstr "just a"\n')
    monkeypatch.setattr(mod, "LOCALES", tmp_path)
    problems = mod.check_placeholder_parity()
    assert len(problems) == 1
    assert "positional-placeholder mismatch" in problems[0]


def test_placeholder_parity_flags_a_positional_conversion_type_swap(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
):
    # Same count, different conversion: "%d" % ("Alice",) is a TypeError, so a %s→%d swap must
    # fail the gate even though both strings have exactly one positional placeholder.
    mod = _load_i18n_coverage()
    _write_po(tmp_path, 'msgid ""\nmsgstr ""\n\nmsgid "Hi %s"\nmsgstr "你好 %d"\n')
    monkeypatch.setattr(mod, "LOCALES", tmp_path)
    problems = mod.check_placeholder_parity()
    assert len(problems) == 1
    assert "positional-placeholder mismatch" in problems[0]


def test_placeholder_parity_ignores_fuzzy_entries(tmp_path: Path, monkeypatch: pytest.MonkeyPatch):
    # A fuzzy entry is already flagged by the completeness gate; parity must not double-report
    # (and must not trip over the very mismatch fuzzy entries are prone to).
    mod = _load_i18n_coverage()
    _write_po(
        tmp_path,
        'msgid ""\nmsgstr ""\n\n#, fuzzy\nmsgid "needs %(error)s"\nmsgstr "需要 %(detail)s"\n',
    )
    monkeypatch.setattr(mod, "LOCALES", tmp_path)
    assert mod.check_placeholder_parity() == []


def test_placeholder_parity_accepts_matching_named_and_plural_forms(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
):
    mod = _load_i18n_coverage()
    _write_po(
        tmp_path,
        'msgid ""\nmsgstr ""\n"Plural-Forms: nplurals=2; plural=(n != 1);\\n"\n\n'
        'msgid "keep %(name)s"\nmsgstr "保留 %(name)s"\n\n'
        'msgid "%(n)d file %s"\nmsgid_plural "%(n)d files %s"\n'
        'msgstr[0] "%(n)d 個檔 %s"\nmsgstr[1] "%(n)d 個檔 %s"\n',
    )
    monkeypatch.setattr(mod, "LOCALES", tmp_path)
    assert mod.check_placeholder_parity() == []


def test_placeholder_parity_skips_an_untranslated_plural_form(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
):
    mod = _load_i18n_coverage()
    _write_po(
        tmp_path,
        'msgid ""\nmsgstr ""\n"Plural-Forms: nplurals=2; plural=(n != 1);\\n"\n\n'
        'msgid "%(n)d item"\nmsgid_plural "%(n)d items"\n'
        'msgstr[0] "%(n)d 項"\nmsgstr[1] ""\n',
    )
    monkeypatch.setattr(mod, "LOCALES", tmp_path)
    assert mod.check_placeholder_parity() == []  # the empty form is the completeness gate's job


def test_placeholder_parity_passes_the_shipped_catalogs():
    assert _load_i18n_coverage().check_placeholder_parity() == []


def test_po_syntax_allows_a_valid_msgctxt_line(tmp_path: Path, monkeypatch: pytest.MonkeyPatch):
    # msgctxt is standard gettext disambiguation; its quoted string must not be misread as an
    # unquoted continuation line (the round that added a placeholder-parity check fixed this).
    mod = _load_i18n_coverage()
    _write_po(
        tmp_path,
        'msgid ""\nmsgstr ""\n\nmsgctxt "menu"\nmsgid "Open"\nmsgstr "開啟"\n',
    )
    monkeypatch.setattr(mod, "LOCALES", tmp_path)
    assert mod.check_po_syntax() == []


# ==========================================================================
# module-typed entries with NO deps still need their package.json "type"
# ==========================================================================


def test_ensure_module_manifest_writes_the_type(tmp_path: Path):
    js_deps.ensure_module_manifest(tmp_path, "commonjs")
    pkg = json.loads((tmp_path / "package.json").read_text(encoding="utf-8"))
    assert pkg == {"private": True, "type": "commonjs"}


def test_ensure_module_manifest_flavorless_writes_nothing(tmp_path: Path):
    js_deps.ensure_module_manifest(tmp_path, "")  # a plain .js/.ts origin pins no flavor
    assert not (tmp_path / "package.json").exists()


def test_ensure_module_manifest_rewrites_only_on_change(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
):
    pkg = tmp_path / "package.json"
    js_deps.ensure_module_manifest(tmp_path, "module")  # absent → written
    assert json.loads(pkg.read_text(encoding="utf-8")) == {"private": True, "type": "module"}
    writes: list[str] = []
    real_write = Path.write_text
    monkeypatch.setattr(
        Path,
        "write_text",
        lambda self, *a, **k: (writes.append(self.name), real_write(self, *a, **k))[1],
    )
    js_deps.ensure_module_manifest(tmp_path, "module")  # already correct → no rewrite
    assert writes == []
    js_deps.ensure_module_manifest(tmp_path, "commonjs")  # differs → rewrite
    assert writes == ["package.json"]
    assert json.loads(pkg.read_text(encoding="utf-8"))["type"] == "commonjs"


def test_build_writes_a_module_manifest_for_a_deps_free_module_typed_entry(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
):
    # The bricking sibling: a CommonJS .cts/.cjs entry with no deps, flattened to script.ts/.js,
    # runs as ESM under deno (require is not defined). A minimal package.json "type" fixes it.
    _runner_env(monkeypatch)
    monkeypatch.setattr(
        js_deps, "ensure_installed", lambda *a, **k: pytest.fail("no deps: no install")
    )
    entry = _entry(tmp_path, dependencies=None, source="tool.cjs")
    launch.RunnerLaunch().build(entry, [], None, None)
    pkg = json.loads((entry.dir / "package.json").read_text(encoding="utf-8"))
    assert pkg == {"private": True, "type": "commonjs"}


def test_build_writes_no_manifest_for_a_flavorless_deps_free_entry(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
):
    _runner_env(monkeypatch)
    entry = _entry(tmp_path, dependencies=None, source="tool.js")  # plain .js: runner's default
    launch.RunnerLaunch().build(entry, [], None, None)
    assert not (entry.dir / "package.json").exists()


def test_ensure_module_manifest_rewrites_a_non_utf8_package_json(tmp_path: Path):
    # An externally-corrupted package.json (a BOM / wrong-encoding hand-edit) must be rewritten,
    # not crash with an uncaught UnicodeDecodeError — matching needs_install and the marker read,
    # which both treat invalid UTF-8 as "rewrite".
    (tmp_path / "package.json").write_bytes(b'\xff\xfe{"private":true}')
    js_deps.ensure_module_manifest(tmp_path, "module")
    assert json.loads((tmp_path / "package.json").read_text(encoding="utf-8")) == {
        "private": True,
        "type": "module",
    }
