"""Round-11 design-audit fixes — real-behavior coverage (exit codes, exact refusal copy,
filesystem state, stored PEP 723 text, the two lazy `packaging` validators in isolation).

Every assertion pins an observable contract of the round-11 (round-10 finding) fixes:

  * exe entries can never cross the drafts boundary: --exe / --kind exe / --ref / an
    INFERRED exe on skit's OWN kept draft is refused (exit 2, the widened message naming
    Drop --ref/--exe, the draft kept, no entry) — a program entry is reference-by-construction
    and the store would hold nothing;
  * --dep / --python are validated (PEP 440/508 via `packaging`) at the intake, BEFORE the
    pipe is read or the editor opens or a draft materializes — validate-then-write, so an
    unparseable value never bricks a future run and never costs an authoring session; and
    '-'/'none' normalize to "" (automatic);
  * the interactive deps and python asks are re-ask loops on an invalid entry;
  * kind_for_draft keys its suffix-outranks-shebang exception on the RATIONALE (a
    placeholder-bodied kind: .prompt as well as .prompt.md), and an extensionless draft
    still falls to the shebang;
  * the unknown+shebang refusal offers --exe to an on-disk file but only --kind to a draft.

These never chdir and never touch the real user dirs (the local SKIT_* fixture).
"""

from __future__ import annotations

import os
from pathlib import Path

import pytest
from typer.testing import CliRunner

from skit import cli, i18n, pep723, store
from skit.langs import registry
from skit.paths import drafts_dir, is_draft

runner = CliRunner()


@pytest.fixture(autouse=True)
def tmp_store(tmp_path, monkeypatch):
    monkeypatch.setenv("SKIT_DATA_DIR", str(tmp_path / "data"))
    monkeypatch.setenv("SKIT_STATE_DIR", str(tmp_path / "state"))
    monkeypatch.setenv("SKIT_CONFIG_DIR", str(tmp_path / "config"))
    monkeypatch.setenv("SKIT_LANG", "en")
    i18n.init("en")


@pytest.fixture
def tty(monkeypatch):
    monkeypatch.setattr("sys.stdin.isatty", lambda: True, raising=False)


def _flat(text: str) -> str:
    return " ".join(text.split())


def _draft(name: str, body: str) -> Path:
    drafts_dir().mkdir(parents=True, exist_ok=True)
    p = drafts_dir() / name
    p.write_text(body, encoding="utf-8")
    return p


def _drafts_files() -> list[Path]:
    d = drafts_dir()
    return sorted(d.iterdir()) if d.exists() else []


def _capture_ask(
    monkeypatch, answers: list[str]
) -> list[tuple[tuple[object, ...], dict[str, object]]]:
    it = iter(answers)
    calls: list[tuple[tuple[object, ...], dict[str, object]]] = []

    def fake(*a: object, **kw: object) -> object:
        calls.append((a, kw))
        return next(it)

    monkeypatch.setattr(cli.Prompt, "ask", fake)
    return calls


# ==========================================================================
# 1. The pep723 validators (lazy `packaging` imports), in isolation
# ==========================================================================


def test_requires_python_error_is_none_for_valid_constraints():
    assert pep723.requires_python_error(">=3.11") is None
    assert pep723.requires_python_error(">=3.12,<3.13") is None


def test_requires_python_error_localizes_a_message_for_an_invalid_constraint():
    msg = pep723.requires_python_error("not-a-version")
    assert msg is not None
    assert msg.startswith("not-a-version isn't a Python version constraint")


def test_requires_python_error_rejects_a_bare_version_without_operator():
    # `3.11` (no comparison operator) is a real, common mistake — PEP 440 refuses it.
    assert pep723.requires_python_error("3.11") is not None


def test_requirement_error_is_none_for_valid_requirements():
    assert pep723.requirement_error("requests") is None
    assert pep723.requirement_error("rich>=13,<16") is None
    assert pep723.requirement_error("demo[bold]") is None  # extras are valid PEP 508


def test_requirement_error_localizes_a_message_for_an_invalid_requirement():
    msg = pep723.requirement_error("@@@")
    assert msg is not None
    assert msg.startswith("@@@ isn't a package requirement")


# ==========================================================================
# 2. _validate_python_flags — validate + '-'/'none' normalization
# ==========================================================================


