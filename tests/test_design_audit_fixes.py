"""Behavior coverage for the design-audit fixes (rounds 1, 2, 5 and 6), headless + CLI half.

Each section keeps one verified bug dead:

A. ``rewrite.read_for_block_edit`` / ``write_block_edit`` — the ONE byte-lossless
   comment-block write-back pair the six write sites now share (surrogateescape, LF fold,
   newline restore, atomic + mode-preserving), plus the ``errors="strict"`` keyword the two
   source-rewriting lanes (``--normalize``, the deps sync) pass so the fold/detect discipline
   lives in exactly one place. The TUI half of A (the AddReviewScreen corruption that
   actually shipped) lives in tests/test_design_audit_tui.py.
B. ``params.is_secret_name`` — TWO word sources per segment (the jam AND its camel
   sub-words) with one plural fold, and TOKEN's count context scoped by shape. The
   substring rule made ``--max-tokens`` a permanent password field on the reader lane;
   round 2's repair let plurals (``API_KEYS``) and acronym jams (``APIkey``) through
   unmarked, which is the publishing direction; round 5's kept the jam and lost
   ``awsSecretKey``. Round 6 owes every direction at once.
C. ``skit remove`` / ``skit preset delete`` — the non-interactive contract (worded exit-2
   refusal naming --yes, never a confirm that eats piped stdin) plus preset deletion's new
   confirmation ceremony.
D. Extra-args provenance — a remembered tail records HOW it was captured, and every replay
   in either face follows that record instead of the face doing the replaying.
J. …and a marker-less replay that carries token/glob syntax SAYS it is passed as-is:
   replaying literally is the design, doing it silently was the bug. Round 6 moved the
   predicate into ``flows.tail_looks_expandable`` (a leading ``~`` counts) so both faces
   ask one question; the TUI half of J lives in tests/test_design_audit_tui.py.
H. ``params --manage`` on a kind with no analyzer names the ``--add`` door it does have —
   with the entry name shell-quoted, because the hint is a command to paste.
I. ``params --json`` rows carry an additive ``binding`` key beside the frozen ``kind``.
K. ``flows.prefill`` accepts the state its caller already loaded: one state read per
   interaction, so a form's values, tail and provenance describe one snapshot.
"""

from __future__ import annotations

import json
import stat
import sys
import types
from pathlib import Path

import pytest
from typer.testing import CliRunner

from conftest import without_block
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


def _shell_block_edit(text: str) -> str:
    """The real edit every caller of the pair performs: params_io.write inserts/rewrites the
    comment block and touches nothing else."""
    shell = spec_for("shell")
    assert shell is not None
    assert shell.params_io is not None
    return shell.params_io.write(
        text, [ParamDecl(name="WIDTH", binding="const", type="int", default=800)]
    )


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
    assert without_block(after, newline) == original


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
    assert without_block(after, b"\n") == original


def test_read_for_block_edit_strict_refuses_what_the_default_carries(tmp_path: Path) -> None:
    """The two STRICT lanes (--normalize, the deps sync) rewrite the script's OWN text, so they
    need the decode to REFUSE rather than smuggle surrogates into a re-encode. errors="strict"
    raises on the very bytes the default carries through, and the newline discipline is the same
    shared mechanics either way — so a fold/detect fix can never miss a lane again."""
    path = tmp_path / "raw.sh"
    path.write_bytes(b"#!/usr/bin/env bash\r\nWIDTH=800\r\nprintf '\xff\\n'\r\n")

    with pytest.raises(UnicodeDecodeError):
        rewrite.read_for_block_edit(path, errors="strict")

    # The default handler is unchanged by the new keyword: same bytes, carried, same style.
    text, newline = rewrite.read_for_block_edit(path)
    assert "\udcff" in text
    assert newline == "\r\n"


def test_read_for_block_edit_strict_still_folds_and_detects(tmp_path: Path) -> None:
    """A strict lane gets the identical (LF-folded text, detected style) pair a lenient one
    does — that is the whole point of routing --normalize and the deps sync through here
    instead of their own read_bytes().decode(): the CRLF fold the LF-based block/splice
    engines need cannot drift between lanes."""
    path = tmp_path / "crlf.sh"
    path.write_bytes(b'#!/usr/bin/env bash\r\nWIDTH=800\r\necho "$WIDTH"\r\n')

    text, newline = rewrite.read_for_block_edit(path, errors="strict")

    assert "\r" not in text
    assert newline == "\r\n"


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
# B. params.is_secret_name — segment matching, plural folding, TOKEN's count context
# ==========================================================================

