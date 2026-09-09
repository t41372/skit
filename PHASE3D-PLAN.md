# Phase3d implementation spec (v3, approved)

Base: `integration/ui-walker-on-mouse` at `33d3863`. Uncommitted working note, like `HANDOFF.md`.

Two independent advisors reviewed v1 and v2. Every ruling below is settled. Implement exactly this.

Baseline, verified green before any change:

| scope | result |
| --- | --- |
| `cargo test --locked -p skit-cli-rs --lib cli::tui_real_walker` | 8 passed |
| `cargo test --locked -p skit-cli-rs --lib cli::tui_real_host` | 33 passed |
| `cargo test --locked -p skit-tui-walker-support --lib` | 61 passed |
| `cargo test --locked -p skit-tui --test model_walker` | 166 passed, 4 ignored |

## 0. Measured facts that drive the design

- **`#[cfg(test)]` modules inside `crates/*/src/` are coverage-instrumented and gated.** Measured:
  `cargo llvm-cov -p skit-cli-rs --lib` emits `SF:` records for `cli/tui_real_host.rs` (3819 `DA:`
  lines, 7 zero-hit, all bare `}` that `check_coverage.sh` filters) and for `cli/tui_real_walker.rs`.
  Source under `crates/*/tests/**` gets no `SF:` record. So the new skit-cli module is fully
  coverage-gated, and any pattern copied from the legacy integration-test owner that has mutually
  exclusive branches will fail the gate.
- **`cargo mutants` skips `#[cfg(test)]` items**, so the skit-cli module is mutation-blind while
  walker-support is mutation-visible. Every rule that can honestly live in walker-support should.
- **walker-support has no `std::fs`, `std::path`, `PathBuf`, or `read_dir`** today. It does already
  use `std::io` types in `asciicast.rs`. Adding `PathBuf` is a type, not an effect.
- The legacy writer **and** reader both build the chunk-view first-appearance set **per profile**.
- `ArtifactError::new` is private, but `From<serde_json::Error>` is public.
- `SchemaThreeTrace` carries neither the operations vector nor the final-liveness request.
- There are **four** disagreeing cast-canvas sources today: legacy reader `max(row viewports)`;
  legacy writer `canvas_size(initial, operations)`; real sink live recorder `factory.size`; real sink
  rebuild inside `finish` `rows[0].viewport`. They agree only because the smoke has no resize.
- The legacy `validate_host_transition_boundaries` cannot be ported verbatim: real `HostObservation`
  embeds `state` and a `transcript` drained per interval.
- No production path constructs `skit_form::TypedValue::Decimal`. The only two construction sites in
  the workspace are `skit-form/src/field.rs`'s own unit test and `fake_host.rs`'s deliberate
  `f64::NAN`.
- Every existing `build_chunks`/`validate_chunks` caller passes 2 or more (legacy 24; contract tests
  2 and 3).

## 1. Goal and non-goals

Goal: write the already-reviewed in-memory schema-3 real-host trace to disk as an immutable
content-addressed bundle, install it atomically, read the **installed** bundle back, and prove every
reference, digest, view, and cast byte.

Non-goals, deferred by `HANDOFF.md`: the 100-operation vector, the four review profiles,
`CorpusManifest`, `coverage.json`, `review-progress.json`, `README.md`, `review.md`, FakeHost
migration or deletion, the Terra/Luna comparison, and umask normalization.

## 2. On-disk layout

```
bundle-<digest>/
  run.json                                  # RunMetadata, canonical JSON, no trailing newline
  operations.json                           # canonical JSON array, no trailing newline
  final-liveness.json                       # canonical JSON value, no trailing newline
  objects/reducer/<sha256>.json             # canonical envelope bytes, no trailing newline
  objects/host/<sha256>.json
  objects/session/<sha256>.json
  objects/styled-frame/<sha256>.json
  objects/geometry/<sha256>.json
  views/reducer/<sha256>.json               # pretty inner value plus exactly one "\n"
  views/host/<sha256>.json
  views/session/<sha256>.json
  views/geometry/<sha256>.json
  views/frames/<sha256>.txt                 # human frame view; note "frames", not "styled-frame"
  profiles/<id>/profile.json                # ProfileManifest, canonical, no trailing newline
  profiles/<id>/timeline.ndjson             # canonical row per line, exactly one trailing newline
  profiles/<id>/trace.cast                  # asciicast v3 raw bytes
  profiles/<id>/chunks/<id>-NNNN.json       # canonical row array, no trailing newline
  profiles/<id>/chunks/<id>-NNNN.md         # markdown review view
```

