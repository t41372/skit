"""Behavior coverage for the design-audit fixes (rounds 1 and 2), headless + CLI half.

Each section keeps one verified bug dead:

A. ``rewrite.read_for_block_edit`` / ``write_block_edit`` — the ONE byte-lossless
   comment-block write-back pair the six write sites now share (surrogateescape, LF fold,
   newline restore, atomic + mode-preserving). The TUI half of A (the AddReviewScreen
   corruption that actually shipped) lives in tests/test_design_audit_tui.py.
B. ``params.is_secret_name`` — whole-word matching with the jammed spellings kept. The
   substring rule made ``--max-tokens`` a permanent password field on the reader lane;
   round 2's repair must not swing the other way and let ``APIKEY`` through unmarked.
C. ``skit remove`` / ``skit preset delete`` — the non-interactive contract (worded exit-2
   refusal naming --yes, never a confirm that eats piped stdin) plus preset deletion's new
   confirmation ceremony.
D. Extra-args provenance — a remembered tail records HOW it was captured, and every replay
   in either face follows that record instead of the face doing the replaying.
H. ``params --manage`` on a kind with no analyzer names the ``--add`` door it does have.
I. ``params --json`` rows carry an additive ``binding`` key beside the frozen ``kind``.
"""

from __future__ import annotations

import json
import stat
import sys
import types
from pathlib import Path

import pytest
from typer.testing import CliRunner

from skit import argstate, cli, flows, launcher, params, rewrite, store
from skit.langs.python import metawriter
from skit.langs.registry import spec_for
from skit.params import ParamDecl

runner = CliRunner()


@pytest.fixture(autouse=True)
def tmp_store(tmp_path: Path, monkeypatch: pytest.MonkeyPatch):
    monkeypatch.setenv("SKIT_DATA_DIR", str(tmp_path / "data"))
    monkeypatch.setenv("SKIT_STATE_DIR", str(tmp_path / "state"))
    monkeypatch.setenv("SKIT_CONFIG_DIR", str(tmp_path / "config"))
    monkeypatch.setenv("SKIT_LANG", "en")
    return tmp_path


@pytest.fixture
def run_entry_spy(monkeypatch: pytest.MonkeyPatch):
    """Capture the delivery-ready material handed to launcher.run_entry (nothing runs)."""
    calls: dict[str, object] = {}

    def fake(
        entry,
        extra_args=None,
        *,
        values=None,
        invoke_cwd=None,
        script_override=None,
        env_overlay=None,
        runner=None,
        prepared=None,
    ):
        calls["entry"] = entry
        calls["extra"] = list(extra_args or [])
        calls["values"] = dict(values or {})
        return 0

    monkeypatch.setattr(launcher, "run_entry", fake)
    return calls


def _py(tmp_path: Path, body: str, name: str = "job.py") -> Path:
    p = tmp_path / name
    p.write_text(body, encoding="utf-8")
    return p


def _fake_tty(monkeypatch: pytest.MonkeyPatch) -> None:
    """A real terminal, as far as the CLI's interactivity gates can tell."""
    monkeypatch.setattr(sys, "stdin", types.SimpleNamespace(isatty=lambda: True, read=lambda: ""))
    monkeypatch.setattr("sys.stdout.isatty", lambda: True, raising=False)
    monkeypatch.setattr(cli, "_is_interactive", lambda: True)


# ==========================================================================
# A. rewrite.read_for_block_edit / write_block_edit
# ==========================================================================

_SHELL_BODY = b'#!/usr/bin/env bash\nWIDTH=800\necho "$WIDTH"\n'
_BLOCK_OPEN = b"# /// script"
_BLOCK_CLOSE = b"# ///"


def _shell_block_edit(text: str) -> str:
    """The real edit every caller of the pair performs: params_io.write inserts/rewrites the
    comment block and touches nothing else."""
    shell = spec_for("shell")
    assert shell is not None
    assert shell.params_io is not None
    return shell.params_io.write(
        text, [ParamDecl(name="WIDTH", binding="const", type="int", default=800)]
    )


