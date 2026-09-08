//! Product-coupled oracles for the skit terminal walker.
//!
//! This crate is a development tool. `publish = false` keeps it inside the workspace.
//! Its rules stay visible to `cargo mutants` and to a workspace coverage run.
//! Each oracle reads real product state and names the rule that the state breaks.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

/// Walk artifacts and the environment values that scale one walk.
///
/// A bundle writer stages one directory beside its parent and renames it into place, so a reader
/// sees a complete bundle or nothing. Each environment reader returns one named value.
pub mod artifacts;
/// Named invariants over the reducer state at one walker checkpoint.
pub mod invariants;
/// The random operation model and its late binder.
///
/// One operation carries an ordinal, not a resolved target. The binder resolves the ordinal
/// against the live inventory of the current frame, so one stored trace follows whatever the
/// frame offers. The nine families and their weights, the resize shapes, the paste payloads, and
/// the profile matrix are the tables that one walk draws from.
pub mod model;
/// Keyboard and mouse parity probes over one rendered frame.
pub mod parity;

#[cfg(test)]
mod artifacts_tests;
#[cfg(test)]
mod invariants_tests;
#[cfg(test)]
mod model_tests;
#[cfg(test)]
mod parity_tests;
