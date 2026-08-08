//! Rust composition root and command-line interface for skit.

#![forbid(unsafe_code)]

mod cli;
mod run;

pub use cli::entry;
