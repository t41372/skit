//! Rust composition root and command-line interface for skit.

#![forbid(unsafe_code)]

mod cli;
mod library;
mod run;

pub use cli::entry;
#[doc(hidden)]
pub use library::library_surface;