_SECRET_TRUE = [
    "API_KEY",
    "api_key",
    "apiKey",
    "APIKey",
    # Nonstandard casing: round 2's camel-splitting regex cut "APIkey" into API + key and
    # "SSHkey" into SSH + key, so neither reached the KEY-compound rule and both published a
    # live literal. Segments never split inside a word, so the casing cannot matter.
    "APIkey",
    "SSHkey",
    "GPGkey",
    "AWSkey",
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
    "gpgkey",
    # PLURALS — the round-2 regression this round repaired: every one of these read as
    # non-secret, which is the publishing direction. One trailing S folds away, so a plural
    # credential matches exactly like its singular.
    "API_KEYS",
    "SECRETS",
    "PASSWORDS",
    "GITHUB_TOKENS",
    "apikeys",
    "SSH_KEYS",
    "DB_PASSWORDS",
    # A qualifier that survives the count check reads as a credential, plural or not: these
    # are GitHub PATs and session credentials, not counters.
    "github_tokens",
    "access tokens",
    "session_token",
    # RE-RULED this round (was False under the word-list rule, is True now): anything ending
    # in TOKEN with no count qualifier errs secret, jammed spellings included. "photokens" is
    # not a word anyone means; a variable named it is far likelier to hold a credential than
    # a count, and ambiguity errs toward masking.
    "photokens",
    # ROUND 6 — the camelCase word source, restored beside the jammed one. Round 5 matched
    # the JAMMED segment only, so every one of these published a live literal: AWSSECRETKEY
    # ends in KEY but "AWSSECRET" is no listed qualifier, PASSWORDFILE ends in neither rule's
    # suffix, STRIPEKEY's "STRIPE" is not a prefix. Split into the words the rules already
    # know — AWS/SECRET/KEY, PASSWORD/FILE, STRIPE/KEY — every one of them matches.
    "awsSecretKey",
    "openaiApiKey",
    "passwordFile",
    "secretsFile",  # ...and the plural fold reaches a camel sub-word too (SECRETS → SECRET)
    "adminPasswordHash",
    "stripeKey",
    "deployKey",
    "clientSecretValue",
    "apiKeyId",
    # SENTENCE shape — a credential ask whose parenthetical happens to mention a size. Under
    # the name rule ("a count word ANYWHERE suppresses") these went unmasked, the publishing
    # direction, because "max"/"limit" appears somewhere after the token word.
    "Enter your API token (max 64 chars):",
    "Paste your GitHub token (limit 1 per line):",
    # The same shape at its shortest: the count word is the only other word in the text, and
    # it still FOLLOWS the token word instead of qualifying it.
    "Token (max 64):",
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
    # RE-RULED this round (were True under the word-list rule, are False now): the count
    # context suppresses the SINGULAR too. `--max-token`, `--token-limit` and `--token-count`
    # are LLM knobs people type constantly, and masking one costs a prefill and a memory on a
    # lane with no override — the same defect --max-tokens was reported for.
    "MAX_TOKEN",
    "token_limit",
    "token_count",
    # The count context, in its other spellings: a fused qualifier (nTokens), a plural
    # qualifier (token_limits), a qualifier that trails the noun (tokens_per_minute).
    "n_tokens",
    "num_tokens",
    "token_limits",
    "tokens_per_minute",
    # ROUND 6 — the camel word source cuts BOTH ways: these are the LLM knobs people actually
    # type, and round 2's segment-only rule re-masked every one of them (MAXOUTPUTTOKENS has
    # no count word to see until it is split into MAX/OUTPUT/TOKENS). A count word anywhere in
    # a NAME suppresses, however many words sit between it and the noun.
    "maxOutputTokens",
    "maxNewTokens",
    "maxInputTokens",
    "maxCompletionTokens",
    "numPredictTokens",
    # ...and the sentence shape's own rule: a count word IMMEDIATELY BEFORE the token word.
    "max tokens:",
    "How many tokens do you want?",
    # Padding is not shape: a name with stray whitespace around it is still a name, so the
    # "anywhere" rule applies (the sentence rule would leave this masked).
    "  maxOutputTokens  ",
]


