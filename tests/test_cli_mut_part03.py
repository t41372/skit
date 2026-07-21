"""Behavioural tests targeting mutation survivors in skit/cli.py (chunk 3/6).

Covers the parameter-editing helpers (`_edit_params`, `_edit_declared_params`,
`_normalize_params`) and add-time hints/onboarding (`_print_add_hints`, `_onboard_params`).
Style follows tests/test_cli.py / tests/test_cli_mut.py:
CliRunner for the non-interactive command path, direct calls for the pure helpers, exact
message text (English catalog) so string mutants can't hide, and on-disk/param assertions so
dropped or nulled keyword arguments are observable.
"""

from __future__ import annotations

import dataclasses
from pathlib import Path

import pytest
import typer
from typer.testing import CliRunner

from skit import argstate, cli, i18n, store
from skit.langs import registry
from skit.langs.base import LangSpec
from skit.langs.python import analyzer, metawriter
from skit.models import Mode
from skit.params import ParamDecl

runner = CliRunner()


@pytest.fixture(autouse=True)
def tmp_store(tmp_path: Path, monkeypatch: pytest.MonkeyPatch):
    monkeypatch.setenv("SKIT_DATA_DIR", str(tmp_path / "data"))
    monkeypatch.setenv("SKIT_STATE_DIR", str(tmp_path / "state"))
    monkeypatch.setenv("SKIT_CONFIG_DIR", str(tmp_path / "config"))
    monkeypatch.setenv("SKIT_LANG", "en")
    i18n.init("en")
    return tmp_path


def _py(tmp_path: Path, body: str, name: str = "job.py") -> Path:
    p = tmp_path / name
    p.write_text(body, encoding="utf-8")
    return p


def _shell(tmp_path: Path, body: str, *, name: str, mode: Mode = "copy") -> store.Entry:
    src = tmp_path / f"{name}.sh"
    src.write_text(body, encoding="utf-8")
    return store.add_script(src, kind="shell", name=name, mode=mode)


def _degraded_shell_spec() -> LangSpec:
    """The shell kind with its analyzer degraded to None (the A2 grammar-failure state), but
    normalizer and params_io intact — the input that separates the capability guards' `or`
    chain from its and-mutants."""
    base = registry.spec_for("shell")
    assert base is not None
    return dataclasses.replace(base, analyzer=None)


def _exe(tmp_path: Path, name: str = "prog") -> store.Entry:
    prog = tmp_path / "t"
    prog.write_text("#!/bin/sh\necho hi\n", encoding="utf-8")
    prog.chmod(0o755)
    return store.add_exe(prog, name=name)


def _norm(text: str) -> str:
    return " ".join(text.split())


# --------------------------------------------------------------------------
# _print_add_hints
# --------------------------------------------------------------------------


def test_print_add_hints_argv_and_two_filenames_exact(capsys):
    # A script that both reads argv AND embeds two filename literals: one call exercises both
    # hint branches, and two literals make the ", " name-join separator observable.
    text = "import sys\ndata = sys.argv[1]\nsave('a.png')\nsave('b.png')\n"
    result = analyzer.analyze(text)
    assert result.uses_argv is True
    assert result.filename_literals == ["a.png", "b.png"]

    cli._print_add_hints(result, "myscript")
    out = _norm(capsys.readouterr().out)

    # argv branch: exact copy (kills the lowercase and XX-wrap mutants)
    assert (
        "This script reads command-line arguments; the run form has an extra-arguments field "
        "for them." in out
    )
    # filename branch: names really are the joined reprs (kills names=None, repr(None), and the
    # "XX, XX" join), and the full sentence keeps its case (kills lowercase / XX-wrap).
    assert (
        "💡 'a.png', 'b.png' are written directly inside the code, so skit can't turn them into "
        "form fields. To manage one, first give it a name at the top of the script, e.g. "
        "OUTPUT = '…' (skit edit myscript)." in out
    )
    assert "None" not in out  # kills names=None and escape(repr(None))
    assert "XX" not in out  # kills "[dim]" -> "XX[dim]XX" and every string-wrap mutant


# --------------------------------------------------------------------------
# _onboard_params — the reader-succeeded ("✓ skit read …") branch
# --------------------------------------------------------------------------


