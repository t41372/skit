"""Round-8 design-audit fixes — real-behavior coverage (exit codes, stored meta, filesystem).

Every assertion pins an observable contract of the round-7 fixes:
  * add's lane SELECTORS are mutually exclusive (one-voice refusal, nothing added);
  * versioned-python shebangs are the registry's rule on every lane;
  * --runner is validated BEFORE any editor opens or a draft materializes;
  * the editor lanes own --no-input (--edit always refuses it; --prompt honors it under a
    pipe and refuses it in a terminal), and thread --description into the stored entry;
  * a post-editor refusal keeps the draft AND says so (the short "kept" line);
  * a resumed draft that reaches the store is unlinked (copy mode) or kept (--ref);
  * every reader-driven add lane surfaces the same "skit read your parser" notice;
  * a python argparse read view no longer advertises --manage;
  * flipping a reader-driven entry to managed params announces the trade-off.

These never chdir and never touch the real user dirs (conftest + the local SKIT_* fixture).
"""

from __future__ import annotations

import json
from pathlib import Path

import pytest
from typer.testing import CliRunner

from skit import cli, store
from skit.paths import drafts_dir

runner = CliRunner()


@pytest.fixture(autouse=True)
def tmp_store(tmp_path, monkeypatch):
    monkeypatch.setenv("SKIT_DATA_DIR", str(tmp_path / "data"))
    monkeypatch.setenv("SKIT_STATE_DIR", str(tmp_path / "state"))
    monkeypatch.setenv("SKIT_CONFIG_DIR", str(tmp_path / "config"))
    monkeypatch.setenv("SKIT_LANG", "en")


def _flat(text: str) -> str:
    """Collapse rich's soft-wrap so an 80-col-split message matches as one string."""
    return " ".join(text.split())


def _drafts() -> list[Path]:
    return list(drafts_dir().glob("skit-*"))


def _boom_editor(*_a, **_k):
    raise AssertionError("the editor must not be launched here")


def _editor_writes(monkeypatch, content: str):
    """Monkeypatch the $EDITOR hop to write `content`, recording the temp path it saw."""
    seen: dict[str, Path] = {}

    def fake(path):
        seen["path"] = path
        path.write_text(content, encoding="utf-8")
        return 0

    monkeypatch.setattr(cli.editor, "open_in_editor", fake)
    return seen


# ==========================================================================
# 1. Lane selectors are mutually exclusive
# ==========================================================================


def test_selector_collisions_are_refused_one_voice(tmp_path, monkeypatch):
    """--cmd / --edit / stdin('-') / a file path each pick a DIFFERENT add lane; any pair is
    a usage error with the single 'each pick a different way to add' voice, BEFORE the flag
    matrix or any editor. Nothing is added and skit's drafts home is never touched."""
    # The editor must never open for the --edit collisions (the refusal precedes dispatch).
    monkeypatch.setattr(cli.editor, "open_in_editor", _boom_editor)
    monkeypatch.setattr(cli, "_is_interactive", lambda: True)
    real = tmp_path / "real.py"
    real.write_text("print(1)\n", encoding="utf-8")
    cases = [
        (["add", str(real), "--cmd", "echo {x}"], "a file path"),
        (["add", "-", "--cmd", "echo {x}"], "stdin ('-')"),
        (["add", "--edit", str(real)], "--edit"),
        (["add", "--edit", "-"], "--edit"),
    ]
    for argv, needle in cases:
        result = runner.invoke(cli.app, argv, input="print(1)\n")
        assert result.exit_code == 2, (argv, result.output)
        flat = _flat(result.output)
        assert "each pick a different way to add" in flat, argv
        assert needle in flat, argv  # the colliding selectors are named
        assert store.list_entries() == [], argv  # nothing landed
        assert _drafts() == [], argv  # drafts home untouched — no anonymous fingerprint
        # The AssertionError from _boom_editor would surface as result.exception.
        assert not isinstance(result.exception, AssertionError), argv


# ==========================================================================
# 2. Versioned python shebang is the registry's rule on every lane
# ==========================================================================