def test_validate_python_flags_passes_valid_and_normalizes_the_constraint():
    assert cli._validate_python_flags(["requests", "rich>=13,<16"], ">=3.11") == ">=3.11"


def test_validate_python_flags_normalizes_dash_and_none_to_empty():
    assert cli._validate_python_flags(None, "-") == ""
    assert cli._validate_python_flags(None, "none") == ""
    assert cli._validate_python_flags(None, "  NONE  ") == ""


def test_validate_python_flags_returns_none_when_no_python_given():
    assert cli._validate_python_flags(["requests"], None) is None


def test_validate_python_flags_treats_an_empty_python_as_empty():
    assert cli._validate_python_flags(None, "   ") == ""


def test_validate_python_flags_skips_empty_dep_strings():
    # A whitespace-only --dep is dropped (not routed to the validator), matching the block-write.
    assert cli._validate_python_flags(["  "], None) is None


def test_validate_python_flags_exits_2_on_a_bad_dep():
    with pytest.raises(cli.typer.Exit) as exc:
        cli._validate_python_flags(["@@@"], None)
    assert exc.value.exit_code == cli.EXIT_USAGE


def test_validate_python_flags_exits_2_on_a_bad_python():
    with pytest.raises(cli.typer.Exit) as exc:
        cli._validate_python_flags(None, "not-a-version")
    assert exc.value.exit_code == cli.EXIT_USAGE


# ==========================================================================
# 3. The interactive deps / python asks are re-ask loops on invalid input
# ==========================================================================

_NOPIN_TEXT = "#!/usr/bin/env python3\nimport requests\nprint(requests)\n"


def test_interactive_deps_reask_then_python_reask_then_accept(tty, monkeypatch):
    """An invalid deps answer re-asks (never stored); '-' means none. An invalid python
    constraint re-asks; a valid one is finally recorded. Four asks: deps twice, python twice."""
    calls = _capture_ask(monkeypatch, ["@@@", "-", "not-a-version", ">=3.11"])
    deps, py = cli._resolve_python_metadata(_NOPIN_TEXT, None, None, no_input=False)
    assert deps == []  # '-' at the deps ask means none
    assert py == ">=3.11"  # the valid constraint was accepted after the re-ask
    assert len(calls) == 4  # deps asked twice, python asked twice (each re-asked once)


def test_interactive_valid_deps_accepted_first_try(tty, monkeypatch):
    """The complement: a valid deps list is taken on the first ask (the inner validate loop
    completes with bad=None), and '-' at the python ask means automatic."""
    calls = _capture_ask(monkeypatch, ["rich>=13,<16", "-"])
    deps, py = cli._resolve_python_metadata(_NOPIN_TEXT, None, None, no_input=False)
    assert deps == ["rich>=13,<16"]
    assert py == ""  # '-' -> automatic
    assert len(calls) == 2  # no re-ask


# ==========================================================================
# 4. exe / reference can never cross the drafts boundary (every face)
# ==========================================================================

_DRAFT_HEAD = "one of skit's own kept drafts"
_DRAFT_DROP = "Drop --ref/--exe"


def test_exe_flag_on_a_kept_draft_is_refused_and_keeps_it(tmp_path):
    draft = _draft("skit-new-prog.py", "print('run me')\n")
    assert is_draft(draft)
    result = runner.invoke(cli.app, ["add", str(draft), "-n", "p1", "--exe", "--no-input"])
    assert result.exit_code == 2, result.output
    flat = _flat(result.output)
    assert _DRAFT_HEAD in flat
    assert _DRAFT_DROP in flat
    assert draft.exists()  # a refused add consumes nothing
    with pytest.raises(store.NotFoundError):
        store.resolve("p1")


def test_kind_exe_on_a_kept_draft_is_refused_and_keeps_it(tmp_path):
    draft = _draft("skit-new-prog2.py", "print('run me')\n")
    result = runner.invoke(cli.app, ["add", str(draft), "-n", "p2", "--kind", "exe", "--no-input"])
    assert result.exit_code == 2, result.output
    assert _DRAFT_DROP in _flat(result.output)
    assert draft.exists()
    with pytest.raises(store.NotFoundError):
        store.resolve("p2")


