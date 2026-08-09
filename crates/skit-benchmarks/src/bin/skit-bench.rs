//! Rust front door for the performance-evaluation pipeline.

use std::{
    fs,
    hint::black_box,
    path::{Path, PathBuf},
    str::FromStr as _,
    time::Instant,
};

use anyhow::{Context as _, Result};
use clap::{Parser, Subcommand};
use skit_benchmarks::{
    BenchmarkProfile, Results,
    budget::{evaluate, load_budgets, propose, render_report},
    compare::{compare, render_markdown},
    dataset::{DEFAULT_SEED, DEFAULT_STATE_FRACTION, generate},
    pipeline::{ExecutionRequest, execute},
    report::summarize_directory,
};

const DEFAULT_BUDGETS: &str = "benchmarks/budgets.toml";

#[derive(Debug, Parser)]
#[command(
    name = "skit-bench",
    about = "Run skit's performance evaluation pipeline"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Generate one deterministic benchmark library.
    Datasets {
        /// Number of library entries.
        #[arg(long)]
        n: usize,
        /// Destination directory.
        #[arg(long)]
        out: PathBuf,
        /// Reproducibility seed.
        #[arg(long, default_value_t = DEFAULT_SEED)]
        seed: u64,
        /// Fraction of entries with remembered state.
        #[arg(long, default_value_t = DEFAULT_STATE_FRACTION)]
        state_fraction: f64,
    },
    /// Run one profile from dataset generation through summary publication.
    Run {
        /// Profile name: pr, full, or compare.
        #[arg(long)]
        profile: String,
        /// Durable artifact directory.
        #[arg(long, default_value = ".bench")]
        out: PathBuf,
        /// Budget contract.
        #[arg(long, default_value = DEFAULT_BUDGETS)]
        budgets: PathBuf,
        /// Harness checkout. Footprint builds this checkout.
        #[arg(long, default_value = ".")]
        repo: PathBuf,
        /// Checkout whose Git identity the artifact records.
        #[arg(long)]
        measured_repo: Option<PathBuf>,
        /// Product binary to measure.
        #[arg(long)]
        skit_binary: PathBuf,
    },
    /// Rebuild results.json and results.md from one completed run.
    Summarize {
        /// Completed run directory.
        bench_dir: PathBuf,
        /// Budget contract.
        #[arg(long, default_value = DEFAULT_BUDGETS)]
        budgets: PathBuf,
    },
    /// Evaluate one result artifact against the budget contract.
    Check {
        /// Result artifact.
        results: PathBuf,
        /// Budget contract.
        #[arg(long, default_value = DEFAULT_BUDGETS)]
        budgets: PathBuf,
        /// Print a refreshed ratchet proposal.
        #[arg(long)]
        propose: bool,
        /// Permit a proposal to widen a ratchet ceiling.
        #[arg(long)]
        allow_regression: bool,
        /// Fail when no enforced row applies.
        #[arg(long)]
        require_enforced: bool,
    },
    /// Render a warn-only A/B comparison.
    Compare {
        /// Base result artifact.
        base: PathBuf,
        /// Head result artifact.
        head: PathBuf,
    },
    /// Run one fresh-process internal probe.
    #[command(hide = true)]
    Probe {
        #[command(subcommand)]
        probe: Probe,
    },
}

#[derive(Debug, Subcommand)]
enum Probe {
    /// Parse one generated analyzer source.
    Analyze {
        /// Language identifier.
        #[arg(long)]
        kind: String,
        /// Generated source file.
        #[arg(long)]
        source: PathBuf,
    },
    /// Drive a headless terminal session.
    Tui {
        /// Expected entry count.
        #[arg(long)]
        entries: usize,
        /// One-character filter input.
        #[arg(long)]
        probe_char: char,
    },
}

fn main() {
    let code = match dispatch(Cli::parse()) {
        Ok(code) => code,
        Err(error) => {
            eprintln!("skit-bench: {error:#}");
            1
        }
    };
    std::process::exit(code);
}

fn dispatch(cli: Cli) -> Result<i32> {
    match cli.command {
        Command::Datasets {
            n,
            out,
            seed,
            state_fraction,
        } => {
            let manifest = generate(
                &out,
                isize::try_from(n).context("dataset size does not fit isize")?,
                seed,
                state_fraction,
            )?;
            println!(
                "generated {} entries in {}",
                manifest.n,
                manifest.root.display()
            );
            Ok(0)
        }
        Command::Run {
            profile,
            out,
            budgets,
            repo,
            measured_repo,
            skit_binary,
        } => {
            let profile = BenchmarkProfile::from_str(&profile)?;
            let budgets = read_budgets(&budgets)?;
            let harness = std::env::current_exe().context("could not resolve benchmark harness")?;
            let results = execute(ExecutionRequest {
                profile,
                bench_dir: &out,
                repo_root: &repo,
                measured_repo: measured_repo.as_deref(),
                skit: &skit_binary,
                harness: &harness,
                budgets: Some(&budgets),
            })?;
            println!(
                "results: {} ({} metrics)",
                out.join("results.json").display(),
                results.metrics.len()
            );
            Ok(0)
        }
        Command::Summarize { bench_dir, budgets } => {
            let budgets = read_budgets(&budgets)?;
            let results = summarize_directory(&bench_dir, Some(&budgets))?;
            println!(
                "results: {} ({} metrics)",
                bench_dir.join("results.json").display(),
                results.metrics.len()
            );
            Ok(0)
        }
        Command::Check {
            results,
            budgets,
            propose: should_propose,
            allow_regression,
            require_enforced,
        } => {
            let results = read_results(&results)?;
            let budgets = read_budgets(&budgets)?;
            if should_propose {
                print!("{}", propose(&budgets, &results, allow_regression)?);
                return Ok(0);
            }
            let report = evaluate(&budgets, &results);
            print!("{}", render_report(&report));
            if !report.failures().is_empty()
                || (require_enforced && report.enforced_evaluated() == 0)
            {
                if require_enforced && report.enforced_evaluated() == 0 {
                    eprintln!("check: zero applicable enforced rows were evaluated");
                }
                Ok(1)
            } else {
                Ok(0)
            }
        }
        Command::Compare { base, head } => {
            let base = read_results(&base)?;
            let head = read_results(&head)?;
            print!("{}", render_markdown(&base, &head, &compare(&base, &head)));
            Ok(0)
        }
        Command::Probe { probe } => run_probe(probe),
    }
}

fn run_probe(probe: Probe) -> Result<i32> {
    match probe {
        Probe::Analyze { kind, source } => {
            let started = Instant::now();
            let text = fs::read_to_string(&source)
                .with_context(|| format!("could not read {}", source.display()))?;
            black_box(skit_language::detect_candidates(&kind, black_box(&text)));
            println!("{}", started.elapsed().as_secs_f64() * 1_000.0);
        }
        Probe::Tui {
            entries,
            probe_char,
        } => {
            let output = skit_benchmarks::tui_probe::run(entries, probe_char)?;
            println!("{}", serde_json::to_string(&output)?);
        }
    }
    Ok(0)
}

fn read_budgets(path: &Path) -> Result<Vec<skit_benchmarks::budget::Budget>> {
    let text = fs::read_to_string(path)
        .with_context(|| format!("could not read budget contract {}", path.display()))?;
    load_budgets(&text).map_err(Into::into)
}

fn read_results(path: &Path) -> Result<Results> {
    let text = fs::read_to_string(path)
        .with_context(|| format!("could not read result artifact {}", path.display()))?;
    Results::from_json(&text).map_err(Into::into)
}
