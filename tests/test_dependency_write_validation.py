"""Dependency-write validation and draft-refusal contracts (exit codes, exact refusal copy,
filesystem/meta state, stored PEP 723 text, the store's validate-then-write chokepoint, the
suggest-dependencies self-fabrication filter).

Every assertion pins an observable dependency-write or draft-refusal contract:

  * validate-then-write now covers EVERY uv-metadata writer, not just `skit add`: `skit deps`
    routes through store.update_dependencies, whose new `_validate_uv_metadata` refuses an
    unparseable --dep / --python BEFORE any write (exit 2, the localized validator message,
    meta AND the stored PEP 723 block untouched) — an npm-flavor entry is skipped (the npm
    installer owns that grammar), and '-'/'none' normalize to "" (automatic);
  * the deps-before-needs abort order holds: a refused deps write leaves the needs untouched;
  * pep723.suggest_dependencies filters its own output through requirement_error — a legal
    Python identifier that is an illegal PEP 508 name (`import café`) is never suggested, so a
    --no-input add of such a script writes NO block instead of a bricked one;
  * the two new draft refusals: an INFERRED exe on a kept draft (no flag to drop) gets the
    --kind variant, while a shebang-less draft skit can't classify gets the classify variant —
    and the SAME file outside drafts/ still gets the full --exe/--cmd escape;
  * the drafts guard precedes the .md "looks like a prompt" ask — a refusal never follows an
    answered question.

These never chdir and never touch the real user dirs (the local SKIT_* fixture).
"""

from __future__ import annotations

import json
import os
from pathlib import Path

import pytest
from typer.testing import CliRunner

from skit import cli, i18n, pep723, store
from skit.paths import drafts_dir, is_draft

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


def _draft(name: str, body: str) -> Path:
    drafts_dir().mkdir(parents=True, exist_ok=True)
    p = drafts_dir() / name
    p.write_text(body, encoding="utf-8")
    return p


def _stored_block(slug: str) -> str:
    return (store.resolve(slug).dir / "script.py").read_text(encoding="utf-8")


# ==========================================================================
# 1. `skit deps` validate-then-write (the CLI face of the store chokepoint)
# ==========================================================================


def test_deps_garbage_dep_is_refused_and_nothing_changes(tmp_path):
    store.add_python(
        _py(
            tmp_path, 'import requests\n# /// script\n# dependencies = ["rich"]\n# ///\nprint(1)\n'
        ),
        name="a",
    )
    before_block = _stored_block("a")
    before_deps = store.resolve("a").meta.dependencies
    result = runner.invoke(cli.app, ["deps", "a", "--dep", "@@@"])
    assert result.exit_code == 2, result.output
    assert "isn't a package requirement" in _flat(result.output)
    assert store.resolve("a").meta.dependencies == before_deps  # meta untouched
    assert _stored_block("a") == before_block  # the PEP 723 block untouched (no partial write)


def test_deps_garbage_python_is_refused_and_nothing_changes(tmp_path):
    store.add_python(_py(tmp_path, "print(1)\n"), name="a")
    store.update_dependencies("a", ["requests"], requires_python=">=3.11")
    before_block = _stored_block("a")
    result = runner.invoke(cli.app, ["deps", "a", "--python", "not-a-version"])
    assert result.exit_code == 2, result.output
    assert "isn't a Python version constraint" in _flat(result.output)
    assert store.resolve("a").meta.requires_python == ">=3.11"  # unchanged
    assert _stored_block("a") == before_block


def test_deps_dash_python_clears_meta_and_unpins_the_block(tmp_path):
    """'-' means automatic AND an explicit unpin. Inverted from the old "preserve on unpin"
    behavior that otherwise creates a false statement on three surfaces at once: meta,
    `--json`, and the visible `run` command all reported "—" while uv still silently enforced
    the block's own `requires-python`. Now the caller's explicit unpin (requires_python == "",
    the '-' token) reaches the block uv actually reads: meta clears to "", the `requires-python`
    line is REMOVED, `--json` agrees, and the dry-run command carries no `--python`."""
    store.add_python(_py(tmp_path, "print(1)\n"), name="a")
    store.update_dependencies("a", ["requests"], requires_python=">=3.11")
    assert 'requires-python = ">=3.11"' in _stored_block("a")  # pinned first
    result = runner.invoke(cli.app, ["deps", "a", "--python", "-"])
    assert result.exit_code == 0, result.output
    assert store.resolve("a").meta.requires_python == ""  # meta cleared
    assert "requires-python" not in _stored_block("a")  # block unpinned — the line is gone
    view = runner.invoke(cli.app, ["deps", "a", "--json"])
    assert json.loads(view.stdout)["requires_python"] == ""  # --json agrees (no stale pin)
    dry = runner.invoke(cli.app, ["run", "a", "--dry-run", "--no-input"])
    assert dry.exit_code == 0, dry.output
    assert "--python" not in _flat(dry.output)  # the run uv would launch carries no constraint


