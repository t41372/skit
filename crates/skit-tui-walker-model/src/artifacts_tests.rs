use std::{fs, num::NonZeroUsize, path::PathBuf, process::Command};

use crate::artifacts::{
    CASES_VARIABLE, FAILURE_CAST_NAME, LIVENESS_EVERY_VARIABLE, LivenessSampling,
    PROFILES_VARIABLE, ProfileMode, RECORD_SUCCESS_VARIABLE, REPRO_NAME, REPRO_VARIABLE,
    ReproSource, STEPS_VARIABLE, SUCCESS_CAST_NAME, SuccessRecording, liveness_every, positive,
    publish_bundle, record_success, replay, repro_source, sampling, selected, walk_cases,
    walk_profiles, walk_steps, write_failure_bundle, write_success_bundle,
};

fn repro(marker: &str) -> serde_json::Value {
    serde_json::json!({"version": 1, "marker": marker})
}

fn entry_names(directory: &std::path::Path) -> Vec<String> {
    let mut names = fs::read_dir(directory)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    names.sort();
    names
}

// --------------------------------------------------------------------------
// bundle writers (legacy `driver.rs:3929`, `:3964`)
// --------------------------------------------------------------------------

#[test]
fn failure_artifacts_keep_each_complete_bundle_separate() {
    let parent = tempfile::tempdir().unwrap();
    let directory = parent.path().join("ui-walker-artifacts");

    let first = write_failure_bundle(&directory, &repro("first"), b"first cast").unwrap();
    let shrunk = write_failure_bundle(&directory, &repro("shrunk"), b"shrunk cast").unwrap();

    assert_ne!(first, shrunk);
    for bundle in [&first, &shrunk] {
        assert!(
            bundle
                .file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("failure-")
        );
    }
    assert_eq!(
        fs::read(first.join(FAILURE_CAST_NAME)).unwrap(),
        b"first cast"
    );
    assert_eq!(
        fs::read(shrunk.join(FAILURE_CAST_NAME)).unwrap(),
        b"shrunk cast"
    );
    let stored: serde_json::Value =
        serde_json::from_slice(&fs::read(shrunk.join(REPRO_NAME)).unwrap()).unwrap();
    assert_eq!(stored, repro("shrunk"));

    let names = entry_names(&directory);
    assert_eq!(
        names.len(),
        2,
        "the staged directories are renamed: {names:?}"
    );
    assert!(names.iter().all(|name| !name.starts_with('.')));
}

#[test]
fn requested_success_artifact_contains_the_replay_and_cast_in_one_bundle() {
    let directory = tempfile::tempdir().unwrap();
    let bundle =
        write_success_bundle(directory.path(), &repro("passed"), b"successful cast").unwrap();

    assert!(
        bundle
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("success-")
    );
    assert_eq!(
        fs::read(bundle.join(SUCCESS_CAST_NAME)).unwrap(),
        b"successful cast"
    );
    let stored: serde_json::Value =
        serde_json::from_slice(&fs::read(bundle.join(REPRO_NAME)).unwrap()).unwrap();
    assert_eq!(stored, repro("passed"));
}

#[test]
fn a_bundle_without_a_cast_holds_the_replay_alone() {
    let directory = tempfile::tempdir().unwrap();
    let bundle = write_failure_bundle(directory.path(), &repro("no cast"), b"").unwrap();

    assert_eq!(entry_names(&bundle), vec![REPRO_NAME.to_owned()]);
}

#[test]
fn a_bundle_writer_that_cannot_create_its_parent_writes_nothing() {
    let parent = tempfile::tempdir().unwrap();
    let blocker = parent.path().join("blocker");
    fs::write(&blocker, b"not a directory").unwrap();
    let directory = blocker.join("ui-walker-artifacts");

    let failure = write_failure_bundle(&directory, &repro("blocked"), b"cast").unwrap_err();

    assert!(!failure.is_empty());
    assert!(!directory.exists());
    assert_eq!(entry_names(parent.path()), vec!["blocker".to_owned()]);
}

