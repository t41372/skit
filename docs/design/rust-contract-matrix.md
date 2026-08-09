# Rust compatibility matrix

The latest Python development revision on `main` is the behavioral oracle, not the version 0.4.0
release tag. This review is pinned to `origin/main@206f9ef946fc45835cb2479593794431f2620c32`.
Version 0.5.0 can add capabilities, but it cannot remove behavior from that revision or replace it
with a shortcut. This table records both the required contract and its executable evidence. `In
progress` is a release blocker; it is not a permitted behavior change.

| Contract | Status | Rust evidence and pinned Python-main oracle |
| --- | --- | --- |
| Metadata, open kinds, unknown TOML, bytes, permissions, identity, and CAS | In progress | `skit-domain/tests/contract.rs`, `skit-store/tests/mutations*.rs`; pinned-main store tests |
| Reads never migrate metadata, state, config, or registry projections | In progress | `skit-store/tests/registry_fast_read.rs`, `form_state_store.rs`, `config_store.rs` |
| Stable management exit 1/2 and run exit 2/125/126/127 contracts | In progress | `skit-cli/tests/v040_compatibility.rs`, `run_cli.rs`; pinned-main `test_cli.py` and command tests |
| Stable JSON records for list/show/params/deps/config/runner/preset/doctor | In progress | `skit-cli/tests/v040_compatibility.rs`; two-version golden command outputs |
| Presets, exact optional last-run snapshots, remembered values, and secrets | In progress | `skit-application/tests/form_state*.rs`, `skit-store/tests/form_state_*.rs` |
| Runner seeds, raw rows, reason codes, malformed containers, duplicates, and CAS | In progress | `skit-store/tests/config_store.rs`, `skit-cli/tests/v040_compatibility.rs`; pinned-main prompt tests |
| Python semantic analysis, argparse, Click, Typer, reconciliation, and injection | In progress | `skit-language/tests`; pinned-main analyzer/argspec/reconcile/shim tests and corpus |
| Shell/Fish semantic analysis, CLI reflection, reconciliation, injection, normalization | In progress | `skit-language/tests`; pinned-main shell/fish tests and corpus |
| JS/TS/PowerShell semantic analysis, CLI reflection, reconciliation, and injection | In progress | `skit-language/tests`; pinned-main JS/PowerShell tests and corpus |
| Prompt/command Unicode placeholders, reserved names, raw substitution, and extra argv | In progress | `skit-language/tests`, `skit-runtime/tests`; pinned-main prompt/launcher tests |
| Dependency detection, authoritative inline metadata, source order, and project compatibility | In progress | `skit-language/tests`, `skit-runtime/tests`; pinned-main dependency tests |
| uv consent and one-time mirror onboarding | In progress | CLI PTY/application port tests; pinned-main uv/config onboarding tests |
| Typed form plan: bool/choice/number/path/list/secret/default/help/required/provenance | In progress | `skit-form` plan tests and pinned-main form/widget tests |
| Library activity order, subsequence search, complete detail, rerun, help, and search workflow | In progress | `skit-ui/tests/reducer.rs`, `skit-tui/tests/render.rs`; pinned-main TUI tests |
| Add lanes, drafts, kind picker, analysis review, edit/reanalyse, and atomic commit | In progress | application/UI/TUI workflow tests; pinned-main add-lane and review tests |
| Run, preset, token, environment, file, runner, and default form commands | In progress | application/UI/TUI workflow tests; pinned-main form and prompt TUI tests |
| Settings, preferences, health, runner manager, Agent Skill, and dirty guards | In progress | application/UI/TUI workflow tests; pinned-main settings/health/runner tests |
| Every advertised TUI command has keyboard and mouse positive tests at all size tiers | In progress | Ratatui `TestBackend` and reducer command-registry tests |
| Complete en/zh-CN/zh-TW catalog and stable machine English | In progress | `skit-i18n/tests/catalog.rs`, crate localization tests |
| PyPI/Maturin wheel, `uv tool`, archives, security, coverage, docs, benchmarks | In progress | release gates and CI smoke tests |
| Future Tauri uses the same application/form/UI state and command registry | In progress | serializable `skit-ui` round-trip and frontend-parity tests |

The matrix is a release contract. A row can become `Complete` only after the pinned latest-Python
oracle is represented by executable Rust tests and all additive behavior has independent tests.
New frontends and entry kinds must use the same application ports, form plans, UI command registry,
and stable machine surfaces.
