# Rust compatibility matrix

Legend: ✅ implemented and contract-tested · 🟡 designed/partial · ⬜ not ported

| Surface | Status | Contract source |
| --- | --- | --- |
| Read legacy and current `meta.toml` | ✅ | `skit-store/tests/file_store.rs` |
| Open-ended entry kinds | ✅ | `skit-domain/tests/contract.rs` |
| Per-entry corruption isolation | ✅ | `skit-store/tests/file_store.rs` |
| Resolve by slug, then exact display name | ✅ | `skit-store/tests/file_store.rs`, `skit-store/tests/registry_resolve.rs` |
| Ambiguous display-name refusal | ✅ | `skit-store/tests/file_store.rs`, `skit-store/tests/registry_resolve.rs` |
| Stable `list --json` / `show --json` | ✅ | `skit-cli/tests/cli.rs` |
| Ratatui keyboard and mouse browsing | ✅ | `skit-ui/tests/reducer.rs`, `skit-tui/tests/render.rs` |
| Registry-backed list + exact-name resolve | ✅ | `skit-store/tests/registry_fast_read.rs`, `skit-store/tests/registry_resolve.rs` |
| Registry projection, recovery, and rollback | ✅ | `skit-store/tests/registry_projection.rs`, `skit-store/tests/registry_edge_contracts.rs` |
| Add / remove / rename / describe | ✅ | `skit-store/tests/mutations.rs`, `skit-store/tests/mutation_refusals.rs`, `skit-cli/tests/mutations_cli.rs` |
| Identity claim + source compare-and-swap | ✅ | `skit-store/tests/mutations.rs`, `skit-store/tests/mutation_refusals.rs` |
| Universal parameter model, serialization, and typed defaults | ✅ | `skit-domain/tests/parameters.rs`, `skit-domain/tests/parameter_serialization_edges.rs` |
| Declared parameter extraction and template synthesis | ⬜ | `tests/test_declared_params.py`, language analyzer tests |
| Presets, remembered values, secret scrubbing, and form assembly | ⬜ | Python params/argstate/flows tests |
| Staged external-editor UX | ⬜ | PR #34 round 17–18 tests |
| Differential performance evidence | ⬜ | Python performance tests and future Rust benchmark harness |
| Language analysis and injection | ⬜ | `tests/corpus`, analyzer tests |
| Prompt runners | ⬜ | prompt design and runner tests |
| Spawn-under-lock / wait-outside | ⬜ | PR #34 round 18 tests |
| Config, i18n, doctor, agent skill | ⬜ | current Python gates |
| Future Tauri adapter | 🟡 | dependency direction in `rust-rewrite.md` |

The matrix is a release gate, not a progress badge. A row becomes ✅ only when its tests exercise
both the ordinary path and its important refusal/race/corruption path.