#[test]
fn a_bundle_that_cannot_reach_its_place_leaves_no_residue() {
    let directory = tempfile::tempdir().unwrap();
    let staged = directory.path().join(".failure-blocked");
    fs::create_dir(&staged).unwrap();
    fs::write(staged.join(REPRO_NAME), b"{}").unwrap();
    // A file in the parent position makes the final path unreachable on every platform.
    let blocker = directory.path().join("blocked-parent");
    fs::write(&blocker, b"another artifact").unwrap();
    let destination = blocker.join("failure-blocked");

    let failure = publish_bundle(&staged, &destination).unwrap_err();

    assert!(!failure.is_empty());
    assert!(!staged.exists(), "the staged directory stayed behind");
    assert_eq!(fs::read(&blocker).unwrap(), b"another artifact");
    assert_eq!(
        entry_names(directory.path()),
        vec!["blocked-parent".to_owned()],
        "the artifact directory keeps no staged residue"
    );
}

#[test]
fn a_bundle_that_reaches_its_place_keeps_the_staged_content() {
    let directory = tempfile::tempdir().unwrap();
    let staged = directory.path().join(".failure-ready");
    fs::create_dir(&staged).unwrap();
    fs::write(staged.join(REPRO_NAME), b"{}").unwrap();
    let destination = directory.path().join("failure-ready");

    assert_eq!(publish_bundle(&staged, &destination), Ok(()));

    assert!(!staged.exists());
    assert_eq!(entry_names(&destination), vec![REPRO_NAME.to_owned()]);
}

// --------------------------------------------------------------------------
// positive integer values
// --------------------------------------------------------------------------

#[test]
fn a_positive_value_takes_the_environment_number() {
    assert_eq!(
        positive(Some("5".to_owned()), 16_u32, "cases must be positive"),
        5
    );
    assert_eq!(
        positive(Some("120".to_owned()), 100_usize, "steps must be positive"),
        120
    );
}

#[test]
fn a_zero_or_invalid_value_takes_the_default() {
    assert_eq!(
        positive(Some("0".to_owned()), 16_u32, "cases must be positive"),
        16
    );
    assert_eq!(
        positive(Some("many".to_owned()), 16_u32, "cases must be positive"),
        16
    );
    assert_eq!(positive(None, 16_u32, "cases must be positive"), 16);
}

#[test]
#[should_panic(expected = "SKIT_WALKER_CASES must be greater than zero")]
fn a_zero_default_refuses_the_walk() {
    let _ = positive(
        Some("0".to_owned()),
        0_u32,
        "SKIT_WALKER_CASES must be greater than zero",
    );
}

#[test]
fn walk_cases_and_walk_steps_read_their_own_variables() {
    assert_eq!(CASES_VARIABLE, "SKIT_WALKER_CASES");
    assert_eq!(STEPS_VARIABLE, "SKIT_WALKER_STEPS");
    assert_eq!(walk_cases(7), 7);
    assert_eq!(walk_steps(13), 13);
}

// --------------------------------------------------------------------------
// named choices
// --------------------------------------------------------------------------

#[test]
fn a_named_choice_takes_the_first_value_by_default() {
    assert_eq!(selected(&["0", "1"], None, "must be 0 or 1"), 0);
    assert_eq!(
        selected(
            &["bounded", "complete"],
            None,
            "must be bounded or complete"
        ),
        0
    );
}

#[test]
fn a_named_choice_takes_the_position_of_the_environment_value() {
    assert_eq!(
        selected(&["0", "1"], Some("1".to_owned()), "must be 0 or 1"),
        1
    );
    assert_eq!(
        selected(
            &["bounded", "complete"],
            Some("complete".to_owned()),
            "must be bounded or complete"
        ),
        1
    );
}

#[test]
#[should_panic(expected = "SKIT_WALKER_RECORD_SUCCESS must be 0 or 1")]
fn a_named_choice_refuses_an_unknown_value() {
    let _ = selected(
        &["0", "1"],
        Some("yes".to_owned()),
        "SKIT_WALKER_RECORD_SUCCESS must be 0 or 1",
    );
}

#[test]
fn walk_profiles_and_record_success_read_their_own_variables() {
    assert_eq!(PROFILES_VARIABLE, "SKIT_WALKER_PROFILES");
    assert_eq!(RECORD_SUCCESS_VARIABLE, "SKIT_WALKER_RECORD_SUCCESS");
    assert_eq!(walk_profiles(), ProfileMode::Bounded);
    assert_eq!(record_success(), SuccessRecording::Skip);
}

// --------------------------------------------------------------------------
// liveness sampling and the stored replay
// --------------------------------------------------------------------------

