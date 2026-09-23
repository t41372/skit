//! Mechanical port of the Python oracle module `tests/test_rewrite.py`
//! (`origin/main@206f9ef`): "rewrite.py's line-ending helpers." The comment-block engine
//! matches on `"\n"` only, so every write path folds a script to LF to run it and must
//! restore the copy's own style before writing back — otherwise a one-checkbox edit would
//! rewrite every line of a CRLF script. These two primitives detect the style and re-apply
//! it. They moved out of `cli.py` once the TUI's Script settings needed the same discipline
//! (it was flattening CRLF and burning non-UTF-8 bytes to U+FFFD on every save). Each
//! `#[test]` keeps its Python `def test_*` name and its "WHY" comment.
//!
//! Concept mapping used throughout:
//! - Python `rewrite.detect_newline(raw: bytes) -> str` (returns `"\r\n"` / `"\r"` / `"\n"`)
//!   -> `LosslessSource::from_bytes(bytes).newline_style()` (returns the typed
//!   `NewlineStyle::{CrLf, Cr, Lf}`). The Rust private `detect_newline` fn carries the same
//!   preference logic; it is observed through the public constructor + accessor.
//! - Python `rewrite.restore_newline(text: str, newline: str) -> str` -> the private Rust
//!   `restore_newlines`, observed through `LosslessSource::restore_bytes(edited)` which
//!   re-applies the source's own detected `newline_style()`. To pin a specific style this
//!   port builds the source from bytes that carry that terminator, then restores the LF text.
//!   Two notes preempt the obvious "you are not testing the primitive" objection:
//!   (a) `restore_bytes` is `decode_invalid_utf8 ∘ restore_newlines`, and the decode step is
//!   an identity for these ASCII, marker-free inputs, so only the line-ending map is exercised;
//!   (b) pinning the style by constructing from styled bytes leans on `detect_newline`, which
//!   is acceptable because `test_detect_newline_...` pins the detect step independently.
//!
//! Buckets:
//! - Bucket 1 (API EXISTS): both tests. The two Python-private line-ending primitives are
//!   exercised through the public `LosslessSource` boundary that wraps them — same crate
//!   (`skit-language`), no forbidden dependency edge. None are cross-crate, absent, or a
//!   divergence.
//! - Out of scope: the oracle module's own docstring states the round-trip through a real
//!   script is covered by the callers' tests; `apply_byte_spans` / `write_injected` and the
//!   other `rewrite.py` members are not exercised by `test_rewrite.py`, so this port does not
//!   add tests for them.

use skit_language::{LosslessSource, NewlineStyle};

/// Python `rewrite.detect_newline(raw)`, observed through the public boundary.
fn style_of(bytes: &[u8]) -> NewlineStyle {
    LosslessSource::from_bytes(bytes).newline_style()
}

/// Python `rewrite.restore_newline(edited, newline)`, observed through the public boundary.
///
/// `styled` carries the terminator that fixes the source's detected style; `edited` is the
/// LF-normalized text the engine produced. `restore_bytes` re-applies the detected style.
fn restore_with(styled: &[u8], edited: &str) -> Vec<u8> {
    LosslessSource::from_bytes(styled).restore_bytes(edited)
}

#[test]
fn test_detect_newline_prefers_crlf_then_lone_cr_then_lf() {
    assert_eq!(style_of(b"a\r\nb\r\n"), NewlineStyle::CrLf);
    assert_eq!(style_of(b"a\rb\r"), NewlineStyle::Cr);
    assert_eq!(style_of(b"a\nb\n"), NewlineStyle::Lf);
    assert_eq!(style_of(b"no terminator at all"), NewlineStyle::Lf);
    // A mixed file: CRLF wins if any is present, so the pathological case normalizes to the
    // dominant real style rather than to LF.
    assert_eq!(style_of(b"a\nb\r\nc"), NewlineStyle::CrLf);
}

#[test]
fn test_restore_newline_is_a_no_op_for_lf_and_exact_otherwise() {
    // LF is an identity (after the fold every terminator is already a lone "\n"); CRLF and CR
    // map each folded "\n" back exactly.
    assert_eq!(restore_with(b"a\nb\n", "a\nb\n"), b"a\nb\n");
    assert_eq!(restore_with(b"a\r\nb\r\n", "a\nb\n"), b"a\r\nb\r\n");
    assert_eq!(restore_with(b"a\rb\r", "a\nb\n"), b"a\rb\r");
}
