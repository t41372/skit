"""Effective UV metadata contracts for the two independently editable axes.

The central rule is:
*meta blank + block truthy → the block is the truth; untouched and cleared are different
inputs*. It applies equally to dependencies and the Python constraint, and to both the
`deps` command and the settings screen.

Every assertion pins an OBSERVABLE contract — the stored PEP 723 block uv actually reads, the
`--json` machine contract, the Input a user sees prefilled — never an internal flag:

  * HIGH-2 (the deps axis): `skit add x.py --dep requests` records the dep ONLY in the copy's
    block (meta deliberately blank). A later `skit deps x --python …` used to reconstruct the
    deps list from that blank meta and ERASE the block's `requests` under a green constraint
    line — next run ModuleNotFoundError. Now a python-only edit leaves the deps axis UNTOUCHED,
    so the block keeps `requests` AND gains the pin, and every read surface reports both.
  * HIGH-1 (the settings screen): the ScriptSettingsScreen prefilled #st-deps/#st-python from
    RAW meta — blank for a block-only add-time entry — which made "untouched blank" and "user
    cleared" the same word, so a deps-only save wiped a pin the screen never showed. Now the
    fields prefill from the EFFECTIVE metadata (block fallback) and each axis is diffed against
    that baseline: an untouched axis travels as None (don't-touch), only a changed one is written.

  * store units for the new `effective_uv_metadata` read and the None/[]/"" chokepoint grammar.

These never chdir and never touch the real user dirs (the local SKIT_* fixture); --json is read
off result.stdout for purity, and block assertions use exact PEP 723 text where it is cheap.
"""

from __future__ import annotations

import json
from pathlib import Path

import pytest
from textual.widgets import Input
from typer.testing import CliRunner

from skit import cli, i18n, store, tui
from skit.langs.javascript import deps as js_deps
from skit.tui_settings import ScriptSettingsScreen

runner = CliRunner()


@pytest.fixture(autouse=True)
def tmp_store(tmp_path, monkeypatch):
    monkeypatch.setenv("SKIT_DATA_DIR", str(tmp_path / "data"))
    monkeypatch.setenv("SKIT_STATE_DIR", str(tmp_path / "state"))
    monkeypatch.setenv("SKIT_CONFIG_DIR", str(tmp_path / "config"))
    monkeypatch.setenv("SKIT_LANG", "en")
    i18n.init("en")


def _py(tmp_path: Path, body: str = "print(1)\n", name: str = "x.py") -> Path:
    p = tmp_path / name
    p.write_text(body, encoding="utf-8")
    return p


def _js(tmp_path: Path, body: str = "console.log(1)\n", name: str = "a.js") -> Path:
    p = tmp_path / name
    p.write_text(body, encoding="utf-8")
    return p


def _block(slug: str) -> str:
    return store.resolve(slug).script_path.read_text(encoding="utf-8")


# ==========================================================================
# 1. HIGH-2 pilot end to end: block-only add-time deps + a python-only edit
#    keeps the deps AND gains the pin, reported on every surface.
#    (The reverse split — meta-carried deps + a python-only edit — is already
#    pinned by test_cli_gaps_cov.test_deps_python_only_preserves_existing_deps;
#    not duplicated here.)
# ==========================================================================


def test_add_dep_then_python_pin_keeps_block_deps_end_to_end(tmp_path):
    """The dependency-axis regression, driven through the real CLI: `skit add --dep`
    injects the dep into the copy's PEP 723 block and leaves meta blank; `skit deps --python`
    then adds the pin WITHOUT erasing the block's dep. Both live in the block uv reads, and
    `deps --json` reports both — the old code's blank-meta reconstruction dropped `requests`
    to `dependencies = []` under a green "constraint updated" line."""
    add = runner.invoke(cli.app, ["add", str(_py(tmp_path)), "--dep", "requests", "--no-input"])
    assert add.exit_code == 0, add.output
    assert store.resolve("x").meta.dependencies is None  # meta deliberately blank
    assert '"requests"' in _block("x")  # the dep lives ONLY in the block

    pin = runner.invoke(cli.app, ["deps", "x", "--python", ">=3.12"])
    assert pin.exit_code == 0, pin.output

    block = _block("x")
    assert '"requests"' in block  # the block dep SURVIVED the python-only edit
    assert 'requires-python = ">=3.12"' in block  # and the pin landed in the same block

    view = runner.invoke(cli.app, ["deps", "x", "--json"])
    assert view.exit_code == 0, view.output
    payload = json.loads(view.stdout)
    assert payload["dependencies"] == ["requests"]  # --json reports the effective dep
    assert payload["requires_python"] == ">=3.12"


