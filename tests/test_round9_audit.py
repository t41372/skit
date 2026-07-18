"""Round-9 design-audit fixes — real-behavior coverage (exit codes, stored PEP 723 text,
--json, filesystem state).

Every assertion pins an observable contract of the round-8 fixes verified this round:
  * skit's OWN kept drafts (paths.is_draft) are classified shebang-first, so a bash-shebang
    draft named `skit-new-*.py` resumes as SHELL, an awk one as "unknown" (--kind escape);
  * is_draft needs BOTH halves — the drafts dir AND the `skit-` prefix — so a user's file
    merely parked in the drafts dir is never consumed, and the draft rule never leaks to a
    real `.py` outside the drafts dir;
  * python2 is unregistered (uv runs scripts on python3): a python2 shebang is refused, never
    fabricated into an entry that dies at run time;
  * a versioned python shebang (python3.12) pins requires-python in the stored copy AND says
    so — explicit --python and an existing PEP 723 block both win over it, silently;
  * the manage-a-constant offer keys on flows.reader_fields (a MODELED form), not merely
    "self-parses": docopt/dynamic scripts keep the candidate offer; modeled ones suppress it;
  * reference entries get the same honest read but drop the --manage advice for the
    reference-mode teaching, and their add lane says whether a form survived.

These never chdir and never touch the real user dirs (conftest + the local SKIT_* fixture).
"""

from __future__ import annotations

import json
from pathlib import Path

import pytest
from typer.testing import CliRunner

from skit import cli, flows, store
from skit.langs.registry import (
    kind_for_draft,
    python_version_pin,
    spec_for,
)
from skit.paths import drafts_dir, is_draft

runner = CliRunner()


@pytest.fixture(autouse=True)
def tmp_store(tmp_path, monkeypatch):
    monkeypatch.setenv("SKIT_DATA_DIR", str(tmp_path / "data"))
    monkeypatch.setenv("SKIT_STATE_DIR", str(tmp_path / "state"))
    monkeypatch.setenv("SKIT_CONFIG_DIR", str(tmp_path / "config"))
    monkeypatch.setenv("SKIT_LANG", "en")


def _flat(text: str) -> str:
    return " ".join(text.split())


def _draft(name: str, body: str) -> Path:
    drafts_dir().mkdir(parents=True, exist_ok=True)
    p = drafts_dir() / name
    p.write_text(body, encoding="utf-8")
    return p


def _stored(name: str) -> str:
    return (store.resolve(name).dir / "script.py").read_text(encoding="utf-8")


# ==========================================================================
# 1. python_version_pin / kind_for_draft / is_draft — the registry units
# ==========================================================================


@pytest.mark.parametrize(
    ("program", "expected"),
    [
        ("python", ""),
        ("python3", ""),
        ("python3.12", ">=3.12,<3.13"),
        ("python3.12.1", ">=3.12,<3.13"),  # a patch-versioned shebang still pins the minor
        ("python2", ""),  # unregistered — no pin
        ("python2.7", ""),
        ("bash", ""),
        ("", ""),
        (None, ""),
    ],
)
def test_python_version_pin_rows(program, expected):
    assert python_version_pin(program) == expected


def test_kind_for_draft_shebang_first(tmp_path):
    # skit's OWN drafts are classified by shebang, not the mkstemp .py suffix.
    assert kind_for_draft(_draft("skit-new-a.py", "#!/usr/bin/env bash\necho hi\n")) == "shell"
    assert (
        kind_for_draft(_draft("skit-new-b.py", "#!/usr/bin/awk -f\nBEGIN{print 1}\n")) == "unknown"
    )
    assert kind_for_draft(_draft("skit-new-c.py", "print('x')\n")) == "python"  # no shebang: suffix


def test_is_draft_needs_both_dir_and_prefix(tmp_path):
    drafts_dir().mkdir(parents=True, exist_ok=True)
    assert is_draft(drafts_dir() / "skit-new-x.py") is True
    assert is_draft(drafts_dir() / "mytool.sh") is False  # parked, no skit- prefix
    assert is_draft(tmp_path / "skit-new-x.py") is False  # skit- prefix but not in drafts dir


