# Rust + Ratatui rewrite

Status: **migration in progress**. The Python implementation remains the release implementation
until the compatibility matrix is green. The Rust workspace is additive and must not silently
change an existing user's library.

## Baseline

The rewrite follows the behavior of `fix/design-audit` at
`b47eab53eadc583bff19ade742ef8f225f0cceb2`, not an older `main` snapshot. In particular, the
Rust design treats a slug as an address rather than an identity, preserves the launch-boundary
identity check, keeps the entry lock only through spawn (never through child wait), and requires
identity plus source-version compare-and-swap for staged edits.

## Dependency direction

```text
skit-domain          pure values and invariants
      ↑
skit-application     use cases, ports, errors, stable frontend data
      ↑              ↑
skit-store       skit-ui (pure reducer / serializable view model)
      ↑              ↑
      └──────── skit-tui (Ratatui adapter)
                       ↑
                    skit-cli (composition root)

future: skit-tauri ──→ skit-application + skit-ui
```

`skit-application` must never depend on Clap, Ratatui, Crossterm, Tauri, TOML, or a concrete
filesystem. `skit-ui` contains no terminal concepts. Ratatui and the future Tauri shell are peers:
both translate user input into application/UI actions and render serializable state.

## TDD rule

Every behavior slice lands in this order:

1. a contract test that fails for the intended reason;
2. the smallest implementation that makes it pass;
3. refactoring while the test remains green;
4. differential tests against the Python baseline when the behavior already exists there.

The first slice gates line coverage at 90%; each migration slice must ratchet it upward, and cutover
requires the same 100% floor as the Python implementation.

A test that merely snapshots an implementation detail is not a contract. Store tests use real
temporary directories; UI tests drive the reducer and Ratatui `TestBackend`; process tests use a
fake launcher until the final spawn boundary test.

## Compatibility constraints

- Existing `SKIT_DATA_DIR/scripts/<slug>/meta.toml` files remain readable.
- `kind` is an open string. A newer/unknown kind can still list, show, and remove cleanly.
- A missing `id` is a legacy entry, not malformed metadata.
- Corruption is isolated per entry during listing and becomes a diagnostic; one bad entry cannot
  hide the rest of the library.
- Non-interactive interfaces never prompt or guess. Machine output stays structured and English.
- Exit classification remains 2 / 125 / 126 / 127 / 130, while a launched child keeps its own
  exit code untouched.
- User-authored bytes, line endings, and permissions are preserved by write paths. No write path
  may be introduced without an identity/CAS test.

## Migration slices

| Slice | Scope | Exit criterion |
| --- | --- | --- |
| 0 | Architecture + contract inventory | dependency rules and CI are enforced |
| 1 | Read-only library vertical slice | compatible `list`, `show`, and Ratatui browser |
| 2 | Store mutations | atomic writes, locks, identity claim, remove/rename/edit CAS |
| 3 | Parameters | typed schema, token expansion, presets, secret non-persistence |
| 4 | Language adapters | Python/shell/JS/TS analyzers, injectors, normalizers, golden corpus |
| 5 | Launch | deterministic planning, runner resolution, spawn-under-lock/wait-outside |
| 6 | Remaining product surfaces | config, i18n, doctor, agent skill, completion, benchmarks |
| 7 | Cutover | differential matrix green; Python implementation removed |
| 8 | Tauri adapter | invokes the same application ports; no duplicated business rules |

The read path began with authoritative `meta.toml` scans, then introduced the rebuildable
`registry.toml` projection only after its trust boundary was contract-tested. Listing trusts a row
only when its shape and exact metadata timestamp match, falls back per entry to authoritative
metadata, and attempts one nonblocking batch repair. Exact slug resolution still opens the selected
metadata directly. Exact display-name resolution uses registry rows only to select a unique
candidate; a stale claim, miss, or multiple claimants trigger the same authoritative freshness
sweep and deterministic ambiguity refusal as listing. A fast unique hit never sweeps or repairs
unrelated rows. Differential performance evidence remains a separate release gate: functional fast
paths are not, by themselves, proof that the application is lightweight.