def test_add_dep_then_python_pin_run_command_carries_both(tmp_path):
    """The launch surface agrees with the block: the dry-run command shows the pin (--python)
    and `--script` (uv reads the surviving `requests` straight from the block) — no --with, no
    dropped dependency."""
    runner.invoke(cli.app, ["add", str(_py(tmp_path)), "--dep", "requests", "--no-input"])
    runner.invoke(cli.app, ["deps", "x", "--python", ">=3.12"])
    dry = runner.invoke(cli.app, ["run", "x", "--dry-run", "--no-input"])
    assert dry.exit_code == 0, dry.output
    flat = " ".join(dry.output.split())
    assert "--python" in flat  # the pin reaches the launch command
    assert ">=3.12" in flat
    assert "--script" in flat  # uv reads the block's `requests` inline


# ==========================================================================
# 2. HIGH-1 pilot end to end (TUI): the settings screen prefills from the
#    block and diffs each axis against that baseline.
# ==========================================================================


async def test_settings_prefills_deps_and_python_from_the_block(tmp_path):
    """The WYSIWYG fix: a block-only add-time entry (meta blank) opens the ScriptSettingsScreen
    with #st-deps AND #st-python prefilled from the block — not the empty strings raw meta would
    have shown. What the screen shows is what a save keeps."""
    store.add_python(_py(tmp_path), name="x", dependencies=["requests"], requires_python=">=3.11")
    assert store.resolve("x").meta.dependencies is None  # meta blank...
    assert store.resolve("x").meta.requires_python == ""  # ...on both axes
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        screen = ScriptSettingsScreen(store.resolve("x"))
        app.push_screen(screen)
        await pilot.pause()
        assert screen.query_one("#st-deps", Input).value == "requests"
        assert screen.query_one("#st-python", Input).value == ">=3.11"


async def test_settings_deps_only_edit_preserves_the_block_pin(tmp_path):
    """Editing ONLY the deps field on a block-only pinned
    entry must not unpin. Because #st-python was prefilled from the block, it now equals its
    baseline and travels as None (don't-touch) — so the pin survives while the new dep lands."""
    store.add_python(_py(tmp_path), name="x", dependencies=["requests"], requires_python=">=3.11")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        screen = ScriptSettingsScreen(store.resolve("x"))
        app.push_screen(screen)
        await pilot.pause()
        screen.query_one("#st-deps", Input).value = "requests, rich"  # deps axis only
        screen.action_save()
        await pilot.pause()
        assert not isinstance(app.screen, ScriptSettingsScreen)  # committed & dismissed
    block = _block("x")
    assert 'requires-python = ">=3.11"' in block  # the pin the user never touched SURVIVED
    assert '"rich"' in block  # the deps edit landed
    assert '"requests"' in block  # alongside the original


async def test_settings_clearing_python_on_block_only_entry_unpins(tmp_path):
    """The block-only twin of the meta-carried unpin: clearing #st-python (now visibly
    prefilled from the block) differs from its baseline, so it travels explicitly and the save
    removes the block's requires-python line. Clearing what you SEE clears it for real."""
    store.add_python(_py(tmp_path), name="x", dependencies=["requests"], requires_python=">=3.11")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        screen = ScriptSettingsScreen(store.resolve("x"))
        app.push_screen(screen)
        await pilot.pause()
        assert screen.query_one("#st-python", Input).value == ">=3.11"  # user sees the pin
        screen.query_one("#st-python", Input).value = ""  # and clears it
        screen.action_save()
        await pilot.pause()
        assert not isinstance(app.screen, ScriptSettingsScreen)
    block = _block("x")
    assert "requires-python" not in block  # block unpinned
    assert '"requests"' in block  # deps axis untouched by a python-only clear


