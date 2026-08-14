//! Exact gate-2 cleanup contract from Python v0.4 `tests/test_js_inject.py`.
#![cfg(unix)]

#[path = "support/js_inject_gate_case.rs"]
mod gate_case;

#[test]
fn test_gate2_failure_removes_the_temp_copy() {
    gate_case::run_case();
}
