"""Add feedback contracts (exit codes, stored PEP 723 text,
prompt-ask labels, filesystem state, notice counts).

Every assertion pins an observable add-feedback contract:
  * --ref on skit's OWN kept draft is REFUSED (exit 2, draft kept, no entry) — a reference
    entry into drafts/ would leave a live entry's file listed as a resumable draft;
  * an unknown-kind refusal is shebang-AWARE: a file with a #! is "names no interpreter",
    a shebang-less one keeps the "isn't a script or an executable" voice;
  * the interactive python ask label tells the truth about Enter — with a #! pin as the
    default, "Enter accepts the #! pin" (and '-' means automatic), not "leave empty";
  * a micro-versioned shebang (python3.12.1) keeps its .1 in the recorded requires-python;
  * a .prompt.md kept draft whose body opens with a #! resumes as a PROMPT (the compound
    suffix is the user's lane choice; the prompt lanes never read the shebang);
  * the extra-arguments field is named exactly ONCE — the argv hint yields to the reader
    notice when a framework was detected;
  * the $0 self-location warning is true in both storage modes (manual ${NAME:-value} rewrite,
    --normalize named as the shortcut ON A STORED COPY).

These never chdir and never touch the real user dirs (the local SKIT_* fixture).
"""

from __future__ import annotations

from pathlib import Path

import pytest
from typer.testing import CliRunner

from skit import analysis, cli, i18n, pep723, store
from skit.langs.registry import python_version_pin
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
    monkeypatch.setattr("sys.stdout.isatty", lambda: True, raising=False)
    monkeypatch.setattr(cli, "_is_interactive", lambda: True)


def _flat(text: str) -> str:
    return " ".join(text.split())


def _draft(name: str, body: str) -> Path:
    drafts_dir().mkdir(parents=True, exist_ok=True)
    p = drafts_dir() / name
    p.write_text(body, encoding="utf-8")
    return p


def _capture_ask(
    monkeypatch, answers: list[str]
) -> list[tuple[tuple[object, ...], dict[str, object]]]:
    """Stub cli.Prompt.ask, capturing every call's (args, kwargs) in order."""
    it = iter(answers)
    calls: list[tuple[tuple[object, ...], dict[str, object]]] = []

    def fake(*a: object, **kw: object) -> object:
        calls.append((a, kw))
        return next(it)

    monkeypatch.setattr(cli.Prompt, "ask", fake)
    return calls


# ==========================================================================
# 1. --ref on a kept draft is refused
# ==========================================================================


def test_ref_on_kept_draft_is_refused_and_keeps_it(tmp_path):
    """--ref into drafts/ would leave a live entry's file resumable — refuse it: exit 2, the
    'kept drafts' message naming Drop --ref, the draft kept, and NO entry created."""
    draft = _draft("skit-new-linkme.py", "print('link me')\n")
    assert is_draft(draft)  # the precondition the refusal keys on
    result = runner.invoke(cli.app, ["add", str(draft), "-n", "linky", "--ref", "--no-input"])
    assert result.exit_code == 2, result.output
    flat = _flat(result.output)
    assert "one of skit's own kept drafts" in flat
    assert "Drop --ref" in flat
    assert draft.exists()  # a refused add consumes nothing
    with pytest.raises(store.NotFoundError):
        store.resolve("linky")


def test_ref_on_a_normal_file_still_works(tmp_path):
    """The refusal is scoped to drafts: --ref on a user's OWN file is untouched (a reference
    entry that points at a real path is exactly what --ref is for)."""
    src = tmp_path / "mine.py"
    src.write_text("print('mine')\n", encoding="utf-8")
    result = runner.invoke(cli.app, ["add", str(src), "-n", "mine", "--ref", "--no-input"])
    assert result.exit_code == 0, result.output
    assert store.resolve("mine").meta.mode == "reference"
    assert src.exists()


# ==========================================================================
# 2. Prompt draft with a #! body resumes as a PROMPT (compound suffix outranks shebang)
# ==========================================================================


def test_prompt_draft_with_shebang_body_resumes_as_prompt(tmp_path):
    """A `skit-new-*.prompt.md` draft whose body opens `#!/usr/bin/env bash` resumes as a
    PROMPT, not shell — the .prompt.md suffix is the user's lane choice, and a prompt body may
    legitimately quote a shebang line. The consumed draft is unlinked on success."""
    draft = _draft("skit-new-summ.prompt.md", "#!/usr/bin/env bash\nSummarize {{text}}.\n")
    result = runner.invoke(cli.app, ["add", str(draft), "-n", "summ", "--no-input"])
    assert result.exit_code == 0, result.output
    assert store.resolve("summ").meta.kind == "prompt"  # compound suffix wins over the shebang
    assert not draft.exists()  # consumed on success


