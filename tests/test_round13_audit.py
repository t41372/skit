"""Round-13 design-audit fixes — real-behavior coverage (exit codes, exact refusal/confirmation
copy, meta + stored PEP 723 text, the store chokepoints in isolation).

Every assertion pins an OBSERVABLE contract of the three round-13 (round-12 LOW) fixes:

  * the drafts-boundary refusal names ONLY the flags actually typed — "Drop --ref." /
    "Drop --exe." / "Drop --kind exe.", joined with "/" when more than one is passed —
    because commanding the user to drop a flag they never passed is its own small lie;
  * `skit deps --python`-only prints "Python constraint of NAME updated: …", not the lying
    "Dependencies of NAME updated: —" — a --python edit is not a dependency edit;
  * the '-'/'none' → automatic normalization is gated on uv flavor: on an npm (js/ts) entry
    EVERY --python spelling is inapplicable and refused (exit 2 / StoreUsageError), where it
    used to become a value-dependent silent success;
  * add_python grows the same strip-and-drop + validate-before-build belt update_dependencies
    already has, so a future caller can't route an unparseable dep past the intake.

These never chdir and never touch the real user dirs (the local SKIT_* fixture).
"""

from __future__ import annotations

from pathlib import Path

import pytest
from typer.testing import CliRunner

from skit import cli, i18n, store
from skit.langs import registry
from skit.paths import drafts_dir

runner = CliRunner()


@pytest.fixture(autouse=True)
def tmp_store(tmp_path, monkeypatch):
    monkeypatch.setenv("SKIT_DATA_DIR", str(tmp_path / "data"))
    monkeypatch.setenv("SKIT_STATE_DIR", str(tmp_path / "state"))
    monkeypatch.setenv("SKIT_CONFIG_DIR", str(tmp_path / "config"))
    monkeypatch.setenv("SKIT_LANG", "en")
    i18n.init("en")


def _flat(text: str) -> str:
    return " ".join(text.split())


def _py(tmp_path, body: str, name: str = "s.py") -> Path:
    p = tmp_path / name
    p.write_text(body, encoding="utf-8")
    return p


def _js(tmp_path, body: str = "console.log(1)\n", name: str = "t.js") -> Path:
    p = tmp_path / name
    p.write_text(body, encoding="utf-8")
    return p


def _draft(name: str, body: str) -> Path:
    drafts_dir().mkdir(parents=True, exist_ok=True)
    p = drafts_dir() / name
    p.write_text(body, encoding="utf-8")
    return p


def _stored_block(slug: str) -> str:
    return (store.resolve(slug).dir / "script.py").read_text(encoding="utf-8")


# ==========================================================================
# 1. The drafts-boundary refusal names ONLY the flags actually typed
# ==========================================================================


def test_two_flags_together_are_both_named_and_joined(tmp_path):
    """--ref AND --exe together → both are named, joined with "/" in passing order
    ("Drop --ref/--exe."). --kind (never passed) stays out of the message."""
    draft = _draft("skit-new-both.py", "print('x')\n")
    result = runner.invoke(
        cli.app, ["add", str(draft), "-n", "both", "--ref", "--exe", "--no-input"]
    )
    assert result.exit_code == 2, result.output
    flat = _flat(result.output)
    assert "one of skit's own kept drafts" in flat
    assert "Drop --ref/--exe." in flat  # both named, joined in passing order
    assert "--kind" not in flat  # never passed — never named
    assert draft.exists()  # a refused add consumes nothing


def test_kind_exe_alone_names_only_kind_exe(tmp_path):
    """--kind exe alone → the refusal names ONLY "--kind exe"; the "--ref"/"--exe" flag
    literals (neither passed) are absent — the honest-naming rule end to end."""
    draft = _draft("skit-new-kindonly.py", "print('x')\n")
    result = runner.invoke(
        cli.app, ["add", str(draft), "-n", "kindonly", "--kind", "exe", "--no-input"]
    )
    assert result.exit_code == 2, result.output
    flat = _flat(result.output)
    assert "Drop --kind exe." in flat
    assert "--ref" not in flat
    assert "--exe" not in flat  # "--kind exe" is not the bare "--exe" flag literal
    assert draft.exists()


# ==========================================================================
# 2. '-'/'none' → automatic is gated on uv flavor
# ==========================================================================