def test_reader_fields_predicate_rows():
    py = spec_for("python")
    sh = spec_for("shell")
    docopt = '"""Usage: x --city=<c>"""\nimport docopt\nprint(docopt.docopt(__doc__))\n'
    modeled = (
        "import argparse\np=argparse.ArgumentParser()\np.add_argument('--n')\np.parse_args()\n"
    )
    getopts2 = '#!/usr/bin/env bash\nwhile getopts "n:v" o; do :; done\n'
    dyn = '#!/usr/bin/env bash\nOPTS="n:v"\nwhile getopts "$OPTS" o; do :; done\n'
    assert flows.reader_fields(py, docopt) == 0  # self-parses but skit can't model it
    assert flows.reader_fields(py, modeled) == 1  # one add_argument -> one modeled field
    assert flows.reader_fields(sh, getopts2) == 2
    assert flows.reader_fields(sh, dyn) == 0  # dynamic optstring: ok=False -> 0
    assert flows.reader_fields(None, modeled) == 0  # no spec
    assert flows.reader_fields(py, "") == 0  # no text


# ==========================================================================
# 2. Draft resume reclassification on the CLI path lane (the round-8 HIGH)
# ==========================================================================


def test_cli_add_bash_shebang_draft_lands_as_shell_and_unlinks(tmp_path):
    """A bash-shebang draft named `skit-new-*.py` (mkstemp's suffix, not a user signal)
    resumes as a SHELL entry — never a broken python entry with a bash body — and the
    consumed draft is unlinked."""
    draft = _draft("skit-new-ship.py", "#!/usr/bin/env bash\necho drafted\n")
    result = runner.invoke(cli.app, ["add", str(draft), "-n", "ship", "--no-input"])
    assert result.exit_code == 0, result.output
    assert store.resolve("ship").meta.kind == "shell"  # reclassified by shebang
    assert not draft.exists()  # consumed on success


def test_cli_add_awk_shebang_draft_is_unknown_kept_with_kind_escape(tmp_path):
    """An awk shebang is unregistered: the draft is "unknown", refused with exit 2 and the
    --kind escape (never a fabricated entry), and KEPT because the add never reached the
    consume-on-success unlink."""
    draft = _draft("skit-new-awk.py", "#!/usr/bin/awk -f\nBEGIN{print 1}\n")
    result = runner.invoke(cli.app, ["add", str(draft), "-n", "awky", "--no-input"])
    assert result.exit_code == 2, result.output
    assert "--kind" in result.output  # the escape is named
    assert "isn't a script or an executable" in _flat(result.output)
    assert draft.exists()  # a refused add consumes nothing
    with pytest.raises(store.NotFoundError):
        store.resolve("awky")


def test_cli_add_no_shebang_draft_falls_back_to_python(tmp_path):
    """No shebang at all: the suffix is all there is, so a `skit-new-*.py` draft is python
    (the fallback branch of kind_for_draft)."""
    draft = _draft("skit-new-plain.py", "print('resume me')\n")
    result = runner.invoke(cli.app, ["add", str(draft), "-n", "plain", "--no-input"])
    assert result.exit_code == 0, result.output
    assert store.resolve("plain").meta.kind == "python"
    assert not draft.exists()


def test_cli_add_bash_shebang_py_outside_drafts_stays_python(tmp_path):
    """The draft rule must NOT leak: a real `.py` file with a bash shebang living OUTSIDE the
    drafts dir classifies by its extension (python) — only skit's own drafts are shebang-first,
    and a user's original is never unlinked."""
    src = tmp_path / "thing.py"
    src.write_text("#!/usr/bin/env bash\necho hi\n", encoding="utf-8")
    result = runner.invoke(cli.app, ["add", str(src), "-n", "thing", "--no-input"])
    assert result.exit_code == 0, result.output
    assert store.resolve("thing").meta.kind == "python"  # extension wins outside drafts
    assert src.exists()  # not a draft: the original is left alone


# ==========================================================================
# 3. is_draft scoping on the path-lane unlink
# ==========================================================================


def test_cli_add_parked_user_file_in_drafts_dir_is_not_unlinked(tmp_path):
    """A user file merely PARKED in the drafts dir (no `skit-` prefix) is added but NOT
    consumed — is_draft needs both halves, so this file is not skit's artifact to delete."""
    parked = _draft("mytool.sh", "#!/usr/bin/env bash\necho hi\n")
    result = runner.invoke(cli.app, ["add", str(parked), "-n", "parked", "--no-input"])
    assert result.exit_code == 0, result.output
    assert store.resolve("parked").meta.kind == "shell"
    assert parked.exists()  # no skit- prefix -> not consumed


