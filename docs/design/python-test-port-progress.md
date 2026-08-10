# Python test port progress

Behavioral oracle: `main@206f9ef946fc45835cb2479593794431f2620c32`.

The port does not use the existing Rust tests as evidence of parity. A migrated contract must keep
its Python test name, appear in `python-test-port-map.json`, exist as one executable non-ignored Rust
test in exactly one declared target, and pass against the Rust implementation.

## Executable direct ports

The machine-readable map now contains 206 executable direct ports and 13 named deferrals across
nine started behavior modules. The 13 deferrals belong only to higher-layer `test_reconcile.py`
operations. They do not count as completed tests.

- `test_analyzer.py`: 37 executable ports.
- `test_analyzer_signals.py`: 9 executable ports.
- `test_argspec.py`: 34 executable ports.
- `test_argspec_click_typer.py`: 67 executable ports.
- `test_callmatch.py`: 9 executable ports.
- `test_reconcile.py`: 14 executable report-level ports; 13 higher-layer contracts remain explicit
  deferrals.
- `test_tokens.py`: 21 executable ports across the deterministic application scanner and the
  ambient-state CLI adapter.
- `test_path_type.py`: 14 executable ports across CLI and TUI ownership boundaries.
- `test_hermeticity.py`: 1 black-box fallback-directory isolation port.

The pinned inventory contains 175 Python test modules: 84 behavior modules with 3,018 test
definitions, 72 mutation modules with 1,010 definitions, and 19 coverage modules with 578
definitions. The machine gate rejects incomplete `done` rows, unnamed partial gaps, ignored ports,
unmapped `port_test_*.rs` files, duplicate physical targets, and target files that contain tests not
claimed by the port map.

## Argspec tranche execution evidence

The branch-only GitHub Actions job used Rust 1.97.1 and ran the two new test binaries before its
format-publish step. The step published commit `1bc27d89871b6336e74d8acd3c6ebd87b21a0895`, so both preceding test commands completed
successfully:

- `port_test_argspec`: 34 Python-named tests passed;
- `port_test_argspec_click_typer`: 67 Python-named tests passed.

The two published blobs are byte-identical to the locally reviewed `rustfmt` output. No assertion or
implementation was changed by that formatting commit.

## Repository-wide gate status

The ordinary PR matrix exposed a pre-existing Windows build blocker in the Rust rewrite before any
test could run. Rust 1.97.1 rejects the unstable standard-library calls
`MetadataExt::volume_serial_number`, `MetadataExt::file_index`, and `MetadataExt::change_time` in
`crates/skit-store/src/mutations/registry.rs`. This failure is unrelated to the argspec ports, but it
means the full Windows workspace gate is not yet green.

The branch-only verification workflow is read-only. It checks formatting, the 101 argspec tests,
the machine-readable port manifest, and the existing path ports. It cannot modify the branch or
hide a failing oracle.