def test_js_deps_python_dash_is_refused_as_inapplicable(tmp_path):
    """`skit deps <js> --python -` is REFUSED (exit 2, "doesn't apply"), NOT silently accepted:
    normalizing '-' to "" first would make a kind-inapplicable flag succeed for some spellings
    (— / none) and fail for others (>=3.11) — value-dependent acceptance."""
    store.add_script(_js(tmp_path), kind="js", name="jsx")
    result = runner.invoke(cli.app, ["deps", "jsx", "--python", "-"])
    assert result.exit_code == 2, result.output
    assert "A Python constraint doesn't apply to js scripts." in _flat(result.output)


def test_js_deps_python_none_is_refused_as_inapplicable(tmp_path):
    """The other automatic token behaves identically: '-' and 'none' are NOT special-cased
    into acceptance on an npm entry."""
    store.add_script(_js(tmp_path), kind="js", name="jsx")
    result = runner.invoke(cli.app, ["deps", "jsx", "--python", "none"])
    assert result.exit_code == 2, result.output
    assert "A Python constraint doesn't apply to js scripts." in _flat(result.output)


def test_python_deps_python_dash_is_still_automatic(tmp_path):
    """The regression: on a uv-flavor (python) entry, '-' STILL normalizes to automatic — the
    gate narrows the normalization to uv entries, it does not remove it."""
    store.add_python(_py(tmp_path, "print(1)\n"), name="a")
    store.update_dependencies("a", ["requests"], requires_python=">=3.11")
    result = runner.invoke(cli.app, ["deps", "a", "--python", "-"])
    assert result.exit_code == 0, result.output
    assert store.resolve("a").meta.requires_python == ""  # cleared to automatic


def test_store_npm_spec_plus_dash_reaches_the_npm_refusal(tmp_path):
    """The store unit: an npm-flavor entry + '-' is NOT normalized before the npm branch, so it
    reaches the 'doesn't apply' refusal (StoreUsageError) instead of a silent accept."""
    store.add_script(_js(tmp_path), kind="js", name="jsx")
    with pytest.raises(store.StoreUsageError) as exc:
        store.update_dependencies("jsx", [], requires_python="-")
    assert "doesn't apply" in str(exc.value)


def test_store_uv_spec_plus_dash_normalizes(tmp_path):
    """The complement unit: a uv-flavor entry + '-' IS normalized to "" (the gate's True
    branch) — meta records automatic, no error."""
    store.add_python(_py(tmp_path, "print(1)\n"), name="a")
    entry = store.update_dependencies("a", ["requests"], requires_python="none")
    assert entry.meta.requires_python == ""


# ==========================================================================
# 3. add_python's strip-and-drop + validate-before-build belt
# ==========================================================================


def test_add_python_belt_rejects_a_bad_dep_before_any_entry_exists(tmp_path):
    """A direct store.add_python with an unparseable dependency raises at the belt — BEFORE the
    source is read or a meta/entry dir is built, so no half-made entry is registered."""
    src = _py(tmp_path, "print(1)\n")
    with pytest.raises(store.StoreUsageError) as exc:
        store.add_python(src, name="belt", dependencies=["@@@"])
    assert "isn't a package requirement" in str(exc.value)
    with pytest.raises(store.NotFoundError):
        store.resolve("belt")  # nothing was created


def test_add_python_belt_rejects_a_bad_python_before_any_entry_exists(tmp_path):
    """The constraint half of the belt: an unparseable requires-python is refused the same way."""
    src = _py(tmp_path, "print(1)\n")
    with pytest.raises(store.StoreUsageError) as exc:
        store.add_python(src, name="belt", requires_python="not-a-version")
    assert "isn't a Python version constraint" in str(exc.value)
    with pytest.raises(store.NotFoundError):
        store.resolve("belt")


def test_add_python_belt_drops_a_whitespace_dep_from_the_block(tmp_path):
    """The strip-and-drop half: a whitespace-only entry alongside a real one is dropped — the
    stored block declares only the real dependency, never the "" that would brick every run."""
    src = _py(tmp_path, "print(1)\n")
    entry = store.add_python(src, name="belt2", dependencies=["  ", "rich"])
    block = (entry.dir / "script.py").read_text(encoding="utf-8")
    assert "rich" in block
    assert '"  "' not in block  # the whitespace entry never reached the PEP 723 block