def test_deps_only_edit_still_preserves_the_blocks_own_pin(tmp_path):
    """The companion the unpin must not break: a DEPS-ONLY edit (no --python flag, so
    requires_python is None at the chokepoint) still PRESERVES the block's own
    requires-python — the original derive rule, intact for the case it was written for."""
    store.add_python(_py(tmp_path, "print(1)\n"), name="a")
    store.update_dependencies("a", ["requests"], requires_python=">=3.11")
    result = runner.invoke(cli.app, ["deps", "a", "--dep", "rich"])
    assert result.exit_code == 0, result.output
    assert 'requires-python = ">=3.11"' in _stored_block("a")  # block pin preserved
    assert "rich" in _stored_block("a")  # and the deps edit landed


def test_deps_none_python_clears_meta_when_nothing_to_preserve(tmp_path):
    """'none' is the other automatic token: meta clears to "" and, with no prior block
    constraint to preserve, the block carries none either."""
    store.add_python(_py(tmp_path, "print(1)\n"), name="a")
    store.update_dependencies("a", ["requests"])  # deps only, no python
    result = runner.invoke(cli.app, ["deps", "a", "--dep", "requests", "--python", "none"])
    assert result.exit_code == 0, result.output
    assert store.resolve("a").meta.requires_python == ""
    assert "requires-python" not in _stored_block("a")


def test_deps_valid_dep_and_python_still_write(tmp_path):
    """The complement: a valid requirement + a valid constraint pass the validator and land
    in both meta and the stored block."""
    store.add_python(_py(tmp_path, "print(1)\n"), name="a")
    result = runner.invoke(cli.app, ["deps", "a", "--dep", "requests>=2,<3", "--python", "~=3.12"])
    assert result.exit_code == 0, result.output
    meta = store.resolve("a").meta
    assert meta.dependencies == ["requests>=2,<3"]
    assert meta.requires_python == "~=3.12"
    block = _stored_block("a")
    assert "requests>=2,<3" in block
    assert 'requires-python = "~=3.12"' in block


def test_deps_refused_write_leaves_needs_untouched(tmp_path):
    """The documented deps-before-needs abort order: a --dep refusal raises at the store
    chokepoint before ANY write, so a --need in the same call never lands (a partial apply a
    --json/CI caller couldn't detect)."""
    entry = store.add_python(_py(tmp_path, "print(1)\n"), name="a")
    store.update_needs(entry.slug, ["jq"])
    result = runner.invoke(cli.app, ["deps", "a", "--dep", "@@@", "--need", "ffmpeg"])
    assert result.exit_code == 2, result.output
    assert store.resolve("a").meta.needs == ["jq"]  # the needs write never ran


def test_deps_npm_entry_takes_an_npm_shaped_dep_that_fails_pep508(tmp_path):
    """An npm-flavor (js) entry is NOT routed through the PEP 508 validator: a scoped package
    (`@scope/thing` — which requirement_error rejects) is accepted, because the npm installer
    owns that grammar, not skit's validator."""
    js = tmp_path / "t.js"
    js.write_text('import x from "@scope/thing";\nconsole.log(x)\n', encoding="utf-8")
    store.add_script(js, kind="js", name="jsx")
    assert pep723.requirement_error("@scope/thing") is not None  # would fail if validated
    result = runner.invoke(cli.app, ["deps", "jsx", "--dep", "@scope/thing"])
    assert result.exit_code == 0, result.output
    assert store.resolve("jsx").meta.dependencies == ["@scope/thing"]


# ==========================================================================
# 2. store._validate_uv_metadata via the public update_dependencies
# ==========================================================================


def test_update_dependencies_uv_invalid_dep_raises_usage_error(tmp_path):
    store.add_python(_py(tmp_path, "print(1)\n"), name="a")
    with pytest.raises(store.StoreUsageError) as exc:
        store.update_dependencies("a", ["@@@"])
    assert "isn't a package requirement" in str(exc.value)
    assert store.resolve("a").meta.dependencies is None  # nothing written