# ==========================================================================
# 4. python2 is unregistered on every lane
# ==========================================================================


def test_stdin_python2_shebang_is_refused(tmp_path):
    """`#!/usr/bin/env python2` piped in with no --kind is refused (skit runs scripts through
    uv on python3 — a python2 entry could only die at run time)."""
    result = runner.invoke(
        cli.app, ["add", "-", "-n", "p2"], input="#!/usr/bin/env python2\nprint(1)\n"
    )
    assert result.exit_code == 2, result.output
    assert "names no interpreter" in result.output
    with pytest.raises(store.NotFoundError):
        store.resolve("p2")


def test_path_add_python2_extensionless_is_refused(tmp_path):
    """The same rule on the path lane: an extensionless python2-shebang file is not a python
    entry — the --kind escape applies."""
    src = tmp_path / "legacy"
    src.write_text("#!/usr/bin/env python2\nprint(1)\n", encoding="utf-8")
    result = runner.invoke(cli.app, ["add", str(src), "-n", "legacy", "--no-input"])
    assert result.exit_code == 2, result.output
    assert "--kind" in result.output


# ==========================================================================
# 5. A versioned python shebang pins requires-python (and both overrides win silently)
# ==========================================================================


def test_stdin_versioned_shebang_pins_requires_python_and_announces(tmp_path):
    """python3.12 with no --python and no PEP 723 block records requires-python
    ">=3.12,<3.13" into the STORED copy's PEP 723 block, and says so on a path with no ask."""
    result = runner.invoke(
        cli.app, ["add", "-", "-n", "v"], input="#!/usr/bin/env python3.12\nprint(1)\n"
    )
    assert result.exit_code == 0, result.output
    assert "recording requires-python >=3.12,<3.13" in _flat(result.output)  # the dim note
    assert 'requires-python = ">=3.12,<3.13"' in _stored("v")  # landed in the stored copy


def test_explicit_python_beats_the_shebang_pin_silently(tmp_path):
    """An explicit --python is the user's own move: it wins over the shebang pin and prints
    NO note (nothing was recorded without an ask)."""
    result = runner.invoke(
        cli.app,
        ["add", "-", "-n", "vo", "--python", ">=3.11"],
        input="#!/usr/bin/env python3.12\nprint(1)\n",
    )
    assert result.exit_code == 0, result.output
    assert "recording requires-python" not in result.output  # no note on the explicit path
    assert 'requires-python = ">=3.11"' in _stored("vo")
    assert ">=3.12,<3.13" not in _stored("vo")  # the shebang pin did NOT override --python


def test_existing_pep723_block_beats_the_shebang_pin_silently(tmp_path):
    """An existing PEP 723 block already owns the constraint: the shebang pin is dropped, no
    note, the block's own requires-python is preserved verbatim."""
    body = "#!/usr/bin/env python3.12\n# /// script\n# requires-python = '>=3.9'\n# ///\nprint(1)\n"
    result = runner.invoke(cli.app, ["add", "-", "-n", "vb"], input=body)
    assert result.exit_code == 0, result.output
    assert "recording requires-python" not in result.output
    text = _stored("vb")
    assert ">=3.9" in text  # the block won
    assert ">=3.12,<3.13" not in text  # the shebang pin was not injected


def test_dep_flag_present_still_pins_from_the_shebang(tmp_path):
    """--dep given but --python NOT: the shebang pin still rides in on the explicit-deps
    branch (announced) and lands alongside the dependency in the stored block."""
    result = runner.invoke(
        cli.app,
        ["add", "-", "-n", "vd", "--dep", "rich"],
        input="#!/usr/bin/env python3.12\nprint(1)\n",
    )
    assert result.exit_code == 0, result.output
    assert "recording requires-python >=3.12,<3.13" in _flat(result.output)
    text = _stored("vd")
    assert 'requires-python = ">=3.12,<3.13"' in text
    assert "rich" in text  # the dependency landed too


