//! Mechanical port of `tests/test_rewrite.py` from Python `main@206f9ef`.
//!
//! Python's private newline primitives map to the public lossless-source boundary:
//! `detect_newline(bytes)` -> `LosslessSource::newline_style()` and
//! `restore_newline(text, style)` -> `LosslessSource::restore_bytes(text)` for a source
//! carrying that exact physical newline style.

use skit_language::{LosslessSource, NewlineStyle};

fn detected(bytes: &[u8]) -> NewlineStyle {
    LosslessSource::from_bytes(bytes).newline_style()
}

fn restored(text: &str, style: NewlineStyle) -> Vec<u8> {
    let seed: &[u8] = match style {
        NewlineStyle::Lf => b"seed\n",
        NewlineStyle::CrLf => b"seed\r\n",
        NewlineStyle::Cr => b"seed\r",
    };
    let source = LosslessSource::from_bytes(seed);
    assert_eq!(source.newline_style(), style);
    source.restore_bytes(text)
}

#[test]
fn test_detect_newline_prefers_crlf_then_lone_cr_then_lf() {
    assert_eq!(detected(b"a\r\nb\r\n"), NewlineStyle::CrLf);
    assert_eq!(detected(b"a\rb\r"), NewlineStyle::Cr);
    assert_eq!(detected(b"a\nb\n"), NewlineStyle::Lf);
    assert_eq!(detected(b"no terminator at all"), NewlineStyle::Lf);
    // A mixed file: CRLF wins if any is present, so the pathological case normalizes to the
    // dominant real style rather than to LF.
    assert_eq!(detected(b"a\nb\r\nc"), NewlineStyle::CrLf);
}

#[test]
fn test_restore_newline_is_a_no_op_for_lf_and_exact_otherwise() {
    assert_eq!(restored("a\nb\n", NewlineStyle::Lf), b"a\nb\n");
    assert_eq!(restored("a\nb\n", NewlineStyle::CrLf), b"a\r\nb\r\n");
    assert_eq!(restored("a\nb\n", NewlineStyle::Cr), b"a\rb\r");
}
