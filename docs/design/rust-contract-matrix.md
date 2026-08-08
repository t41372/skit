# Rust compatibility matrix

Each row names the tests that hold its contract. Run them before a release; this table records
where the evidence is, not that a release has shipped.

| Surface | Main contract tests |
| --- | --- |
| Version 0.4 metadata, state, configuration, and skill paths | `skit-cli/tests/v040_compatibility.rs` |
| Open entry kinds, identity, and serialization | `skit-domain/tests/contract.rs` |
| Registry reads, repair, recovery, and rollback | `skit-store/tests/registry_*.rs` |
| Add, remove, rename, describe, edit, identity, and source CAS | `skit-store/tests/mutations.rs`, `skit-cli/tests/mutations_cli.rs` |
| Parameters, declarations, defaults, secrets, and delivery | `skit-domain/tests/parameter_*.rs`, `skit-application/tests/*.rs` |
| Presets, remembered values, and state recovery | `skit-store/tests/form_state_*.rs` |
| Python, shell, JS/TS, fish, PowerShell, prompt, and long-tail analysis | `skit-language/tests/*.rs` |
| Injection, shell normalization, prompt rendering, and byte preservation | `skit-language/tests/*.rs` |
| Python uv bootstrap and JavaScript dependency installation | `skit-runtime/tests/*.rs`, `skit-cli/tests/edge_workflows.rs` |
| Process plans, runner resolution, locks, and exit codes | `skit-runtime/tests/launch_plan.rs`, `skit-cli/tests/run_cli.rs` |
| Stable CLI, JSON, no-input, dry-run, and completion | `skit-cli/tests/product_contract.rs`, `terminal_pty.rs` |
| Ratatui keyboard, mouse, narrow layout, forms, and host effects | `skit-ui/tests/reducer.rs`, `skit-tui/tests/render.rs` |
| Config, mirrors, doctor, editor drafts, Agent Skill, and i18n | `skit-cli/tests/edge_workflows.rs`, `skit-cli/tests/surface_edges.rs`, `skit-i18n/tests/catalog.rs` |
| Typed message catalog completeness and per-locale error text | `skit-i18n/tests/catalog.rs`, `skit-*/tests/localization.rs`, `skit-cli/tests/typed_error_locales.rs` |
| PyPI and `uv tool` compatibility without Python product source | CI wheel smoke job and repository Python-file gate |
| Future Tauri adapter seam | serializable `skit-ui` JSON round-trip tests |

The matrix is a release contract. New frontends and new entry kinds must use the same application
ports and stable machine surfaces.
