//! Content hashing through the maintained RustCrypto implementation.

use sha2::{Digest as _, Sha256};

/// Return the byte-exact SHA-256 spelling used by the Python implementation.
#[must_use]
pub fn content_hash(bytes: &[u8]) -> String {
    format!("sha256:{}", hex::encode(Sha256::digest(bytes)))
}