def test_onboard_params_reader_ok_message_exact(capsys):
    # argparse that argspec can model statically: spec.ok and spec.fields -> the ✓ message with
    # the field count. This branch runs before the no_input/tty check, so no_input=True is fine.
    text = (
        "import argparse\nap = argparse.ArgumentParser()\nap.add_argument('--x')\nap.parse_args()\n"
    )
    specs = cli._onboard_params(text, "clitool", no_input=True)
    assert specs == []
    out = _norm(capsys.readouterr().out)
    assert (
        "✓ skit read this script's own arguments (1 field). Running it opens a form — "
        "nothing to memorize." in out
    )
    assert "XX" not in out


# --------------------------------------------------------------------------
# _edit_params — the now-secret purge message + the capability guard
# --------------------------------------------------------------------------


def test_edit_params_purged_secret_message_exact(tmp_path):
    # Two managed consts with remembered plaintext values, both promoted to secret in one call:
    # both get purged, so the message lists them ", "-joined and dim-wrapped.
    text = metawriter.write_params(
        "A = 'x'\nB = 'y'\nprint(A, B)\n",
        [
            ParamDecl(name="A", binding="const", type="str"),
            ParamDecl(name="B", binding="const", type="str"),
        ],
    )
    entry = store.add_python(_py(tmp_path, text), name="j")
    argstate.save_last(entry.slug, values={"A": "aval", "B": "bval"})
    result = runner.invoke(cli.app, ["params", "j", "--secret", "A", "--secret", "B"])
    assert result.exit_code == 0, result.output
    out = _norm(result.output)
    assert "Removed previously stored plaintext value(s) for now-secret parameter(s): A, B" in out
    assert "XX" not in out  # kills "[dim]" -> "XX[dim]XX", the msgid wrap, and "XX, XX".join
    # The purge really happened (the stored plaintext is gone).
    remaining = argstate.load_state(entry.slug)["values"]
    assert "A" not in remaining
    assert "B" not in remaining


def test_edit_params_degraded_shell_spec_refuses_instead_of_crashing(tmp_path, monkeypatch, capsys):
    # A grammar-degraded shell kind (params_io present, analyzer None — the A2 amendment's real
    # supported state) must be refused by the `... or analyzer is None` guard, not fall through.
    # Kills the mutant that ANDs the last two conditions (it would proceed and hit None.analyze).
    store.add_python(_py(tmp_path, "print(1)\n"), name="p")
    entry = store.resolve("p")
    degraded = _degraded_shell_spec()
    monkeypatch.setattr(cli, "spec_for", lambda _kind: degraded)
    with pytest.raises(typer.Exit) as ei:
        cli._edit_params(
            entry,
            resync=True,
            manage=[],
            unmanage=[],
            secret=[],
            no_secret=[],
            prompts={},
            env_sources={},
            malformed=[],
        )
    assert ei.value.exit_code == 1
    assert "has no managed parameters — its kind has no analyzer to read them from." in _norm(
        capsys.readouterr().err
    )


def test_edit_params_none_spec_refuses_before_attribute_access(tmp_path, monkeypatch, capsys):
    # spec_for returns None (an entry whose kind this skit doesn't know): the leading
    # `entry_spec is None` must short-circuit. The mutant that ANDs it with `params_io is None`
    # would dereference None.params_io and crash instead of refusing cleanly.
    store.add_python(_py(tmp_path, "print(1)\n"), name="p")
    entry = store.resolve("p")
    monkeypatch.setattr(cli, "spec_for", lambda _kind: None)
    with pytest.raises(typer.Exit) as ei:
        cli._edit_params(
            entry,
            resync=True,
            manage=[],
            unmanage=[],
            secret=[],
            no_secret=[],
            prompts={},
            env_sources={},
            malformed=[],
        )
    assert ei.value.exit_code == 1
    assert "has no managed parameters — its kind has no analyzer to read them from." in _norm(
        capsys.readouterr().err
    )


# --------------------------------------------------------------------------
# _edit_declared_params — allowed-delivery set, threaded kwargs, messages
# --------------------------------------------------------------------------


