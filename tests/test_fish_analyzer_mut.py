"""Mutation-kill pins for ``skit.langs.fish.analyzer``.

Each test exercises a real, observable behaviour of the fish hand scanner through a real code
path (the analyzer's own functions and the public ``analyze`` surface), pinning a specific
surviving mutant. Companion to ``tests/test_fish.py`` (kept separate so parallel mutant-kill
work never collides with the main suite).

Two scanner mutants replace a loop counter (``j += 1`` / ``i += 1``) with an assignment that
makes the scanner fail to terminate; the analyzer is documented as **total**, so those are
pinned by asserting the call both terminates (within a bounded wait) and returns the right
value. ``_run_bounded`` runs the call on a daemon thread and reports whether it finished — a
non-terminating mutant leaves it alive.
"""

from __future__ import annotations

import threading

from skit.analysis import Candidate
from skit.langs.fish import analyzer as fa


def cands(src: str) -> dict[str, Candidate]:
    return {c.name: c for c in fa.analyze(src).candidates}


def _run_bounded(fn, *args, timeout: float = 3.0):
    """Run ``fn(*args)`` on a daemon thread; return ``(finished, result)``. A mutant that turns
    a loop counter into a constant assignment spins forever and leaves ``finished`` False."""
    box: dict[str, object] = {}

    def go() -> None:
        box["r"] = fn(*args)

    t = threading.Thread(target=go, daemon=True)
    t.start()
    t.join(timeout)
    return (not t.is_alive()), box.get("r")


# ---------------------------------------------------------------- _classify_set


def test_classify_set_prefix_run_terminates_and_is_not_a_set():
    # Two leading conjunctions with no `set` -> None. The `j += 1` counter must advance past
    # every prefix; a `j = 1` mutant re-reads words[1] forever.
    finished, result = _run_bounded(fa._classify_set, ["and", "and"])
    assert finished
    assert result is None


# ---------------------------------------------------------------- _clobbered_names


def test_nested_statement_does_not_stop_the_clobber_scan():
    # A depth>0 statement sits between the idiom and a later top-level `set PORT 9090`. The scan
    # must `continue` past the nested line to still record the clobber (which suppresses PORT);
    # a `break` there would stop early, leave PORT un-clobbered, and wrongly emit it.
    src = "set -q PORT; or set PORT 8080\nfunction f\necho hi\nend\nset PORT 9090\n"
    assert cands(src) == {}


# ---------------------------------------------------------------- _code


def test_code_joins_stripped_lines_with_a_single_newline():
    assert fa._code("a\nb") == "a\nb"


# ---------------------------------------------------------------- _dequote


def test_dequote_single_quote_open_not_at_start_terminates():
    # The `'` opens at index 2; the post-quote `i += 1` must step forward. An `i = 1` reset
    # walks backwards and reprocesses the tail.
    assert fa._dequote("aa'") == "aa"


def test_dequote_single_quote_trailing_backslash_is_total():
    # Unterminated single quote ending in a lone backslash: no next char, so the backslash is
    # literal. A widened escape window (`i - 1 < n` / `i + 1 <= n`) indexes past the end.
    assert fa._dequote("'\\") == "\\"


def test_dequote_single_quote_escaped_backslash_before_close():
    # `\'` inside single quotes yields a literal `'`; a narrowed window (`i + 2 < n`) misses it.
    assert fa._dequote("'\\'") == "'"


def test_dequote_single_quote_close_advances_by_one():
    # `''a` = empty single-quoted segment then a bare `a`. Skipping two past the close eats `a`.
    assert fa._dequote("''a") == "a"


def test_dequote_double_quote_open_not_at_start_terminates():
    assert fa._dequote('aa"') == "aa"


def test_dequote_double_quote_trailing_backslash_is_total():
    assert fa._dequote('"\\') == "\\"


def test_dequote_double_quote_escaped_quote_before_close():
    assert fa._dequote('"\\"') == '"'


