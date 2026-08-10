# Python test port progress

Behavioral oracle: `main@206f9ef946fc45835cb2479593794431f2620c32`.

The port does not use the existing Rust tests as evidence of parity. A migrated contract must keep
its Python test name, appear in `python-test-port-map.json`, exist as one executable non-ignored Rust
test in exactly one declared target, and pass against the Rust implementation.

## Executable direct ports

The machine-readable map now contains 210 executable direct ports and 30 named deferrals across ten
started behavior modules. A deferral does not count as a completed test.

- `test_analyzer.py`: 37 executable ports.
- `test_analyzer_signals.py`: 9 executable ports.
- `test_argspec.py`: 34 executable ports.
- `test_argspec_click_typer.py`: 67 executable ports.
- `test_callmatch.py`: 9 executable ports.
- `test_langs.py`: 4 executable CLI ports; 17 owner-level contracts remain explicit deferrals.
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

## Argspec tranche

`test_argspec.py` and `test_argspec_click_typer.py` are complete direct ports. Their 101 Python test
names each map to one Rust test. The tests keep the Python rationale comments. The branch workflow
ran both binaries successfully:

- `port_test_argspec`: 34 passed; 0 failed; 0 ignored;
- `port_test_argspec_click_typer`: 67 passed; 0 failed; 0 ignored.

## Language and health tranche

Four direct ports from `test_langs.py` exposed two invented Rust contracts:

- `doctor` treated `uv` as required for a library that contained only non-Python entries;
- the executable parameter view did not print the Python message for an entry with no managed
  parameters.

The implementation now follows the Python oracle. An empty library or a library with a Python entry
requires `uv`. A non-empty library with no Python entry reports `uv` as not required. The executable
parameter view prints the localized no-managed-parameters message and does not advertise the
unsupported `--manage` path.

The same tranche removed unstable Windows metadata calls from the registry cache. Unix keeps the
file-identity and change-time shortcut. Windows uses stable metadata and verifies the metadata file
content hash before it trusts a cached row. This keeps correctness without `unsafe` code or unstable
standard-library APIs.

Targeted validation for commit `a6049184893a112faabc656c5f9182befbc0c5e7` passed:

- registry cache: 13 passed;
- direct `test_langs.py` ports: 4 passed;
- corrected contextual-uv product contract: 1 passed;
- shared health inspector unit contract: 1 passed;
- i18n catalog: 14 passed;
- parity manifest: 1 passed.

`python-contract-port-validation.txt` contains the exact commands and counts.

## Gate status

The branch-only workflow is read-only. It checks formatting, stable Windows compilation of
`skit-store`, registry-cache correctness, the 105 direct tests in the current argspec/language
tranches, the corrected product and health contracts, i18n, and the machine-readable port map. It
cannot commit or push.

The ordinary pull-request workflow still owns the complete workspace test, Clippy, Rustdoc,
packaging, coverage, and platform matrix. This document does not mark those gates green until their
current pull-request runs complete successfully.