async def test_settings_untouched_save_never_touches_the_deps_axis(tmp_path, monkeypatch):
    """No edit to either field → both axes equal their baseline → pending_deps is None, so
    update_dependencies is NEVER called (no unpin, no dep-wipe, no needless block rewrite). Pinned
    both ways: a monkeypatched call counter AND the block staying byte-identical."""
    store.add_python(_py(tmp_path), name="x", dependencies=["requests"], requires_python=">=3.11")
    before = _block("x")
    calls: list[object] = []
    real = store.update_dependencies

    def _counting(*args, **kwargs):
        calls.append(args)
        return real(*args, **kwargs)

    # tui_settings reaches the chokepoint via the store module attribute at call time.
    monkeypatch.setattr(store, "update_dependencies", _counting)
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        screen = ScriptSettingsScreen(store.resolve("x"))
        app.push_screen(screen)
        await pilot.pause()
        screen.action_save()  # nothing edited
        await pilot.pause()
        assert not isinstance(app.screen, ScriptSettingsScreen)  # committed & dismissed
    assert calls == []  # the deps chokepoint was never entered
    assert _block("x") == before  # block byte-identical


# ==========================================================================
# 3. Effective read views — human `deps`, `deps --json`, `show --json`
# ==========================================================================


def test_deps_read_human_reports_effective_block_only(tmp_path):
    """`skit deps x` (human) reads EFFECTIVE metadata: a block-only entry prints its real dep and
    pin, never the "—"/blank raw meta would have shown for a list uv installs."""
    store.add_python(_py(tmp_path), name="x", dependencies=["requests"], requires_python=">=3.11")
    result = runner.invoke(cli.app, ["deps", "x"])
    assert result.exit_code == 0, result.output
    assert "Dependencies of x: requests" in result.output
    assert "Python constraint: >=3.11" in result.output


def test_deps_read_json_reports_effective_block_only(tmp_path):
    store.add_python(_py(tmp_path), name="x", dependencies=["requests"], requires_python=">=3.11")
    result = runner.invoke(cli.app, ["deps", "x", "--json"])
    assert result.exit_code == 0, result.output
    payload = json.loads(result.stdout)
    assert payload["dependencies"] == ["requests"]
    assert payload["requires_python"] == ">=3.11"


def test_show_json_reports_effective_deps_for_block_only(tmp_path):
    """`skit show x --json` reports the same effective metadata — the record must describe what a
    run actually does, not the deliberately-blank meta."""
    store.add_python(_py(tmp_path), name="x", dependencies=["requests"], requires_python=">=3.11")
    result = runner.invoke(cli.app, ["show", "x", "--json"])
    assert result.exit_code == 0, result.output
    payload = json.loads(result.stdout)
    assert payload["dependencies"] == ["requests"]
    assert payload["requires_python"] == ">=3.11"


def test_deps_read_meta_carried_entry_is_unchanged(tmp_path):
    """A meta-carried entry (deps recorded in meta, e.g. reference mode) reads straight from meta —
    the block fallback is a python-copy-only path and never fires here."""
    store.add_python(_py(tmp_path), name="x", mode="reference", dependencies=["requests"])
    entry = store.resolve("x")
    assert entry.meta.dependencies == ["requests"]  # meta carries it (reference records in meta)
    result = runner.invoke(cli.app, ["deps", "x", "--json"])
    payload = json.loads(result.stdout)
    assert payload["dependencies"] == ["requests"]


def test_deps_read_js_entry_falls_through_to_meta(tmp_path):
    """A non-python (npm) kind never reads a PEP 723 block — the helper returns meta verbatim. Its
    deps live in meta and its `--json` reports them, with no constraint axis at all."""
    store.add_script(_js(tmp_path), kind="js", name="j")
    store.update_dependencies("j", ["chalk@^5"])
    result = runner.invoke(cli.app, ["deps", "j", "--json"])
    assert result.exit_code == 0, result.output
    payload = json.loads(result.stdout)
    assert payload["dependencies"] == ["chalk@^5"]
    assert payload["requires_python"] == ""  # js carries no python constraint


# ==========================================================================
# 4. store.update_dependencies — the None / [] / "" grammar on BOTH axes
# ==========================================================================


def test_update_dependencies_none_none_is_a_full_no_op(tmp_path):
    """(None, None): touch nothing. Meta unchanged on both axes AND the copy's block byte-identical
    — the chokepoint's "don't-touch" is the whole-record identity, not just meta."""
    store.add_python(_py(tmp_path), name="x", dependencies=["requests"], requires_python=">=3.11")
    before_block = _block("x")
    before_meta = store.resolve("x").meta
    store.update_dependencies("x", None, requires_python=None)
    after = store.resolve("x").meta
    assert after.dependencies == before_meta.dependencies  # still None
    assert after.requires_python == before_meta.requires_python
    assert _block("x") == before_block  # block untouched, byte for byte


