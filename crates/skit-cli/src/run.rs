//! Compose the `skit run` command without exposing Clap to core crates.

mod command;

pub(crate) use command::{
    RunArgs, RunError, apply_sets, run, run_with_roots, source_text, token_context,
};