def test_py_draft_with_shebang_body_still_resumes_as_shell(tmp_path):
    """The complement / regression pin: a SCRIPT-starter `.py` draft is still shebang-first,
    so a bash body resumes as shell (only the compound prompt suffix outranks the shebang)."""
    draft = _draft("skit-new-shellish.py", "#!/usr/bin/env bash\necho drafted\n")
    result = runner.invoke(cli.app, ["add", str(draft), "-n", "shellish", "--no-input"])
    assert result.exit_code == 0, result.output
    assert store.resolve("shellish").meta.kind == "shell"
    assert not draft.exists()


# ==========================================================================
# 3. The interactive python ask label is honest about what Enter does
# ==========================================================================

_PIN_TEXT = "#!/usr/bin/env python3.12\nimport requests\nprint(requests)\n"
_NOPIN_TEXT = "#!/usr/bin/env python3\nimport requests\nprint(requests)\n"


def test_python_ask_label_names_the_pin_and_enter_records_it(tty, monkeypatch):
    """With a #! pin as the default, the label reads 'Enter accepts the #! pin' (never the
    'leave empty' lie), and returning the pin (Enter) records it."""
    calls = _capture_ask(monkeypatch, ["-", ">=3.12,<3.13"])  # deps '-' (none), then Enter=pin
    deps, py = cli._resolve_python_metadata(_PIN_TEXT, None, None, no_input=False)
    assert deps == []
    label = str(calls[1][0][0])  # the python ask is the 2nd call
    assert "Enter accepts the #! pin" in label
    assert "leave empty" not in label
    assert py == ">=3.12,<3.13"  # Enter recorded the pin


def test_python_ask_dash_records_automatic_even_with_a_pin(tty, monkeypatch):
    """'-' at the pin-aware ask really means automatic — an empty requires-python, not the pin."""
    _capture_ask(monkeypatch, ["-", "-"])  # deps none, python '-' -> automatic
    _deps, py = cli._resolve_python_metadata(_PIN_TEXT, None, None, no_input=False)
    assert py == ""


def test_python_ask_label_is_leave_empty_without_a_pin(tty, monkeypatch):
    """No #! pin: the label keeps the original 'leave empty for automatic' voice, and '-'
    there is automatic too."""
    calls = _capture_ask(monkeypatch, ["-", "-"])
    _deps, py = cli._resolve_python_metadata(_NOPIN_TEXT, None, None, no_input=False)
    label = str(calls[1][0][0])
    assert "leave empty for automatic" in label
    assert "Enter accepts the #! pin" not in label
    assert py == ""


# ==========================================================================
# 4. A micro-versioned shebang keeps its .1 in the stored PEP 723 block
# ==========================================================================


def test_micro_version_pin_unit():
    assert python_version_pin("python3.12.1") == ">=3.12.1,<3.13"
    assert python_version_pin("python3.12.1.7") == ">=3.12.1.7,<3.13"  # every micro group kept


def test_micro_versioned_shebang_lands_in_stored_pep723(tmp_path):
    """`#!/usr/bin/env python3.12.1` records requires-python `>=3.12.1,<3.13` in the stored
    copy's PEP 723 block AND announces the pin (a value recorded on a no-ask path is said aloud)."""
    p = tmp_path / "mv.py"
    p.write_text("#!/usr/bin/env python3.12.1\nprint(1)\n", encoding="utf-8")
    result = runner.invoke(cli.app, ["add", str(p), "-n", "mv", "--no-input"])
    assert result.exit_code == 0, result.output
    assert "requires-python >=3.12.1,<3.13" in _flat(result.output)  # announced
    stored = (store.resolve("mv").dir / "script.py").read_text(encoding="utf-8")
    assert 'requires-python = ">=3.12.1,<3.13"' in stored  # and landed in the block


# ==========================================================================
# 5. The unknown-kind refusal is shebang-aware
# ==========================================================================


def test_shebangless_unknown_uses_the_isnt_a_script_voice(tmp_path):
    """A shebang-LESS unknown file keeps the original 'isn't a script or an executable' message
    (the registered-shebang complement has its own test)."""
    f = tmp_path / "mystery"
    f.write_text("just some text, no shebang\n", encoding="utf-8")
    result = runner.invoke(cli.app, ["add", str(f), "-n", "mys", "--no-input"])
    assert result.exit_code == 2, result.output
    flat = _flat(result.output)
    assert "isn't a script or an executable" in flat
    assert "names no interpreter" not in flat  # the shebang voice is not used here


