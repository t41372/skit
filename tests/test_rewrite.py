"""rewrite.py's line-ending helpers.

`detect_newline` / `restore_newline` bracket every comment-block write path: the block
engine matches on "\\n" only, so each path folds the script to LF to run it and restores the
copy's own style before writing back — otherwise a one-checkbox edit rewrites every line of
a CRLF script. They lived in cli.py until the TUI's Script settings needed the same
discipline (it was flattening CRLF and burning non-UTF-8 bytes to U+FFFD on every save), so
they moved here, to the module that already owns source-text surgery.

The round-trip through a real script is covered by the callers' own tests; these pin the two
primitives directly.
"""

from __future__ import annotations

from skit.rewrite import detect_newline, restore_newline


def test_detect_newline_prefers_crlf_then_lone_cr_then_lf():
    assert detect_newline(b"a\r\nb\r\n") == "\r\n"
    assert detect_newline(b"a\rb\r") == "\r"
    assert detect_newline(b"a\nb\n") == "\n"
    assert detect_newline(b"no terminator at all") == "\n"
    # A mixed file: CRLF wins if any is present, so the pathological case normalizes to the
    # dominant real style rather than to LF.
    assert detect_newline(b"a\nb\r\nc") == "\r\n"


def test_restore_newline_is_a_no_op_for_lf_and_exact_otherwise():
    assert restore_newline("a\nb\n", "\n") == "a\nb\n"
    assert restore_newline("a\nb\n", "\r\n") == "a\r\nb\r\n"
    assert restore_newline("a\nb\n", "\r") == "a\rb\r"