def test_stdin_versioned_python_shebang_lands_as_python(tmp_path):
    """`#!/usr/bin/env python3.12` piped in with no --kind is a python entry — the stdin lane
    reads the shebang through the same registry rule as the path/editor lanes."""
    result = runner.invoke(
        cli.app, ["add", "-", "-n", "v"], input="#!/usr/bin/env python3.12\nprint(1)\n"
    )
    assert result.exit_code == 0, result.output
    show = runner.invoke(cli.app, ["show", "v", "--json"])
    assert show.exit_code == 0, show.output
    assert json.loads(show.stdout)["kind"] == "python"


def test_editor_lane_versioned_python_shebang_onboards_as_python(monkeypatch):
    """A draft whose shebang names python3.12 is onboarded as python (not refused as an
    unregistered interpreter) — the versioned rule reaches the editor lane too."""
    monkeypatch.setattr(cli, "_is_interactive", lambda: True)
    _editor_writes(monkeypatch, "#!/usr/bin/env python3.12\nprint('hi')\n")
    result = runner.invoke(cli.app, ["add", "-e", "-n", "vpy"])
    assert result.exit_code == 0, result.output
    assert store.resolve("vpy").meta.kind == "python"


# ==========================================================================
# 3. --runner is validated before any editor opens or a draft materializes
# ==========================================================================


def test_stdin_prompt_bogus_runner_refused_before_any_draft(tmp_path):
    """The round-7 HIGH: a bogus --runner on the stdin prompt lane exits 2 with 'Unknown
    runner' and materializes NO draft (the old code left a silent, anonymous file behind)."""
    result = runner.invoke(
        cli.app, ["add", "-", "--prompt", "--runner", "bogus", "-n", "p"], input="x {{u}}\n"
    )
    assert result.exit_code == 2, result.output
    assert "Unknown runner" in result.output
    assert store.list_entries() == []
    assert _drafts() == []  # nothing was written to drafts/ before the refusal


def test_prompt_editor_bogus_runner_refused_before_the_editor(monkeypatch):
    """--runner names static config, so the TTY prompt-editor lane refuses it BEFORE opening
    $EDITOR — the editor is never launched (the same before-authoring rule as name conflicts)."""
    monkeypatch.setattr(cli, "_is_interactive", lambda: True)
    monkeypatch.setattr(cli.editor, "open_in_editor", _boom_editor)
    result = runner.invoke(cli.app, ["add", "--prompt", "--runner", "bogus", "-n", "p"])
    assert result.exit_code == 2, result.output
    assert "Unknown runner" in result.output
    assert not isinstance(result.exception, AssertionError)  # editor stayed shut
    assert _drafts() == []


# ==========================================================================
# 4. --no-input on the editor lanes
# ==========================================================================


def test_edit_no_input_is_refused_with_the_pipe_spelling(monkeypatch):
    """--edit opens an editor — interaction — so --no-input can't keep the never-prompt
    promise: it is refused up front, pointing at the stdin spelling. (Interactive is forced
    True to prove the no_input check fires first, not the interactivity gate.)"""
    monkeypatch.setattr(cli, "_is_interactive", lambda: True)
    monkeypatch.setattr(cli.editor, "open_in_editor", _boom_editor)
    result = runner.invoke(cli.app, ["add", "-e", "-n", "x", "--no-input"])
    assert result.exit_code == 2, result.output
    assert "skit add - -n NAME" in result.output  # the pipe spelling
    assert not isinstance(result.exception, AssertionError)


def test_prompt_editor_no_input_in_a_terminal_is_refused(monkeypatch):
    """--prompt with no path in a terminal opens an editor; --no-input there is refused with
    the prompt pipe spelling — no body can arrive from a keyboard-attached stdin."""
    monkeypatch.setattr(cli, "_is_interactive", lambda: True)
    monkeypatch.setattr(cli.editor, "open_in_editor", _boom_editor)
    result = runner.invoke(cli.app, ["add", "--prompt", "-n", "p", "--no-input"])
    assert result.exit_code == 2, result.output
    assert "skit add - --prompt -n NAME" in result.output
    assert not isinstance(result.exception, AssertionError)