def test_declared_deliver_env_on_exe_takes_effect(tmp_path):
    # A bare exe add defaults to flag; --deliver W=env must move it to env. "env" is in the
    # exe kind's allowed set, and the deliveries dict must reach edit_declared. Kills the
    # "env" removed-from-allowed mutants and deliveries=None / dropped-deliveries.
    entry = _exe(tmp_path)
    result = runner.invoke(cli.app, ["params", "prog", "--add", "W", "--deliver", "W=env"])
    assert result.exit_code == 0, result.output
    (decl,) = store.read_parameters(entry.slug)
    assert decl.delivery == "env"


def test_declared_redeliver_back_to_flag_on_exe(tmp_path):
    # Start at env, then set flag: "flag" must be a member of the exe kind's allowed set, or the
    # tweak is refused (bad-delivery) and the value stays env. Kills the "flag" corruption mutants.
    entry = _exe(tmp_path)
    assert (
        runner.invoke(cli.app, ["params", "prog", "--add", "W", "--deliver", "W=env"]).exit_code
        == 0
    )
    assert store.read_parameters(entry.slug)[0].delivery == "env"
    result = runner.invoke(cli.app, ["params", "prog", "--deliver", "W=flag"])
    assert result.exit_code == 0, result.output
    assert store.read_parameters(entry.slug)[0].delivery == "flag"


def test_declared_redeliver_to_placeholder_on_command(tmp_path):
    # A template's allowed set is (env, placeholder). Move msg (a real placeholder) to env, then
    # back to placeholder: "placeholder" must stay a member or the second tweak is refused and the
    # value stays env. Kills the "placeholder" removed-from-allowed mutants.
    entry = store.add_command("echo {msg}", name="cmd")
    assert (
        runner.invoke(cli.app, ["params", "cmd", "--add", "msg", "--deliver", "msg=env"]).exit_code
        == 0
    )
    assert store.read_parameters(entry.slug)[0].delivery == "env"
    result = runner.invoke(cli.app, ["params", "cmd", "--deliver", "msg=placeholder"])
    assert result.exit_code == 0, result.output
    assert store.read_parameters(entry.slug)[0].delivery == "placeholder"


def test_declared_help_text_threaded_through(tmp_path):
    # --help-text must reach edit_declared (kills help_texts=None and the dropped kwarg).
    entry = _exe(tmp_path)
    result = runner.invoke(cli.app, ["params", "prog", "--add", "W", "--help-text", "W=How wide"])
    assert result.exit_code == 0, result.output
    assert store.read_parameters(entry.slug)[0].help == "How wide"


def test_declared_prompt_threaded_through(tmp_path):
    # --prompt must reach edit_declared (kills prompts=None and the dropped kwarg).
    entry = _exe(tmp_path)
    result = runner.invoke(cli.app, ["params", "prog", "--add", "W", "--prompt", "W=Width? "])
    assert result.exit_code == 0, result.output
    assert store.read_parameters(entry.slug)[0].prompt == "Width? "


def test_declared_required_threaded_through(tmp_path):
    # --required must reach edit_declared (kills the dropped required kwarg): the default is False.
    entry = _exe(tmp_path)
    result = runner.invoke(cli.app, ["params", "prog", "--add", "W", "--required", "W"])
    assert result.exit_code == 0, result.output
    assert store.read_parameters(entry.slug)[0].required is True


def test_declared_no_secret_threaded_through(tmp_path):
    # secret then no_secret in one pass must net to non-secret: dropping no_secret leaves it secret.
    entry = _exe(tmp_path)
    result = runner.invoke(
        cli.app, ["params", "prog", "--add", "W", "--secret", "W", "--no-secret", "W"]
    )
    assert result.exit_code == 0, result.output
    assert store.read_parameters(entry.slug)[0].secret is False


def test_declared_malformed_value_message_exact(tmp_path):
    _exe(tmp_path)
    result = runner.invoke(cli.app, ["params", "prog", "--type", "NOEQUALS"])
    assert result.exit_code == 0, result.output
    out = _norm(result.output)
    assert "Ignored a malformed value: --type: NOEQUALS (expected NAME=VALUE)." in out
    assert "XX" not in out


