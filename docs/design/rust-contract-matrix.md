# Rust compatibility matrix

The latest Python development revision on `main` is the behavioral oracle, not the version 0.4.0
release tag. This review is pinned to `origin/main@206f9ef946fc45835cb2479593794431f2620c32`.
Version 0.5.0 can add capabilities, but it cannot remove behavior from that revision or replace it
with a shortcut. This table records both the required contract and its executable evidence. `In
progress` is a release blocker; it is not a permitted behavior change.

| Contract | Status | Rust evidence and pinned Python-main oracle |
| --- | --- | --- |
| Metadata, open kinds, unknown TOML, bytes, permissions, identity, and CAS | In progress | `skit-domain/tests/contract.rs`, `skit-store/tests/mutations*.rs`; pinned-main store tests |
| Reads never migrate user data (metadata, state, config). The registry projection is the oracle-defined exception: `list` opportunistically self-heals a stale row under a non-blocking lock (`_repair_rows`), and a corrupt index degrades to empty after a `.corrupt` backup (`_load_registry`) -- `resolve` never self-heals | In progress | `skit-store/tests/registry_fast_read.rs`, `port_test_store.rs`, `registry_resolve.rs`, `form_state_store.rs`, `config_store.rs` |
| Stable management exit 1/2 and run exit 2/125/126/127 contracts | In progress | `skit-cli/tests/v040_compatibility.rs`, `run_cli.rs`; pinned-main `test_cli.py` and command tests |
| Stable JSON records for list/show/params/deps/config/runner/preset/doctor | Complete | `skit-cli/tests/v040_compatibility.rs`, command-specific exact JSON owners, and two-version golden records |
| Presets, exact optional last-run snapshots, remembered values, and secrets | Complete | `skit-application/tests/form_state*.rs`, `skit-store/tests/form_state_*.rs`, coordinated source-secrecy rollback and completed-run race owners |
| Runner seeds, raw rows, reason codes, malformed containers, duplicates, and CAS | Complete | `skit-store/tests/config_store.rs`, `skit-cli/tests/v040_compatibility.rs`, and pinned-main prompt/config manifests |
| Python semantic analysis, argparse, Click, Typer, reconciliation, and injection | Complete | `skit-language/tests`, real-Python runtime/compile owners, pinned-main analyzer/argspec/reconcile/shim manifests, and byte-exact corpus tests |
| Shell/Fish semantic analysis, CLI reflection, reconciliation, injection, normalization | In progress | `skit-language/tests`; pinned-main shell/fish tests and corpus |
| JS/TS/PowerShell semantic analysis, CLI reflection, reconciliation, and injection | In progress | `skit-language/tests`; pinned-main JS/PowerShell tests and corpus |
| Prompt/command Unicode placeholders, reserved names, raw substitution, and extra argv | Complete | `skit-language/tests`, `skit-runtime/tests`, and pinned-main prompt/launcher exact-owner manifests |
| Dependency detection, authoritative inline metadata, source order, and project compatibility | In progress | `skit-language/tests`, `skit-runtime/tests`; pinned-main dependency tests |
| uv consent and one-time mirror onboarding | Complete | real CLI PTY consent/EOF/decline owners, FileConfigStore mirror owners, and the 36-name uvman manifest |
| Typed form plan: bool/choice/number/path/list/secret/default/help/required/provenance | Complete | `skit-form` plan tests, pinned-main form/widget tests, and real plain/TUI projection owners |
| Library activity order, subsequence search, complete detail, rerun, help, and search workflow | In progress | `skit-ui/tests/reducer.rs`, `skit-tui/tests/render.rs`; pinned-main TUI tests |
| Add lanes, drafts, kind picker, analysis review, edit/reanalyse, and atomic commit | In progress | application/UI/TUI workflow tests; pinned-main add-lane and review tests |
| Run, preset, token, environment, file, runner, and default form commands | In progress | application/UI/TUI workflow tests; pinned-main form and prompt TUI tests |
| Settings, preferences, health, runner manager, Agent Skill, and dirty guards | In progress | application/UI/TUI workflow tests; pinned-main settings/health/runner tests |
| Every advertised TUI command has keyboard and mouse positive tests at all size tiers | In progress | Ratatui `TestBackend` and reducer command-registry tests |
| Complete en/zh-CN/zh-TW catalog and stable machine English | Complete | `skit-i18n/tests/catalog.rs`, crate localization tests, English/tooling gates, and three-locale PTY/TestBackend owners |
| PyPI/Maturin wheel, `uv tool`, archives, security, coverage, docs, benchmarks | In progress | Local wheel/sdist installs, embedded-Skill and real-run smoke, corpus/manifest checks, deny/audit/Zizmor, 100% LCOV, docs build/link check, 112 metrics with 8/8 evaluated enforced budgets, and 23 Criterion estimates pass; native release wheels and approved mutation remain |
| Future Tauri uses the same application/form/UI state and command registry | Complete | serializable `skit-ui` round-trip, frontend-neutral effects, typed application/form ports, and frontend-parity tests |

The matrix is a release contract. A row can become `Complete` only after the pinned latest-Python
oracle is represented by executable Rust tests and all additive behavior has independent tests.
New frontends and entry kinds must use the same application ports, form plans, UI command registry,
and stable machine surfaces.

The current local snapshot has 21 rows: 9 complete and 12 in progress. The integrated candidate
passes 4,014 workspace tests with 0 failures and 542 classified ignores, complete executable-source
line coverage, warnings-denied Clippy and Rustdoc, all three locale catalogs, and the local
supply-chain/docs gates. Rows that still say `In progress` need native-platform, hands-on UI, or
final package/benchmark/mutation evidence; a lower-layer green test does not close them.
