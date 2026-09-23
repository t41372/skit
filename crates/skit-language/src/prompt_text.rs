//! Strict text boundary for prompt payloads.

use skit_i18n::{Localize, Message};
use thiserror::Error;

/// A prompt body is not valid UTF-8.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
#[error("Prompt {path} isn't valid UTF-8 (invalid byte at offset {offset}).")]
pub struct PromptEncodingError {
    /// Source path or the stable `<stdin>` label.
    pub path: String,
    /// Offset of the first invalid byte.
    pub offset: usize,
}

impl Localize for PromptEncodingError {
    fn message(&self) -> Message {
        Message::new("Prompt {} isn't valid UTF-8 (invalid byte at offset {}).")
            .with(&self.path)
            .with(self.offset)
    }
}

/// Decode one prompt body without changing its bytes.
pub fn decode_prompt(bytes: &[u8], path: impl Into<String>) -> Result<&str, PromptEncodingError> {
    std::str::from_utf8(bytes).map_err(|error| PromptEncodingError {
        path: path.into(),
        offset: error.valid_up_to(),
    })
}