`run.json` is the root index. There is **no** `manifest.json`, so that name keeps exactly one
meaning: the four-profile `CorpusManifest` the later phase writes. No new manifest DTO is invented;
the per-profile file holds the **existing** `ProfileManifest`.

## 3. walker-support additions

New `crates/skit-tui-walker-support/src/bundle.rs`, public, no `std::fs`. Public items only if a
test executes them; nothing speculative.

```rust
pub const OBJECT_DIRECTORY: &str = "objects";
pub const VIEW_DIRECTORY: &str = "views";
pub const FRAME_VIEW_DIRECTORY: &str = "frames";
pub const PROFILE_DIRECTORY: &str = "profiles";
pub const CHUNK_DIRECTORY: &str = "chunks";
pub const PROFILE_FILE: &str = "profile.json";
pub const TIMELINE_FILE: &str = "timeline.ndjson";
pub const CAST_FILE: &str = "trace.cast";
pub const OPERATIONS_FILE: &str = "operations.json";
pub const FINAL_LIVENESS_FILE: &str = "final-liveness.json";
pub const RUN_FILE: &str = "run.json";

#[must_use] pub const fn kind_directory(kind: ObjectKind) -> &'static str;
#[must_use] pub fn object_path(reference: &ObjectRef) -> PathBuf;
/// Total over every kind. StyledFrame maps to `views/frames/<digest>.txt`.
#[must_use] pub fn object_view_path(reference: &ObjectRef) -> PathBuf;
#[must_use] pub fn profile_directory(profile: &str) -> PathBuf;
#[must_use] pub fn chunk_json_path(profile: &str, chunk: &str) -> PathBuf;
#[must_use] pub fn chunk_view_path(profile: &str, chunk: &str) -> PathBuf;

pub fn object_view_bytes(value: &Value) -> Result<Vec<u8>, ArtifactError>;
pub fn frame_view_bytes(frame: &StyledFrameSnapshot) -> Result<Vec<u8>, ArtifactError>;
pub fn timeline_ndjson_bytes(rows: &[TimelineRow]) -> Result<Vec<u8>, ArtifactError>;
pub fn decode_timeline_ndjson(bytes: &[u8]) -> Result<Vec<TimelineRow>, ArtifactError>;
pub fn chunk_json_bytes(rows: &[TimelineRow]) -> Result<Vec<u8>, ArtifactError>;
pub fn decode_canonical_bytes(bytes: &[u8]) -> Result<Value, ArtifactError>;

/// One profile's first-appearance cursor. Advance it over that profile's chunks in ordinal order.
#[derive(Clone, Debug, Default)]
pub struct ChunkViewCursor { /* BTreeSet<(ObjectKind, String)> */ }
impl ChunkViewCursor {
    #[must_use] pub fn new() -> Self;
    pub fn chunk_view_bytes(
        &mut self,
        rows: &[TimelineRow],
        chunk: &ChunkDescriptor,
    ) -> Result<Vec<u8>, ArtifactError>;
}

/// The cast canvas: the maximum viewport across every row.
pub fn cast_canvas(rows: &[TimelineRow]) -> Result<(u16, u16), ArtifactError>;

/// Loader errors are plain strings because the loader is the caller's I/O boundary.
pub fn rebuild_presented_cast(
    rows: &[TimelineRow],
    load_frame: impl FnMut(&ObjectRef) -> Result<StyledFrameSnapshot, String>,
) -> Result<Vec<u8>, ArtifactError>;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RunMetadata {
    pub schema: u16,
    pub source_revision: String,
    pub deterministic_result: String,
    pub operation_count: usize,
    pub operations_sha256: String,
    pub final_liveness_sha256: String,
    pub profiles: Vec<String>,
}
pub fn validate_run_metadata(run: &RunMetadata) -> Result<(), ArtifactError>;

/// The expected profile identity, supplied by the owner and never inferred from the artifact.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ProfileExpectation<'a> {
    pub id: &'a str,
    pub locale: &'a str,
    pub viewport: RectSnapshot,
    pub operations_sha256: &'a str,
}
pub fn validate_profile_manifest(
    profile: &ProfileManifest,
    expected: ProfileExpectation<'_>,
) -> Result<(), ArtifactError>;

/// Bind every profile document digest and the source revision into one immutable name.
pub fn bundle_digest(
    profile_digests: &BTreeMap<String, String>,
    source_revision: &str,
) -> Result<String, ArtifactError>;

/// Combine a git head and worktree state into one stable revision string.
/// `read_untracked` receives each NUL-separated relative path from `git ls-files -o`.
pub fn source_revision(
    head: &str,
    diff: &[u8],
    untracked: &[u8],
    read_untracked: impl FnMut(&str) -> Result<Vec<u8>, String>,
) -> Result<String, ArtifactError>;

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct BundleLayout {
    pub files: BTreeSet<PathBuf>,
    pub directories: BTreeSet<PathBuf>,
}
impl BundleLayout {
    /// Declare the root files, the object and view trees derived from the rows, and one profile.
    pub fn expect_bundle(
        &mut self,
        profile: &ProfileManifest,
        rows: &[TimelineRow],
    ) -> Result<(), ArtifactError>;
    pub fn insert_file(&mut self, relative: impl Into<PathBuf>);
    pub fn insert_directory(&mut self, relative: impl Into<PathBuf>);
    /// Exact set equality in both directions. An unreferenced object file is an extra file and a
    /// missing one is a missing file, so this single check also closes the object set.
    pub fn compare(&self, actual: &Self) -> Result<(), ArtifactError>;
}
```

