//! Reversible source text for parser and comment-block edit boundaries.
//!
//! Rust strings cannot contain Python's surrogate-escape code points. This adapter
//! uses a source-specific marker that is absent from both the valid source text and
//! reserved edit values. Invalid UTF-8 bytes become marker-delimited byte tokens.
//! Valid Unicode stays unchanged, so parsers see the original text rather than a
//! lossy replacement view.

use skit_domain::parameters::{ParamDecl, ParameterValue};

use crate::{LanguageError, has_uv_metadata_block, write_managed_params, write_uv_metadata};

const MARKER_SCALAR: char = '\u{f0000}';

/// The physical newline convention of one source byte buffer.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum NewlineStyle {
    /// Line feed (`\n`).
    #[default]
    Lf,
    /// Carriage return and line feed (`\r\n`).
    CrLf,
    /// Carriage return (`\r`).
    Cr,
}

/// A reversible, LF-normalized text view over arbitrary source bytes.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LosslessSource {
    normalized: String,
    newline: NewlineStyle,
    marker: String,
}

impl LosslessSource {
    /// Create a reversible view over arbitrary source bytes.
    #[must_use]
    pub fn from_bytes(bytes: &[u8]) -> Self {
        Self::from_bytes_avoiding(bytes, &[])
    }

    /// Create a reversible view whose private marker is absent from future edit text.
    ///
    /// Pass every user-controlled string that an editor can insert. This prevents a
    /// generated value from looking like an encoded invalid byte.
    #[must_use]
    pub fn from_bytes_avoiding(bytes: &[u8], reserved: &[&str]) -> Self {
        let newline = detect_newline(bytes);
        let normalized_bytes = normalize_newlines(bytes);
        let marker = select_marker(&normalized_bytes, reserved);
        let normalized = encode_invalid_utf8(&normalized_bytes, &marker);
        Self {
            normalized,
            newline,
            marker,
        }
    }

    /// Return the reversible LF-normalized parser and editor view.
    #[must_use]
    pub fn normalized_text(&self) -> &str {
        &self.normalized
    }

    /// Return the source's physical newline convention.
    #[must_use]
    pub const fn newline_style(&self) -> NewlineStyle {
        self.newline
    }

    /// Restore edited text to source bytes.
    ///
    /// `edited` must derive from [`Self::normalized_text`]. Editors that insert
    /// user text must reserve it through [`Self::from_bytes_avoiding`].
    #[must_use]
    pub fn restore_bytes(&self, edited: &str) -> Vec<u8> {
        restore_newlines(decode_invalid_utf8(edited, &self.marker), self.newline)
    }
}

/// Edit a managed comment block without changing unrelated source bytes.
pub fn write_managed_params_bytes(
    kind: &str,
    bytes: &[u8],
    params: &[ParamDecl],
) -> Result<Vec<u8>, LanguageError> {
    let reserved = parameter_text(params);
    let source = LosslessSource::from_bytes_avoiding(bytes, &reserved);
    let written = write_managed_params(kind, source.normalized_text(), params)?;
    Ok(source.restore_bytes(&written))
}

/// Edit Python inline dependency metadata without changing unrelated source bytes.
pub fn write_uv_metadata_bytes(
    bytes: &[u8],
    dependencies: &[String],
    requires_python: &str,
) -> Result<Vec<u8>, LanguageError> {
    let mut reserved = dependencies.iter().map(String::as_str).collect::<Vec<_>>();
    reserved.push(requires_python);
    let source = LosslessSource::from_bytes_avoiding(bytes, &reserved);
    let written = write_uv_metadata(source.normalized_text(), dependencies, requires_python)?;
    Ok(source.restore_bytes(&written))
}

/// Return whether arbitrary Python source bytes contain a PEP 723 fence.
///
/// This predicate does not parse the block body. An invalid existing block is
/// still authoritative for onboarding and dependency edits.
#[must_use]
pub fn has_uv_metadata_block_bytes(bytes: &[u8]) -> bool {
    let source = LosslessSource::from_bytes(bytes);
    has_uv_metadata_block(source.normalized_text())
}

