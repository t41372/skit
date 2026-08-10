//! Public-API ports of the Python v0.4 add-review Python shebang auto-pin contract.
//!
//! A versioned Python shebang seeds the editable requires-python field. This target pins only the
//! source fact itself; frontend override persistence remains a separate UI concern.

use std::path::Path;

use skit_language::{infer_kind, python_version_pin, shebang_program};

#[test]
fn test_env_python_minor_shebang_extracts_program_and_exact_minor_range() {
    let line = "#!/usr/bin/env python3.12";
    assert_eq!(shebang_program(line), Some("python3.12"));
    assert_eq!(
        python_version_pin("python3.12"),
        Some(">=3.12,<3.13".to_owned())
    );
    assert_eq!(
        infer_kind(Path::new("script"), Some(line), false),
        Some("python")
    );
}

#[test]
fn test_direct_python_minor_shebang_extracts_basename() {
    let line = "#!/opt/python/bin/python3.11";
    assert_eq!(shebang_program(line), Some("python3.11"));
    assert_eq!(
        python_version_pin("/opt/python/bin/python3.11"),
        Some(">=3.11,<3.12".to_owned())
    );
    assert_eq!(
        infer_kind(Path::new("script"), Some(line), false),
        Some("python")
    );
}

#[test]
fn test_versioned_python_micro_pin_keeps_micro_lower_bound_and_next_minor_upper_bound() {
    assert_eq!(
        python_version_pin("python3.12.4"),
        Some(">=3.12.4,<3.13".to_owned())
    );
}

#[test]
fn test_unversioned_python_and_python3_do_not_invent_requires_python_constraint() {
    for program in ["python", "python3", "/usr/bin/python3"] {
        assert_eq!(python_version_pin(program), None, "{program}");
    }
    assert_eq!(
        infer_kind(Path::new("script"), Some("#!/usr/bin/env python3"), false),
        Some("python")
    );
}

#[test]
fn test_invalid_versioned_python_names_do_not_invent_constraints() {
    for program in [
        "python3.x",
        "python3.",
        "python3.12rc1",
        "python2.7",
        "python312",
        "notpython3.12",
    ] {
        assert_eq!(python_version_pin(program), None, "{program}");
    }
}

#[test]
fn test_env_flags_are_skipped_when_locating_the_shebang_program() {
    let line = "#!/usr/bin/env -S python3.12 -I";
    assert_eq!(shebang_program(line), Some("python3.12"));
    assert_eq!(
        infer_kind(Path::new("job"), Some(line), false),
        Some("python")
    );
}

#[test]
fn test_extension_kind_still_wins_over_an_unrelated_shebang() {
    assert_eq!(
        infer_kind(Path::new("script.py"), Some("#!/usr/bin/env bash"), false),
        Some("python")
    );
}

#[test]
fn test_unknown_shebang_without_extension_stays_unknown_unless_file_is_executable() {
    let line = "#!/usr/bin/env mystery";
    assert_eq!(infer_kind(Path::new("job"), Some(line), false), None);
    assert_eq!(infer_kind(Path::new("job"), Some(line), true), Some("exe"));
}
