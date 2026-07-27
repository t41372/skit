"""Behavior coverage for the design-audit fixes (rounds 1, 2, 5, 6 and 7), headless + CLI half.

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
   ``awsSecretKey``. Round 6 owes every direction at once. Round 7 made the verdict
   per-SEGMENT (``_judge_segment``) so a camel fragment can no longer veto a credential
   in the segment next door (``N8N_TOKEN``) and digits stay inside their segment
   (``api_key2``); round 7b split count WORDS from bare NUMBERS, because a number is an
   index unless it stands in front (``GITHUB_TOKEN_2`` is a second GitHub token).
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
import shlex
import stat
import sys
import types
from pathlib import Path

import pytest
from typer.testing import CliRunner

from conftest import without_block
from skit import analysis, argstate, cli, flows, launcher, params, rewrite, store, tokens
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
    # ROUND 7 — digits stay INSIDE their segment now. The old [^A-Za-z]+ split shattered
    # N8N into N + N, and that stray count word ("N") vetoed the TOKEN in the segment next
    # door: every self-hosted n8n credential published a live literal.
    "N8N_TOKEN",
    "n8n_token",
    "n8nToken",
    "gpt4_token",
    "gpt4Token",
    # ...and _forms strips digits per word, so a credential word glued to one still matches.
    "api_key2",
    "base64Key",
    # ROUND 7b — a bare number is an INDEX unless it stands in front. Round 7 made any
    # number count context for the whole name, so a second GitHub token read as a token
    # COUNT and went unmasked — the publishing direction, on names people really write.
    "GITHUB_TOKEN_2",
    "API_TOKEN_1",
    "slack-token-3",
    "TOKEN_2",
    "token_2",
    # SENTENCE shape, round 7: an ask that ALSO quotes a rate is still an ask. The old rule
    # suppressed on ANY count-preceded mention, so the parenthetical vetoed the credential.
    "Enter your API token (limit: 4096 tokens)",
    "Paste your GitHub token (rate limit 60 tokens/min)",
    # RE-RULED in round 7 (was False): digits no longer split "max64chars" into a bare MAX,
    # so this jam keeps its TOKEN hit. The direction is masking, and every real count
    # spelling still reads as a count (see _SECRET_FALSE).
    "EnterYourAPIToken(max64chars)",
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
    # ROUND 7 — a count word still wins inside a digit-bearing name, from either source: the
    # segment's own camel fragment (MAX2 → MAX) or a segment of its own.
    "MAX2TOKENS",
    "max_2_tokens",
    "max64Tokens",
    # ROUND 7b — a bare number IN FRONT is the count it looks like, in both shapes.
    "2_tokens",
    "60 tokens",
    "max 4096 tokens",
    # RE-RULED in round 7 (were True): the bare-plural knob reads as a count in ANY casing.
    # No compound can hide inside six letters the way API + key hides inside "APIkey", so
    # the old "both word sources must spell TOKENS" condition was an accident of the camel
    # split, not a rule.
    "toKens",
    "TokenS",
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


def test_the_bare_plural_exemption_reads_the_letters_not_the_casing() -> None:
    """RE-RULED in round 7 (toKens/TokenS were True): the exemption is for the bare word
    "tokens", and six letters have no room to hide a compound. Rounds 2-6 required both word
    sources to spell TOKENS, which made the camel split decide the answer — so "toKens" and
    "TokenS" read as credentials while "tokens" and "Tokens" read as counts, on names that are
    the same word. That condition was an accident of the split, not a rule; the letters are.

    The APIkey class it was borrowed from is genuinely different: there the casing hides API +
    key, two words the rules know. TO + KENS are not words, and neither is TOKEN + S."""
    for spelling in ("tokens", "Tokens", "toKens", "TokenS", "TOKENS"):
        assert params.is_secret_name(spelling) is False, spelling
    # A companion word is what makes the plural a credential again — in any casing.
    assert params.is_secret_name("github_tokens") is True
    assert params.is_secret_name("gitHubToKens") is True


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
    # ...and the very same words spaced into a NAME take the "anywhere" rule instead.
    assert params.is_secret_name("enter_your_API_token_max_64_chars") is False
    # RE-RULED in round 7 (was False): jammed against digits there is no bare MAX left to
    # find — "max64chars" is one segment whose forms are MAX64CHARS/MAXCHARS, neither a count
    # word — so the name keeps its TOKEN hit. Masking a synthetic jam costs a prefill;
    # unmasking a credential publishes it, and every count spelling people really type still
    # reads as a count (max_64_tokens, max64Tokens, MAX2TOKENS, gpt4_max_tokens).
    assert params.is_secret_name("EnterYourAPIToken(max64chars)") is True
    assert params.is_secret_name("max_64_tokens") is False
    assert params.is_secret_name("gpt4_max_tokens") is False
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


# --------------------------------------------------------------------------
# B (round 7). The verdict is per SEGMENT — and a bare number qualifies FORWARD
# --------------------------------------------------------------------------


def test_forms_folds_the_plural_and_strips_digits_per_word() -> None:
    """The match variants of ONE word. Digits no longer split a name apart (that is what
    shattered N8N), so they have to be lifted off each word instead — otherwise "key2" is a
    word no rule has ever heard of and api_key2 publishes a live key. Both folds are offered
    together, so a digit-bearing plural (KEYS2) reaches KEY as well."""
    assert params._forms("KEY") == {"KEY"}
    assert params._forms("KEYS") == {"KEYS", "KEY"}
    assert params._forms("KEY2") == {"KEY2", "KEY"}
    # ...and the two folds compose, so a digit-bearing plural still reaches KEY.
    assert params._forms("KEYS2") == {"KEYS2", "KEYS", "KEY"}
    assert params._forms("N8N") == {"N8N", "NN"}
    # An all-digit word strips to nothing, and the empty string is not a form of anything — so
    # the filter drops it (inverting that filter leaves _forms holding nothing else).
    assert params._forms("4096") == {"4096"}
    assert "" not in params._forms("64")
    # ...and the rules really consult the stripped form, on both sides of a name.
    assert params.is_secret_name("api_key2") is True
    assert params.is_secret_name("base64Key") is True


def test_token_form_lifts_exactly_one_fused_count_qualifier() -> None:
    """The single-qualifier slice, at the position that lifts it off the noun: NTOKEN/MAXTOKEN
    are counts jammed into one word, PHOTOKEN is a word that merely ends the same way. A slice
    off by one turns either answer into the other."""
    assert params._token_form("TOKEN") is True
    assert params._token_form("PHOTOKEN") is True
    assert params._token_form("NTOKEN") is False
    assert params._token_form("MAXTOKEN") is False
    assert params._token_form("TOKENS") is False  # the fold is the caller's job, not this one
    assert params._token_form("KEY") is False


def test_a_segment_is_judged_whole_and_reports_the_six_things_the_rules_ask() -> None:
    """The round-8 structure: one verdict per segment, six independent bits. Secrecy is a
    whole-name answer; a token mention belongs to the segment that carries it; whether that
    mention is PLURAL decides what a bare number in front of it means; a count word INSIDE the
    segment is a separate bit from a count word that IS the segment, because the first vetoes
    only at home and only in a name, and the second qualifies its neighbours.

    Round 7 fused the last two into one bit (the token hit was pre-vetoed inside the verdict),
    which is why a prompt spelling perUserToken read as a count knob. Split, each bit is
    applied by the rule that owns it."""
    assert params._judge_segment("token") == params._SegmentVerdict(
        secret=False,
        token=True,
        token_plural=False,
        internal_count=False,
        county=False,
        numeric=False,
    )
    assert params._judge_segment("apiKey") == params._SegmentVerdict(
        secret=True,
        token=False,
        token_plural=False,
        internal_count=False,
        county=False,
        numeric=False,
    )
    assert params._judge_segment("tokens") == params._SegmentVerdict(
        secret=False,
        token=True,
        token_plural=True,
        internal_count=False,
        county=False,
        numeric=False,
    )
    assert params._judge_segment("limit") == params._SegmentVerdict(
        secret=False,
        token=False,
        token_plural=False,
        internal_count=True,
        county=True,
        numeric=False,
    )
    assert params._judge_segment("60") == params._SegmentVerdict(
        secret=False,
        token=False,
        token_plural=False,
        internal_count=False,
        county=False,
        numeric=True,
    )
    # A segment's OWN count word is reported (is_secret_name applies it in NAME shape), and the
    # token hit it qualifies survives in the verdict so a SENTENCE can still see the mention.
    assert params._judge_segment("maxOutputTokens").internal_count is True
    assert params._judge_segment("maxOutputTokens").token is True
    assert params._judge_segment("nTokens").internal_count is True
    assert params._judge_segment("photokens").internal_count is False
    assert params._judge_segment("photokens").token is True


def test_only_a_count_word_that_is_the_whole_segment_qualifies_its_neighbours() -> None:
    """THE round-8 leak. county is the UNSTRIPPED jam and nothing else. Round 7 asked _forms —
    which strips digits — so N26 offered its stripped N, a count word, as context for the
    segment beside it: `N26_TOKEN` published an n26 API token. A digit run is part of the
    acronym, not a qualifier that happens to be spelled next to one.

    The other half has to be pinned with it: a real count word still qualifies from anywhere in
    a name, or max_tokens starts masking."""
    assert params._judge_segment("N26").county is False  # stripping would find a count "N"
    assert params._judge_segment("N8N").county is False
    assert params._judge_segment("N").county is True  # ...the literal word still counts
    assert params._judge_segment("MAXS").county is True  # ...and so does its plural
    for indexed in ("N26_TOKEN", "N1_TOKEN", "n8_token", "N8N_TOKEN"):
        assert params.is_secret_name(indexed) is True, indexed
    assert params.is_secret_name("max_tokens") is False
    assert params.is_secret_name("n_tokens") is False


def test_the_camel_split_never_cuts_at_a_digit() -> None:
    """Round 7 split on digit→Upper as well, which shattered the N8N family into a stray count
    "N" fragment (N8NToken → N8 | N | Token) and unmasked the credential. Digits are recovered
    by _digit_split for MATCHING instead, so nothing is lost by not cutting here.

    Restoring the digit alternative flips every N8N spelling below back to unmasked."""
    assert params._CAMEL_BOUNDARY.sub(" ", "N8NToken") == "N8N Token"
    assert params._CAMEL_BOUNDARY.sub(" ", "base64Key") == "base64Key"  # left whole for the...
    assert params.is_secret_name("base64Key") is True  # ...digit source to split instead
    assert params._CAMEL_BOUNDARY.sub(" ", "awsSecretKey") == "aws Secret Key"  # still cuts
    assert params._CAMEL_BOUNDARY.sub(" ", "APIKey") == "API Key"  # ...on both boundaries
    for spelling in ("N8NToken", "N8NTOKEN", "n8nToken", "N8N_TOKEN"):
        assert params.is_secret_name(spelling) is True, spelling


def test_digit_glued_words_are_recovered_for_matching() -> None:
    """The third word source (round 8). base64key is ONE segment whose jam is BASE64KEY and
    whose camel words are the same — no rule had ever heard of it, so a base64-encoded key was
    published in current_defaults. Splitting the jam on its digit runs finds the KEY.

    Matching only: the parts must never become count context (that is the N8N trap above), so
    the shards feed the pool and nothing else."""
    assert params._digit_split("BASE64KEY") == ["BASE", "KEY"]
    assert params._digit_split("N8NTOKEN") == ["N", "NTOKEN"]
    assert params._digit_split("64") == []  # an all-digit jam has no parts at all
    assert params._digit_split("KEY") == ["KEY"]
    hashed_keys = ("base64key", "BASE64KEY", "sha256key", "s3key", "md5key", "x509key", "gpt4key")
    for hashed in hashed_keys:
        assert params.is_secret_name(hashed) is True, hashed
    # ...and the shard N from N8NTOKEN is in the pool WITHOUT becoming a qualifier.
    assert "N" in params._digit_split("N8NTOKEN")
    assert params.is_secret_name("N8NTOKEN") is True


def test_the_camel_split_cuts_on_the_separator_its_own_sub_inserts() -> None:
    """One decision spelled in two places: _CAMEL_BOUNDARY.sub writes a SPACE at each boundary
    and the split cuts on that same space, so the two must stay the same character. Widening
    the split to "any whitespace" would be a second, wider rule the sub never asked for — and
    a segment cannot carry whitespace anyway, because is_secret_name has already cut the name
    on every non-alphanumeric character."""
    assert params._judge_segment("apiKey").secret is True  # the boundary the sub inserts
    assert params._judge_segment("api key").secret is True  # ...that same character, literally
    assert params._judge_segment("api\tkey").secret is False  # ...and only that character


def test_a_camel_fragment_vetoes_only_at_home_and_only_in_a_name() -> None:
    """Two rounds' worth of rules, pinned together because each one alone reads as an extreme.

    A count word INSIDE a segment vetoes that segment's own token hit — that is what makes
    maxOutputTokens a count knob — but it never reaches the segment next door (round 7's HIGH:
    N8N's fragment vetoed the credential beside it), and in round 8 it stopped reaching across
    SHAPES too: a prompt that spells the camel name is still an ask for that value. "Enter your
    perUserToken:" is a person typing a credential into a form, and PER is a preposition there,
    not a quantity."""
    assert params._judge_segment("N8N").county is False  # never leaks out...
    assert params._judge_segment("maxOutputTokens").internal_count is True  # ...vetoes at home
    assert params.is_secret_name("N8N_TOKEN") is True
    assert params.is_secret_name("n8nToken") is True
    assert params.is_secret_name("maxOutputTokens") is False
    # The NAME/SENTENCE asymmetry the internal veto now respects.
    assert params.is_secret_name("perUserToken") is False  # a name: PER is a rate qualifier
    assert params.is_secret_name("Enter your perUserToken: ") is True  # a prompt: an ask
    assert params.is_secret_name("Enter your maxOutputTokens: ") is True


def test_the_internal_veto_takes_a_literal_letter_but_never_a_one_letter_remnant() -> None:
    """The length test inside internal_count, at the boundary it actually has. N is the only
    single-letter count word, so the rule is "a ONE-LETTER remnant of stripping never counts" —
    a camel word that IS the count word still counts at one letter (nTokens), and any shard of
    two letters or more counts (max64Tokens' MAX).

    Both directions have a killing case. Accept one-letter remnants and N8N's shard vetoes a
    real credential; reject multi-letter ones and a plural count word stops reading as a count.
    (`maxsTokens` and `tokenN8` are the smallest names that separate them — synthetic, but they
    are what makes the boundary a tested fact rather than an arbitrary minimum.)"""
    assert params._judge_segment("nTokens").internal_count is True  # the literal camel N
    assert params._judge_segment("max64Tokens").internal_count is True  # digit shard, 3 letters
    assert params._judge_segment("maxsTokens").internal_count is True  # camel shard, 3 letters
    assert params._judge_segment("N8NToken").internal_count is False  # a one-letter remnant
    assert params._judge_segment("tokenN8").internal_count is False
    assert params.is_secret_name("nTokens") is False
    assert params.is_secret_name("max64Tokens") is False
    assert params.is_secret_name("maxsTokens") is False
    assert params.is_secret_name("MAX2TOKENS") is False
    assert params.is_secret_name("N8NToken") is True
    assert params.is_secret_name("N8NTOKEN") is True
    assert params.is_secret_name("tokenN8") is True
    # The word list is what makes the boundary 1 and not 2: N is the only count word a
    # single letter can be, so a rule keyed on any other length would be keyed on nothing.
    assert [w for w in params._COUNT_WORDS if len(w) == 1] == ["N"]
    assert not [w for w in params._COUNT_WORDS if len(w) == 2]


def test_a_bare_number_is_count_context_only_for_what_follows_it() -> None:
    """ROUND 7b. "60 tokens" is a count by construction; GITHUB_TOKEN_2 is a second GitHub
    token. Round 7 made every number count context for the whole name and unmasked the entire
    indexed-credential family — the publishing direction, on names people really write.

    So a number qualifies FORWARD only, while a count WORD still qualifies a name from
    anywhere (max_tokens keeps its qualifier three words away)."""
    assert params.is_secret_name("2_tokens") is False  # the number stands in front
    assert params.is_secret_name("60 tokens") is False  # ...in either shape
    for indexed in ("GITHUB_TOKEN_2", "API_TOKEN_1", "slack-token-3", "TOKEN_2", "token_2"):
        assert params.is_secret_name(indexed) is True, indexed
    # ...and the two kinds of context are not interchangeable: merging them back into one bit
    # is exactly the round-7 bug.
    assert params._judge_segment("2").numeric is True
    assert params._judge_segment("2").county is False
    assert params._judge_segment("max").county is True
    assert params._judge_segment("max").numeric is False
    assert params.is_secret_name("max_tokens_2") is False  # a count WORD reaches from anywhere


def test_a_number_counts_a_plural_and_indexes_a_singular() -> None:
    """ROUND 8 narrowed the forward rule to the shape English actually uses. "2 tokens" counts
    them; "step 2 token" numbers ONE of them — and the numbered one is a credential, so
    STEP_2_TOKEN, USER_2_TOKEN and their prompt spellings must stay masked. Round 7b let any
    number excuse the mention behind it, which unmasked that whole family the moment the index
    sat in the middle of the name instead of the end.

    The plural is the discriminator, and it is read from all three word sources."""
    assert params.is_secret_name("2_tokens") is False  # plural: a count
    assert params.is_secret_name("60 tokens") is False
    assert params.is_secret_name("max 4096 tokens") is False
    for indexed in ("STEP_2_TOKEN", "USER_2_TOKEN", "TENANT_2_TOKEN", "Enter step 2 token:"):
        assert params.is_secret_name(indexed) is True, indexed
    # token_plural is what the number consults, and it reads the jam/camel/digit-split words.
    assert params._judge_segment("tokens").token_plural is True
    assert params._judge_segment("token").token_plural is False
    assert params._judge_segment("myTokens").token_plural is True
    # ...and a segment with no token hit never claims to be a plural one.
    assert params._judge_segment("2").token_plural is False


def test_a_number_excuses_the_mention_it_stands_in_front_of_never_the_whole_name() -> None:
    """The quantifier in the name branch is ALL mentions, not any. A name that counts tokens
    somewhere and also names a token elsewhere is still asking for a credential, and the
    credential wins — the same direction the sentence rule takes.

    (The two-mention spelling is synthetic; nobody names a variable this. It is the only shape
    that separates "every mention is number-qualified" from "some mention is", and the rule it
    pins is the one that keeps GITHUB_TOKEN_2 masked.)"""
    assert params.is_secret_name("2_tokens") is False  # every mention qualified → a count
    assert params.is_secret_name("token_2_tokens") is True  # one bare mention → a credential
    assert params.is_secret_name("2_tokens_token") is True


def test_the_sentence_rule_is_secret_if_any_mention_is_unqualified() -> None:
    """ROUND 7's other leak direction, flipped: the old rule suppressed as soon as ANY mention
    was count-preceded, so a parenthetical rate note vetoed the ask beside it. An ask that
    quotes a rate AND asks for a credential is an ask for a credential.

    Three separable pieces, one killing case each: the mention must have a predecessor at all
    (a leading Token), that predecessor is the one BEFORE it (not after, not wrapped), and the
    verdict is the NEGATION of qualification (an unqualified mention is what masks)."""
    # No predecessor: the token word is first, so nothing can qualify it.
    assert params.is_secret_name("Token (max 64):") is True
    # The predecessor is the segment immediately BEFORE — "max tokens" is a count, and a count
    # word that merely appears later never suppresses.
    assert params.is_secret_name("max tokens:") is False
    assert params.is_secret_name("tokens max:") is True
    # The negation: one mention with count context, one without — the one without wins.
    assert params.is_secret_name("Paste your GitHub token (rate limit 60 tokens/min)") is True
    assert params.is_secret_name("Enter your API token (limit: 4096 tokens)") is True
    # ...and with every mention qualified there is nothing left to mask.
    assert params.is_secret_name("How many tokens do you want?") is False
    assert params.is_secret_name("rate limit 60 tokens/min") is False


def test_both_kinds_of_count_context_qualify_a_sentence_mention() -> None:
    """qualified() reads county OR numeric, and each half owns cases the other cannot see:
    "many tokens" has no number, "60 tokens" has no count word. Dropping either half re-masks
    an LLM knob on a lane with no override."""
    assert params.is_secret_name("many tokens") is False  # the count-word half
    assert params.is_secret_name("60 tokens") is False  # the bare-number half
    assert params.is_secret_name("tokens") is False  # (and the bare plural is neither)


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
        (["{cwd}"], True),
        (["{env:HOME}"], True),
        (["*.png"], True),
        (["report?.txt"], True),
        (["log[0-9].txt"], True),
        # Round 6: tokens.expand also expands a LEADING ~, so a tail that starts with one
        # would silently arrive unexpanded — the exact surprise the note exists for.
        (["~/backups"], True),
        # Round 7: the brace half is tokens.has_tokens now, and the hand-rolled check it
        # replaces was wrong in BOTH directions. `}}` halves to `}` — silently, with no note;
        # a bare `{x}` is no token at all and passes through untouched, so the note fired on
        # tails nothing would have changed.
        (["}}"], True),
        (["{{"], True),
        (["{x}"], False),
        (["--flag"], False),
        (["a~b"], False),  # ...only LEADING: a tilde inside a word is just a character
        ([], False),
    ],
    ids=[
        "token",
        "cwd",
        "env",
        "star",
        "question",
        "bracket",
        "tilde",
        "close-escape",
        "open-escape",
        "unknown-brace",
        "flag",
        "inner-tilde",
        "empty",
    ],
)
def test_tail_looks_expandable_is_the_one_predicate_behind_the_note(
    tail: list[str], expandable: bool
) -> None:
    """THE predicate, in one place: both faces (the CLI replay line and the TUI's two run
    paths) ask flows.tail_looks_expandable, so they cannot drift into two notions of "this
    would have expanded". It answers for the syntax tokens.expand actually acts on — the token
    grammar, brace escapes and a leading ~ — plus the glob characters assemble's own pass
    acts on, and stays quiet for anything else."""
    assert flows.tail_looks_expandable(tail) is expandable


@pytest.mark.parametrize(
    "piece",
    ["out_{today}.png", "{cwd}", "{env:HOME}", "{x}", "{{", "}}", "{{cwd}}", "~/backups", "a~b"],
)
def test_the_token_half_of_the_predicate_is_the_expander_itself(piece: str, tmp_path: Path) -> None:
    """Round 7's structural point: the note's question is "would expand() have changed this?",
    and tokens is THE authority on that answer — so the predicate delegates instead of
    re-deriving. The hand-rolled brace check had already forked (missing `}}`, over-firing on
    `{x}`), and a fork here is a lie printed to the user either way.

    Asked against the real expander, with no glob character in sight so only the token half
    can answer."""
    changed = tokens.expand(piece, cwd=tmp_path, env={"HOME": "/home/x"}) != piece
    assert flows.tail_looks_expandable([piece]) is changed


@pytest.mark.parametrize("char", flows._GLOB_CHARS)
def test_the_glob_half_of_the_predicate_reads_the_same_constant_assemble_does(char: str) -> None:
    """ROUND 8's version of the same structural point, for the other half. The predicate used to
    spell its glob characters as a literal "*?[" beside the _GLOB_CHARS tuple assemble's own
    glob pass consults — two spellings of one decision, and the note is a claim ABOUT what that
    pass did. Adding a character to the constant would have taught assemble to expand it while
    the note kept quiet, which is the silent-expansion bug the note exists to prevent.

    Parametrized over the constant itself, so the contract widens automatically with it."""
    assert flows.tail_looks_expandable([f"file{char}.txt"]) is True
    assert flows.tail_looks_expandable(["file.txt"]) is False  # ...and nothing else fires


@pytest.mark.parametrize(
    "piece", ["photos/*.png", "img_?.jpg", "log[0-9].txt", "plain.txt", "a b", "--flag"]
)
def test_the_note_agrees_with_the_pass_that_does_the_globbing(piece: str, tmp_path: Path) -> None:
    """The coupling from the other side, behaviourally: glob_feedback is the surface that acts
    on _GLOB_CHARS, and it answers None for exactly the values it would not expand. Whatever it
    calls a pattern, the note must call expandable — a divergence is a user watching their tail
    change with no note printed, or reading a note about a tail that never moved."""
    treated_as_a_pattern = flows.glob_feedback(piece, tmp_path) is not None
    assert flows.tail_looks_expandable([piece]) is treated_as_a_pattern


@pytest.mark.parametrize(
    "tail",
    [
        ["out_{today}.png"],
        ["*.png"],
        ["report?.txt"],
        ["log[0-9].txt"],
        ["~/backups"],
        # Round 7: `}}` halves to `}` on an expanding replay, so a literal one differs from
        # what the user would have got — and the old hand-rolled check said nothing at all.
        ["done}}"],
    ],
    ids=["token", "star", "question", "bracket", "tilde", "close-escape"],
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


@pytest.mark.parametrize(
    "tail",
    [["--limit", "MAX"], ["{x}"]],
    ids=["plain-tail", "unknown-brace"],
)
def test_replaying_an_unmarked_plain_tail_stays_quiet(
    tmp_path: Path, run_entry_spy, tail: list[str]
) -> None:
    """The note is a surprise-avoidance line, not a banner: a tail that expands to itself under
    either regime would collect the caveat on every single replay. Round 7 added the second
    case — `{x}` is no token skit knows, so expand() passes it through and the literal replay
    is byte-identical to the expanding one."""
    entry = store.add_python(_py(tmp_path, "print(1)\n"), name="j")
    argstate.save_last(entry.slug, extra_args=tail)

    result = runner.invoke(cli.app, ["run", "j", "--no-input"])

    assert result.exit_code == 0, result.output
    assert run_entry_spy["extra"] == tail
    assert "Reusing your last arguments" in result.stderr  # the reuse line still prints...
    assert _AS_IS not in result.stderr  # ...without the caveat


def test_the_cli_note_is_the_shared_msgid_verbatim(tmp_path: Path, run_entry_spy) -> None:
    """ONE msgid, ONE home: round 7 moved the sentence into flows.as_is_note() and both faces
    call it (the TUI half is pinned in tests/test_design_audit_tui.py). Asserting the exact
    string here is what makes the two faces provably the same sentence — a substring check
    would pass on two wordings that share a prefix."""
    entry = store.add_python(_py(tmp_path, "print(1)\n"), name="j")
    argstate.save_last(entry.slug, extra_args=["*.png"])

    result = runner.invoke(cli.app, ["run", "j", "--no-input"])

    assert result.exit_code == 0, result.output
    assert flows.as_is_note() in " ".join(result.stderr.split())


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


def test_the_add_hint_names_the_slug_so_it_needs_no_quoting(tmp_path: Path) -> None:
    """ROUND 8, superseding round 7's platform quoting. The hint is a copy-pasteable COMMAND and
    entry names may contain spaces, so round 7 quoted the name with the running platform's
    convention. That was still the wrong axis: the shell a user pastes into is not always the
    shell skit is running under (an agent shelling out, a POSIX skit printing into a Windows
    terminal, a name carrying & | ^ which list2cmdline leaves bare for cmd.exe).

    A slug needs no convention anywhere — that is what its charset is for — and resolve()
    accepts it wherever a name works. So the prose half names the entry as the user sees it and
    the command half names the slug, which is pasteable in every shell."""
    entry = _exe(tmp_path, name="my tool")
    result = runner.invoke(cli.app, ["params", "my tool", "--manage", "WIDTH"])
    assert result.exit_code == 1
    out = " ".join(result.output.split())
    assert "my tool has no managed parameters" in out  # prose half: the name as the user sees it
    assert f"Declare one instead: skit params {entry.slug} --add PARAM" in out
    assert entry.slug == "my-tool"  # ...and the slug is quoting-free by construction
    for convention in ("'my tool'", '"my tool'):  # no platform's quoting, on any platform
        assert convention not in out


def test_the_pasted_hint_resolves_back_to_the_entry_it_came_from(tmp_path: Path) -> None:
    """The hint is only worth printing if it WORKS. Lift the command out of the refusal, run it
    the way a user would paste it, and it must land on the same entry — a name that needs
    quoting is exactly the case where the round-7 hint broke, so this is the round trip that
    proves the slug closes it. A metacharacter name (`a & b`) is the case no quoting convention
    survived."""
    entry = _exe(tmp_path, name="a & b")

    result = runner.invoke(cli.app, ["params", "a & b", "--manage", "WIDTH"])

    assert result.exit_code == 1
    out = " ".join(result.output.split())
    hint = out.split("Declare one instead: ")[1]
    pasted = shlex.split(hint)[: len(["skit", "params", entry.slug, "--add", "PARAM"])]
    assert pasted == ["skit", "params", entry.slug, "--add", "PARAM"]
    assert store.resolve(pasted[2]).slug == entry.slug  # the pasted target really resolves
    # ...and running it does the thing the sentence promised.
    added = runner.invoke(cli.app, [*pasted[1:], "--type", "PARAM=int"])
    assert added.exit_code == 0, added.output
    declared = params.declared_from_meta(store.resolve(entry.slug).meta.parameters)
    assert [(d.name, d.type) for d in declared] == [("PARAM", "int")]


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


# --------------------------------------------------------------------------
# H2 (round 8). EVERY paste-able hint names the slug — one rule, seven sites
# --------------------------------------------------------------------------

# The name every case below is registered under: a space AND a shell metacharacter, so a
# hint that leaked the display name is broken in every shell rather than only some.
_AWKWARD = "a & b"


def test_drift_lines_keeps_the_name_in_prose_and_the_target_in_the_command() -> None:
    """The split round 8 introduced: one identity for the human, another for the shell. The
    header is prose about an entry the user knows by name; the last line is a command they
    paste. Passing one string for both meant the resync line said `skit params a & b --resync`,
    which cmd.exe and sh both read as two commands and a background job."""
    report = analysis.Report(missing=[ParamDecl(name="GONE", type="str", binding="const")])
    lines = analysis.drift_lines(report, _AWKWARD, target="a-b")

    assert _AWKWARD in lines[0]  # prose: the name as the user sees it
    assert lines[-1] == "To refresh the definitions, run: skit params a-b --resync"
    assert _AWKWARD not in lines[-1]


def test_drift_lines_falls_back_to_the_name_when_a_caller_has_only_one() -> None:
    """The default keeps the signature usable for a caller holding a single identity — the
    behaviour is unchanged from before the parameter existed, which is what makes adding it
    safe. Dropping the fallback would make the resync line say "None"."""
    report = analysis.Report(missing=[ParamDecl(name="GONE", type="str", binding="const")])

    assert analysis.drift_lines(report, "solo") == analysis.drift_lines(report, "solo", target=None)
    assert analysis.drift_lines(report, "solo")[-1].endswith("skit params solo --resync")


_DRIFTED = """# /// script
# dependencies = []
#
# [tool.skit]
# schema = 1
#
# [[tool.skit.params]]
# name = "GONE"
# kind = "const"
# type = "str"
# ///
print("hi")
"""


def test_the_drift_banner_a_run_prints_names_the_slug(tmp_path: Path) -> None:
    """The caller side of the same fix: plan_for_entry hands drift_lines both identities, so the
    banner a user actually meets on the launch menu carries a resync command that runs."""
    entry = store.add_python(_py(tmp_path, _DRIFTED), name=_AWKWARD)

    plan = flows.plan_for_entry(store.resolve(entry.slug))

    assert plan.drift_lines
    assert any(_AWKWARD in line for line in plan.drift_lines)  # prose still names the entry
    assert f"skit params {entry.slug} --resync" in plan.drift_lines[-1]
    assert f"skit params {_AWKWARD} --resync" not in plan.drift_lines[-1]


def test_the_prompt_body_drift_line_names_the_slug(tmp_path: Path) -> None:
    """A managed placeholder that left the body: the line tells you to go fix the parameters,
    and the command it hands you has to be one you can paste.

    Asserted as the WHOLE sentence, with two departed names, because that is the only shape
    that pins all of it: the wording, the separator between the names, and the identity in the
    command. A substring check passes on a line that has quietly become something else."""
    body = tmp_path / "p.prompt.md"
    body.write_text("Summarise {{topic}} for {{reader}}.\n", encoding="utf-8")
    entry = store.add_prompt(body, name=_AWKWARD)
    runner.invoke(cli.app, ["params", entry.slug, "--manage", "topic", "--manage", "reader"])
    left = "Summarise nothing.\n"
    store.resolve(entry.slug).script_path.write_text(left, encoding="utf-8")

    plan = flows.plan_for_entry(store.resolve(entry.slug))

    assert plan.drift_lines == [
        "No longer in the prompt (the value would be ignored): topic, reader — "
        f"edit the body or update parameters with: skit params {entry.slug}"
    ]
    assert _AWKWARD not in plan.drift_lines[0]
    assert plan.text == left  # ...and the plan still carries the body the run will send


def test_the_flood_cap_hint_names_the_slug(tmp_path: Path) -> None:
    """The one hint printed at ADD time, when the entry is brand new and the user has never
    typed its slug — precisely the moment they would paste what skit shows them."""
    from skit.langs.prompt.analyzer import AUTO_MANAGE_LIMIT

    body = tmp_path / "many.prompt.md"
    many = " ".join("{{h" + str(i) + "}}" for i in range(AUTO_MANAGE_LIMIT + 5))
    body.write_text(many + "\n", encoding="utf-8")

    result = runner.invoke(cli.app, ["add", str(body), "-n", _AWKWARD, "--no-input"])

    assert result.exit_code == 0, result.output
    out = " ".join(result.output.split())
    slug = store.resolve(_AWKWARD).slug
    assert "too many to manage automatically" in out
    assert f"skit params {slug} --add NAME" in out


def test_the_no_runner_refusal_names_the_entry_and_the_slug_apart(tmp_path: Path) -> None:
    """ONE msgid carrying both identities (%(name)s prose, %(target)s command). The
    non-interactive contract says this refusal must not guess a runner — so the pin command it
    offers instead is the entire recovery path, and it has to be pasteable."""
    body = tmp_path / "p.prompt.md"
    body.write_text("Say hi.\n", encoding="utf-8")
    entry = store.add_prompt(body, name=_AWKWARD)

    result = runner.invoke(cli.app, ["run", entry.slug, "--no-input"])

    assert result.exit_code == 126
    out = " ".join(result.output.split())
    assert f"No runner selected for {_AWKWARD}." in out  # prose half
    assert f"skit params {entry.slug} --runner NAME" in out  # command half


def test_the_injection_failure_resync_line_names_the_slug(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """The one hint printed from inside a RUN, when the script and its definitions have already
    stopped matching. The user is mid-launch and stuck, so the resync command is the whole exit
    — it has to be one they can paste rather than one they have to repair first."""
    from skit.langs.python import shim

    def boom(*a: object, **k: object) -> None:
        raise shim.ShimError("nope")

    text = metawriter.write_params(
        'CITY = "Taipei"\nprint(CITY)\n', [ParamDecl(name="CITY", binding="const", type="str")]
    )
    entry = store.add_python(_py(tmp_path, text), name=_AWKWARD)
    argstate.save_last(entry.slug, values={"CITY": "Kaohsiung"})
    monkeypatch.setattr(shim, "inject", boom)

    result = runner.invoke(cli.app, ["run", entry.slug, "--no-input"])

    assert result.exit_code == 125
    out = " ".join(result.output.split())
    assert f"Run `skit params {entry.slug} --resync` to fix it." in out
    assert _AWKWARD not in out


def test_the_normalize_hint_names_the_slug(tmp_path: Path) -> None:
    """The A5 amendment's own door: --normalize is the one consent-gated rewrite skit performs,
    and this hint is how a user finds it. A broken command here sends them to the shell to
    improvise on a script-rewriting flag."""
    script = tmp_path / "s.sh"
    script.write_text('DIR="$(dirname "$0")"\nOUT=out\necho "$DIR/$OUT"\n', encoding="utf-8")
    entry = store.add_script(script, kind="shell", name=_AWKWARD)

    result = runner.invoke(cli.app, ["params", entry.slug])

    assert result.exit_code == 0, result.output
    out = " ".join(result.output.split())
    assert "This script locates itself" in out
    assert f"skit params {entry.slug} --normalize NAME" in out
    assert f"skit params {_AWKWARD} --normalize" not in out


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
