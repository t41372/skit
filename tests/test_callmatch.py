"""Direct unit tests for callmatch.match_calls — the language-neutral 1:1 binding of stored
(order, prompt) input definitions onto the current source's call sites.

match_calls is a pure function ``(stored, current) -> {stored_order: (current_order, ambiguous)}``,
shared by reconcile (drift detection) and the injectors (actual value injection) so both agree on
exactly the same call site for a given definition. These tests pin the two-pass behaviour the
callers depend on: the equal-count multiset pass, its claim bookkeeping (a positional fallback must
never recover a site an exact/multiset match already owns), and the prompt-vs-position fallback with
its ambiguous flag. tests/test_reconcile.py and tests/test_analyzer.py exercise the same function
through its callers; this file drives it directly.
"""

from __future__ import annotations

from skit.callmatch import match_calls


def test_equal_count_duplicate_prompts_bind_in_positional_order():
    # A retry pattern: two managed input("Go? ") calls, both still present. Stored and current have
    # the SAME number of call sites for the identical prompt, so the multiset pass pairs them in
    # sorted-position order rather than flagging the shape as a rebind on every run. Neither is
    # ambiguous — the pairing is exact.
    stored = [(0, "Go? "), (1, "Go? ")]
    current = [(0, "Go? "), (1, "Go? ")]
    assert match_calls(stored, current) == {0: (0, False), 1: (1, False)}


def test_duplicate_prompt_gone_from_source_falls_back_to_position_ambiguous():
    # Two stored specs share the identical literal prompt (a retry pair), but that prompt now
    # appears NOWHERE in the current source (both calls were renamed/edited). The multiset pass must
    # not fire (no current call sites carry the prompt), so both entries fall back to bare position
    # and are flagged ambiguous — the prompt vanished, which is exactly the silent-rebind risk the
    # caller must turn into a warning. (Also proves the multiset pass reads an EMPTY candidate list,
    # never a None default, when a duplicated prompt is absent from the current source.)
    stored = [(0, "Go? "), (1, "Go? ")]
    current = [(0, "Other: "), (1, "Another: ")]
    assert match_calls(stored, current) == {0: (0, True), 1: (1, True)}


def test_promptless_entry_cannot_recover_a_site_the_multiset_pass_claimed():
    # A retry pair input("Go? ") stored at orders 1 and 2, plus a THIRD managed input with no
    # recorded prompt (prompt="") at order 0. In the current source one earlier call was deleted, so
    # the retry pair now sits at current orders 0 and 1; the multiset pass binds the pair there and
    # CLAIMS both those current orders. The prompt-less entry at order 0 has only its bare position
    # to go on — and position 0 is now owned by the retry pair, so it must come back MISSING (absent
    # from the result), never silently recover a call site another definition already claimed.
    stored = [(1, "Go? "), (2, "Go? "), (0, "")]
    current = [(0, "Go? "), (1, "Go? ")]
    assert match_calls(stored, current) == {1: (0, False), 2: (1, False)}


def test_unique_prompt_after_a_multiset_match_still_resolves():
    # Three stored inputs: a retry pair input("A") at orders 0,1 (resolved by the multiset pass) and
    # a distinct input("B") at order 2. In the current source a call was inserted before "B",
    # shifting it from position 2 to 3 while the "A" pair stays at 0,1. The per-entry uniqueness pass
    # must keep scanning PAST the already-resolved multiset entries to bind "B" to its shifted site
    # by prompt — if it stopped at the first resolved entry, "B" would be reported missing even
    # though its prompt uniquely identifies its new position.
    stored = [(0, "A"), (1, "A"), (2, "B")]
    current = [(0, "A"), (1, "A"), (3, "B")]
    assert match_calls(stored, current) == {0: (0, False), 1: (1, False), 2: (3, False)}


def test_single_shared_prompt_resolves_by_uniqueness_not_multiset():
    # Exactly ONE stored entry carries a given prompt: the multiset pass (which only handles 2+
    # stored sites for one prompt) must leave it alone, and the per-entry uniqueness pass resolves it
    # by following the prompt to its shifted position. A new input() was inserted before the managed
    # one, so its bare position moved 0 -> 1, but the prompt still uniquely identifies it: bound to
    # current order 1, not ambiguous.
    stored = [(0, "Password: ")]
    current = [(0, "Username: "), (1, "Password: ")]
    assert match_calls(stored, current) == {0: (1, False)}


def test_no_recorded_prompt_falls_back_to_position_silently():
    # A legacy/dynamic-prompt entry (prompt="") has no stronger signal than position; resolving by
    # position is NOT a new risk introduced by prompt-matching, so it must bind silently
    # (ambiguous=False), preserving pre-prompt behaviour.
    assert match_calls([(0, "")], [(0, "Anything: ")]) == {0: (0, False)}


def test_renamed_prompt_with_a_call_still_at_position_is_flagged_ambiguous():
    # The stored prompt no longer appears anywhere (renamed), but a call still exists at the stored
    # position: fall back to position AND flag it, so the caller surfaces a rebind warning rather
    # than silently trusting a value onto a different question.
    assert match_calls([(0, "Old: ")], [(0, "New: ")]) == {0: (0, True)}


def test_missing_when_neither_prompt_nor_position_resolves():
    # The prompt matches nothing and the stored bare position no longer exists either: the entry is
    # genuinely gone and must be absent from the result (the caller reports it missing).
    assert match_calls([(2, "Gone: ")], [(0, "Other: ")]) == {}


def test_current_site_with_dynamic_prompt_is_ignored_for_prompt_matching():
    # A current call site with no literal prompt (prompt="", e.g. input(greeting)) carries no text to
    # key on, so it is excluded from the prompt index: the stored "Name: " entry must resolve to the
    # literal-prompt site at order 1, never the dynamic one at order 0.
    stored = [(0, "Name: ")]
    current = [(0, ""), (1, "Name: ")]
    assert match_calls(stored, current) == {0: (1, False)}