def test_declared_purged_secret_message_exact(tmp_path):
    # Two declared env params with remembered plaintext, both promoted to secret: the purge line
    # lists them ", "-joined and dim-wrapped. Kills "[dim]" -> "XX[dim]XX", the msgid wrap, and
    # the "XX, XX".join separator.
    entry = _exe(tmp_path)
    store.write_parameters(
        entry.slug,
        [
            ParamDecl(name="TOKA", delivery="env"),
            ParamDecl(name="TOKB", delivery="env"),
        ],
    )
    argstate.save_last(entry.slug, values={"TOKA": "sa", "TOKB": "sb"})
    result = runner.invoke(cli.app, ["params", "prog", "--secret", "TOKA", "--secret", "TOKB"])
    assert result.exit_code == 0, result.output
    out = _norm(result.output)
    assert (
        "Removed previously stored plaintext value(s) for now-secret parameter(s): TOKA, TOKB"
        in out
    )
    assert "XX" not in out
    remaining = argstate.load_state(entry.slug)["values"]
    assert "TOKA" not in remaining
    assert "TOKB" not in remaining


def test_declared_updated_message_two_params_exact(tmp_path):
    # Two declared params so the "Declared parameters" list exercises its ", " separator; a
    # --prompt tweak triggers the update line. Kills the "XX, XX".join and the msgid wrap.
    entry = _exe(tmp_path)
    store.write_parameters(
        entry.slug,
        [ParamDecl(name="a", delivery="flag"), ParamDecl(name="b", delivery="flag")],
    )
    result = runner.invoke(cli.app, ["params", "prog", "--prompt", "a=Ay"])
    assert result.exit_code == 0, result.output
    out = _norm(result.output)
    assert "Updated prog. Declared parameters: a, b" in out
    assert "XX" not in out


def test_declared_updated_message_all_removed_shows_dash(tmp_path):
    # Removing the sole declared param leaves the list empty, so it falls back to "—".
    entry = _exe(tmp_path)
    store.write_parameters(entry.slug, [ParamDecl(name="only", delivery="flag")])
    result = runner.invoke(cli.app, ["params", "prog", "--rm", "only"])
    assert result.exit_code == 0, result.output
    out = _norm(result.output)
    assert "Updated prog. Declared parameters: —" in out  # kills `or "—"` -> `or "XX—XX"`
    assert "XX" not in out
    assert store.read_parameters(entry.slug) == []


# --------------------------------------------------------------------------
# _normalize_params — messages, the reanchor guard, the capability guard
# --------------------------------------------------------------------------


def test_normalize_non_shell_message_exact(tmp_path):
    store.add_python(_py(tmp_path, "WIDTH = 800\n"), name="py")
    result = runner.invoke(cli.app, ["params", "py", "--normalize", "WIDTH"])
    assert result.exit_code == 1
    out = _norm(result.output)
    assert 'py has no --normalize: it is a shell idiom (VAR=value -> VAR="${VAR:-value}").' in out
    assert "XX" not in out


def test_normalize_reference_mode_message_exact(tmp_path):
    _shell(tmp_path, "#!/usr/bin/env bash\nWIDTH=800\n", name="rf", mode="reference")
    result = runner.invoke(cli.app, ["params", "rf", "--normalize", "WIDTH"])
    assert result.exit_code == 1
    out = _norm(result.output)
    assert (
        "rf is in reference mode, and skit never writes the original file. "
        'Change the line to VAR="${VAR:-value}" in the source directly.' in out
    )
    assert "XX" not in out


def test_normalize_no_stored_copy_message_exact(tmp_path):
    entry = _shell(tmp_path, "#!/usr/bin/env bash\nWIDTH=800\n", name="gone")
    entry.script_path.unlink()
    result = runner.invoke(cli.app, ["params", "gone", "--normalize", "WIDTH"])
    assert result.exit_code == 1
    out = _norm(result.output)
    assert "gone has no stored copy to edit." in out
    assert "XX" not in out


def test_normalize_refuses_a_non_utf8_script_untouched(tmp_path):
    # A stored copy that isn't valid UTF-8 can't round-trip through the strict parse-and-splice
    # pipeline, so --normalize refuses the whole file (exit 1) and leaves it byte-for-byte
    # untouched — the errors="replace" read would have baked U+FFFD over the raw byte instead.
    src = tmp_path / "raw.sh"
    original = b"#!/usr/bin/env bash\nWIDTH=800\nprintf '\xff\\n'\necho \"$WIDTH\"\n"
    src.write_bytes(original)
    entry = store.add_script(src, kind="shell", name="raw")
    result = runner.invoke(cli.app, ["params", "raw", "--normalize", "WIDTH"])
    assert result.exit_code == 1
    out = _norm(result.output)
    assert (
        "raw isn't valid UTF-8, so --normalize can't rewrite it safely; nothing was changed "
        "— its constants keep being injected into a temporary copy." in out
    )
    assert "XX" not in out
    assert entry.script_path.read_bytes() == original