@pytest.mark.parametrize("name", _SECRET_TRUE)
def test_is_secret_name_matches_real_credential_spellings(name: str) -> None:
    assert params.is_secret_name(name) is True


@pytest.mark.parametrize("name", _SECRET_FALSE)
def test_is_secret_name_rejects_lookalikes(name: str) -> None:
    assert params.is_secret_name(name) is False


# One case per member of each rule's word list, so no member can be dropped, renamed or
# typo'd without a red test — and the completeness assertion in each test means a member
# ADDED without its own case fails too.

_SUFFIX_CASES = {"SECRET": "MYSECRET", "PASSWORD": "DBPASSWORD", "PASSWD": "MYPASSWD"}
_KEY_PREFIX_CASES = {
    "API": "APIKEY",
    "AUTH": "AUTHKEY",
    "ACCESS": "ACCESSKEY",
    "SECRET": "SECRETKEY",
    "PRIVATE": "privatekey",
    "PASS": "passkey",
    "SSH": "sshkey",
    "GPG": "gpgkey",
    "AWS": "AWSkey",
    "MASTER": "MASTERKEY",
    "SIGNING": "SIGNINGKEY",
    "LICENSE": "licensekey",
    "ENCRYPTION": "ENCRYPTIONKEY",
}
_COUNT_CASES = {
    "MAX": "MAX_TOKENS",
    "MIN": "min_tokens",
    "NUM": "num_tokens",
    "N": "n_tokens",
    "COUNT": "token_count",
    "TOTAL": "total_tokens",
    "LIMIT": "token_limit",
    "MANY": "How many tokens?",
    "NUMBER": "number_of_tokens",
    "PER": "tokens_per_minute",
}


@pytest.mark.parametrize(("suffix", "name"), sorted(_SUFFIX_CASES.items()))
def test_every_secret_suffix_masks_its_jammed_spelling(suffix: str, name: str) -> None:
    """Each long suffix matches at the END of a segment, so the jammed spellings people
    actually write (MYSECRET, DBPASSWORD) stay masked — the exact-word case is covered by the
    matrix above. Losing one member publishes that whole family."""
    assert set(_SUFFIX_CASES) == set(params._SECRET_SUFFIXES)  # every member has a case
    assert params.is_secret_name(name) is True
    assert params.is_secret_name(name + "s") is True  # ...and so does its plural (one fold)


@pytest.mark.parametrize(("prefix", "name"), sorted(_KEY_PREFIX_CASES.items()))
def test_every_key_prefix_masks_its_jammed_compound(prefix: str, name: str) -> None:
    """KEY is too short for a bare suffix rule (MONKEY, TURKEY, WHISKEY), so a jammed KEY
    compound counts only behind a credential qualifier. Each qualifier is a real thing people
    name a variable, and every one of them must stay masked."""
    assert set(_KEY_PREFIX_CASES) == set(params._KEY_PREFIXES)  # every member has a case
    assert params.is_secret_name(name) is True
    assert params.is_secret_name(name + "S") is True  # the plural folds to the same compound


@pytest.mark.parametrize(("word", "name"), sorted(_COUNT_CASES.items()))
def test_every_count_word_suppresses_the_token_match(word: str, name: str) -> None:
    """TOKEN is the LLM-era collision: these names are counters, and masking one costs a
    prefill and a remembered value on the reader lane, where no override exists to turn it
    off. Losing a member re-masks that whole family of knobs."""
    assert set(_COUNT_CASES) == set(params._COUNT_WORDS)  # every member has a case
    assert params.is_secret_name(name) is False


def test_a_count_word_fused_into_the_segment_suppresses_too() -> None:
    """maxTokens/nTokens carry the qualifier INSIDE the segment, where it never appears as a
    word of its own — the suppression has to happen while matching the segment, not only in
    the whole-name pass. The credential spelling with the same shape (photokens) is unaffected:
    only a real count word suppresses."""
    assert params.is_secret_name("maxTokens") is False
    assert params.is_secret_name("nTokens") is False
    assert params.is_secret_name("photokens") is True