fn parameter_text(params: &[ParamDecl]) -> Vec<&str> {
    let mut output = Vec::new();
    for parameter in params {
        output.extend([
            parameter.name.as_str(),
            parameter.prompt.as_str(),
            parameter.help.as_str(),
            parameter.env_source.as_str(),
            parameter.flag.as_str(),
            parameter.action.as_str(),
            parameter.env_target.as_str(),
        ]);
        output.extend(parameter.choices.iter().map(String::as_str));
        if let Some(ParameterValue::String(value)) = &parameter.default {
            output.push(value);
        }
    }
    output
}

fn detect_newline(bytes: &[u8]) -> NewlineStyle {
    if bytes.windows(2).any(|pair| pair == b"\r\n") {
        NewlineStyle::CrLf
    } else if bytes.contains(&b'\r') {
        NewlineStyle::Cr
    } else {
        NewlineStyle::Lf
    }
}

fn normalize_newlines(bytes: &[u8]) -> Vec<u8> {
    let mut output = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'\r' {
            if bytes.get(index.saturating_add(1)) == Some(&b'\n') {
                index = index.saturating_add(1);
            }
            output.push(b'\n');
        } else {
            output.push(bytes[index]);
        }
        index = index.saturating_add(1);
    }
    output
}

fn restore_newlines(bytes: Vec<u8>, newline: NewlineStyle) -> Vec<u8> {
    if newline == NewlineStyle::Lf {
        return bytes;
    }
    let mut output = Vec::with_capacity(bytes.len());
    for byte in bytes {
        if byte == b'\n' {
            if newline == NewlineStyle::CrLf {
                output.push(b'\r');
                output.push(b'\n');
            } else {
                output.push(b'\r');
            }
        } else {
            output.push(byte);
        }
    }
    output
}

fn select_marker(bytes: &[u8], reserved: &[&str]) -> String {
    let mut marker = MARKER_SCALAR.to_string();
    while bytes
        .windows(marker.len())
        .any(|window| window == marker.as_bytes())
        || reserved.iter().any(|value| value.contains(&marker))
    {
        marker.push(MARKER_SCALAR);
    }
    marker
}

fn encode_invalid_utf8(bytes: &[u8], marker: &str) -> String {
    let mut output = String::new();
    let mut remaining = bytes;
    while !remaining.is_empty() {
        match std::str::from_utf8(remaining) {
            Ok(valid) => {
                output.push_str(valid);
                break;
            }
            Err(error) => {
                let valid_end = error.valid_up_to();
                output.push_str(
                    std::str::from_utf8(&remaining[..valid_end])
                        .expect("the UTF-8 error identifies a valid prefix"),
                );
                let invalid_len = error
                    .error_len()
                    .unwrap_or_else(|| remaining.len().saturating_sub(valid_end));
                for byte in &remaining[valid_end..valid_end.saturating_add(invalid_len)] {
                    push_byte_token(&mut output, marker, *byte);
                }
                remaining = &remaining[valid_end.saturating_add(invalid_len)..];
            }
        }
    }
    output
}

fn push_byte_token(output: &mut String, marker: &str, byte: u8) {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    output.push_str(marker);
    output.push('~');
    output.push(char::from(HEX[usize::from(byte >> 4)]));
    output.push(char::from(HEX[usize::from(byte & 0x0f)]));
    output.push('~');
    output.push_str(marker);
}

fn decode_invalid_utf8(text: &str, marker: &str) -> Vec<u8> {
    let mut output = Vec::with_capacity(text.len());
    let mut remaining = text;
    while let Some(index) = remaining.find(marker) {
        output.extend_from_slice(&remaining.as_bytes()[..index]);
        let after_marker = &remaining[index + marker.len()..];
        if let Some((byte, consumed)) = byte_token(after_marker, marker) {
            output.push(byte);
            remaining = &after_marker[consumed..];
        } else {
            output.extend_from_slice(marker.as_bytes());
            remaining = after_marker;
        }
    }
    output.extend_from_slice(remaining.as_bytes());
    output
}

fn byte_token(text: &str, marker: &str) -> Option<(u8, usize)> {
    let prefix = text.as_bytes().get(..4)?;
    if prefix[0] != b'~'
        || prefix[3] != b'~'
        || !text
            .get(4..)
            .is_some_and(|suffix| suffix.starts_with(marker))
    {
        return None;
    }
    let high = hex_value(prefix[1])?;
    let low = hex_value(prefix[2])?;
    Some(((high << 4) | low, 4 + marker.len()))
}

const fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        _ => None,
    }
}