`source_revision` reproduces the legacy shape: a clean tree returns the trimmed head; a dirty tree
returns `"{head}+worktree:{sha256}"` over the diff followed by, for each untracked path, the
big-endian `u64` path length, the path bytes, the big-endian `u64` content length, and the content
bytes. Both branches get unit tests, which is what makes the coverage gate reachable.

### Edits inside `crates/skit-tui-walker-support/src/lib.rs`

1. **Extract, do not duplicate.** Lift the per-profile block out of `validate_manifest` into
   `validate_profile_manifest(profile, ProfileExpectation)`. `validate_manifest` then builds a
   `ProfileExpectation` from each `REQUIRED_PROFILES` entry plus `manifest.operations_sha256` and
   calls it. Message text and check order must stay byte-exact: locale, viewport, operations digest,
   cast digest, cast byte size, row count, non-empty chunks, then `validate_chunk_ranges`.
   Write the characterization tests **first**, pinning every current `validate_manifest` and
   `validate_chunk_ranges` message and its precedence. Only `"four review profiles"` is asserted
   outside this crate (`crates/skit-tui/tests/model_walker/corpus.rs:1883`).
2. **Fix one doc comment.** `build_chunks` says "without splitting one operation or liveness chain",
   which is false: the body slices strictly by `maximum_rows`, and the continuation flags exist
   because chains are split. Replace it with a description of row-count slicing plus continuation
   flags. **Do not** add a minimum row limit: no caller passes 1, it gives Phase3d nothing, it
   changes three shared public functions the frozen legacy walker calls, and it is redundant because
   `validate_chunk_ranges` already rejects a chain-less non-initial chunk start.
3. Declare `pub mod bundle;` and keep `#![deny(missing_docs)]` satisfied.

### Engine contract

Add one test in `engine_tests.rs` proving that one checkpoint performs exactly one
`FrontendAdapter::observe` and exactly one `HostAdapter::observe` call. `observe` drains the real
host transcript and advances `PathMap` counters, so the call count is load-bearing, and main/replay
byte equality cannot detect a symmetric extra call.

## 4. skit-cli additions

### `crates/skit-cli/src/cli/tui_real_walker.rs` edits

1. `SchemaThreeSink::finish` returns a `RealTrace` that carries what the writer needs, because
   `SchemaThreeTrace` carries neither payload today:

   ```rust
   struct RealTrace {
       review_profile: String,
       locale: String,
       viewport: RectSnapshot,
       operations: Vec<Value>,
       final_liveness_requested: Value,
       rows: Vec<TimelineRow>,
       objects: BTreeMap<(ObjectKind, String), Vec<u8>>,
       cast: Vec<u8>,
   }
   ```

   `operation_count` is derived from `operations.len()`; do not store both.
