//! Native-binary import census compatibility metrics.

use std::{collections::BTreeMap, fs};

use crate::{
    Metric, SuiteKind, SuiteOutput, SuitePlan,
    process::{ProcessSpec, run as run_process},
    runner::{PROBE_TIMEOUT, RunContext, path_arg},
};

use super::SuiteError;

pub(super) fn run(context: &RunContext, plan: &SuitePlan) -> Result<SuiteOutput, SuiteError> {
    let mut output = SuiteOutput {
        suite: SuiteKind::Imports,
        duration_seconds: 0.0,
        metrics: BTreeMap::new(),
        skipped: Vec::new(),
        raw: BTreeMap::new(),
    };
    let first = *plan
        .library_sizes
        .first()
        .ok_or_else(|| SuiteError::Contract("imports plan needs n=0".to_owned()))?;
    let mut probes = vec![("version".to_owned(), first, vec!["--version".to_owned()])];
    probes.extend(plan.library_sizes.iter().map(|n| {
        (
            format!("list_json.n{n}"),
            *n,
            vec!["list".to_owned(), "--json".to_owned()],
        )
    }));
    for (name, n, args) in probes {
        let mut argv = vec![path_arg(&context.skit)];
        argv.extend(args);
        let process = run_process(&ProcessSpec {
            argv,
            cwd: context.workdir.clone(),
            env: context.environment(n)?,
            timeout: PROBE_TIMEOUT,
            check: true,
        })?;
        for (suffix, value) in [
            ("modules", 0.0),
            ("has_typer", 0.0),
            ("has_rich", 0.0),
            ("has_textual", 0.0),
            ("has_tree_sitter", 0.0),
        ] {
            output.metrics.insert(
                format!("imports.{name}.{suffix}"),
                Metric::single(value, if suffix == "modules" { "count" } else { "bool" }),
            );
        }
        output
            .raw
            .insert(format!("census_{name}"), serde_json::json!([]));
        output.raw.insert(
            format!("census_{name}_native"),
            serde_json::json!({
                "python_modules": [],
                "architecture": "native Rust binary; no Python module graph",
                "stdout_bytes": process.stdout.len(),
            }),
        );
    }
    output
        .raw
        .insert("importtime_top".to_owned(), serde_json::json!([]));
    output.raw.insert(
        "importtime_top_native".to_owned(),
        serde_json::json!({
            "rows": [],
            "reason": "native Rust binary has no Python import-time tree"
        }),
    );
    let artifacts = context.out_dir.join("artifacts");
    fs::create_dir_all(&artifacts).map_err(|error| {
        SuiteError::Contract(format!(
            "could not create import artifact directory {}: {error}",
            artifacts.display()
        ))
    })?;
    let importtime = artifacts.join("importtime.txt");
    fs::write(
        &importtime,
        "not applicable: native Rust binary has no Python import-time tree\n",
    )
    .map_err(|error| {
        SuiteError::Contract(format!(
            "could not write import artifact {}: {error}",
            importtime.display()
        ))
    })?;
    Ok(output)
}