def test_dequote_double_quote_escaped_backslash_is_recognised():
    # `\\` inside double quotes collapses to one backslash; the escape set must contain `"\\"`.
    assert fa._dequote('"\\\\') == "\\"


def test_dequote_double_quote_close_advances_by_one():
    assert fa._dequote('""a') == "a"


def test_dequote_bare_backslash_escapes_a_final_char():
    # Outside quotes, `\a` (a at the last index) escapes to `a`; `i + 2 < n` refuses it.
    assert fa._dequote("\\a") == "a"


# ---------------------------------------------------------------- _envdefault_candidates


def test_first_half_must_be_a_query_not_just_any_set():
    # `and set X 1` is a conditional assignment, not a `set -q` query, so it can't be the idiom's
    # first half. (`not is_query or name is None` must stay `or`, not `and`.)
    assert cands("and set X 1\nor set X 2\n") == {}


def test_query_below_top_level_is_not_paired_with_a_following_top_level_set():
    # `set -q X` inside a function (depth 1) must be rejected by the depth guard even when the
    # next recorded statement (`or set X 5`) is back at top level.
    assert cands("function f\nset -q X\nend\nor set X 5\n") == {}


def test_conditional_half_must_not_itself_be_a_query():
    # `or set -q X 5` is a query, so it cannot serve as the idiom's assignment half.
    assert cands("set -q X\nor set -q X 5\n") == {}


def test_idiom_after_a_query_with_a_plain_next_still_scans_on():
    # `set -q X` followed by a non-set `echo hi` fails the pairing; the loop must `continue` so a
    # later valid idiom is still found. A `break` would drop PORT.
    src = "set -q X\necho hi\nset -q PORT; or set PORT 8080\n"
    assert cands(src)["PORT"].default == 8080


def test_name_mismatch_pair_does_not_stop_the_scan():
    # `set -q X; or set Y 1` has mismatched names; the loop must `continue` to the next idiom.
    src = "set -q X; or set Y 1\nset -q PORT; or set PORT 8080\n"
    assert cands(src)["PORT"].default == 8080


def test_underscore_name_skip_does_not_stop_the_scan():
    # A leading-underscore idiom is skipped, but the scan must `continue` to reach PORT.
    src = "set -q _X; or set _X 1\nset -q PORT; or set PORT 8080\n"
    assert cands(src)["PORT"].default == 8080


def test_multi_token_default_is_space_joined():
    # `or set MSG hello world` -> the default joins the value tokens with a single space.
    assert cands("set -q MSG; or set MSG hello world\n")["MSG"].default == "hello world"


def test_candidate_records_the_query_statement_line_number():
    # Idiom on physical line 2 -> lineno 2 (distinguishes lineno=lineno from None and from the
    # field default 0).
    assert cands("\nset -q PORT; or set PORT 8080\n")["PORT"].lineno == 2


# ---------------------------------------------------------------- _is_query


def test_long_flag_containing_q_is_not_a_query():
    # Only the exact `--query` (or a short cluster) is a query; the `not startswith("--")` guard
    # keeps a long flag like `--quiet` from matching on its inner `q`.
    assert fa._is_query(["--quiet"]) is False


# ---------------------------------------------------------------- _logical_lines


def test_three_trailing_backslashes_continue_the_line():
    # An odd count (3) is a continuation: two literal backslashes + one marker. `% 3` would treat
    # 3 as non-continuation and split the line.
    assert fa._logical_lines("a" + "\\" * 3 + "\nb") == [("a" + "\\" * 2 + "b", 1)]


def test_continuation_drops_only_the_final_backslash():
    # `ab\` continues onto `c` as `abc`; dropping `combined[:1]` instead of `[:-1]` gives `ac`.
    assert fa._logical_lines("ab\\\nc") == [("abc", 1)]


# ---------------------------------------------------------------- _statements_with_depth