2. **Separate the two profile identities.** `WalkerSeedSpec::profile` stays the seed/world label that
   produces the `<profile:...>` normalization token. `RealWalkerFactory` gains
   `review_profile: String`, which flows into `TimelineRow::profile`, `ProfileManifest::id`, and the
   `profiles/<id>/` directory. The smoke keeps seed `engine-smoke` and uses review profile
   `engine-smoke-60x24`. No existing test asserts on `TimelineRow::profile`; the smoke's string
   assertion is on the seed token `<profile:engine-smoke>` and is unaffected.
3. **Fix the cast canvas.** Give `SchemaThreeSink::new` an explicit canvas `Size` separate from the
   frontend viewport. Assert in `finish` that `cast_canvas(&rows)` equals that canvas, and build the
   rebuild recorder from the canvas instead of `rows[0].viewport`. This removes the third and fourth
   disagreeing size sources and is a correct invariant rather than a placeholder.

### New `crates/skit-cli/src/cli/tui_walker_bundle.rs`, private, `#[cfg(test)]`

```rust
/// One I/O funnel so every filesystem failure shares one covered line.
fn io<T>(result: io::Result<T>, path: &Path) -> Result<T, String>;

struct BundleWriter { root: PathBuf }
impl BundleWriter {
    fn create(root: &Path, profile: &str) -> Result<Self, String>;
    /// Write already-canonical object bytes after verifying them against their reference.
    fn put_object(&self, reference: &ObjectRef, bytes: &[u8]) -> Result<(), String>;
    /// Compare or create. A differing existing byte refuses.
    fn write_same_or_new(&self, relative: &Path, bytes: &[u8]) -> Result<(), String>;
    fn write_trace(&self, trace: &RealTrace, revision: &str) -> Result<ProfileManifest, String>;
}

fn read_tree(root: &Path) -> Result<(BundleLayout, BTreeMap<PathBuf, Vec<u8>>), String>;
fn read_bundle(root: &Path, expected: ProfileExpectation<'_>) -> Result<(), String>;
fn real_git(arguments: &[&str]) -> Result<Vec<u8>, String>;
fn source_identity() -> Result<String, String>;
fn install_bundle(
    parent: &Path,
    trace: &RealTrace,
    fingerprint: &mut impl FnMut() -> Result<String, String>,
) -> Result<PathBuf, String>;
```

`read_tree` walks with `fs::symlink_metadata`, refuses a symlink and refuses a non-regular entry,
and returns both the layout and every file's bytes. The byte map is what makes the install decision
and the tree comparison exact.

**Transaction, restructured so every branch is reachable.** There is no pre-check on the
destination, because a test cannot create the destination between a check and a rename, which would
leave that branch permanently uncovered.

1. sample the fingerprint;
2. stage into a sibling `.walker-XXXXXX` temporary directory inside the parent, so a partial bundle
   can never match `bundle-*`;
3. write and validate the staged tree completely;
4. sample the fingerprint again and refuse on any change, leaving no `bundle-*` and no `.walker-*`
   directory behind;
5. compute `bundle-<bundle_digest>` and attempt `fs::rename` unconditionally;
6. on success, read the **installed** tree back and validate it, then return its path;
7. on rename failure, read the installed tree; if it is byte-identical to the staged tree, drop the
   staging directory and return the existing path; otherwise refuse and leave the installed tree
   untouched.

On Linux `rename` over a non-empty directory fails with `ENOTEMPTY`, so pre-creating the destination
reaches step 7 deterministically. This covers the error branch and removes the untestable race
narrative.

`HANDOFF.md` requires reading the **installed** bundle back, which is also the only way the
`bundle-<digest>` directory-name check is reachable.

**`source_identity` is split for coverage.** `real_git` is thin, and its non-zero-exit refusal is
covered by one deliberately invalid argument. The pure combining logic is
`bundle::source_revision`, which has clean-tree and dirty-tree unit tests in walker-support, where
it is also mutation-visible. `source_identity` only composes them. The legacy single function
escaped the gate only because it lives in an integration-test target with no `SF:` records.