def test_update_dependencies_uv_invalid_python_raises_usage_error(tmp_path):
    store.add_python(_py(tmp_path, "print(1)\n"), name="a")
    with pytest.raises(store.StoreUsageError) as exc:
        store.update_dependencies("a", ["requests"], requires_python="not-a-version")
    assert "isn't a Python version constraint" in str(exc.value)


def test_update_dependencies_drops_a_whitespace_only_dep_at_the_chokepoint(tmp_path):
    """The chokepoint strip-and-drops empty entries BEFORE validating or writing: a
    whitespace-only requirement is "nothing", never recorded — the valid neighbour commits
    alone. Exact-equality pins the DROP (a tolerated "  " would leave a 2-element list and,
    written verbatim, brick every run with uv's raw "Empty field" error); and it never reaches
    the block either. Without the strip-and-drop the "  " would instead reach requirement_error
    and RAISE, so a clean commit is itself the proof."""
    store.add_python(_py(tmp_path, "print(1)\n"), name="a")
    entry = store.update_dependencies("a", ["  ", "requests"])
    assert entry.meta.dependencies == ["requests"]  # dropped, not tolerated
    block = _stored_block("a")
    assert "requests" in block
    assert '"  "' not in block  # the whitespace entry never reached the PEP 723 block


def test_update_dependencies_all_whitespace_list_clears_deps(tmp_path):
    """The degenerate case: a list of nothing-but-whitespace strip-and-drops to empty, which
    clears the record — meta.dependencies back to None and the block's dependencies emptied."""
    store.add_python(_py(tmp_path, "print(1)\n"), name="a")
    store.update_dependencies("a", ["requests"])
    entry = store.update_dependencies("a", ["   ", "\t"])
    assert entry.meta.dependencies is None  # cleared
    block = _stored_block("a")
    assert "dependencies = []" in block
    assert "requests" not in block


def test_update_dependencies_npm_flavor_skips_uv_validation(tmp_path):
    """The npm branch of _validate_uv_metadata: an @scope/thing that PEP 508 rejects is
    accepted for a js entry — the validator returns early on deps_flavor == 'npm'."""
    js = tmp_path / "t.js"
    js.write_text("console.log(1)\n", encoding="utf-8")
    store.add_script(js, kind="js", name="jsx")
    entry = store.update_dependencies("jsx", ["@scope/thing"])
    assert entry.meta.dependencies == ["@scope/thing"]


def test_update_dependencies_normalizes_dash_python_before_validating(tmp_path):
    """A literal '-' reaches the store on the deps path too: it normalizes to "" BEFORE the
    validator (which would reject '-' as a specifier), leaving meta automatic."""
    store.add_python(_py(tmp_path, "print(1)\n"), name="a")
    entry = store.update_dependencies("a", ["requests"], requires_python="-")
    assert entry.meta.requires_python == ""


# ==========================================================================
# 3. suggest_dependencies filters its own fabrications through requirement_error
# ==========================================================================


def test_suggest_dependencies_drops_a_name_pep508_refuses():
    """`café` is a legal Python identifier but an illegal PEP 508 distribution name — it must
    not be suggested (the non-interactive add takes suggestions as-is). `requests` is kept."""
    suggested = pep723.suggest_dependencies("import café\nimport requests\nprint(1)\n")
    assert suggested == ["requests"]
    assert all(pep723.requirement_error(s) is None for s in suggested)


def test_no_input_add_of_an_illegally_named_import_writes_no_block(tmp_path):
    """End-to-end: a --no-input add of a script whose only third-party import is `café` writes
    NO PEP 723 block (the old code fabricated `café` into the block, bricking every run)."""
    src = _py(tmp_path, "import café\nprint(café)\n", "cafe.py")
    result = runner.invoke(cli.app, ["add", str(src), "-n", "cafe", "--no-input"])
    assert result.exit_code == 0, result.output
    stored = _stored_block("cafe")
    assert "# /// script" not in stored  # no block fabricated (nothing valid to declare)
    assert store.resolve("cafe").meta.dependencies is None  # café never recorded as a dep


# ==========================================================================
# 4. The two new draft refusal messages + the outside-drafts regression
# ==========================================================================

_DRAFT_HEAD = "one of skit's own kept drafts"