def test_a_fused_count_qualifier_suppresses_with_no_camel_boundary_to_help() -> None:
    """The all-caps spelling of the same names: MAXTOKENS/NTOKENS split into nothing, so the
    ONLY thing that can see the qualifier is the fused-single-qualifier rule inside the token
    check — the exact position of the slice that lifts it off the noun. (The multi-word jam
    MAXOUTPUTTOKENS is the deliberate exception: no single qualifier to lift, so it stays
    masked.)"""
    assert params.is_secret_name("MAXTOKENS") is False
    assert params.is_secret_name("NTOKENS") is False
    assert params.is_secret_name("MAXOUTPUTTOKENS") is True


def test_a_bare_plural_tokens_is_a_count_but_a_qualified_one_is_not() -> None:
    """ "tokens" alone is the LLM count knob; the singular "token" is what an auth header holds.
    Add ANY companion word and the plural reads as a credential again (github_tokens), unless
    that companion is a count qualifier."""
    assert params.is_secret_name("tokens") is False
    assert params.is_secret_name("token") is True
    assert params.is_secret_name("github_tokens") is True
    assert params.is_secret_name("max_tokens") is False


def test_the_bare_plural_exemption_needs_both_word_sources_to_agree() -> None:
    """The exemption is for the bare word "tokens", and BOTH sources have to say so. A
    nonstandard casing that jams to TOKENS but splits into something else is the APIkey class
    of spelling — the one rounds 2 and 5 each got wrong in turn — and it is not the count knob,
    so ambiguity errs toward masking."""
    assert params.is_secret_name("tokens") is False
    assert params.is_secret_name("Tokens") is False  # ...the same bare word, capitalized
    assert params.is_secret_name("toKens") is True
    assert params.is_secret_name("TokenS") is True


def test_both_word_sources_are_matched_not_one_or_the_other() -> None:
    """Round 6's structural repair: each segment contributes TWO word sources, and each source
    owns cases the other cannot see. Rounds 2 and 5 each kept one and regressed the other's
    family — this pins the pair, so dropping either source turns half of these red.

    Only the JAM sees APIkey (the camel rule cuts it at the acronym boundary into AP + Ikey);
    only the CAMEL words see awsSecretKey (the jam AWSSECRETKEY ends in KEY behind no listed
    qualifier). Neither spelling is exotic — both are how people really name credentials."""
    # jam-only: a lowercase tail on an acronym defeats every camel boundary there is
    assert params.is_secret_name("APIkey") is True
    assert params.is_secret_name("SSHkey") is True
    # camel-only: the jammed compound matches no rule at all
    assert params.is_secret_name("awsSecretKey") is True
    assert params.is_secret_name("passwordFile") is True
    # ...and the non-credential lookalikes stay unmasked under both sources
    assert params.is_secret_name("monkey") is False
    assert params.is_secret_name("publickey") is False


def test_count_suppression_is_scoped_by_shape_name_versus_sentence() -> None:
    """THE round-6 seam. A NAME suppresses on a count word anywhere in its pool, because
    maxOutputTokens is one identifier whose qualifier sits three words from the noun. A
    SENTENCE cannot afford that rule: "Enter your API token (max 64 chars):" is a credential
    ask whose parenthetical merely mentions a size, and unmasking it publishes the token.

    Whitespace (after stripping) is the discriminator, so the same words decide differently
    depending on which shape they arrive in — deliberately."""
    assert params.is_secret_name("maxOutputTokens") is False  # name: MAX anywhere suppresses
    assert params.is_secret_name("max tokens:") is False  # sentence: MAX right before the noun
    assert params.is_secret_name("Enter your API token (max 64 chars):") is True
    # ...and the very same words jammed into a NAME take the "anywhere" rule instead.
    assert params.is_secret_name("EnterYourAPIToken(max64chars)") is False
    # Stripping is what keeps a padded name a name — the sentence rule would mask this one.
    assert params.is_secret_name("  maxOutputTokens  ") is False