**Read-back covers**: `run.json` canonical bytes plus `validate_run_metadata`, and its digests
against the real files; `operations.json` canonical bytes, array shape, count, and digest;
`final-liveness.json` canonical bytes and digest; `profile.json` canonical bytes plus
`validate_profile_manifest`; timeline NDJSON exact trailing-newline count, per-line canonicality,
and `validate_timeline_semantics` with the decoded final-liveness request; profile metadata against
the timeline; `validate_chunks`; each chunk file's canonical bytes, digest, and row equality; cast
validity, digest, byte size, and byte-exact rebuild from stored Presented frames using
`cast_canvas`; every referenced object's canonical bytes, schema, kind, and digest; every non-frame
object view and every frame view byte; each chunk markdown byte through one per-profile
`ChunkViewCursor`; production reducer replay for every transition; the store-quiet host invariant
below; effect-chain termination; live-locale transitions; and `BundleLayout::compare`.

**The store-quiet host invariant, replacing the legacy port.** "Each row's host object embeds that
row's reducer state" is a tautology in the current sink, because `record` builds the reducer object
from `checkpoint.host.state`. Keep that cheap binding, but add the invariant the legacy check
actually meant: across an adjacent pair whose current boundary is **not** `HostAction`, the host
object's `surface`, `config`, `form_state`, `prompt_runner`, `drafts`, and `tree` must be unchanged.
`state` and `transcript` are excluded, with a comment giving the reason: `state` mirrors the reducer
by construction, and `transcript` is drained per interval, so the initial row legitimately carries
the whole seed port sequence. This proves a pure reducer or session transition touched no store and
no file. If a field-scoped comparison turns out to fail on some field other than those two, narrow
it to `tree` plus `surface` rather than dropping the invariant.

### `crates/skit-cli/src/cli.rs`

Add `#[cfg(test)] mod tui_walker_bundle;` beside the existing walker modules.

## 5. Legacy delegation, in scope

`crates/skit-tui/tests/model_walker/corpus.rs` and `cast_projection.rs` delete their private
`kind_directory`, `object_view_bytes`, `frame_view_bytes`, `chunk_view_bytes`, `RunMetadata`, and
`rebuild_presented_cast` body and delegate to the new walker-support functions, adding
`.map_err(|error| error.to_string())` at each call site.

This is mechanical and it is the only thing that proves the reproduction is byte-faithful: the
legacy 166 tests become characterization tests for the new public API, so the two copies cannot
drift into two different agent-review corpora. If the delegation turns out not to be mechanical,
stop, back it out, and instead pin the exact expected markdown and view bytes for a fixture inside
walker-support, recording the duplication as a deletion item in the FakeHost step.

`corpus.rs` keeps every product-coupled validator: reducer replay, live locales, host boundaries,
`GeometrySnapshot`, coverage, and the four-profile owner paths. Nothing else in it moves.

## 6. Test plan

Failing contracts first. Coverage design rules, both load-bearing because the new skit-cli module is
gated: every filesystem failure goes through the single `io` helper, and every refusal branch is
reachable by manipulating a real `TempDir`. **Every public item in `bundle.rs` needs at least one
test that executes it**; drop any item that does not earn one.

walker-support:

- characterization tests pinning every current `validate_manifest` and `validate_chunk_ranges`
  message and precedence, written **before** the extraction;
- path layout for all five kinds, including the `views/frames` asymmetry and `object_view_path`
  totality;
- `object_view_bytes` pretty form with exactly one trailing newline;
- `frame_view_bytes` header, quoted readable rows, run-length style section, and default-style
  omission;
- `timeline_ndjson_bytes`/`decode_timeline_ndjson` round trip; refusals for no trailing newline, two
  trailing newlines, an empty line, and a non-canonical line;
- `chunk_json_bytes` equals the canonical row array with no trailing newline;
- `decode_canonical_bytes` refuses non-canonical bytes;
- `ChunkViewCursor` first appearance across two chunks of one profile, and independence between two
  cursors;
- `cast_canvas` maximum over mixed viewports and its empty-row refusal;
- `rebuild_presented_cast` skips NotPresented rows, accumulates their interval, and propagates a
  loader error;
- `validate_profile_manifest` accepts the smoke shape and refuses each mutated field and each
  mismatched expectation;
- `validate_run_metadata` refusals: wrong schema, wrong result, empty revision, bad digest, empty
  profile list, unsorted or duplicate profile ids;