def test_prompt_no_input_piped_still_adds(tmp_path):
    """The documented non-interactive route: under a pipe there is no editor, so --prompt
    --no-input reads the body from stdin and adds — this must keep working."""
    result = runner.invoke(
        cli.app, ["add", "--prompt", "-n", "pp", "--no-input"], input="Summarize {{url}}\n"
    )
    assert result.exit_code == 0, result.output
    entry = store.resolve("pp")
    assert entry.meta.kind == "prompt"
    assert entry.meta.params == ["url"]


# ==========================================================================
# 5. --description threads into the editor-lane stored entry
# ==========================================================================


def test_edit_description_flag_wins_over_python_docstring(monkeypatch):
    """A python draft with a docstring: --description is stored verbatim, not the docstring
    (the flag threads through _onboard_python to store.add_python)."""
    monkeypatch.setattr(cli, "_is_interactive", lambda: True)
    _editor_writes(monkeypatch, '"""Docstring one"""\nprint(1)\n')
    result = runner.invoke(cli.app, ["add", "-e", "-n", "dpy", "--description", "flag wins"])
    assert result.exit_code == 0, result.output
    assert store.resolve("dpy").meta.description == "flag wins"


def test_edit_description_flag_on_non_python_draft_is_stored(monkeypatch):
    """A bash-shebang draft records --description too (the drafted-kind store.add_script call
    now threads it) — the description is not a python-only field."""
    monkeypatch.setattr(cli, "_is_interactive", lambda: True)
    _editor_writes(monkeypatch, "#!/usr/bin/env bash\necho hi\n")
    result = runner.invoke(cli.app, ["add", "-e", "-n", "dsh", "--description", "shell note"])
    assert result.exit_code == 0, result.output
    entry = store.resolve("dsh")
    assert entry.meta.kind == "shell"
    assert entry.meta.description == "shell note"


# ==========================================================================
# 6. A post-editor refusal keeps the draft AND says so (short form)
# ==========================================================================


def test_edit_post_editor_refusal_keeps_draft_and_announces_short(monkeypatch):
    """--dep against a non-python draft is refused post-editor (typer.Exit). The draft is the
    user's only copy: it is kept on disk AND the SHORT 'kept at' line is printed (not the long
    'fix the problem and add it with' resumable form — this usage refusal names its own fix)."""
    monkeypatch.setattr(cli, "_is_interactive", lambda: True)
    seen = _editor_writes(monkeypatch, "#!/usr/bin/env bash\necho drafted\n")
    result = runner.invoke(cli.app, ["add", "-e", "-n", "d", "--dep", "foo"])
    try:
        assert result.exit_code == 2, result.output
        assert "python flags" in result.output  # the --dep refusal
        assert "Your draft was kept at" in result.output  # the kept announcement…
        assert "fix the problem and add it with" not in result.output  # …in its SHORT form
        assert seen["path"].exists()  # the draft survived the refusal
        with pytest.raises(store.NotFoundError):
            store.resolve("d")  # nothing added
    finally:
        seen["path"].unlink(missing_ok=True)


# ==========================================================================
# 8. Resume cleanup on the CLI path lane
# ==========================================================================


def test_path_add_of_a_drafts_home_file_unlinks_it_on_copy(tmp_path):
    """A resumed draft (a file living in skit's OWN drafts home) added in copy mode reaches
    the store, then the source is unlinked — the same 'the store holds the copy' cleanup the
    authoring lanes do. Only files under drafts home; a user's original is never touched."""
    drafts_dir().mkdir(parents=True, exist_ok=True)
    draft = drafts_dir() / "skit-new-resumeme.py"
    draft.write_text("print('resume')\n", encoding="utf-8")
    result = runner.invoke(cli.app, ["add", str(draft), "-n", "res", "--no-input"])
    assert result.exit_code == 0, result.output
    assert store.resolve("res").meta.mode == "copy"
    assert not draft.exists()  # the resumed draft was cleaned up


