# Python test port progress

Behavioral oracle: `main@206f9ef946fc45835cb2479593794431f2620c32`.

The port does not use the existing Rust tests as evidence of parity. A migrated contract must keep
its Python test name, appear in `python-test-port-map.json`, exist as one executable non-ignored Rust
test in exactly one declared target, and pass against the Rust implementation.

## Verified first tranche

- `test_analyzer.py`: 37 executable ports.
- `test_analyzer_signals.py`: 9 executable ports.
- `test_callmatch.py`: 9 executable ports.
- `test_reconcile.py`: 14 executable report-level ports; 13 higher-layer contracts remain explicit
  deferrals and are not represented by ignored stubs.
- `test_tokens.py`: 21 executable ports across the deterministic application scanner and the
  ambient-state CLI adapter.
- `test_hermeticity.py`: 1 black-box fallback-directory isolation port.

The pinned inventory contains 175 Python test modules: 84 behavior modules with 3,018 test
definitions, 72 mutation modules with 1,010 definitions, and 19 coverage modules with 578
definitions. The machine gate rejects incomplete `done` rows, unnamed partial gaps, ignored ports,
unmapped `port_test_*.rs` files, duplicate physical targets, and target files that contain tests not
claimed by the port map.

## Execution evidence

The first independent execution on GitHub Actions completed formatting and all targeted test
binaries before the publishing step:

- `port_test_tokens`: 20 passed, 0 failed, 0 ignored;
- `port_test_reconcile`: 14 passed, 0 failed, 0 ignored;
- `port_test_tokens_ambient`: 1 passed, 0 failed, 0 ignored;
- `port_test_hermeticity`: 1 passed, 0 failed, 0 ignored;
- `python_port_manifest`: 1 passed, 0 failed, 0 ignored.

The original publishing step rejected two additional files changed by workspace-wide `rustfmt`.
Those files were existing analyzer ports, not behavior changes. Their exact `rustfmt` output is now
committed separately, so the finalization job can publish only its declared oracle, ledger, manifest,
and executable-test changes without broadening the generated-file allowlist.