def test_in_a_sentence_only_the_word_immediately_before_the_token_word_suppresses() -> None:
    """The sentence rule is positional, over the camel words IN ORDER: "max tokens" is a count
    knob, "token (max 64)" is a credential with a length note. A count word that merely appears
    later in the text — or wraps around to the front of the list — must not suppress."""
    assert params.is_secret_name("max tokens:") is False  # count word, then the token word
    assert params.is_secret_name("Rate limits: tokens per minute") is False  # ...plural folded
    # The token word FIRST: nothing precedes it, so nothing can suppress it. (An index check
    # that wrapped would consult the last word — "max" — and unmask a credential prompt.)
    assert params.is_secret_name("Token (max 64):") is True
    assert params.is_secret_name("Paste your GitHub token (limit 1 per line):") is True


def test_an_all_caps_jam_of_a_multi_word_count_stays_masked() -> None:
    """The deliberate direction of the one case neither source can split: MAXOUTPUTTOKENS has
    no boundary to cut on and no count word as a segment, so it stays a TOKEN hit. Masking a
    count costs a prefill; unmasking a credential publishes it — and the two spellings that
    DO carry a boundary both read as counts."""
    assert params.is_secret_name("MAXOUTPUTTOKENS") is True
    assert params.is_secret_name("MAX_OUTPUT_TOKENS") is False
    assert params.is_secret_name("maxOutputTokens") is False


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


# --------------------------------------------------------------------------
# J. the literal replay says so — but only where it could surprise
# --------------------------------------------------------------------------

_AS_IS = "(passed as-is"


@pytest.mark.parametrize(
    ("tail", "expandable"),
    [
        (["out_{today}.png"], True),
        (["*.png"], True),
        (["report?.txt"], True),
        (["log[0-9].txt"], True),
        # Round 6: tokens.expand also expands a LEADING ~, so a tail that starts with one
        # would silently arrive unexpanded — the exact surprise the note exists for.
        (["~/backups"], True),
        (["--flag"], False),
        (["a~b"], False),  # ...only LEADING: a tilde inside a word is just a character
        ([], False),
    ],
    ids=["token", "star", "question", "bracket", "tilde", "flag", "inner-tilde", "empty"],
)
def test_tail_looks_expandable_is_the_one_predicate_behind_the_note(
    tail: list[str], expandable: bool
) -> None:
    """THE predicate, in one place: both faces (the CLI replay line and the TUI's two run
    paths) ask flows.tail_looks_expandable, so they cannot drift into two notions of "this
    would have expanded". It answers for the syntax tokens.expand actually acts on — {braces},
    glob characters, and a leading ~ — and stays quiet for anything else."""
    assert flows.tail_looks_expandable(tail) is expandable


@pytest.mark.parametrize(
    "tail",
    [["out_{today}.png"], ["*.png"], ["report?.txt"], ["log[0-9].txt"], ["~/backups"]],
    ids=["token", "star", "question", "bracket", "tilde"],
)
def test_replaying_an_unmarked_tail_with_syntax_says_it_is_passed_as_is(
    tmp_path: Path, run_entry_spy, tail: list[str]
) -> None:
    """Legacy state (and any CLI-captured tail) replays literally BY DESIGN — silently was the
    bug: the user sees `*.png` come back and reasonably expects the glob to expand. Every
    character that carries token/glob meaning triggers the note, and it goes to stderr with the
    reuse line it explains (the script owns stdout)."""
    entry = store.add_python(_py(tmp_path, "print(1)\n"), name="j")
    argstate.save_last(entry.slug, extra_args=tail)

    result = runner.invoke(cli.app, ["run", "j", "--no-input"])

    assert result.exit_code == 0, result.output
    assert run_entry_spy["extra"] == tail  # ...still literal: the note explains, never changes
    assert _AS_IS in result.stderr
    assert "typed into the launch menu" in result.stderr  # ...and names the one-time repair
    assert _AS_IS not in result.stdout


def test_replaying_an_unmarked_plain_tail_stays_quiet(tmp_path: Path, run_entry_spy) -> None:
    """The note is a surprise-avoidance line, not a banner: a tail of plain words expands to
    itself under either regime, so saying "passed as-is" would be noise on every replay."""
    entry = store.add_python(_py(tmp_path, "print(1)\n"), name="j")
    argstate.save_last(entry.slug, extra_args=["--limit", "MAX"])

    result = runner.invoke(cli.app, ["run", "j", "--no-input"])

    assert result.exit_code == 0, result.output
    assert run_entry_spy["extra"] == ["--limit", "MAX"]
    assert "Reusing your last arguments" in result.stderr  # the reuse line still prints...
    assert _AS_IS not in result.stderr  # ...without the caveat


