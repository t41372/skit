# AGENTS.md

## Product

skit is a Rust script launcher and parameter manager. Users keep Python, shell, JavaScript,
TypeScript, fish, PowerShell, Ruby, Perl, Lua, R, executable, command-template, and prompt entries
in one library. They can use the Ratatui interface or the deterministic CLI.

## Product rules

1. Localize every user-visible string. English is the source locale. Simplified Chinese and
   Traditional Chinese must be complete. JSON keys and `skills/skit/SKILL.md` are English-only
   machine contracts.
2. Keep each TUI action available by keyboard and mouse. Every visible footer chip is a click
   target. Every advertised key needs a positive test.
3. Keep the interface discoverable. A user must not need to remember commands, keys, or script
   arguments.
4. Keep every TUI capability available through the CLI. Machine paths need `--json`, deterministic
   exit codes, `--no-input`, `--dry-run` where applicable, and dynamic completion.
5. Keep the quality gates hard: complete executable-source line coverage, Clippy with warnings as
   errors, Rustdoc warnings as errors, cargo-mutants with zero survivors, and complete i18n.
6. Do not change global or third-party configuration without explicit consent. skit can change only
   its own data, state, and configuration directories.

## Trust model

skit is a local launcher. It is not a sandbox or a security boundary. The user owns and trusts the
machine, library, scripts, command-line arguments, templates, prompts, environment values, output,
and logs. Running user-authored code is the primary product operation.

Do not add sanitization, escaping, character blocks, permission policies, secret-handling policies,
or other threat mitigations for user-controlled local content. Do not refuse an operation because
the trusted script or argument could run a command. Keep the operating system and language runtime
semantics unless version 0.4 already defines a different behavior.

Security-related work is in scope only when it does one of these things:

- It keeps the exact version 0.4 behavior.
- It prevents skit from accidentally changing files outside its own directories.
- It prevents skit from losing, corrupting, or partially committing user data.
- It is required by an explicit product rule in this file.

Format validation is a correctness concern when a runtime or file format needs valid input. Do not
expand format validation into a threat model. If a proposed mitigation is not in the preceding
list, do not implement it without explicit user consent.

## Architecture

- `skit-domain`: pure values and invariants.
- `skit-application`: use cases, ports, typed errors, and stable frontend data.
- `skit-language`: parser-backed analysis, injection, normalization, and prompt rendering.
- `skit-form`: frontend-neutral form plans.
- `skit-store`: filesystem, TOML, compatibility, locking, and atomic mutation adapters.
- `skit-runtime`: process plans, uv bootstrap, JavaScript dependency materialization, and spawn.
- `skit-ui`: serializable state and reducer. This is the future Tauri seam.
- `skit-tui`: Ratatui rendering and Crossterm input mapping.
- `skit-cli`: Clap commands and composition root.
- `skit-i18n`: complete static translation catalog.

Dependency direction must point toward domain and application. `skit-application` cannot depend on
Clap, Ratatui, Crossterm, Tauri, TOML, or a concrete filesystem. Do not put product rules in a
frontend adapter.

## Language rules

Analyzers can use the language parser. Launch paths cannot depend on parser crates. A language
adapter shares one parse layer for analysis, injection, and normalization.

`--normalize` is the only opt-in semantic edit to a stored script. It can convert one shell
constant to the environment-default form. All other skit metadata edits to a script are confined to
the supported comment block. Never change the user's original for a copy entry.

Prompt rendering uses raw substitution. It does not use a shell or shell quoting. Non-interactive
runner selection is `--runner`, then entry pin, then exit 126. It does not rank or guess.

Files in `tests/corpus/` are byte-exact inputs. Do not normalize line endings, trailing spaces,
final newlines, or Unicode.

## TDD

Write a failing contract test before an implementation change. Then implement the smallest change
and refactor while tests stay green. Add refusal, corruption, race, and rollback tests for stateful
boundaries. Store tests use temporary directories. UI tests drive the reducer and Ratatui
`TestBackend`.

## Commands

```bash
cargo build --locked
cargo run -p skit-cli-rs -- --help
cargo fmt --all --check
cargo clippy --locked --workspace --all-targets --all-features -- -D warnings
RUSTDOCFLAGS='-D warnings' cargo doc --locked --workspace --all-features --no-deps
cargo test --locked --workspace --all-targets --all-features
cargo llvm-cov --locked --workspace --all-targets --all-features --lcov --output-path lcov.info
bash scripts/check_coverage.sh lcov.info
cargo mutants --workspace --all-features --cargo-arg=--locked --jobs 2 --timeout 300
cargo deny --locked check
cargo audit --deny warnings
zizmor .github/workflows .github/actions/install-hyperfine/action.yml
```

Documentation:

```bash
cd docs
npm ci
npm run types:check
npm run build
```

Benchmarks:

```bash
cargo build --locked --release -p skit-cli-rs
bash benchmarks/run.sh pr .bench target/release/skit
bash benchmarks/check.sh .bench/results.json benchmarks/budgets.toml --require-enforced
cargo bench --locked --workspace --all-features
```

## English text

New English comments, user copy, errors, and documentation must follow ASD-STE100. Use short direct
sentences. Use one term for one meaning. Use active voice when possible.

## Compatibility

Version 0.4 data is authoritative. Keep unknown TOML fields, open-ended entry kinds, stored source
bytes, permissions, state, presets, and secrets policy. Do not migrate or rewrite data during a
read. Use identity checks and source hashes for writes. A corrupt entry must not hide valid entries.

## Packaging

The product has no Python implementation. Maturin builds a binary wheel so `uv tool install
skit-cli` and PyPI upgrades keep working. Python files are permitted only as script inputs, demo
inputs, or benchmark subjects. Do not add Python tooling or implementation files.

`skills/skit/SKILL.md` is embedded in the Rust binary. Keep its commands synchronized with the real
CLI.