def _without_block(raw: bytes, newline: bytes) -> bytes:
    """Drop the inserted comment block, keeping every other byte — terminators included —
    exactly where it lies, so the comparison below is a real byte-for-byte claim about the
    rest of the file rather than a normalized diff."""
    keep: list[bytes] = []
    inside = False
    for chunk in raw.split(newline):
        if chunk == _BLOCK_OPEN:
            inside = True
            continue
        if inside:
            inside = chunk != _BLOCK_CLOSE
            continue
        keep.append(chunk)
    return newline.join(keep)


@pytest.mark.parametrize(
    ("newline", "expected"),
    [(b"\r\n", "\r\n"), (b"\r", "\r"), (b"\n", "\n")],
    ids=["crlf", "lone-cr", "lf"],
)
def test_block_edit_pair_round_trips_every_line_ending_style(
    tmp_path: Path, newline: bytes, expected: str
) -> None:
    """A CRLF, a lone-CR and an LF copy each survive read → block edit → write with ONLY the
    block changed: the style comes back verbatim, no foreign terminator is introduced, and
    every non-block byte is identical to what went in. Path.write_text (the old TUI path)
    re-expanded \\n to the host os.linesep and rewrote every line of the file."""
    path = tmp_path / "s.sh"
    original = _SHELL_BODY.replace(b"\n", newline)
    path.write_bytes(original)

    text, detected = rewrite.read_for_block_edit(path)
    assert detected == expected
    assert "\r" not in text  # folded to LF for the LF-based block engine
    rewrite.write_block_edit(path, _shell_block_edit(text), detected)

    after = path.read_bytes()
    assert b"[tool.skit]" in after  # the edit really landed
    # The copy's own style, and nothing else: stripping every occurrence of the terminator
    # must leave no stray \r or \n anywhere in the file.
    stripped = after.replace(newline, b"")
    assert b"\r" not in stripped
    assert b"\n" not in stripped
    # ...and every byte outside the block is exactly what was there before.
    assert _without_block(after, newline) == original


def test_block_edit_pair_round_trips_non_utf8_bytes(tmp_path: Path) -> None:
    """A shell/fish copy may legitimately hold arbitrary bytes. surrogateescape carries them
    through a comment-only edit untouched; the strict/replace reads this pair replaced either
    raised or baked U+FFFD over every one of them."""
    path = tmp_path / "raw.sh"
    original = b"#!/usr/bin/env bash\nWIDTH=800\nprintf '\xff\xfe\\n'\n"
    path.write_bytes(original)

    text, newline = rewrite.read_for_block_edit(path)
    assert "�" not in text  # not decoded lossily...
    assert "\udcff" in text  # ...but carried as surrogates
    rewrite.write_block_edit(path, _shell_block_edit(text), newline)

    after = path.read_bytes()
    assert b"[tool.skit]" in after
    assert b"\xff\xfe" in after  # the raw bytes round-tripped exactly
    assert b"\xef\xbf\xbd" not in after  # ...and none became U+FFFD
    assert _without_block(after, b"\n") == original


def test_write_block_edit_keeps_the_executable_bit(tmp_path: Path) -> None:
    """mkstemp's temp file is always 0600, so a tmp+replace without the mode carry would strip
    the execute bit off a stored copy that copy2 preserved at add — and the entry's next launch
    would fail with 'exists but isn't executable'."""
    path = tmp_path / "x.sh"
    path.write_bytes(_SHELL_BODY)
    path.chmod(0o755)
    # What chmod actually produced — Windows has no POSIX mode bits and reports what it likes.
    # The contract under test is PRESERVATION, not a particular value.
    expected = stat.S_IMODE(path.stat().st_mode)

    text, newline = rewrite.read_for_block_edit(path)
    rewrite.write_block_edit(path, _shell_block_edit(text), newline)

    assert b"[tool.skit]" in path.read_bytes()
    assert stat.S_IMODE(path.stat().st_mode) == expected


