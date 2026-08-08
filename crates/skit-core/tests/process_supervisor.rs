use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use skit_core::{LaunchPlan, RunError, run_launch};
use tempfile::tempdir;

fn plan(root: &Path, mode: &str) -> Result<LaunchPlan, Box<dyn std::error::Error>> {
    Ok(LaunchPlan {
        argv: vec![
            env::current_exe()?.to_string_lossy().into_owned(),
            "--exact".to_owned(),
            "supervisor_child".to_owned(),
            "--nocapture".to_owned(),
        ],
        cwd: root.to_owned(),
        env_overlay: [("SKIT_SUPERVISOR_CHILD".to_owned(), mode.to_owned())]
            .into_iter()
            .collect(),
    })
}

#[test]
fn supervisor_child() {
    let Ok(mode) = env::var("SKIT_SUPERVISOR_CHILD") else {
        return;
    };
    match mode.as_str() {
        "exit7" => process::exit(7),
        "inspect" => {
            let result = env::var("SKIT_SUPERVISOR_RESULT").unwrap_or_default();
            let overlay = env::var("SKIT_OVERLAY").unwrap_or_default();
            let cwd = env::current_dir().unwrap_or_else(|_| PathBuf::new());
            if result.is_empty()
                || fs::write(result, format!("{}\n{overlay}\n", cwd.display())).is_err()
            {
                process::exit(98);
            }
            process::exit(0);
        }
        "sleep" => {
            thread::sleep(Duration::from_secs(30));
            process::exit(0);
        }
        _ => process::exit(99),
    }
}

#[test]
fn normal_child_exit_code_is_returned() -> Result<(), Box<dyn std::error::Error>> {
    let root = tempdir()?;
    let interrupted = AtomicBool::new(false);
    assert_eq!(run_launch(&plan(root.path(), "exit7")?, &interrupted)?, 7);
    Ok(())
}

#[test]
fn cwd_and_environment_overlay_reach_the_child() -> Result<(), Box<dyn std::error::Error>> {
    let root = tempdir()?;
    let result_path = root.path().join("result.txt");
    let mut launch = plan(root.path(), "inspect")?;
    launch.env_overlay.insert(
        "SKIT_SUPERVISOR_RESULT".to_owned(),
        result_path.to_string_lossy().into_owned(),
    );
    launch
        .env_overlay
        .insert("SKIT_OVERLAY".to_owned(), "child-value".to_owned());

    let interrupted = AtomicBool::new(false);
    assert_eq!(run_launch(&launch, &interrupted)?, 0);
    let content = fs::read_to_string(result_path)?;
    let mut lines = content.lines();
    assert_eq!(lines.next(), Some(root.path().to_string_lossy().as_ref()));
    assert_eq!(lines.next(), Some("child-value"));
    Ok(())
}

#[test]
fn interruption_kills_and_reaps_before_returning_130() -> Result<(), Box<dyn std::error::Error>> {
    let root = tempdir()?;
    let interrupted = Arc::new(AtomicBool::new(false));
    let trigger = Arc::clone(&interrupted);
    let setter = thread::spawn(move || {
        thread::sleep(Duration::from_millis(100));
        trigger.store(true, Ordering::SeqCst);
    });

    let started = Instant::now();
    let code = run_launch(&plan(root.path(), "sleep")?, &interrupted)?;
    setter.join().map_err(|_| "interrupt setter panicked")?;
    assert_eq!(code, 130);
    assert!(started.elapsed() < Duration::from_secs(5));
    Ok(())
}

#[test]
fn preexisting_interrupt_does_not_spawn_the_program() -> Result<(), Box<dyn std::error::Error>> {
    let root = tempdir()?;
    let launch = LaunchPlan {
        argv: vec![root.path().join("does-not-exist").to_string_lossy().into_owned()],
        cwd: root.path().to_owned(),
        env_overlay: Default::default(),
    };
    let interrupted = AtomicBool::new(true);
    assert_eq!(run_launch(&launch, &interrupted)?, 130);
    Ok(())
}

#[test]
fn spawn_failure_is_named() -> Result<(), Box<dyn std::error::Error>> {
    let root = tempdir()?;
    let missing = root.path().join("does-not-exist");
    let launch = LaunchPlan {
        argv: vec![missing.to_string_lossy().into_owned()],
        cwd: root.path().to_owned(),
        env_overlay: Default::default(),
    };
    let interrupted = AtomicBool::new(false);
    assert!(matches!(
        run_launch(&launch, &interrupted),
        Err(RunError::Spawn { .. })
    ));
    Ok(())
}

#[cfg(unix)]
#[test]
fn posix_signal_death_is_normalized_to_shell_status() -> Result<(), Box<dyn std::error::Error>> {
    let root = tempdir()?;
    let launch = LaunchPlan {
        argv: vec![
            "sh".to_owned(),
            "-c".to_owned(),
            "kill -TERM $$".to_owned(),
        ],
        cwd: root.path().to_owned(),
        env_overlay: Default::default(),
    };
    let interrupted = AtomicBool::new(false);
    assert_eq!(run_launch(&launch, &interrupted)?, 143);
    Ok(())
}