def test_add_python_belt_with_no_deps_is_unchanged(tmp_path):
    """The None branch of the belt (`dependencies` falsy → stays None): a plain add still writes
    no block and records no dependencies — the belt is transparent when there is nothing to
    filter."""
    src = _py(tmp_path, "print(1)\n")
    entry = store.add_python(src, name="plain")
    assert entry.meta.dependencies is None
    assert "# /// script" not in (entry.dir / "script.py").read_text(encoding="utf-8")


# ==========================================================================
# 4. `skit deps` confirmation-line honesty
# ==========================================================================


def test_deps_python_only_prints_the_constraint_line_not_the_deps_line(tmp_path):
    """--python alone edited only the constraint, so the confirmation says so — and does NOT
    claim "Dependencies … updated", which would describe an edit that never happened."""
    store.add_python(_py(tmp_path, "print(1)\n"), name="a")
    result = runner.invoke(cli.app, ["deps", "a", "--python", ">=3.11"])
    assert result.exit_code == 0, result.output
    flat = _flat(result.output)
    assert "Python constraint of a updated: >=3.11" in flat
    assert "Dependencies" not in flat  # the edit that didn't happen isn't reported


def test_deps_python_only_dash_reports_the_dash_placeholder(tmp_path):
    """--python - clears to automatic; the constraint line shows the em-dash placeholder for
    "no constraint recorded" (the `escape(...) or '—'` fallback)."""
    store.add_python(_py(tmp_path, "print(1)\n"), name="a")
    store.update_dependencies("a", ["requests"], requires_python=">=3.11")
    result = runner.invoke(cli.app, ["deps", "a", "--python", "-"])
    assert result.exit_code == 0, result.output
    flat = _flat(result.output)
    assert "Python constraint of a updated: —" in flat
    assert "Dependencies" not in flat


def test_deps_dep_only_prints_the_deps_line(tmp_path):
    """--dep alone edited the dependency list, so the confirmation is the deps line — not the
    constraint line (the else branch of the message selection)."""
    store.add_python(_py(tmp_path, "print(1)\n"), name="a")
    result = runner.invoke(cli.app, ["deps", "a", "--dep", "requests"])
    assert result.exit_code == 0, result.output
    flat = _flat(result.output)
    assert "Dependencies of a updated: requests" in flat
    assert "Python constraint of" not in flat


def test_deps_dep_and_python_together_prints_the_deps_line(tmp_path):
    """--dep AND --python: a dependency edit DID happen, so the deps line is printed (the
    python-only branch requires `dep is None`) — the constraint rides along in the stored block,
    not a second line."""
    store.add_python(_py(tmp_path, "print(1)\n"), name="a")
    result = runner.invoke(cli.app, ["deps", "a", "--dep", "rich", "--python", ">=3.12"])
    assert result.exit_code == 0, result.output
    flat = _flat(result.output)
    assert "Dependencies of a updated: rich" in flat
    assert "Python constraint of" not in flat
    assert store.resolve("a").meta.requires_python == ">=3.12"  # the constraint still landed


def test_deps_clear_prints_the_deps_line(tmp_path):
    """--clear is a dependency edit (to empty), so it takes the deps line too — the python-only
    branch is gated on `not clear`, so an emptied list is never mistaken for a constraint edit."""
    store.add_python(_py(tmp_path, "print(1)\n"), name="a")
    store.update_dependencies("a", ["requests"])
    result = runner.invoke(cli.app, ["deps", "a", "--clear"])
    assert result.exit_code == 0, result.output
    flat = _flat(result.output)
    assert "Dependencies of a updated: —" in flat
    assert "Python constraint of" not in flat


# ==========================================================================
# 5. registry sanity — js is the npm flavor this whole gate keys on
# ==========================================================================


def test_js_is_npm_flavor_and_python_is_not(tmp_path):
    """The premise the uv_flavor gate rests on, pinned directly: js is deps_flavor 'npm'
    (so its --python is inapplicable) and python is not."""
    js_spec = registry.spec_for("js")
    py_spec = registry.spec_for("python")
    assert js_spec is not None
    assert py_spec is not None
    assert js_spec.deps_flavor == "npm"
    assert py_spec.deps_flavor != "npm"