def test_replaying_a_marked_tail_with_syntax_stays_quiet(tmp_path: Path, run_entry_spy) -> None:
    """A raw-marked tail EXPANDS on replay, so there is nothing to warn about: the note would
    claim the opposite of what just happened."""
    entry = store.add_python(_py(tmp_path, "print(1)\n"), name="j")
    argstate.save_last(entry.slug, extra_args=["out_{today}.png"], extra_args_raw=True)

    result = runner.invoke(cli.app, ["run", "j", "--no-input"])

    assert result.exit_code == 0, result.output
    (delivered,) = run_entry_spy["extra"]
    assert delivered.startswith("out_20")  # it really expanded
    assert _AS_IS not in result.stderr


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


def test_the_add_hint_shell_quotes_the_name_it_tells_you_to_paste(tmp_path: Path) -> None:
    """The hint is a copy-pasteable COMMAND, and entry names may contain spaces. Unquoted, the
    line told the user to run `skit params my tool --add PARAM` — two arguments, an entry named
    "my" that doesn't exist, and a refusal that hands out a broken incantation is worse than
    one that hands out none. The sentence half still names the entry plainly."""
    _exe(tmp_path, name="my tool")
    result = runner.invoke(cli.app, ["params", "my tool", "--manage", "WIDTH"])
    assert result.exit_code == 1
    out = " ".join(result.output.split())
    assert "my tool has no managed parameters" in out  # prose half: the name as the user sees it
    assert "Declare one instead: skit params 'my tool' --add PARAM" in out


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


# ==========================================================================
# K. flows.prefill takes the state its caller already loaded
# ==========================================================================


def _prefill_plan() -> flows.FormPlan:
    return flows.FormPlan(
        source="declared",
        fields=[flows.FormField(key="CITY", label="CITY", default="Taipei", has_default=True)],
    )


def test_prefill_uses_a_passed_state_and_never_opens_the_file(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """One interaction, one state read: the TUI loads the entry's state once and hands the SAME
    snapshot to prefill, to the form's extra row and to the provenance baseline. Passing it must
    actually skip the load — otherwise three reads of a file an agent may be rewriting can
    disagree with each other inside one launch."""
    entry = store.add_python(_py(tmp_path, "print(1)\n"), name="p")
    argstate.save_last(entry.slug, values={"CITY": "Kyoto"})
    loads: list[str] = []

    def recording(slug: str) -> dict[str, object]:
        loads.append(slug)
        return {}

    monkeypatch.setattr(flows.argstate, "load_state", recording)

    handed = {"values": {"CITY": "Osaka"}, "presets": {}, "extra_args": [], "last_run": {}}
    assert flows.prefill(_prefill_plan(), entry.slug, state=handed) == {"CITY": "Osaka"}
    assert loads == []  # the passed snapshot won, and nothing was re-read


def test_prefill_without_a_state_still_loads_it_itself(tmp_path: Path) -> None:
    """The default path is unchanged — every non-TUI caller (the CLI, the inline form) keeps
    passing nothing and gets the entry's stored values."""
    entry = store.add_python(_py(tmp_path, "print(1)\n"), name="p")
    argstate.save_last(entry.slug, values={"CITY": "Kyoto"})

    assert flows.prefill(_prefill_plan(), entry.slug) == {"CITY": "Kyoto"}


def test_prefill_reads_the_preset_out_of_the_passed_state_too(tmp_path: Path) -> None:
    """`state` replaces the read WHOLE, not just its values half: a preset resolved against a
    different snapshot than the values it overlays would mix two generations in one form."""
    entry = store.add_python(_py(tmp_path, "print(1)\n"), name="p")
    handed = {
        "values": {"CITY": "Osaka"},
        "presets": {"trip": {"CITY": "Nara"}},
        "extra_args": [],
        "last_run": {},
    }

    assert flows.prefill(_prefill_plan(), entry.slug, "trip", state=handed) == {"CITY": "Nara"}