def test_update_dependencies_none_python_lands_pin_and_preserves_block_deps(tmp_path):
    """(None, '>=3.12'): the deps axis is untouched, so the block's own `requests` is PRESERVED
    while the pin lands — the store-unit form of the HIGH-2 fix."""
    store.add_python(_py(tmp_path), name="x", dependencies=["requests"])
    assert store.resolve("x").meta.dependencies is None  # block-only deps
    store.update_dependencies("x", None, requires_python=">=3.12")
    block = _block("x")
    assert '"requests"' in block  # deps axis preserved from the block
    assert 'requires-python = ">=3.12"' in block  # pin landed


def test_update_dependencies_clear_deps_preserves_the_pin(tmp_path):
    """([], None) on a pinned python entry: an EXPLICIT deps clear empties the deps in meta AND the
    block, but leaves the untouched constraint axis pinned — the dependency-axis twin of unpin."""
    store.add_python(_py(tmp_path), name="x", dependencies=["requests"], requires_python=">=3.11")
    store.update_dependencies("x", [], requires_python=None)
    assert store.resolve("x").meta.dependencies is None  # cleared in meta
    block = _block("x")
    assert '"requests"' not in block  # cleared in the block too
    assert 'requires-python = ">=3.11"' in block  # the pin the caller never touched SURVIVED


def test_update_dependencies_python_only_edit_syncs_block_from_meta_deps(tmp_path):
    """The meta-carried branch of the block-deps derive rule: a copy-mode python entry whose source
    already had a block records its deps in META (deps_injected stays off). A python-only edit
    (deps=None) then syncs the block from meta.dependencies — the LEFT side of `meta deps or block
    deps`, distinct from the block-only RIGHT side above."""
    src = _py(tmp_path, "# /// script\n# dependencies = []\n# ///\nprint(1)\n")
    store.add_python(src, name="x", dependencies=["requests"])
    assert store.resolve("x").meta.dependencies == ["requests"]  # existing block -> meta carries
    store.update_dependencies("x", None, requires_python=">=3.13")
    block = _block("x")
    assert '"requests"' in block  # block synced from meta deps, not wiped
    assert 'requires-python = ">=3.13"' in block


def test_update_dependencies_missing_stored_copy_still_writes_meta(tmp_path):
    """_sync_python_block's early-return branch: with the stored script.py gone, a deps edit still
    persists meta and never crashes (the block sync simply has nothing to write)."""
    store.add_python(_py(tmp_path), name="x", dependencies=["requests"])
    store.resolve("x").script_path.unlink()
    updated = store.update_dependencies("x", ["rich"], requires_python=None)
    assert updated.meta.dependencies == ["rich"]  # meta write survived the missing copy


def test_update_dependencies_npm_none_does_not_sweep_node_modules(tmp_path, monkeypatch):
    """(None) on an npm entry is UNTOUCHED — the node_modules sweep must not fire. Proven by
    monkeypatching js_deps.clear to explode: an untouched axis never reaches it."""
    store.add_script(_js(tmp_path), kind="js", name="j")
    store.update_dependencies("j", ["chalk"])
    entry = store.resolve("j")
    (entry.dir / "package.json").write_text("{}", encoding="utf-8")
    (entry.dir / "node_modules").mkdir(exist_ok=True)

    def _boom(_dir):
        raise AssertionError("js_deps.clear must not run on an untouched (None) deps axis")

    monkeypatch.setattr(js_deps, "clear", _boom)
    store.update_dependencies("j", None)  # untouched: no sweep
    assert (entry.dir / "node_modules").exists()  # tree left intact
    assert store.resolve("j").meta.dependencies == ["chalk"]  # record untouched


def test_update_dependencies_npm_clear_does_sweep_node_modules(tmp_path):
    """([]) on an npm entry is an EXPLICIT clear — the node_modules sweep DOES fire, the twin of
    the None branch above."""
    store.add_script(_js(tmp_path), kind="js", name="j")
    store.update_dependencies("j", ["chalk"])
    entry = store.resolve("j")
    (entry.dir / "package.json").write_text("{}", encoding="utf-8")
    (entry.dir / "node_modules").mkdir(exist_ok=True)
    store.update_dependencies("j", [])  # explicit clear: sweep
    assert not (entry.dir / "node_modules").exists()
    assert not (entry.dir / "package.json").exists()
    assert store.resolve("j").meta.dependencies is None