def test_inferred_exe_on_a_kept_draft_is_refused_and_keeps_it(tmp_path):
    """A hand-planted +x bit on an extensionless draft INFERS exe — the widened guard
    covers the inferred route just like the explicit flags (no smuggling one past). The
    INFERRED route (the user passed no flag) gets the --kind message, not the Drop-flags
    one: there is no flag to drop, so it points at the escape a draft can actually take."""
    draft = _draft("skit-new-binish", "opaque program bytes\n")
    os.chmod(draft, 0o755)  # noqa: S103 — POSIX infer_kind classifies +x as exe
    result = runner.invoke(cli.app, ["add", str(draft), "-n", "b1", "--no-input"])
    assert result.exit_code == 2, result.output
    flat = _flat(result.output)
    assert _DRAFT_HEAD in flat  # still names the drafts boundary
    assert "pass --kind <language> to name its language" in flat  # the inferred-route variant
    assert _DRAFT_DROP not in flat  # NOT the flag-route message — nothing was passed to drop
    assert draft.exists()
    with pytest.raises(store.NotFoundError):
        store.resolve("b1")


def test_ref_flag_on_a_kept_draft_still_refused_with_the_widened_message(tmp_path):
    """--ref keeps refusing (the round-8→10 contract), now under the widened message that
    also names --exe."""
    draft = _draft("skit-new-linkme.py", "print('link me')\n")
    result = runner.invoke(cli.app, ["add", str(draft), "-n", "lk", "--ref", "--no-input"])
    assert result.exit_code == 2, result.output
    flat = _flat(result.output)
    assert _DRAFT_HEAD in flat
    assert _DRAFT_DROP in flat
    assert draft.exists()


def test_a_normal_draft_resume_still_adds_as_a_copy(tmp_path):
    """The complement: a draft added with NO exe/ref flag resumes normally (copy, consumed
    on success) — the guard fires only for the two forbidden shapes."""
    draft = _draft("skit-new-ok.py", "print('ok')\n")
    result = runner.invoke(cli.app, ["add", str(draft), "-n", "okentry", "--no-input"])
    assert result.exit_code == 0, result.output
    assert store.resolve("okentry").meta.mode == "copy"
    assert not draft.exists()  # consumed on success


# ==========================================================================
# 5. --dep / --python validated BEFORE the pipe is read or a draft materializes
# ==========================================================================


def test_stdin_garbage_python_exits_2_and_leaves_the_drafts_dir_empty(tmp_path):
    result = runner.invoke(
        cli.app, ["add", "-", "-n", "x", "--python", "garbage"], input="print(1)\n"
    )
    assert result.exit_code == 2, result.output
    assert "isn't a Python version constraint" in _flat(result.output)
    assert _drafts_files() == []  # refused before mkstemp — no kept-draft fingerprint
    with pytest.raises(store.NotFoundError):
        store.resolve("x")


def test_stdin_garbage_dep_exits_2_and_leaves_the_drafts_dir_empty(tmp_path):
    result = runner.invoke(cli.app, ["add", "-", "-n", "y", "--dep", "@@@"], input="print(1)\n")
    assert result.exit_code == 2, result.output
    assert "isn't a package requirement" in _flat(result.output)
    assert _drafts_files() == []


def test_stdin_dash_python_is_automatic(tmp_path):
    """'-' at --python means automatic: the add succeeds and the stored block carries no
    requires-python."""
    result = runner.invoke(cli.app, ["add", "-", "-n", "auto", "--python", "-"], input="print(1)\n")
    assert result.exit_code == 0, result.output
    stored = (store.resolve("auto").dir / "script.py").read_text(encoding="utf-8")
    assert "requires-python" not in stored


def test_stdin_valid_python_lands_in_the_stored_block(tmp_path):
    result = runner.invoke(
        cli.app, ["add", "-", "-n", "pinned", "--python", ">=3.11"], input="print(1)\n"
    )
    assert result.exit_code == 0, result.output
    stored = (store.resolve("pinned").dir / "script.py").read_text(encoding="utf-8")
    assert 'requires-python = ">=3.11"' in stored


