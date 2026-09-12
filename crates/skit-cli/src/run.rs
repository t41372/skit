//! Compose the `skit run` command without exposing Clap to core crates.

mod command;

pub(crate) use command::{
    RunArgs, RunClock, RunError, RunInvocation, RunPorts, RunServices, apply_sets, run,
    run_with_services, source_text, system_time_from_utc, token_context,
};
#[cfg(test)]
pub(crate) use command::{StageWriteFaultGuard, new_injected_file_with_allocator};
