"""Language-neutral 1:1 call-site matching between stored parameter definitions and the current
source.

Both Python's `input()` and shell's `read` are *ordered, optionally-prompted* value reads, and a
stored value must follow its *question* (the prompt) rather than its bare position across source
edits. That decision is identical in either language, so it lives here as a pure function over
`(order, prompt)` pairs — nothing about it is Python- or shell-specific. Shared by every analyzable
language's reconcile (drift detection) and injector (actual injection) so both agree on exactly the
same call site for a given definition.
"""

from __future__ import annotations


def match_calls(
    stored: list[tuple[int, str]], current: list[tuple[int, str]]
) -> dict[int, tuple[int, bool]]:
    """Bind each stored input (its recorded ``order``, ``prompt``) to a call site in the CURRENT
    source (3a). Shared by reconcile (drift detection) and shim (actual injection) so both agree on
    exactly the same call site for a given definition -- the reason this lives in a neutral module
    rather than in either caller (A2).

    A value must follow its *question*, not its position: keying purely by ``order`` (B1's original
    design) breaks the instant a source edit inserts or deletes an *earlier* input() call, silently
    shifting every later position -- a secret-marked definition can then attach to a different
    prompt with no warning at all. So the literal prompt text is tried first (it survives a shift);
    bare position is only a fallback, and is trusted as "no news" only when neither side has a
    prompt to compare in the first place (a dynamic/absent prompt, or a spec stored before 3a) --
    that case is no worse than the pre-3a behaviour, so it must not manufacture a new warning.

    Returns ``{stored_order: (current_order, ambiguous)}``. A stored order absent from the result
    could not be matched at all (genuinely gone -- the caller reports it as missing). ``ambiguous``
    is True when position had to be trusted *despite* having a prompt to check -- either the prompt
    no longer appears anywhere (likely edited/renamed) or it now matches more than one call site (two
    prompts collide) -- both are exactly the silent-rebind risk this function exists to surface, so
    callers must turn it into a visible warning rather than silently treating it as "ok".

    Two passes: exact prompt matches are resolved first and their current-order claimed, so a
    *different* stored entry's positional fallback can never be handed a call site some other
    definition already owns by an exact prompt match -- e.g. deleting input #1 entirely (its prompt
    now matches nothing) must not let it fall back onto position 0, when input #2's own prompt has
    already, and correctly, claimed position 0 for itself. Without this, the deleted entry would
    silently "recover" a value onto a call site someone else already owns.

    The exact pass itself must also be 1:1, not just enforced against the fallback pass: two or more
    STORED entries can legitimately share the identical literal prompt (a retry pattern like two
    `input("Go? ")` calls, both managed). If the current source now has exactly one call site with
    that prompt (the user deleted one of the two calls), every one of those stored entries would
    otherwise resolve its *own* "unique candidate" check independently and all exact-match onto the
    same current order -- silently binding two different definitions to one call site. Downstream,
    reconcile would call all of them "ok" (no warning at all) and shim would splice two replacements
    over the same `input` callee span, corrupting the injected copy into unparsable source. So the
    exact pass claims its current-order as it goes: the first stored entry (in the order given) that
    uniquely resolves a prompt wins that current order outright, and any later stored entry whose own
    "unique" candidate has *already* been claimed loses the exact match and falls through to the
    positional-fallback pass below -- where it is correctly reported ``missing`` (its bare position no
    longer exists either) or flagged ``ambiguous`` (a different call now sits at that position), but
    never silently double-bound.
    """
    current_by_order = dict(current)
    by_prompt: dict[str, list[int]] = {}
    for order, prompt in current:
        if prompt:
            by_prompt.setdefault(prompt, []).append(order)

    exact: dict[int, int] = {}
    claimed: set[int] = set()
    _match_prompt_multisets(stored, by_prompt, exact, claimed)
    for order, prompt in stored:
        if order in exact:
            continue
        if prompt:
            candidates = by_prompt.get(prompt, [])
            if len(candidates) == 1 and candidates[0] not in claimed:
                exact[order] = candidates[0]
                claimed.add(candidates[0])

    out: dict[int, tuple[int, bool]] = {}
    for order, prompt in stored:
        if order in exact:
            out[order] = (exact[order], False)
            continue
        # No exact prompt match (no prompt to compare, the prompt matches nothing anymore, it
        # collides across multiple call sites, or its one candidate was already claimed by another
        # stored entry's exact match): fall back to position, but never onto a call site an exact
        # match already claimed, and flag it as ambiguous unless there was never a prompt to check
        # in the first place (not a new risk, see the module-level note above).
        if order in current_by_order and order not in claimed:
            out[order] = (order, bool(prompt))
    return out


def _match_prompt_multisets(
    stored: list[tuple[int, str]],
    by_prompt: dict[str, list[int]],
    exact: dict[int, int],
    claimed: set[int],
) -> None:
    """Multiset pass: when the stored and current sides have the SAME number of call
    sites for a prompt, pair them in positional order. A retry pattern — two identical
    `input("Go? ")` calls, both managed — is a stable shape, and without this pass the
    per-entry uniqueness rule would flag it as a rebind on every run, forever (resync
    can't fix what isn't drift)."""
    stored_by_prompt: dict[str, list[int]] = {}
    for order, prompt in stored:
        if prompt:
            stored_by_prompt.setdefault(prompt, []).append(order)
    for prompt, stored_orders in stored_by_prompt.items():
        current_orders = by_prompt.get(prompt, [])
        if len(stored_orders) > 1 and len(current_orders) == len(stored_orders):
            for stored_order, current_order in zip(
                sorted(stored_orders), sorted(current_orders), strict=True
            ):
                exact[stored_order] = current_order
                claimed.add(current_order)