def test_normalize_splices_bytes_minimally_on_a_cjk_script(tmp_path):
    # The A5 byte-minimal promise, byte-for-byte: only WIDTH's value span changes. CJK + emoji
    # pin the multibyte byte offsets through the strict encode/decode pair.
    src = tmp_path / "cjk.sh"
    original = (
        b"#!/usr/bin/env bash\n"
        b"WIDTH=800\n"
        b"# \xe5\xaf\xac\xe5\xba\xa6 \xf0\x9f\x8e\xaf\n"
        b'echo "$WIDTH"\n'
    )
    src.write_bytes(original)
    entry = store.add_script(src, kind="shell", name="cjk")
    result = runner.invoke(cli.app, ["params", "cjk", "--normalize", "WIDTH"])
    assert result.exit_code == 0, result.output
    expected = original.replace(b"WIDTH=800", b'WIDTH="${WIDTH:-800}"')
    assert entry.script_path.read_bytes() == expected


def test_normalize_preserves_a_crlf_copy_and_reanchors_its_managed_definition(tmp_path):
    # A Windows-authored stored copy is CRLF everywhere. The strict-decoded text is folded to LF
    # so the LF-based comment-block engine can match (else params_io.read finds nothing and the
    # re-anchor half is skipped silently — the lane Windows CI caught), then the copy's OWN CRLF
    # style is restored at write-back: the file stays CRLF end to end and skit's edit stays
    # confined to the constant it rewrote (non-invasive / genuinely byte-lossless).
    entry = _shell(tmp_path, '#!/usr/bin/env bash\nWIDTH=800\necho "$WIDTH"\n', name="crlf")
    assert runner.invoke(cli.app, ["params", "crlf", "--manage", "WIDTH"]).exit_code == 0
    raw = entry.script_path.read_bytes().replace(b"\r\n", b"\n").replace(b"\n", b"\r\n")
    entry.script_path.write_bytes(raw)
    result = runner.invoke(cli.app, ["params", "crlf", "--normalize", "WIDTH"])
    assert result.exit_code == 0, result.output
    rewritten = entry.script_path.read_bytes()
    assert b"\r\n" in rewritten  # CRLF preserved...
    stripped = rewritten.replace(b"\r\n", b"")  # ...end to end: no bare \r or \n survives the pairs
    assert b"\r" not in stripped
    assert b"\n" not in stripped
    text = rewritten.decode("utf-8")
    assert 'WIDTH="${WIDTH:-800}"' in text
    assert 'kind = "envdefault"' in text  # the managed definition followed the source


def test_manage_write_back_preserves_crlf(tmp_path):
    # --manage rewrites only the [tool.skit] block in the STORED COPY, and its write-back is
    # byte-lossless: fold to LF for the block engine, then restore the copy's CRLF style. A
    # CRLF-authored copy stays CRLF (write_text used to re-expand \n to the HOST os.linesep,
    # CRLF-ifying the whole copy on Windows / LF-flattening it on POSIX even though only the block
    # changed — the missing half of the "byte-lossless write-backs for --manage" claim).
    entry = _shell(tmp_path, '#!/usr/bin/env bash\nWIDTH=800\necho "$WIDTH"\n', name="crlf")
    crlf = entry.script_path.read_bytes().replace(b"\r\n", b"\n").replace(b"\n", b"\r\n")
    entry.script_path.write_bytes(crlf)
    result = runner.invoke(cli.app, ["params", "crlf", "--manage", "WIDTH"])
    assert result.exit_code == 0, result.output
    rewritten = entry.script_path.read_bytes()
    assert b"\r\n" in rewritten  # CRLF preserved...
    stripped = rewritten.replace(b"\r\n", b"")  # ...with no bare terminator left behind
    assert b"\r" not in stripped
    assert b"\n" not in stripped
    assert "Managed parameters: WIDTH" in _norm(result.output)


