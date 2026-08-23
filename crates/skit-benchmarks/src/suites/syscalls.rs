//! Linux system-call census of the warm JSON list path.

use std::{collections::BTreeMap, fs};

use crate::{
    Metric, SuiteKind, SuiteOutput, SuitePlan,
    parsers::{FILE_OP_SYSCALLS, NETWORK_SYSCALLS, count_group, strace_counts},
    process::{ProcessSpec, run as run_process},
    runner::{PROBE_TIMEOUT, RunContext, path_arg},
};

use super::SuiteError;

pub(super) fn run(context: &RunContext, plan: &SuitePlan) -> Result<SuiteOutput, SuiteError> {
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (context, plan);
        return Ok(SuiteOutput::skip_all(SuiteKind::Syscalls, "not Linux"));
    }
    #[cfg(target_os = "linux")]
    run_linux(context, plan)
}

#[cfg(target_os = "linux")]
fn run_linux(context: &RunContext, plan: &SuitePlan) -> Result<SuiteOutput, SuiteError> {
    let Some(strace) = &context.strace else {
        return Ok(SuiteOutput::skip_all(
            SuiteKind::Syscalls,
            "strace not found",
        ));
    };
    let [n] = plan.library_sizes.as_slice() else {
        return Err(SuiteError::Contract(
            "syscalls IDs are unsuffixed; plan must carry one N".to_owned(),
        ));
    };
    let table = context.workdir.join(format!("strace_n{n}.txt"));
    run_process(&ProcessSpec {
        argv: vec![
            path_arg(strace),
            "-f".to_owned(),
            "-c".to_owned(),
            "-o".to_owned(),
            table.display().to_string(),
            path_arg(&context.skit),
            "list".to_owned(),
            "--json".to_owned(),
        ],
        cwd: context.workdir.clone(),
        env: context.environment(*n)?,
        timeout: PROBE_TIMEOUT,
        check: true,
    })?;
    let text = fs::read_to_string(&table).map_err(|error| {
        SuiteError::Contract(format!("could not read {}: {error}", table.display()))
    })?;
    let counts = strace_counts(&text)?;
    Ok(SuiteOutput {
        suite: SuiteKind::Syscalls,
        duration_seconds: 0.0,
        metrics: BTreeMap::from([
            (
                "syscalls.list_json.file_ops".to_owned(),
                Metric::single(count_group(&counts, FILE_OP_SYSCALLS) as f64, "count"),
            ),
            (
                "syscalls.list_json.network".to_owned(),
                Metric::single(count_group(&counts, NETWORK_SYSCALLS) as f64, "count"),
            ),
        ]),
        skipped: Vec::new(),
        raw: BTreeMap::from([(
            format!("n{n}"),
            serde_json::to_value(counts).expect("counts serialize"),
        )]),
    })
}