def test_suggested_deps_noninteractive_pins_from_the_shebang(tmp_path):
    """A script whose imports SUGGEST a dependency, piped in (non-interactive): skit accepts
    the suggestions as-is AND records + announces the shebang pin on that path."""
    result = runner.invoke(
        cli.app,
        ["add", "-", "-n", "vs"],
        input="#!/usr/bin/env python3.12\nimport requests\nprint(requests)\n",
    )
    assert result.exit_code == 0, result.output
    assert "recording requires-python >=3.12,<3.13" in _flat(result.output)
    text = _stored("vs")
    assert 'requires-python = ">=3.12,<3.13"' in text
    assert "requests" in text  # the suggested dependency was accepted non-interactively


def test_onboard_script_params_returns_empty_for_analyzerless_kind(tmp_path):
    """A data-driven-tail kind with no analyzer/params_io (ruby) has no candidate onboarding —
    the guard returns immediately, never touching the script."""
    p = tmp_path / "task.rb"
    p.write_text("puts 'hi'\n", encoding="utf-8")
    entry = store.add_script(p, kind="ruby", name="task")
    ruby = spec_for("ruby")
    assert ruby is not None
    assert cli._onboard_script_params(entry, ruby, no_input=False) == []


# ==========================================================================
# 6. Modeled-form predicate: docopt/dynamic keep the manage offer; no false flip note
# ==========================================================================


def _add(text: str, name: str, kind: str, ref: bool = False) -> None:
    p = drafts_dir().parent / f"{name}.src"
    p.parent.mkdir(parents=True, exist_ok=True)
    p.write_text(text, encoding="utf-8")
    if kind == "python":
        store.add_python(p, name=name, mode="reference" if ref else "copy")
    else:
        store.add_script(p, kind=kind, name=name, mode="reference" if ref else "copy")


DOCOPT = (
    '"""Usage: dc --city=<c>"""\nimport docopt\nCITY = "x"\nprint(docopt.docopt(__doc__), CITY)\n'
)
DYN_SH = '#!/usr/bin/env bash\nOUTDIR=/tmp\nOPTS="n:v"\nwhile getopts "$OPTS" o; do :; done\necho $OUTDIR\n'


def test_docopt_python_read_view_offers_manage(tmp_path):
    """docopt self-parses but skit can't MODEL it: the run form is passthrough-only, so the
    read view still lists the unmanaged constant AND advertises --manage (additive, not a
    source-flip trap)."""
    _add(DOCOPT, "dc", "python")
    plain = runner.invoke(cli.app, ["params", "dc"])
    assert plain.exit_code == 0, plain.output
    assert "Detected but not yet managed: CITY" in _flat(plain.output)
    assert "--manage" in plain.output
    js = runner.invoke(cli.app, ["params", "dc", "--json"])
    assert json.loads(js.stdout)["unmanaged"] == ["CITY"]


def test_docopt_python_manage_prints_no_flip_note(tmp_path):
    """Managing that constant is NOT setting aside a modeled form (there was none), so no flip
    note fires — the note is reserved for modeled readers being replaced."""
    _add(DOCOPT, "dc", "python")
    result = runner.invoke(cli.app, ["params", "dc", "--manage", "CITY"])
    assert result.exit_code == 0, result.output
    assert "The run form now asks" not in result.output  # no false "form set aside"


def test_dynamic_getopts_read_view_offers_manage(tmp_path):
    """A dynamic optstring shell is detected but unmodelable: the read view lists candidates
    and offers --manage (the passthrough field carries the reader; constants are additive)."""
    _add(DYN_SH, "dyn", "shell")
    plain = runner.invoke(cli.app, ["params", "dyn"])
    assert plain.exit_code == 0, plain.output
    assert "--manage" in plain.output
    js = runner.invoke(cli.app, ["params", "dyn", "--json"])
    assert "OUTDIR" in json.loads(js.stdout)["unmanaged"]


def test_dynamic_getopts_manage_prints_no_flip_note(tmp_path):
    _add(DYN_SH, "dyn", "shell")
    result = runner.invoke(cli.app, ["params", "dyn", "--manage", "OUTDIR"])
    assert result.exit_code == 0, result.output
    assert "The run form now asks" not in result.output


# ==========================================================================
# 7. Reference entries: honest read, no --manage advice, add-lane voice
# ==========================================================================


