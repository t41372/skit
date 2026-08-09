//! Typed requests for live launch-form feedback.

use std::fmt::Debug;

use serde::{Deserialize, Serialize};

/// One value whose glob matches must be counted by a filesystem adapter.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct GlobCountRequest {
    /// Directory in which the child resolves the patterns.
    pub cwd: String,
    /// POSIX-split value pieces in input order.
    pub pieces: Vec<String>,
}

/// A filesystem boundary that counts matches without putting glob rules in a frontend.
pub trait GlobCountPort: Debug {
    /// Count all expanded matches. A non-pattern piece counts as one item.
    fn count_matches(&self, request: &GlobCountRequest) -> usize;
}

/// Build a match-count request when the value contains valid glob-shaped input.
#[must_use]
pub fn glob_count_request(value: &str, cwd: &str) -> Option<GlobCountRequest> {
    if !value.contains(['*', '?', '[']) {
        return None;
    }
    let pieces = shlex::split(value)?;
    Some(GlobCountRequest {
        cwd: cwd.to_owned(),
        pieces,
    })
}
