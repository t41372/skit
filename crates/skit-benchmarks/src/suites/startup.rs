//! Process-cold, filesystem-warm startup latency.

use crate::{
    Metric, SuiteKind, SuiteOutput, SuitePlan,
    hyperfine::Case,
    runner::{RunContext, path_arg, run_hyperfine},
};

use super::SuiteError;

pub(super) fn run(context: &RunContext, plan: &SuitePlan) -> Result<SuiteOutput, SuiteError> {
    let Some(hyperfine) = &context.hyperfine else {
        return Ok(SuiteOutput::skip_all(
            SuiteKind::Startup,
            "hyperfine not found",
        ));
    };
    let _ = hyperfine;
    let Some(python) = &context.python else {
        return Ok(SuiteOutput::skip_all(
            SuiteKind::Startup,
            "python not found",
        ));
    };
    let n = *plan
        .library_sizes
        .first()
        .ok_or_else(|| SuiteError::Contract("startup plan needs n=0".to_owned()))?;
    let skit = path_arg(&context.skit);
    let cases = vec![
        Case::new(
            "startup.python",
            [path_arg(python), "-c".to_owned(), "pass".to_owned()],
        ),
        Case::new("startup.version", [skit.clone(), "--version".to_owned()]),
        Case::new("startup.help", [skit.clone(), "--help".to_owned()]),
        Case::new("startup.list", [skit.clone(), "list".to_owned()]),
        Case::new(
            "startup.list_json",
            [skit, "list".to_owned(), "--json".to_owned()],
        ),
    ];
    let run = run_hyperfine(context, plan, &cases, context.environment(n)?, "startup")?;
    let mut metrics = run.metrics;
    let mut raw = run.raw;
    // These IDs measured Python imports in v0.4. A native binary has no importable
    // Python module, so its semantic import cost and module count are exactly zero.
    metrics.insert(
        "startup.import_skit.median_ms".to_owned(),
        Metric::single(0.0, "ms"),
    );
    metrics.insert(
        "startup.import_skit_cli.median_ms".to_owned(),
        Metric::single(0.0, "ms"),
    );
    if let Some(serde_json::Value::Object(times)) = raw.get_mut("times_s") {
        times.insert("startup.import_skit".to_owned(), serde_json::json!([0.0]));
        times.insert(
            "startup.import_skit_cli".to_owned(),
            serde_json::json!([0.0]),
        );
    }
    raw.insert(
        "native_imports".to_owned(),
        serde_json::json!("not applicable: Rust binary has no Python import path"),
    );
    Ok(SuiteOutput {
        suite: SuiteKind::Startup,
        duration_seconds: 0.0,
        metrics,
        skipped: Vec::new(),
        raw,
    })
}