- `bundle_digest` stability and sensitivity to both inputs;
- `source_revision` clean tree, dirty tree with a diff only, dirty tree with untracked files, a
  reader error, and invalid UTF-8 in an untracked path;
- `BundleLayout::compare` refusals for a missing file, an extra file, a missing directory, and an
  extra directory, and `expect_bundle` deriving the object and view sets from rows;
- one engine test pinning exactly one frontend and one host observation per checkpoint.

skit-cli, over `TempDir`:

- install the real smoke bundle, read the installed bundle back, and assert the exact file and
  directory sets;
- main and replay bundles are byte-identical file for file;
- object deduplication writes one file for a repeated state;
- collision refusals for a differing object, a differing frame view, and a differing generic
  artifact;
- one corruption refusal per read-back check listed above;
- allowlist refusals: extra file, extra directory, missing declared file;
- symlink refusal under `cfg(unix)`, including a symlink whose target is a declared file;
- non-regular-entry refusal under `cfg(unix)` using `std::os::unix::net::UnixListener`, which is
  deterministic where a FIFO may not be;
- fingerprint refusal when the injected sampler changes, asserting it ran exactly twice and that no
  `bundle-*` and no `.walker-*` directory remain;
- reinstall over an identical installed tree returns the existing path and leaves it untouched;
- reinstall over a differing installed tree refuses and leaves it untouched;
- `real_git` refuses an invalid argument, and `source_identity()` returns a nonempty revision;
- the directory-name check refuses a `bundle-` directory whose suffix does not match its contents.

## 7. Verification

```bash
cargo fmt --all --check
cargo clippy --locked -p skit-cli-rs -p skit-tui-walker-support -p skit-tui --all-targets --all-features -- -D warnings
RUSTDOCFLAGS='-D warnings' cargo doc --locked -p skit-cli-rs -p skit-tui-walker-support --all-features --no-deps
cargo test --locked -p skit-tui-walker-support --lib
cargo test --locked -p skit-cli-rs --lib cli::tui_real_walker
cargo test --locked -p skit-cli-rs --lib cli::tui_walker_bundle
cargo test --locked -p skit-tui --test model_walker
bash scripts/check_english.sh
bash scripts/test_coverage.sh
git diff --check
```

Then the workspace coverage gate, and a scoped mutation pass on the new mutation-visible file,
because the design leans on mutation visibility as an argument:

```bash
cargo llvm-cov --locked --workspace --all-targets --all-features --lcov --output-path lcov.info
bash scripts/check_coverage.sh lcov.info
cargo mutants --file crates/skit-tui-walker-support/src/bundle.rs --workspace --all-features \
  --cargo-arg=--locked --jobs 2 --timeout 300
```

## 8. Recorded backlog, deliberately not in Phase3d

- Exhaustive `canonical_action` typed projection. `normalize_action_json` special-cases exactly one
  of `skit_ui::Action`'s variants, `Present(Screen)`, and only four of its screens. Required before
  the 100-operation cutover, with leak and lookalike tests.
- Umask normalization of `portable_mode`. It records `mode & 0o7777` for symlinks, files, and
  directories with no umask handling, so today's real property is same-umask reproducibility. Needed
  before the final four-profile corpus. The fix normalizes umask-derived modes in the observation
  while preserving explicitly set source modes required by v0.4 compatibility. It is not a global
  configuration change.
- `coverage.json`, `review-progress.json`, `README.md`, `review.md`, and the completed-review
  validator, all of which belong to the four-profile review workflow.
- Deletion of the duplicated legacy helpers that section 5 does not already remove.

## 9. Rulings recorded for the reviewers

- **No trust-model violation.** Every refusal here keeps skit from partially committing or silently
  overwriting its own generated data, and each is a literal `HANDOFF.md` Phase3d requirement. No
  escaping, character bans, command sanitization, or permission policy is added for trusted
  user-authored content. The non-finite float path is proven test-only and deliberately left
  unguarded.
- **Localization does not apply.** Both files are dev artifact contracts with no user-visible
  strings. New comments and messages still follow ASD-STE100 and `scripts/check_english.sh`.
- **Dependency direction stays clean.** walker-support gains no product dependency; `tempfile` is
  already a skit-cli dev-dependency; `UnixListener` comes from `std`.
