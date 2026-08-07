# Rust compatibility matrix

Legend: ✅ implemented and contract-tested · 🟡 designed/partial · ⬜ not ported

| Surface | Status | Contract source |
| --- | --- | --- |
| Read legacy and current `meta.toml` | ✅ | `skit-store/tests/file_store.rs` |
| Open-ended entry kinds | ✅ | `skit-domain/tests/contract.rs` |
| Per-entry corruption isolation | ✅ | `skit-store/tests/file_store.rs` |
| Resolve by slug, then exact display name | ✅ | `skit-store/tests/file_store.rs` |
| Ambiguous display-name refusal | ✅ | `skit-store/tests/file_store.rs` |
| Stable `list --json` / `show --json` | ✅ | `skit-cli/tests/cli.rs` |
| Ratatui keyboard and mouse browsing | ✅ | `skit-ui/tests/reducer.rs`, `skit-tui/tests/render.rs` |
| Registry-index fast path | ⬜ | Python store/performance tests |
| Add / remove / rename / describe | ⬜ | Python CLI/store tests |
| Staged editor + identity/source CAS | ⬜ | PR #34 round 17–18 tests |
| Parameters, presets, secret scrubbing | ⬜ | Python params/argstate/flows tests |
| Language analysis and injection | ⬜ | `tests/corpus`, analyzer tests |
| Prompt runners | ⬜ | prompt design and runner tests |
| Spawn-under-lock / wait-outside | ⬜ | PR #34 round 18 tests |
| Config, i18n, doctor, agent skill | ⬜ | current Python gates |
| Future Tauri adapter | 🟡 | dependency direction in `rust-rewrite.md` |

The matrix is a release gate, not a progress badge. A row becomes ✅ only when its tests exercise
both the ordinary path and its important refusal/race/corruption path.