def test_path_add_of_a_drafts_home_file_keeps_it_on_reference(tmp_path):
    """--ref keeps the source: a reference-mode entry still points at the file, so it must not
    be unlinked even though it lives under drafts home."""
    drafts_dir().mkdir(parents=True, exist_ok=True)
    draft = drafts_dir() / "skit-new-keepme.py"
    draft.write_text("print('keep')\n", encoding="utf-8")
    result = runner.invoke(cli.app, ["add", str(draft), "-n", "kep", "--ref", "--no-input"])
    assert result.exit_code == 0, result.output
    assert store.resolve("kep").meta.mode == "reference"
    assert draft.exists()  # reference mode still points at it — kept


def test_path_add_of_a_normal_file_never_unlinks_the_original(tmp_path):
    """The cleanup is scoped to drafts home: a normal user file added in copy mode is left
    exactly where it was (skit copies it into the store, never moves it)."""
    src = tmp_path / "mine.py"
    src.write_text("print('mine')\n", encoding="utf-8")
    result = runner.invoke(cli.app, ["add", str(src), "-n", "mine", "--no-input"])
    assert result.exit_code == 0, result.output
    assert store.resolve("mine").meta.mode == "copy"
    assert src.exists()  # the user's original is untouched


# ==========================================================================
# 9. The reader notice is one voice for every add lane
# ==========================================================================


def test_shell_getopts_add_prints_the_read_notice(tmp_path):
    """A shell script whose getopts optstring skit CAN model statically prints the same
    '✓ skit read this script's own arguments' notice the python lane does (three of four
    kinds used to say nothing here)."""
    sh = tmp_path / "flags.sh"
    sh.write_text('#!/usr/bin/env bash\nwhile getopts "n:v" opt; do :; done\n', encoding="utf-8")
    result = runner.invoke(cli.app, ["add", str(sh), "-n", "flags", "--no-input"])
    assert result.exit_code == 0, result.output
    assert "skit read this script's own arguments" in result.output


def test_shell_dynamic_getopts_add_prints_the_passthrough_notice(tmp_path):
    """A DYNAMIC optstring is detected but unmodelable: the honest passthrough variant fires
    and names the framework (getopts) — not silence, not a false 'read your form'."""
    sh = tmp_path / "dyn.sh"
    sh.write_text(
        '#!/usr/bin/env bash\nOPTS="n:v"\nwhile getopts "$OPTS" opt; do :; done\n', encoding="utf-8"
    )
    result = runner.invoke(cli.app, ["add", str(sh), "-n", "dyn", "--no-input"])
    assert result.exit_code == 0, result.output
    assert "parses its own arguments" in result.output
    assert "getopts" in result.output  # the framework is named


def test_js_parseargs_add_prints_the_read_notice(tmp_path):
    """The reader notice is not shell-only: a js entry with parseArgs surfaces it too."""
    js = tmp_path / "cli.js"
    js.write_text(
        "#!/usr/bin/env node\n"
        "import { parseArgs } from 'node:util'\n"
        "const { values } = parseArgs({ options: { name: { type: 'string' } } })\n"
        "console.log(values)\n",
        encoding="utf-8",
    )
    result = runner.invoke(cli.app, ["add", str(js), "-n", "jscli", "--no-input"])
    assert result.exit_code == 0, result.output
    assert "skit read this script's own arguments" in result.output


# ==========================================================================
# 10. A python argparse read view no longer advertises --manage
# ==========================================================================


def _py(tmp_path: Path, text: str, name: str) -> Path:
    p = tmp_path / f"{name}.py"
    p.write_text(text, encoding="utf-8")
    return p