def test_reference_getopts_read_view_has_no_manage_advice(tmp_path):
    """A reference getopts entry's parser IS the form (reader-driven): the read view says the
    plain 'no managed parameters.' with NO --manage advice, and its plan is reader-driven."""
    _add('#!/usr/bin/env bash\nwhile getopts "n:v" o; do :; done\n', "refg", "shell", ref=True)
    result = runner.invoke(cli.app, ["params", "refg"])
    assert result.exit_code == 0, result.output
    assert "has no managed parameters." in result.output
    assert "--manage" not in result.output
    show = runner.invoke(cli.app, ["show", "refg", "--json"])
    assert json.loads(show.stdout)["param_source"] == "argparse"  # reader-driven plan


def test_reference_constants_read_view_names_unmanaged_with_teaching(tmp_path):
    """A reference constants entry is NOT reader-driven: its unmanaged candidate is named, the
    --manage advice is dropped for the reference-mode teaching, and --json still populates
    unmanaged (the read is honest in both modes)."""
    _add("#!/usr/bin/env bash\nOUTDIR=/tmp\necho $OUTDIR\n", "refc", "shell", ref=True)
    result = runner.invoke(cli.app, ["params", "refc"])
    assert result.exit_code == 0, result.output
    assert "Detected but not yet managed: OUTDIR" in _flat(result.output)
    assert "use --manage to manage them" not in result.output  # no advice on a ref entry
    assert "skit never writes the original file" in _flat(result.output)  # the teaching
    js = runner.invoke(cli.app, ["params", "refc", "--json"])
    assert json.loads(js.stdout)["unmanaged"] == ["OUTDIR"]


def test_reference_reader_add_prints_the_read_notice(tmp_path):
    """A reference-mode add whose script models a form says so — the reader works in reference
    mode, so 'setup was skipped' alone would read as 'the form is lost' (it isn't)."""
    sh = tmp_path / "refadd.sh"
    sh.write_text('#!/usr/bin/env bash\nwhile getopts "n:v" o; do :; done\n', encoding="utf-8")
    result = runner.invoke(cli.app, ["add", str(sh), "-n", "refadd", "--ref", "--no-input"])
    assert result.exit_code == 0, result.output
    assert "skit read this script's own arguments" in result.output
    py = tmp_path / "refap.py"
    py.write_text(
        "import argparse\np=argparse.ArgumentParser()\np.add_argument('--n')\np.parse_args()\n",
        encoding="utf-8",
    )
    r2 = runner.invoke(cli.app, ["add", str(py), "-n", "refap", "--ref", "--no-input"])
    assert r2.exit_code == 0, r2.output
    assert "skit read this script's own arguments" in r2.output  # python reference add too


def test_reference_constants_add_prints_the_skip_line(tmp_path):
    """A reference-mode add of a script with NO modeled form prints the plain 'setup was
    skipped' line (there is no form to reassure about)."""
    sh = tmp_path / "refcadd.sh"
    sh.write_text("#!/usr/bin/env bash\nOUTDIR=/tmp\necho $OUTDIR\n", encoding="utf-8")
    result = runner.invoke(cli.app, ["add", str(sh), "-n", "refcadd", "--ref", "--no-input"])
    assert result.exit_code == 0, result.output
    assert "parameter setup was skipped" in result.output
    assert "skit read this script's own arguments" not in result.output


# ==========================================================================
# 8. Singular vs plural field count in the read notice
# ==========================================================================


def test_one_field_getopts_add_says_singular(tmp_path):
    sh = tmp_path / "one.sh"
    sh.write_text('#!/usr/bin/env bash\nwhile getopts "n:" o; do :; done\n', encoding="utf-8")
    result = runner.invoke(cli.app, ["add", str(sh), "-n", "one", "--no-input"])
    assert result.exit_code == 0, result.output
    assert "(1 field)" in result.output
    assert "(1 fields)" not in result.output


def test_multi_field_getopts_add_says_plural(tmp_path):
    sh = tmp_path / "many.sh"
    sh.write_text('#!/usr/bin/env bash\nwhile getopts "n:v" o; do :; done\n', encoding="utf-8")
    result = runner.invoke(cli.app, ["add", str(sh), "-n", "many", "--no-input"])
    assert result.exit_code == 0, result.output
    assert "(2 fields)" in result.output