def test_onboard_python_degrades_on_a_non_utf8_script_instead_of_crashing(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """The python onboarding lane read its just-stored copy with a STRICT decode, so a
    non-UTF-8 python file escaped as a raw UnicodeDecodeError traceback — after the entry was
    already in the store — where the shell lane degraded gracefully on the same input. Through
    the shared pair it now round-trips the bytes and writes the block."""
    source = tmp_path / "raw.py"
    original = b'CITY = "Taipei"\n# caf\xe9 (latin-1)\nprint(CITY)\n'
    source.write_bytes(original)
    text = source.read_text(encoding="utf-8", errors="replace")  # what `add` hands in
    _fake_tty(monkeypatch)
    monkeypatch.setattr(cli.Prompt, "ask", staticmethod(lambda *a, **k: "1"))  # pick candidate 1

    entry, _deps, managed, _secrets = cli._onboard_python(
        source, text, name="rawpy", description="d", no_input=False
    )

    assert managed == ["CITY"]
    stored = entry.script_path.read_bytes()
    assert b"[tool.skit]" in stored  # the block landed
    assert b"\xe9" in stored  # ...and the latin-1 byte survived it
    assert b"\xef\xbf\xbd" not in stored


# ==========================================================================
# B. params.is_secret_name — the whole-word rule and its jammed exceptions
# ==========================================================================

_SECRET_TRUE = [
    "API_KEY",
    "api_key",
    "apiKey",
    "APIKey",
    "GITHUB_TOKEN",
    "token",
    "access-token",
    "passwd",
    "password",
    "secret",
    "Enter your API key:",
    "sort_key",
    "DB_PASSWORD",
    # Jammed spellings: a false NEGATIVE here publishes a literal into current_defaults,
    # --json output and plaintext state — the dangerous direction (round 2's repair).
    "APIKEY",
    "apikey",
    "AUTHTOKEN",
    "ACCESSTOKEN",
    "SECRETKEY",
    "sshkey",
    "passkey",
    "MYSECRET",
    "DBPASSWORD",
    "licensekey",
    "privatekey",
    "MYPASSWD",  # the fourth long suffix, jammed like its three siblings above
    # One jammed spelling per credential qualifier the KEY rule recognizes: each of these
    # is a real thing people name a variable, and every one of them must stay masked.
    "AUTHKEY",
    "ACCESSKEY",
    "GPGKEY",
    "AWSKEY",
    "MASTERKEY",
    "SIGNINGKEY",
    "ENCRYPTIONKEY",
]

_SECRET_FALSE = [
    # The reported bug: --max-tokens became a masked, never-prefilled, never-remembered
    # password field on the reader lane, where no override exists to turn it off.
    "MAX_TOKENS",
    "max-tokens",
    "maxTokens",
    "keyword",
    "monkey",
    "hotkey",
    "tokens",
    "PASSPHRASE",
    "How many tokens?",
    # KEY is too short for a bare suffix rule — these are why the prefix list exists.
    "turkey",
    "whiskey",
    "donkey",
    "jockey",
    "keyfile",
    "KEYBOARD",
    "publickey",
    "hostkey",
    "primarykey",
    "foreignkey",
]


@pytest.mark.parametrize("name", _SECRET_TRUE)
def test_is_secret_name_matches_real_credential_spellings(name: str) -> None:
    assert params.is_secret_name(name) is True


@pytest.mark.parametrize("name", _SECRET_FALSE)
def test_is_secret_name_rejects_lookalikes(name: str) -> None:
    assert params.is_secret_name(name) is False


def test_secret_heuristic_is_universal_across_lanes(tmp_path: Path) -> None:
    """One rule, every source: a command template's placeholders run through the same
    predicate the analyzers do, so "what counts as secret-looking" cannot fork per lane."""
    store.add_command("run --max-tokens {max_tokens} --key {api_key}", name="c")
    plan = flows.plan_for_entry(store.resolve("c"))
    by = {f.key: f.secret for f in plan.fields}
    assert by["api_key"] is True
    assert by["max_tokens"] is False  # the false positive that had no override anywhere


# ==========================================================================
# C. remove / preset delete — the non-interactive contract
# ==========================================================================


def _entry(tmp_path: Path, name: str = "a") -> store.Entry:
    return store.add_python(_py(tmp_path, "print(1)\n"), name=name)


def test_remove_refuses_without_yes_in_a_pipe(tmp_path: Path) -> None:
    """`skit remove` used to typer.confirm inside pipes/CI — eating a line of piped stdin and
    dying as click's bare 'Aborted.'. It now refuses the way its sibling `runner remove` does:
    a worded exit-2 that names the flag, and nothing removed."""
    _entry(tmp_path)
    result = runner.invoke(cli.app, ["remove", "a"])
    assert result.exit_code == 2
    assert "pass --yes" in result.output
    assert store.resolve("a").meta.name == "a"  # still there


def test_remove_refuses_under_no_input_even_on_a_terminal(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """--no-input is the explicit half of the same contract: on a real terminal it still
    refuses rather than asking, so a script that passes it is deterministic."""
    _entry(tmp_path)
    _fake_tty(monkeypatch)
    result = runner.invoke(cli.app, ["remove", "a", "--no-input"])
    assert result.exit_code == 2
    assert "pass --yes" in result.output
    assert store.resolve("a").meta.name == "a"


def test_remove_with_yes_succeeds_non_interactively(tmp_path: Path) -> None:
    _entry(tmp_path)
    result = runner.invoke(cli.app, ["remove", "a", "--yes"])
    assert result.exit_code == 0, result.output
    with pytest.raises(store.NotFoundError):
        store.resolve("a")


def test_remove_still_confirms_on_a_terminal(tmp_path: Path, monkeypatch: pytest.MonkeyPatch):
    """The interactive path survives the guard: a terminal still gets the ask, and "y" removes."""
    _entry(tmp_path)
    _fake_tty(monkeypatch)
    result = runner.invoke(cli.app, ["remove", "a"], input="y\n")
    assert result.exit_code == 0, result.output
    with pytest.raises(store.NotFoundError):
        store.resolve("a")


def test_remove_abort_keeps_the_entry(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    _entry(tmp_path)
    _fake_tty(monkeypatch)
    result = runner.invoke(cli.app, ["remove", "a"], input="n\n")
    assert result.exit_code == 1  # typer.confirm(abort=True) → Abort
    assert store.resolve("a").meta.name == "a"


def test_preset_delete_refuses_without_yes_in_a_pipe(tmp_path: Path) -> None:
    """A preset is unrecoverable user data that used to be deleted with no ask at all — the
    trivially re-creatable config row was better guarded than the thing users typed in."""
    entry = _entry(tmp_path)
    argstate.save_preset(entry.slug, "prod", {"CITY": "Taipei"})
    result = runner.invoke(cli.app, ["preset", "delete", "a", "prod"])
    assert result.exit_code == 2
    assert "pass --yes" in result.output
    assert argstate.load_state(entry.slug)["presets"] == {"prod": {"CITY": "Taipei"}}


def test_preset_delete_refuses_under_no_input_even_on_a_terminal(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    entry = _entry(tmp_path)
    argstate.save_preset(entry.slug, "prod", {"CITY": "Taipei"})
    _fake_tty(monkeypatch)
    result = runner.invoke(cli.app, ["preset", "delete", "a", "prod", "--no-input"])
    assert result.exit_code == 2
    assert "pass --yes" in result.output
    assert argstate.load_state(entry.slug)["presets"] == {"prod": {"CITY": "Taipei"}}


def test_preset_delete_with_yes_succeeds_non_interactively(tmp_path: Path) -> None:
    entry = _entry(tmp_path)
    argstate.save_preset(entry.slug, "prod", {"CITY": "Taipei"})
    result = runner.invoke(cli.app, ["preset", "delete", "a", "prod", "--yes"])
    assert result.exit_code == 0, result.output
    assert argstate.load_state(entry.slug)["presets"] == {}


def test_preset_delete_still_confirms_on_a_terminal(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    entry = _entry(tmp_path)
    argstate.save_preset(entry.slug, "prod", {"CITY": "Taipei"})
    _fake_tty(monkeypatch)
    result = runner.invoke(cli.app, ["preset", "delete", "a", "prod"], input="y\n")
    assert result.exit_code == 0, result.output
    assert 'Delete preset "prod" from a?' in result.output
    assert argstate.load_state(entry.slug)["presets"] == {}


def test_preset_delete_abort_keeps_the_preset(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    entry = _entry(tmp_path)
    argstate.save_preset(entry.slug, "prod", {"CITY": "Taipei"})
    _fake_tty(monkeypatch)
    result = runner.invoke(cli.app, ["preset", "delete", "a", "prod"], input="n\n")
    assert result.exit_code == 1
    assert argstate.load_state(entry.slug)["presets"] == {"prod": {"CITY": "Taipei"}}


def test_preset_delete_unknown_name_fails_before_any_ask(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """Confirming a deletion that then turns out to target nothing is a wasted question: the
    unknown-name feedback comes BEFORE the confirm, even on a terminal where one would be
    asked. typer.confirm must never be reached."""
    entry = _entry(tmp_path)
    argstate.save_preset(entry.slug, "prod", {"CITY": "Taipei"})
    _fake_tty(monkeypatch)
    asked: list[object] = []
    monkeypatch.setattr(cli.typer, "confirm", lambda *a, **k: asked.append(a))

    result = runner.invoke(cli.app, ["preset", "delete", "a", "ghost"])

    assert result.exit_code == 1
    assert "Unknown preset" in result.output
    assert "prod" in result.output  # ...and says what IS available
    assert asked == []  # never asked
    assert argstate.load_state(entry.slug)["presets"] == {"prod": {"CITY": "Taipei"}}


def test_preset_delete_reports_the_same_error_when_it_vanishes_mid_flight(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """The pre-check reads outside the lock, so a preset can still disappear between the two
    reads (a concurrent agent, another window). delete_preset re-checks UNDER the lock and its
    False lands in the very same message — one error for one condition, not two spellings of
    it depending on who won the race."""
    entry = _entry(tmp_path)
    argstate.save_preset(entry.slug, "prod", {"CITY": "Taipei"})
    monkeypatch.setattr(cli.argstate, "delete_preset", lambda slug, name: False)

    result = runner.invoke(cli.app, ["preset", "delete", "a", "prod", "--yes"])

    assert result.exit_code == 1
    assert "Unknown preset" in result.output


# ==========================================================================
# D. extra-args provenance (argstate + flows + CLI)
# ==========================================================================


def test_save_last_persists_and_clears_the_raw_marker() -> None:
    """The marker travels WITH the tail: written when a raw tail is saved, popped whenever the
    tail is replaced by a processed one or cleared — so a marker can never describe a tail it
    didn't come with."""
    argstate.save_last("prov", extra_args=["{today}.png"], extra_args_raw=True)
    assert argstate.load_state("prov")["extra_args_raw"] is True

    argstate.save_last("prov", extra_args=["--literal"], extra_args_raw=False)
    assert argstate.load_state("prov")["extra_args"] == ["--literal"]
    assert argstate.load_state("prov")["extra_args_raw"] is False

    argstate.save_last("prov", extra_args=["{today}.png"], extra_args_raw=True)
    assert argstate.load_state("prov")["extra_args_raw"] is True
    argstate.save_last("prov", extra_args=[], extra_args_raw=True)  # emptied field
    assert argstate.load_state("prov")["extra_args"] == []
    assert argstate.load_state("prov")["extra_args_raw"] is False


def test_save_last_without_a_tail_leaves_the_marker_alone() -> None:
    """A values-only save carries no tail, so it must not touch the tail's provenance."""
    argstate.save_last("keep", extra_args=["{today}.png"], extra_args_raw=True)
    argstate.save_last("keep", values={"CITY": "Taipei"})
    state = argstate.load_state("keep")
    assert state["extra_args"] == ["{today}.png"]
    assert state["extra_args_raw"] is True


def test_load_state_defaults_the_marker_for_a_legacy_document() -> None:
    """State written before the marker existed holds a tail and no key: it must read back as
    False (already shell-processed → replays literally), never as a missing key the callers
    would have to guess about."""
    argstate.save_last("legacy", extra_args=["*.png"])
    doc = argstate.load_state("legacy")
    assert doc["extra_args"] == ["*.png"]
    assert doc["extra_args_raw"] is False


def test_a_hand_edited_marker_degrades_to_literal_replay() -> None:
    """The house rule for hand-editable bools (`is True`, models.interpolate's discipline):
    a values file is TOML a person can edit, and `extra_args_raw = "no"` must land on the
    safe literal-replay default — a truthy-string coercion would flip the tail toward
    re-expansion, the exact surprise the marker exists to prevent."""
    from skit.paths import values_dir

    values_dir().mkdir(parents=True, exist_ok=True)
    (values_dir() / "edited.toml").write_text(
        'extra_args = ["*.png"]\nextra_args_raw = "no"\n', encoding="utf-8"
    )
    doc = argstate.load_state("edited")
    assert doc["extra_args"] == ["*.png"]
    assert doc["extra_args_raw"] is False


def test_save_after_run_threads_the_provenance_to_argstate(tmp_path: Path) -> None:
    entry = store.add_command("echo {msg}", name="c")
    plan = flows.plan_for_entry(entry)
    flows.save_after_run(
        entry.slug,
        plan,
        {"msg": "hi"},
        ["{today}"],
        0,
        at="2026-01-01T00:00:00+00:00",
        extra_raw=True,
    )
    assert argstate.load_state(entry.slug)["extra_args_raw"] is True

    flows.save_after_run(
        entry.slug,
        plan,
        {"msg": "hi"},
        ["*.png"],
        0,
        at="2026-01-01T00:00:01+00:00",
        extra_raw=False,
    )
    assert argstate.load_state(entry.slug)["extra_args_raw"] is False


def test_cli_run_expands_a_replayed_raw_tail(tmp_path: Path, run_entry_spy) -> None:
    """A tail typed into the TUI form is raw intent text — it never met a shell. Replaying it
    under `skit run` must expand its tokens exactly as `r` would: the two faces launch the same
    argv from the same state. Before the marker, the CLI replayed it literally and the script
    received the bare '{today}'."""
    entry = store.add_python(_py(tmp_path, "print(1)\n"), name="j")
    argstate.save_last(entry.slug, extra_args=["out_{today}.png"], extra_args_raw=True)

    result = runner.invoke(cli.app, ["run", "j", "--no-input"])

    assert result.exit_code == 0, result.output
    (tail,) = run_entry_spy["extra"]
    assert tail != "out_{today}.png"  # expanded, not passed through
    assert tail.startswith("out_20")
    assert tail.endswith(".png")
    # Intent, never expansion, is what stays on disk — and it stays marked raw.
    state = argstate.load_state(entry.slug)
    assert state["extra_args"] == ["out_{today}.png"]
    assert state["extra_args_raw"] is True


def test_cli_run_replays_an_unmarked_tail_literally(tmp_path: Path, run_entry_spy) -> None:
    """The complement: a tail the user's shell already processed (or legacy state with no
    marker) replays verbatim — a second token pass would rewrite what they deliberately
    quoted."""
    entry = store.add_python(_py(tmp_path, "print(1)\n"), name="j")
    argstate.save_last(entry.slug, extra_args=["out_{today}.png"])

    result = runner.invoke(cli.app, ["run", "j", "--no-input"])

    assert result.exit_code == 0, result.output
    assert run_entry_spy["extra"] == ["out_{today}.png"]
    assert argstate.load_state(entry.slug)["extra_args_raw"] is False


def test_cli_fresh_tail_is_never_expanded_and_clears_the_marker(
    tmp_path: Path, run_entry_spy
) -> None:
    """This run's own `-- args` came through the user's shell: never re-expanded, and saved
    UNMARKED — so a raw tail left over from a form run can't lend its expansion regime to the
    literal one that replaced it."""
    entry = store.add_python(_py(tmp_path, "print(1)\n"), name="j")
    argstate.save_last(entry.slug, extra_args=["was_{today}"], extra_args_raw=True)

    result = runner.invoke(cli.app, ["run", "j", "--no-input", "--", "kept_{today}.png"])

    assert result.exit_code == 0, result.output
    assert run_entry_spy["extra"] == ["kept_{today}.png"]  # untouched
    state = argstate.load_state(entry.slug)
    assert state["extra_args"] == ["kept_{today}.png"]
    assert state["extra_args_raw"] is False  # the stale marker is gone


def test_forget_args_clears_the_tail_and_its_marker(tmp_path: Path, run_entry_spy) -> None:
    """--forget-args is the imperative clear: the tail AND the provenance that described it."""
    entry = store.add_python(_py(tmp_path, "print(1)\n"), name="j")
    argstate.save_last(entry.slug, extra_args=["out_{today}.png"], extra_args_raw=True)

    result = runner.invoke(cli.app, ["run", "j", "--no-input", "--forget-args"])

    assert result.exit_code == 0, result.output
    assert run_entry_spy["extra"] == []
    state = argstate.load_state(entry.slug)
    assert state["extra_args"] == []
    assert state["extra_args_raw"] is False


# ==========================================================================
# H. params --manage on a kind with no params_io names the --add door
# ==========================================================================


def _exe(tmp_path: Path, name: str = "prog") -> store.Entry:
    prog = tmp_path / ("t.exe" if sys.platform == "win32" else "t")
    prog.write_text("#!/bin/sh\necho hi\n", encoding="utf-8")
    prog.chmod(0o755)
    return store.add_exe(prog, name=name)


def test_manage_on_an_exe_names_the_declared_lane_it_does_have(tmp_path: Path) -> None:
    """The declared [[parameters]] lane IS an exe's parameter home. A refusal that named no way
    forward hid the exact door built for it."""
    _exe(tmp_path)
    result = runner.invoke(cli.app, ["params", "prog", "--manage", "WIDTH"])
    assert result.exit_code == 1
    out = " ".join(result.output.split())
    assert "prog has no managed parameters — its kind has no analyzer to read them from." in out
    assert "Declare one instead: skit params prog --add PARAM" in out


def test_manage_on_a_python_entry_takes_the_analyzer_path_with_no_hint(tmp_path: Path) -> None:
    """The complement: a kind that HAS an analyzer never sees the refusal — nor its hint. The
    manage really happens."""
    entry = store.add_python(_py(tmp_path, 'CITY = "Taipei"\nprint(CITY)\n'), name="p")
    result = runner.invoke(cli.app, ["params", "p", "--manage", "CITY"])
    assert result.exit_code == 0, result.output
    assert "Declare one instead" not in result.output
    assert "no managed parameters" not in result.output
    io = spec_for("python")
    assert io is not None
    assert io.params_io is not None
    stored = entry.script_path.read_text(encoding="utf-8")
    assert [d.name for d in io.params_io.read(stored)] == ["CITY"]


# ==========================================================================
# I. params --json rows carry "binding" beside the frozen "kind"
# ==========================================================================


def test_params_json_rows_carry_both_kind_and_binding(tmp_path: Path) -> None:
    """ "kind" is the FROZEN on-disk key and carries the BINDING (const/input) — while `show
    --json`'s "kind" is the entry's LANGUAGE. The additive "binding" key lets an agent read one
    axis by one unambiguous name; "kind" stays for the files already on disk."""
    text = metawriter.write_params(
        'CITY = "Taipei"\nNAME = input("Name: ")\nprint(CITY, NAME)\n',
        [
            ParamDecl(name="CITY", binding="const", type="str", default="Taipei"),
            ParamDecl(name="NAME", binding="input", type="str", prompt="Name: "),
        ],
    )
    store.add_python(_py(tmp_path, text), name="p")

    result = runner.invoke(cli.app, ["params", "p", "--json"])
    assert result.exit_code == 0, result.output
    rows = json.loads(result.output)["params"]

    assert [r["name"] for r in rows] == ["CITY", "NAME"]
    assert [r["binding"] for r in rows] == ["const", "input"]
    for row in rows:
        assert row["kind"] == row["binding"]  # same axis, both spellings, no drift


def test_params_json_binding_is_additive_not_a_rename(tmp_path: Path) -> None:
    """The frozen row is still emitted whole — "binding" is added ON TOP of to_block_dict, never
    instead of a key an existing consumer reads."""
    text = metawriter.write_params(
        'TOKEN = "x"\nprint(TOKEN)\n',
        [ParamDecl(name="TOKEN", binding="const", type="str", default="x", secret=True)],
    )
    store.add_python(_py(tmp_path, text), name="s")
    (row,) = json.loads(runner.invoke(cli.app, ["params", "s", "--json"]).output)["params"]
    decl = ParamDecl(name="TOKEN", binding="const", type="str", default="x", secret=True)
    assert row == {**decl.to_block_dict(), "binding": "const"}