# ==========================================================================
# 5. store.effective_uv_metadata — every branch of the read helper
# ==========================================================================


def test_effective_meta_carried_skips_the_block(tmp_path):
    """Both axes present in meta → the block fallback never runs (the `not deps or not constraint`
    guard is False), and meta's values are returned verbatim."""
    store.add_python(_py(tmp_path), name="x", mode="reference", dependencies=["requests"])
    store.update_dependencies("x", ["requests"], requires_python=">=3.11")  # meta carries both
    entry = store.resolve("x")
    assert entry.meta.dependencies == ["requests"]  # meta carries both axes
    assert entry.meta.requires_python == ">=3.11"
    assert store.effective_uv_metadata(entry) == (["requests"], ">=3.11")


def test_effective_block_only_reads_both_axes_from_the_block(tmp_path):
    """Meta blank on both axes, copy-mode python → both deps and constraint come from the block."""
    store.add_python(_py(tmp_path), name="x", dependencies=["requests"], requires_python=">=3.11")
    assert store.effective_uv_metadata(store.resolve("x")) == (["requests"], ">=3.11")


def test_effective_meta_deps_blank_constraint_reads_constraint_from_block(tmp_path):
    """Mixed split: meta carries deps but no pin → the deps axis stays from meta (`if not deps` is
    False) while only the constraint is read from the block."""
    src = _py(tmp_path, '# /// script\n# requires-python = ">=3.9"\n# ///\nprint(1)\n')
    store.add_python(src, name="x", dependencies=["requests"])  # existing block -> meta deps
    entry = store.resolve("x")
    assert entry.meta.dependencies == ["requests"]  # existing block -> meta deps
    assert entry.meta.requires_python == ""  # ...but no pin in meta
    assert store.effective_uv_metadata(entry) == (["requests"], ">=3.9")


def test_effective_meta_constraint_blank_deps_reads_deps_from_block(tmp_path):
    """The mirror mix: meta carries the pin but no deps → the constraint stays from meta (`if not
    constraint` is False) while only the deps come from the block."""
    src = _py(tmp_path, '# /// script\n# dependencies = ["rich"]\n# ///\nprint(1)\n')
    store.add_python(src, name="x", requires_python=">=3.10")  # existing block -> meta pin only
    entry = store.resolve("x")
    assert entry.meta.dependencies is None  # no deps in meta
    assert entry.meta.requires_python == ">=3.10"  # existing block -> meta pin only
    assert store.effective_uv_metadata(entry) == (["rich"], ">=3.10")


def test_effective_both_blank_returns_empty(tmp_path):
    """No deps, no pin, empty block → the helper returns the empty pair (block parse finds
    nothing) — the display baseline that must read as "nothing set", not a crash."""
    store.add_python(_py(tmp_path), name="x")
    assert store.effective_uv_metadata(store.resolve("x")) == ([], "")


def test_effective_reference_mode_python_reads_meta_only(tmp_path):
    """Reference mode short-circuits the block read (`mode == "copy"` is False): even if the
    original file carries a block, the helper reports meta — reference deps live only in meta."""
    src = _py(tmp_path, '# /// script\n# dependencies = ["rich"]\n# ///\nprint(1)\n')
    store.add_python(src, name="x", mode="reference")
    entry = store.resolve("x")
    assert entry.meta.dependencies is None  # reference didn't scrape the block into meta
    assert store.effective_uv_metadata(entry) == ([], "")  # ...and the helper doesn't either


def test_effective_js_entry_reads_meta_only(tmp_path):
    """A non-python kind fails the `kind == "python"` guard → meta verbatim, no block read."""
    store.add_script(_js(tmp_path), kind="js", name="j")
    store.update_dependencies("j", ["chalk"])
    assert store.effective_uv_metadata(store.resolve("j")) == (["chalk"], "")


def test_effective_missing_stored_copy_reads_meta_only(tmp_path):
    """The `script_path.exists()` guard: a block-only entry whose copy is gone can't be read, so
    the helper reports the (blank) meta rather than crashing."""
    store.add_python(_py(tmp_path), name="x", dependencies=["requests"], requires_python=">=3.11")
    store.resolve("x").script_path.unlink()
    assert store.effective_uv_metadata(store.resolve("x")) == ([], "")
