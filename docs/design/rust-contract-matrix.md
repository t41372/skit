# Rust compatibility matrix

The latest Python development revision on `main` is the behavioral oracle, not the version 0.4.0
release tag. This review is pinned to `origin/main@206f9ef946fc45835cb2479593794431f2620c32`.
Version 0.5.0 can add capabilities, but it cannot remove behavior from that revision or replace it
with a shortcut. This table records both the required contract and its executable evidence. `In
progress` is a release blocker; it is not a permitted behavior change.

| Contract | Status | Rust evidence and pinned Python-main oracle |
| --- | --- | --- |
| Metadata, open kinds, unknown TOML, exact source bytes and permissions, durable identity, and metadata/source CAS | Complete | `skit-domain/tests/contract.rs`, `skit-store/tests/mutations*.rs`, scalar known-field corruption owners for runner and runtime lists, coordinated state/cleanup rollback and race owners; pinned-main store tests |
| Reads never migrate user data (metadata, state, config). The registry projection is the oracle-defined exception: `list` opportunistically self-heals a stale row under a non-blocking lock (`_repair_rows`), and a corrupt index degrades to empty after a `.corrupt` backup (`_load_registry`) -- `resolve` never self-heals | Complete | `skit-store/tests/registry_fast_read.rs`, `port_test_store.rs`, `registry_resolve.rs`, `form_state_store.rs`, `config_store.rs`, and read-only lock/filesystem owners |
| Stable management exit 0/1/2, cancel exit 130, and run pass-through/2/125/126/127/signal `128+N` contracts | Complete | `skit-cli/tests/v040_compatibility.rs`, `run_cli.rs`, source-normalization refusal batching, and pinned-main command tests |
| Stable JSON records for list/show/params/deps/config/runner/preset/doctor | Complete | `skit-cli/tests/v040_compatibility.rs`, command-specific exact JSON owners, and two-version golden records |
| Presets, exact optional last-run snapshots, remembered values, and secrets | Complete | `skit-application/tests/form_state*.rs`, `skit-store/tests/form_state_*.rs`, coordinated source-secrecy rollback and completed-run race owners |
| Runner seeds, raw rows, reason codes, malformed containers, duplicates, and CAS | Complete | `skit-store/tests/config_store.rs`, `skit-cli/tests/v040_compatibility.rs`, and pinned-main prompt/config manifests |
| Python semantic analysis, argparse, Click, Typer, reconciliation, and injection | Complete | `skit-language/tests`, real-Python runtime/compile owners, pinned-main analyzer/argspec/reconcile/shim manifests, and byte-exact corpus tests |
| Shell analysis, reflection, reconciliation, injection, normalization, and interpreter gate; Fish analysis, reflection, and environment delivery | Complete | language/form/runtime owners, real CLI/child owners, and pinned-main shell/fish manifests and corpus |
| JS/TS analysis, reflection, reconciliation, injection, and Node gate; parser-owned PowerShell analysis, reflection, and flag assembly | Complete | language/form/runtime/CLI owners, exact drift/gate diagnostics, pinned-main manifests, and corpus |
| Prompt/command Unicode placeholders, reserved names, raw substitution, and extra argv | Complete | `skit-language/tests`, `skit-runtime/tests`, and pinned-main prompt/launcher exact-owner manifests |
| Dependency discovery and validation, PEP 723/npm ownership, first-seen order, local-project filtering, module flavor, and atomic materialization | Complete | language/runtime/store/CLI dependency manifests, cleanup/preflight/rollback fault owners, and pinned-main dependency tests |
| uv consent and one-time mirror onboarding | Complete | real CLI PTY consent/EOF/decline owners, FileConfigStore mirror owners, and the 36-name uvman manifest |
| Typed form plan: bool/choice/number/path/list/secret/default/help/required/provenance | Complete | `skit-form` plan tests, pinned-main form/widget tests, and real plain/TUI projection owners |
| Library activity order, subsequence search, complete detail, rerun, help, and search workflow | Complete | reducer, `TestBackend`, real PTY, VHS, and pinned-main TUI workflow owners |
| Add lanes, drafts, kind picker, analysis review, edit/reanalyse, and atomic commit | Complete | application/UI/TUI workflow, transaction, editor, and pinned-main add-lane/review owners |
| Run, preset, token, environment, file, runner, and default form commands | Complete | application/form/UI/TUI, real PTY, picker, and pinned-main form/prompt owners |
| Settings, preferences, health, runner manager, Agent Skill, and dirty guards | Complete | application/UI/TUI transaction and responsive-management owners plus pinned-main manifests |
| Every advertised TUI command has positive keyboard and mouse owners at every documented responsive size tier | Complete | shared command-registry and local-action inventories at `120x30`, `46x12`, and `24x6`, plus reducer, `TestBackend`, and real PTY owners |
| Complete en/zh-CN/zh-TW catalog and stable machine English | Complete | `skit-i18n/tests/catalog.rs`, crate localization tests, English/tooling gates, and three-locale PTY/TestBackend owners |
| PyPI/Maturin wheel, `uv tool`, archives, security, coverage, docs, benchmarks | In progress | Local wheel/sdist installs, embedded-Skill and real-run smoke, corpus/manifest checks, deny/audit/Zizmor, 100% LCOV, docs build/link check, 112 metrics with 8/8 evaluated enforced budgets, and 23 Criterion estimates pass; native and declared-runtime CI, hands-on UI review, and approved mutation remain |
| Future Tauri uses the same application/form/UI state and command registry | Complete | serializable `skit-ui` round-trip, frontend-neutral effects, typed application/form ports, and frontend-parity tests |

The matrix is a release contract. A row can become `Complete` only after the pinned latest-Python
oracle is represented by executable Rust tests and all additive behavior has independent tests.
New frontends and entry kinds must use the same application ports, form plans, UI command registry,
and stable machine surfaces.

The current local snapshot has 21 rows: 20 complete and 1 in progress. The last fully certified
candidate passed 4,054 workspace tests with 0 failures and 526 classified ignores, complete
executable-source line coverage, warnings-denied Clippy and Rustdoc, all three locale catalogs, and
the local supply-chain, docs, package, demo, and benchmark gates. The final authority audit changed
the metadata reader and added two owners; their focused suites pass, but the pushed SHA needs a
fresh full workspace LCOV and static checkpoint. The remaining release row also needs native
platform and declared-runtime CI, the user hands-on UI check, and a green mutation run; a
lower-layer green test does not close it.