def test_editor_lane_refuses_bad_python_before_opening_the_editor(monkeypatch):
    """The editor lane validates BEFORE the editor opens (the name-conflict precedent): a
    bad --python is refused and open_in_editor is never called (no authoring session cost)."""
    monkeypatch.setattr(cli, "_is_interactive", lambda: True)
    opened: list[Path] = []
    monkeypatch.setattr(cli.editor, "open_in_editor", opened.append)
    with pytest.raises(cli.typer.Exit) as exc:
        cli._create_python_in_editor("edX", python_opt="garbage")
    assert exc.value.exit_code == cli.EXIT_USAGE
    assert opened == []  # the editor never opened
    assert _drafts_files() == []  # and no draft was materialized


def test_editor_lane_refuses_bad_dep_before_opening_the_editor(monkeypatch):
    monkeypatch.setattr(cli, "_is_interactive", lambda: True)
    opened: list[Path] = []
    monkeypatch.setattr(cli.editor, "open_in_editor", opened.append)
    with pytest.raises(cli.typer.Exit) as exc:
        cli._create_python_in_editor("edY", deps_opt=["@@@"])
    assert exc.value.exit_code == cli.EXIT_USAGE
    assert opened == []


# ==========================================================================
# 6. kind_for_draft: the exception keys on the rationale (placeholder-bodied kinds)
# ==========================================================================


def test_kind_for_draft_single_prompt_extension_outranks_the_shebang():
    """A `.prompt` (single-extension) draft whose body opens with a #! resumes as a PROMPT:
    the exception is keyed on placeholder_params, not on compound-suffix shape."""
    draft = _draft("skit-new-note.prompt", "#!/usr/bin/env bash\nSummarize {{text}}.\n")
    assert registry.kind_for_draft(draft) == "prompt"


def test_kind_for_draft_extensionless_falls_through_to_the_shebang():
    """No registered extension at all → by_ext is None → the shebang decides (here: shell)."""
    draft = _draft("skit-new-plain", "#!/usr/bin/env bash\necho hi\n")
    assert registry.kind_for_draft(draft) == "shell"


def test_kind_for_draft_script_suffix_stays_shebang_first():
    """A `.py` script suffix is NOT placeholder-bodied, so the shebang still outranks it."""
    draft = _draft("skit-new-shellish.py", "#!/usr/bin/env bash\necho drafted\n")
    assert registry.kind_for_draft(draft) == "shell"


def test_prompt_single_extension_draft_resumes_as_prompt_end_to_end(tmp_path):
    """The CLI face of the single-extension prompt rule: the draft resumes as a prompt entry
    and is consumed on success."""
    draft = _draft("skit-new-summ.prompt", "#!/usr/bin/env bash\nSummarize {{text}}.\n")
    result = runner.invoke(cli.app, ["add", str(draft), "-n", "psumm", "--no-input"])
    assert result.exit_code == 0, result.output
    assert store.resolve("psumm").meta.kind == "prompt"
    assert not draft.exists()  # consumed on success


# ==========================================================================
# 7. The unknown+shebang refusal: --exe for an on-disk file, --kind-only for a draft
# ==========================================================================


def test_nondraft_awk_shebang_refusal_offers_the_exe_escape(tmp_path):
    f = tmp_path / "report.awkish"
    f.write_text("#!/usr/bin/awk -f\nBEGIN { print 1 }\n", encoding="utf-8")
    result = runner.invoke(cli.app, ["add", str(f), "-n", "rep", "--no-input"])
    assert result.exit_code == 2, result.output
    flat = _flat(result.output)
    assert "names no interpreter skit knows" in flat
    assert "--exe to run it directly" in flat  # an on-disk file gets the program escape


def test_kept_draft_awk_shebang_refusal_offers_only_kind(tmp_path):
    """The same awk shebang, but as a KEPT DRAFT: --exe is refused at the boundary, so the
    hint must NOT offer it — only --kind."""
    draft = _draft("skit-new-report.py", "#!/usr/bin/awk -f\nBEGIN { print 1 }\n")
    result = runner.invoke(cli.app, ["add", str(draft), "-n", "repd", "--no-input"])
    assert result.exit_code == 2, result.output
    flat = _flat(result.output)
    assert "names no interpreter skit knows" in flat
    assert "--kind <language> to choose one" in flat
    assert "--exe" not in flat  # the draft variant never offers the program escape
    assert draft.exists()
