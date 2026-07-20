"""Mutation-kill tests for langs/python/metawriter.py — plain-text `[tool.skit]` writes.

These pin the default comment leader ("#") and the exact comment prefix ("# ") that the private
helpers strip and build, through direct calls that rely on those defaults."""

from __future__ import annotations

from skit import pep723
from skit.langs.python import metawriter


def test_drop_synthetic_separator_defaults_the_comment_leader_to_hash():
    # `_drop_synthetic_separator`'s `leader` defaults to "#". Called WITHOUT an explicit leader on
    # a "#"-comment block, it must locate that block and strip inject_block's synthetic blank-line
    # separator, returning the block sitting flush against the following code. A wrong default
    # ("XX#XX") would build a block prefix that is not present in `base`, so the internal
    # `base.index(block_only)` lookup would raise instead of returning the cleaned text.
    block = pep723.build_block([], leader="#")
    base = block + "\n" + "print(1)\n"  # inject_block's synthetic separator present
    original = "print(1)\n"
    assert metawriter._drop_synthetic_separator(base, original) == block + "print(1)\n"


def test_strip_comment_prefix_defaults_leader_and_strips_a_single_space():
    # `_strip_comment_prefix` defaults `leader` to "#" and removes the "# " prefix (hash + exactly
    # ONE space). Called without a leader, "# hello" must become "hello": a wrong default leader
    # ("XX#XX") leaves the line untouched ("# hello"), and a wrong separator (leader + "XX XX")
    # falls through to the bare-"#" branch and leaves the leading space (" hello").
    assert metawriter._strip_comment_prefix("# hello") == "hello"
    # The bare-leader branch (no following space) strips just the leader.
    assert metawriter._strip_comment_prefix("#bare") == "bare"
