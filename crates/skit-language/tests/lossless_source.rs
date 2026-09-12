use skit_domain::parameters::{ParamDecl, ParameterBinding, ParameterDelivery};
use skit_language::{
    LosslessSource, NewlineStyle, has_uv_metadata_block_bytes, managed_params,
    write_managed_params_bytes, write_uv_metadata_bytes,
};

const MARKER_COLLISION: &str = "\u{f0000}~80~\u{f0000}";

#[test]
fn arbitrary_invalid_bytes_round_trip_without_replacement() {
    let raw = b"alpha\x80\xff\xf0\x9fomega\n";
    let source = LosslessSource::from_bytes(raw);

    assert!(!source.normalized_text().contains('\u{fffd}'));
    assert_eq!(source.restore_bytes(source.normalized_text()), raw);
}

#[test]
fn every_non_ascii_byte_round_trips_when_utf8_runs_are_mixed() {
    let raw = (0x80..=0xff)
        .flat_map(|byte| [byte, b'|'])
        .collect::<Vec<_>>();
    let source = LosslessSource::from_bytes(&raw);

    assert_eq!(source.restore_bytes(source.normalized_text()), raw);
}

#[test]
fn valid_unicode_invalid_runs_and_a_real_escape_marker_round_trip_together() {
    let mut raw = format!("台北 {MARKER_COLLISION} 😀").into_bytes();
    raw.extend_from_slice(b"\x80middle\xf0\x9f\n");
    let source = LosslessSource::from_bytes(&raw);

    assert!(
        source
            .normalized_text()
            .contains(&format!("台北 {MARKER_COLLISION} 😀"))
    );
    assert_eq!(source.restore_bytes(source.normalized_text()), raw);
}

#[test]
fn newline_style_is_typed_and_restored_after_lf_normalized_editing() {
    for (raw, style) in [
        (b"a\nb\n".as_slice(), NewlineStyle::Lf),
        (b"a\r\nb\r\n".as_slice(), NewlineStyle::CrLf),
        (b"a\rb\r".as_slice(), NewlineStyle::Cr),
    ] {
        let source = LosslessSource::from_bytes(raw);
        assert_eq!(source.newline_style(), style);
        assert!(!source.normalized_text().contains('\r'));
        assert_eq!(source.restore_bytes(source.normalized_text()), raw);
    }
}

#[test]
fn managed_comment_block_edit_preserves_crlf_and_unrelated_invalid_bytes() {
    let raw = b"#!/bin/sh\r\nWIDTH=800\r\nprintf '\xff\\n'\r\n";
    let mut declaration = ParamDecl::new("WIDTH");
    declaration.binding = ParameterBinding::Const;
    declaration.delivery = ParameterDelivery::Inject;

    let written = write_managed_params_bytes("shell", raw, &[declaration]).unwrap();

    assert!(written.windows(2).any(|pair| pair == b"\r\n"));
    assert!(
        !written
            .windows(2)
            .any(|pair| pair[0] != b'\r' && pair[1] == b'\n')
    );
    assert!(written.contains(&0xff));
    assert!(written.ends_with(b"WIDTH=800\r\nprintf '\xff\\n'\r\n"));
}

#[test]
fn generated_metadata_cannot_collide_with_the_escape_marker_protocol() {
    let mut declaration = ParamDecl::new("VALUE");
    declaration.binding = ParameterBinding::Const;
    declaration.delivery = ParameterDelivery::Inject;
    declaration.prompt = MARKER_COLLISION.to_owned();

    let written = write_managed_params_bytes("python", b"VALUE = 'ok'\n", &[declaration]).unwrap();
    let text = std::str::from_utf8(&written).unwrap();
    let fields = managed_params("python", text);

    assert_eq!(fields[0].prompt, MARKER_COLLISION);
}

#[test]
fn uv_metadata_edit_uses_the_same_lossless_boundary() {
    let raw = b"#!/usr/bin/env python3\r\nprint('\xff')\r\n";
    let written = write_uv_metadata_bytes(raw, &["httpx>=0.28".to_owned()], ">=3.12").unwrap();

    assert!(written.contains(&0xff));
    assert!(
        written
            .windows(b"httpx>=0.28".len())
            .any(|row| row == b"httpx>=0.28")
    );
    assert!(written.ends_with(b"print('\xff')\r\n"));
}

#[test]
fn a_malformed_existing_fence_is_detected_in_non_utf8_source_bytes() {
    let raw = b"# /// script\r\n# dependencies = [\r\n# ///\r\nprint('\xff')\r\n";

    assert!(has_uv_metadata_block_bytes(raw));
    assert!(write_uv_metadata_bytes(raw, &["rich".to_owned()], ">=3.12").is_err());
}