def test_nested_block_depth_is_tracked_by_magnitude():
    # The idiom sits inside the outer `if` (depth 1) after the inner block closes. Miscounting the
    # opener increment (`= 1` / `-= 1`) or the `end` decrement (`- 2`) drops it to depth 0 and
    # wrongly emits P.
    src = "if true\nif true\necho inner\nend\nset -q P; or set P 1\nend\n"
    assert cands(src) == {}


# ---------------------------------------------------------------- _strip_comment


def test_strip_comment_skips_past_a_closing_quote_by_exactly_two():
    # `"\\" #` : the escaped backslash, then the quote closes, then ` #` is a comment. Skipping
    # three inside the quote runs past the closing `"` and never strips the comment.
    assert fa._strip_comment('"\\\\" #') == '"\\\\" '


def test_strip_comment_open_quote_hides_a_hash():
    # An (unterminated) quote holds the `#` literal; the `ch == quote` close test must stay `==`.
    assert fa._strip_comment("' #") == "' #"


def test_strip_comment_double_quote_hides_a_hash():
    assert fa._strip_comment('" #') == '" #'


def test_strip_comment_unterminated_quote_terminates():
    # A lone opening quote: the post-open `i += 1` must advance; an `i -= 1` reset loops forever.
    finished, result = _run_bounded(fa._strip_comment, "'")
    assert finished
    assert result == "'"


def test_strip_comment_close_quote_advances_by_one():
    # `'' #` : the empty quoted pair closes, then ` #` is a comment. Skipping two past the open
    # misses the close and keeps the `#` "inside" the quote.
    assert fa._strip_comment("'' #") == "'' "


def test_strip_comment_backslash_escapes_a_quote_outside_quotes():
    # `\' #` : the backslash escapes the `'` (no quote opens), so ` #` is a real comment. If the
    # backslash check breaks (`ch == "XX\\XX"`) the `'` opens a quote and the comment survives.
    assert fa._strip_comment("\\' #") == "\\' "


def test_strip_comment_backslash_escape_skips_exactly_two():
    # `\ #` : the backslash escapes the space; the `#` (after a space) is a comment. Over-skipping
    # (`i += 3`) or `break`ing jumps past the `#` and leaves it unstripped.
    assert fa._strip_comment("\\ #") == "\\ "


def test_strip_comment_hash_needs_a_preceding_space_and_stays_total():
    # `a#` : the `#` follows a non-space, so it is not a comment. The word-boundary test reads the
    # PRECEDING char (`line[i - 1]`); reading `line[i + 1]` runs off the end.
    assert fa._strip_comment("a#") == "a#"


# ---------------------------------------------------------------- _tokenize


def test_tokenize_bare_prefix_before_quote_stays_one_token():
    # `a'` : the bare `a` and the opening quote belong to one token; `cur = ch` would drop `a`.
    assert fa._tokenize("a'") == ["a'"]


def test_tokenize_escaped_quote_inside_quotes_does_not_close():
    # `'\';` : `\'` keeps the single quote open, so the trailing `;` is literal, not a separator.
    assert fa._tokenize("'\\';") == ["'\\';"]


def test_tokenize_unterminated_quote_trailing_backslash_is_total():
    # `'\` : backslash at the last index, still inside the quote — the escape window must not read
    # past the end (`i - 1 < n` / `i + 1 <= n` would).
    assert fa._tokenize("'\\") == ["'\\"]


def test_tokenize_quoted_escape_continues_the_inner_scan():
    # `'\a;` : after consuming the escaped `a`, the inner scan must `continue` (not `break`), so
    # the `;` stays inside the unterminated quoted token.
    assert fa._tokenize("'\\a;") == ["'\\a;"]


def test_tokenize_bare_trailing_backslash_is_total():
    # A lone bare backslash is literal; the escape window must not index line[i + 1] past the end.
    assert fa._tokenize("\\") == ["\\"]


def test_tokenize_bare_backslash_escapes_a_semicolon():
    # `\;` : the backslash escapes the `;`, keeping it in one token; `i + 2 < n` refuses the
    # escape and splits on the `;`.
    assert fa._tokenize("\\;") == ["\\;"]