def test_shebang_unknown_uses_the_names_no_interpreter_voice(tmp_path):
    """A file WITH an unregistered #! (awk) gets the shebang-aware 'names no interpreter' voice
    even OUTSIDE drafts (path lane, not just resume), naming --kind."""
    f = tmp_path / "report.tricky"
    f.write_text("#!/usr/bin/awk -f\nBEGIN{print 1}\n", encoding="utf-8")
    result = runner.invoke(cli.app, ["add", str(f), "-n", "rep", "--no-input"])
    assert result.exit_code == 2, result.output
    flat = _flat(result.output)
    assert "The #! in report.tricky names no interpreter skit knows" in flat
    assert "--kind" in flat
    assert "isn't a script or an executable" not in flat


# ==========================================================================
# 6. The extra-arguments field is named exactly once (argv hint yields to reader notice)
#    (The $0 self-location warning rewrite is pinned in test_shell_inject.py.)
# ==========================================================================


def test_add_hints_suppresses_argv_when_a_framework_was_detected(capsys):
    """_print_add_hints yields the argv line when a framework was detected (frameworks -> the
    uses_cli_framework property) — the reader notice already named the extra-arguments field;
    the same fact twice reads as two facts."""
    cli._print_add_hints(analysis.Analysis(uses_argv=True, frameworks=["argparse"]), "tool")
    out = _flat(capsys.readouterr().out)
    assert out == ""  # nothing printed: the argv hint yielded


def test_add_hints_prints_argv_when_no_framework(capsys):
    """The complement (unchanged branch): no framework -> the argv hint DOES print."""
    cli._print_add_hints(analysis.Analysis(uses_argv=True, frameworks=[]), "tool")
    out = _flat(capsys.readouterr().out)
    assert "reads command-line arguments" in out


def test_dynamic_optstring_with_argv_names_extra_arguments_once(tmp_path):
    """End-to-end (CLI add): a dynamic-optstring shell that ALSO reads $@ mentions the
    extra-arguments field exactly ONCE — the reader notice, not doubled by the argv hint."""
    sh = tmp_path / "dyn.sh"
    sh.write_text(
        '#!/usr/bin/env bash\nOPTS="n:v"\nwhile getopts "$OPTS" o; do :; done\necho "$@"\n',
        encoding="utf-8",
    )
    result = runner.invoke(cli.app, ["add", str(sh), "-n", "dyn", "--no-input"])
    assert result.exit_code == 0, result.output
    assert _flat(result.output).count("extra-arguments field") == 1


# ==========================================================================
# 7. A sibling local module is not recorded as a PyPI dependency
# ==========================================================================


def test_add_records_only_third_party_deps_not_sibling_modules(tmp_path):
    """End-to-end (CLI add --no-input): a script importing a SIBLING module (helpers.py next to
    it) and a genuine third-party package records only the third-party one in the stored copy's
    PEP 723 block — `_onboard_python` passes script_dir=p.parent, so `helpers` (which would
    resolve to the local file at run time) is filtered out rather than installed as an unrelated
    PyPI `helpers`."""
    (tmp_path / "helpers.py").write_text("def go():\n    return 1\n", encoding="utf-8")
    script = tmp_path / "job.py"
    script.write_text(
        "import helpers\nimport requests\nprint(helpers.go(), requests)\n", encoding="utf-8"
    )
    result = runner.invoke(cli.app, ["add", str(script), "-n", "job", "--no-input"])
    assert result.exit_code == 0, result.output
    stored = (store.resolve("job").dir / "script.py").read_text(encoding="utf-8")
    meta = pep723.parse_block(stored)
    assert meta is not None
    assert meta["dependencies"] == ["requests"]  # helpers excluded as a local sibling


def test_resolve_python_metadata_without_script_dir_does_not_filter():
    """Called WITHOUT script_dir (the default None), `_resolve_python_metadata` has no directory
    to resolve siblings against, so nothing is treated as local — pinning the default parameter
    and the contract for any caller that omits it."""
    deps, py = cli._resolve_python_metadata(
        "import helpers\nimport requests\n", None, None, no_input=True
    )
    assert deps == ["helpers", "requests"]  # unfiltered: both survive
    assert py == ""