def test_normalize_preserves_a_lone_cr_copy_classic_mac(tmp_path):
    # A classic-Mac stored copy uses lone \r line endings. The fold covers \r\n AND lone \r -> \n
    # so the LF-based engine matches (else WIDTH never sits on its own line and normalize finds
    # nothing); the copy's lone-CR style is then restored, so it stays lone-CR and only the
    # constant changes. Dropping either half — the fold or the restore — is observable here.
    entry = _shell(tmp_path, '#!/usr/bin/env bash\nWIDTH=800\necho "$WIDTH"\n', name="cr")
    lone_cr = entry.script_path.read_bytes().replace(b"\r\n", b"\n").replace(b"\n", b"\r")
    entry.script_path.write_bytes(lone_cr)
    result = runner.invoke(cli.app, ["params", "cr", "--normalize", "WIDTH"])
    assert result.exit_code == 0, result.output
    rewritten = entry.script_path.read_bytes()
    assert b"\r" in rewritten  # lone-CR preserved...
    assert b"\n" not in rewritten  # ...never flattened to (or mixed with) LF
    assert 'WIDTH="${WIDTH:-800}"' in rewritten.decode("utf-8")


def test_normalize_success_message_two_names_exact(tmp_path):
    # Two consts normalized in one call: the success line names them ", "-joined. Kills the
    # message-is-None, the msgid wrap, the lowercase, and the "XX, XX".join mutants.
    entry = _shell(
        tmp_path,
        '#!/usr/bin/env bash\nWIDTH=800\nHEIGHT=600\necho "$WIDTH $HEIGHT"\n',
        name="two",
    )
    result = runner.invoke(
        cli.app, ["params", "two", "--normalize", "WIDTH", "--normalize", "HEIGHT"]
    )
    assert result.exit_code == 0, result.output
    out = _norm(result.output)
    assert (
        "Normalized WIDTH, HEIGHT in two: delivered as environment variables from now on "
        "(no temporary copy, and $0 stays your real file)." in out
    )
    assert "XX" not in out
    assert "None" not in out
    # The rewrite really landed on disk.
    text = entry.script_path.read_text(encoding="utf-8")
    assert 'WIDTH="${WIDTH:-800}"' in text
    assert 'HEIGHT="${HEIGHT:-600}"' in text


def test_normalize_keeps_an_unnormalized_managed_envdefault_intact(tmp_path):
    # DEPTH is a pre-existing managed envdefault whose stored default (99) deliberately differs
    # from the source's :-3. Normalizing WIDTH must reanchor ONLY WIDTH; DEPTH is `in envdefaults`
    # but NOT `in normalized`, so the `and` keeps its stored definition. The `or` mutant would
    # reanchor DEPTH too and reset its default from the source. Assert DEPTH's 99 survives.
    body = '#!/usr/bin/env bash\nWIDTH=800\nDEPTH="${DEPTH:-3}"\necho "$WIDTH $DEPTH"\n'
    text = metawriter.write_params(
        body,
        [
            ParamDecl(name="WIDTH", binding="const", type="int", default=800),
            ParamDecl(name="DEPTH", binding="envdefault", type="int", default=99),
        ],
    )
    entry = _shell(tmp_path, text, name="reanchor")
    result = runner.invoke(cli.app, ["params", "reanchor", "--normalize", "WIDTH"])
    assert result.exit_code == 0, result.output
    written = {d.name: d for d in metawriter.read_params(entry.script_path.read_text("utf-8"))}
    assert written["DEPTH"].default == 99  # kept, not re-derived from the source's :-3
    assert written["WIDTH"].binding == "envdefault"  # WIDTH itself did follow the source


def test_normalize_capability_guard_refuses_when_analyzer_missing(tmp_path, capsys):
    # A spec whose analyzer degraded to None (but keeps normalizer + params_io) must be refused by
    # the 4-way `or` guard. The mutant that ANDs the last two conditions would slip past and then
    # dereference None.analyze after a successful normalize. _normalize_params takes the spec as an
    # argument, so a crafted degraded spec drives the guard directly.
    entry = _shell(tmp_path, '#!/usr/bin/env bash\nWIDTH=800\necho "$WIDTH"\n', name="deg")
    degraded = _degraded_shell_spec()
    with pytest.raises(typer.Exit) as ei:
        cli._normalize_params(entry, degraded, ["WIDTH"])
    assert ei.value.exit_code == 1
    assert "has no --normalize" in _norm(capsys.readouterr().err)