def test_inferred_exe_draft_gets_the_kind_variant(tmp_path):
    """A hand-planted +x on an extensionless draft INFERS exe with no flag passed — the refusal
    points at --kind (there is nothing to drop), not the flag-drop message."""
    draft = _draft("skit-new-binish", "opaque program bytes\n")
    os.chmod(draft, 0o755)  # noqa: S103 — POSIX infer_kind classifies +x as exe
    assert is_draft(draft)
    result = runner.invoke(cli.app, ["add", str(draft), "-n", "b1", "--no-input"])
    assert result.exit_code == 2, result.output
    flat = _flat(result.output)
    assert _DRAFT_HEAD in flat
    assert "pass --kind <language> to name its language" in flat
    assert "Drop" not in flat  # not the flag-route message — nothing was passed to drop
    assert draft.exists()  # a refused add consumes nothing


def test_exe_flag_on_the_same_draft_gets_the_drop_variant_naming_only_exe(tmp_path):
    """The flag route on the same kind of file: --exe WAS passed, so the message tells the user
    to drop it (the other branch of the message conditional) — and names ONLY --exe, since that
    is the only flag passed."""
    draft = _draft("skit-new-binish2", "opaque program bytes\n")
    result = runner.invoke(cli.app, ["add", str(draft), "-n", "b2", "--exe", "--no-input"])
    assert result.exit_code == 2, result.output
    flat = _flat(result.output)
    assert "Drop --exe." in flat
    assert "--ref" not in flat  # never passed — never named
    assert "--kind" not in flat
    assert "to name its language" not in flat  # not the inferred-route message
    assert draft.exists()


def test_shebang_less_unclassifiable_draft_gets_the_classify_variant(tmp_path):
    """A weird-extension, shebang-less kept draft infers 'unknown' with no #! — the classify
    variant offers only --kind / --prompt (never --exe or --cmd, which the drafts boundary and
    a fileless template respectively can't take)."""
    draft = _draft("skit-new-weird.xyz", "just some content\n")
    result = runner.invoke(cli.app, ["add", str(draft), "-n", "w1", "--no-input"])
    assert result.exit_code == 2, result.output
    flat = _flat(result.output)
    assert "kept draft skit can't classify" in flat
    assert "--kind <language> to add it as a script" in flat
    assert "--prompt for an AI-agent prompt" in flat
    assert "--exe" not in flat  # the draft variant never offers the program escape
    assert "--cmd" not in flat  # nor the command-template escape
    assert draft.exists()


def test_same_unclassifiable_file_outside_drafts_gets_the_full_escape(tmp_path):
    """Regression: the SAME shebang-less weird-extension file OUTSIDE drafts/ is not a draft,
    so it keeps the full escape message naming --exe and --cmd (which an on-disk file can take)."""
    f = tmp_path / "weird.xyz"
    f.write_text("just some content\n", encoding="utf-8")
    result = runner.invoke(cli.app, ["add", str(f), "-n", "w2", "--no-input"])
    assert result.exit_code == 2, result.output
    flat = _flat(result.output)
    assert "isn't a script or an executable" in flat
    assert "--exe for a program" in flat
    assert "--cmd for a command template" in flat
    assert "kept draft" not in flat


# ==========================================================================
# 5. The drafts guard precedes the .md "looks like a prompt" ask
# ==========================================================================


def test_ref_on_an_md_draft_is_refused_before_the_prompt_ask(monkeypatch):
    """A refusal must not follow an answered question: a .md kept draft with --ref is refused
    at the drafts guard BEFORE the 'looks like a prompt' Confirm ever runs. The environment is
    made interactive (so the ask WOULD be reachable were the ordering wrong), and Confirm.ask
    is monkeypatched to explode if reached."""
    monkeypatch.setattr(cli, "_is_interactive", lambda: True)

    def boom(*a, **kw):
        raise AssertionError("Confirm.ask must not run — the drafts guard precedes it")

    monkeypatch.setattr(cli.Confirm, "ask", boom)
    draft = _draft("skit-new-note.md", "# Summarize {{text}}.\n")
    result = runner.invoke(cli.app, ["add", str(draft), "-n", "md1", "--ref"])
    assert result.exit_code == 2, result.output
    flat = _flat(result.output)
    assert _DRAFT_HEAD in flat
    assert "Drop --ref." in flat  # only --ref was passed — only --ref is named
    assert "--exe" not in flat
    assert draft.exists()  # nothing consumed