#[test]
fn liveness_sampling_reads_a_positive_interval_only() {
    assert_eq!(sampling(None), LivenessSampling::Never);
    assert_eq!(sampling(Some("0".to_owned())), LivenessSampling::Never);
    assert_eq!(sampling(Some("often".to_owned())), LivenessSampling::Never);
    assert_eq!(
        sampling(Some("4".to_owned())),
        LivenessSampling::Every(NonZeroUsize::new(4).unwrap())
    );
    assert_eq!(LIVENESS_EVERY_VARIABLE, "SKIT_WALKER_LIVENESS_EVERY");
    assert_eq!(liveness_every(), LivenessSampling::Never);
}

#[test]
fn a_stored_replay_names_one_file() {
    assert_eq!(replay(None), ReproSource::Fresh);
    assert_eq!(
        replay(Some("/tmp/repro.json".to_owned())),
        ReproSource::Replay(PathBuf::from("/tmp/repro.json"))
    );
    assert_eq!(REPRO_VARIABLE, "SKIT_WALKER_REPRO");
    assert_eq!(repro_source(), ReproSource::Fresh);
}

// --------------------------------------------------------------------------
// the readers with the variables set
//
// `std::env::set_var` is unsafe in edition 2024 and this crate forbids unsafe code. A child
// process carries the values instead. The parent starts this test binary again and asks for one
// ignored helper test by its exact name. The helper reads the variables that the parent set.
// --------------------------------------------------------------------------

const WALKER_VARIABLES: &[&str] = &[
    CASES_VARIABLE,
    STEPS_VARIABLE,
    PROFILES_VARIABLE,
    RECORD_SUCCESS_VARIABLE,
    LIVENESS_EVERY_VARIABLE,
    REPRO_VARIABLE,
];

const REPLAY_PATH: &str = "/tmp/skit-walker-child-repro.json";

fn walker_child(test: &str) -> Command {
    let binary = std::env::current_exe().expect("the test binary has a path");
    let mut command = Command::new(binary);
    command
        .arg(test)
        .arg("--exact")
        .arg("--nocapture")
        .arg("--include-ignored")
        .arg("--test-threads=1");
    for variable in WALKER_VARIABLES {
        command.env_remove(variable);
    }
    command
}

#[test]
#[ignore = "the parent test starts this helper in a child process with the variables set"]
fn every_reader_takes_its_environment_value() {
    assert_eq!(walk_cases(16), 7);
    assert_eq!(walk_steps(100), 42);
    assert_eq!(walk_profiles(), ProfileMode::Complete);
    assert_eq!(record_success(), SuccessRecording::Record);
    assert_eq!(
        liveness_every(),
        LivenessSampling::Every(NonZeroUsize::new(9).unwrap())
    );
    assert_eq!(
        repro_source(),
        ReproSource::Replay(PathBuf::from(REPLAY_PATH))
    );
}

#[test]
#[ignore = "the parent test starts this helper in a child process with an invalid value"]
fn record_success_reads_an_invalid_environment_value() {
    assert_eq!(record_success(), SuccessRecording::Record);
}

#[test]
fn the_readers_take_the_values_of_a_set_environment() {
    let output = walker_child("artifacts_tests::every_reader_takes_its_environment_value")
        .env(CASES_VARIABLE, "7")
        .env(STEPS_VARIABLE, "42")
        .env(PROFILES_VARIABLE, "complete")
        .env(RECORD_SUCCESS_VARIABLE, "1")
        .env(LIVENESS_EVERY_VARIABLE, "9")
        .env(REPRO_VARIABLE, REPLAY_PATH)
        .output()
        .expect("the child test process starts");

    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    assert!(
        output.status.success(),
        "the child failed: stdout={stdout} stderr={stderr}"
    );
    assert!(
        stdout.contains("test result: ok. 1 passed"),
        "the child ran another number of tests: {stdout}"
    );
}

#[test]
fn an_unknown_record_success_value_stops_the_walk() {
    let output = walker_child("artifacts_tests::record_success_reads_an_invalid_environment_value")
        .env(RECORD_SUCCESS_VARIABLE, "2")
        .output()
        .expect("the child test process starts");

    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    assert!(
        !output.status.success(),
        "the child accepted the value: stdout={stdout} stderr={stderr}"
    );
    assert!(
        stderr.contains("SKIT_WALKER_RECORD_SUCCESS must be 0 or 1"),
        "the child named another reason: stderr={stderr}"
    );
    assert!(
        stdout.contains("test result: FAILED. 0 passed; 1 failed"),
        "the child ran another number of tests: {stdout}"
    );
}