def test_params_python_argparse_read_view_is_plain(tmp_path):
    """A python entry that parses its own arguments is reader-driven like every kind: its
    parser IS the run form, so the read view says the plain 'no managed parameters.' and does
    NOT advertise --manage; --json reports unmanaged == [] (no candidate offered)."""
    text = (
        "import argparse\nOUT = 'hi'\n"
        "p = argparse.ArgumentParser()\np.add_argument('--n')\np.parse_args()\nprint(OUT)\n"
    )
    store.add_python(_py(tmp_path, text, "ap"), name="ap")
    plain = runner.invoke(cli.app, ["params", "ap"])
    assert plain.exit_code == 0, plain.output
    assert "has no managed parameters." in plain.output
    assert "--manage" not in plain.output  # reader-driven: --manage would shadow argparse
    js = runner.invoke(cli.app, ["params", "ap", "--json"])
    assert js.exit_code == 0, js.output
    assert json.loads(js.stdout)["unmanaged"] == []


def test_params_python_constants_only_still_offers_manage(tmp_path):
    """The gate is scoped to reader-driven entries: a constants-only python (no argparse) is
    NOT reader-driven, so it keeps advertising --manage and lists the detected candidate."""
    store.add_python(_py(tmp_path, "OUT = 'hi'\nprint(OUT)\n", "co"), name="co")
    result = runner.invoke(cli.app, ["params", "co"])
    assert result.exit_code == 0, result.output
    assert "--manage" in result.output  # a bare-constant python still offers management
    js = runner.invoke(cli.app, ["params", "co", "--json"])
    assert json.loads(js.stdout)["unmanaged"] == ["OUT"]


# ==========================================================================
# 11. Flipping a reader-driven entry to managed params announces the trade-off
# ==========================================================================


def test_manage_flip_note_names_the_reader_form_then_stays_quiet(tmp_path):
    """A getopts shell entry that ALSO holds a constant: the first `--manage CONST` prints the
    flip note naming getopts (managed params REPLACE the reader form). A second --manage on the
    now-managed entry does NOT reprint it (it was reader-driven only before the first flip)."""
    sh = tmp_path / "both.sh"
    sh.write_text(
        '#!/usr/bin/env bash\nCITY=Taipei\nwhile getopts "n:v" opt; do :; done\necho $CITY\n',
        encoding="utf-8",
    )
    runner.invoke(cli.app, ["add", str(sh), "-n", "both", "--no-input"])
    first = runner.invoke(cli.app, ["params", "both", "--manage", "CITY"])
    assert first.exit_code == 0, first.output
    assert "The run form now asks for the managed parameters" in first.output
    assert "getopts" in first.output  # the reader form set aside is named

    # A constant is already managed now → the entry is no longer reader-driven-only, so a
    # second manage prints no flip note.
    sh2 = tmp_path / "second.sh"
    sh2.write_text(
        '#!/usr/bin/env bash\nCITY=Taipei\nPORT=8080\nwhile getopts "n:v" opt; do :; done\n'
        "echo $CITY $PORT\n",
        encoding="utf-8",
    )
    runner.invoke(cli.app, ["add", str(sh2), "-n", "second", "--no-input"])
    runner.invoke(cli.app, ["params", "second", "--manage", "CITY"])  # first flip (has the note)
    again = runner.invoke(cli.app, ["params", "second", "--manage", "PORT"])
    assert again.exit_code == 0, again.output
    assert "The run form now asks for the managed parameters" not in again.output


def test_manage_flip_json_stdout_is_exactly_one_document(tmp_path):
    """Under --json the flip note is silent (the maybe-quiet console) and stdout is EXACTLY one
    JSON document — the note must never leak a human line onto the machine contract."""
    sh = tmp_path / "j.sh"
    sh.write_text(
        '#!/usr/bin/env bash\nCITY=Taipei\nwhile getopts "n:v" opt; do :; done\necho $CITY\n',
        encoding="utf-8",
    )
    runner.invoke(cli.app, ["add", str(sh), "-n", "jflip", "--no-input"])
    result = runner.invoke(cli.app, ["params", "jflip", "--manage", "CITY", "--json"])
    assert result.exit_code == 0, result.output
    payload = json.loads(result.stdout)  # parses whole stdout — one document, no leaked line
    assert "CITY" in [p["name"] for p in payload["params"]]
    assert "The run form now asks" not in result.stdout
