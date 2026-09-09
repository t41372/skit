# Phase4 G4 design, revision 31

Worktree `/home/ubuntu/coding/skit-uiwalker-mouse-integration`, branch `integration/ui-walker-on-mouse`,
HEAD `eb7adcd` (H complete). Codex's uncommitted G4a is in `crates/skit-cli/src/cli/tui_real_walker.rs`
(frozen at `refs/codex-review/g4a-trace-precommit-20260902`, commit `a69e164`, Terra 0/0/1 with the
minor fixed afterwards) and its unreviewed G4b start is in `crates/skit-cli/src/cli/tui_walker_bundle.rs`.

G4 is the last Phase4 step: one aggregate filesystem transaction that installs the four-profile
review corpus, a byte-only core read-back, a separate leak validation with the four in-memory
`LeakOracleFacts`, reviewer-file preservation on regeneration, and a completed-review validator.
`PHASE4-PLAN.md` section "G4 — one aggregate filesystem transaction" is the requirement. This file is
the implementation design. Two advisor reviews (2026-09-04, a full-context Fable fork and a
fresh-context Opus reviewer) shaped revision 31; their accepted changes are folded in below.

## D1. Measurement is separate from the final-corpus policy

`measured_effect_coverage_from_timeline` (G3a) tallies a timeline and also requires the exact
required vocabulary (23 served names, 14 nested names, exact key lists). Codex's G4a
`RecordedRealCorpus::new` also requires the canonical 100-operation vector. Under those rules the
aggregate transaction can only run on the full corpus, about 495 seconds per generation. A test that
generates twice takes about 17 minutes. It cannot be a normal `#[test]`: `.cargo/mutants.toml` sets
`test_workspace = true`, `mutation.yml` passes `--timeout 300`, and `ci.yml` runs
`cargo llvm-cov --workspace --all-targets --all-features` without `--include-ignored`, so an
`#[ignore]` owner contributes no line coverage while every `#[cfg(test)]` line under `crates/*/src/`
is gated. The transaction must therefore be exercisable with a small real corpus (four real stable
profiles, a short vector), and the "this is the final corpus" rules are a separate policy that the
final owner applies.

Decision:

- `measured_effect_coverage_from_timeline(rows)` is pure measurement. It tallies typed
  `TransitionCause::Host.request` rows exactly as today and returns a `measured: true` summary whose
  count maps hold only names that occurred. `EffectCoverageTally::measured_summary` still validates
  through `validate_coverage_summary`, which accepts `(measured: true, Some, Some)` with empty maps.
  Contract: a timeline with no nonstatic host request yields `measured: true` with two empty maps.
- `merge_profile_effect_coverage(profiles: &[(SafeProfileId, CoverageSummary)])` is the plain
  required-order sum: exactly four entries in `required_review_profiles()` order, each measured,
  checked addition.
- `merge_required_profile_effect_coverage` keeps its name: it runs
  `validate_required_effect_coverage_summary` on every per-profile summary, calls the plain merge,
  and validates the aggregate. It is the policy wrapper.
- `validate_stored_effect_coverage_summary(stored, profiles)` compares `stored` with the plain
  merge. Byte-only read-back uses it.
- New `validate_final_review_corpus(operations_bytes: &[u8], operation_count: usize, profile_coverage: &[(SafeProfileId, CoverageSummary)], stored: &CoverageSummary) -> Result<(), String>`
  (CLI) is the final policy: `operation_count` equals the canonical vector length (derived from
  `canonical_corpus_artifact_values()`, not a literal); `operations_bytes` equal the canonical bytes
  from `canonical_corpus_artifact_values()` (7,839 bytes, SHA-256 `824a4273…`); every per-profile
  summary passes `validate_required_effect_coverage_summary`; `stored` equals
  `merge_required_profile_effect_coverage(profile_coverage)`. Its positive unit test uses the
  canonical bytes plus `complete_required_coverage(1)` fixtures; the full-run proof is the frozen
  diagnostic (D7).
- `RecordedRealCorpus::new` requires: exactly four profiles in `required_review_profiles()` order
  with matching locale and viewport; a non-empty `operations` vector identical across the four
  profiles and an identical `final_liveness_requested`; last row liveness `Passed`; no
  `not_applicable` session row; every row's profile equal to its trace profile; stable singleton
  sandbox metadata sharing one platform and one root; and it builds
  `SandboxMetadata::stable(platform, root, four ids)`. It does not compare with the canonical vector.

The G3 tests `complete_timeline_builds_one_valid_required_profile_coverage_summary`,
`required_profile_coverage_refuses_missing_extra_static_and_zero_mutations`, and
`four_profile_coverage_merge_is_exact_checked_and_stored_comparable` keep their assertions by
calling the policy validator explicitly.

## D2. Walker-support additions (pure, mutation-visible)

All in `crates/skit-tui-walker-support/src/bundle.rs`, with contracts in `bundle_tests.rs` or
`aggregate_contract_tests.rs`. No `std::fs`, no product dependency.

```rust
/// Files a reviewer may edit inside an installed review corpus.
pub const REVIEWER_MUTABLE_FILES: [&str; 2] = [REVIEW_FILE, REVIEW_PROGRESS_FILE];
/// The installed review corpus directory prefix.
pub const REVIEW_CORPUS_DIRECTORY_PREFIX: &str = "corpus-";

/// True only for the exact root-level relative paths `review.md` and `review-progress.json`.
/// `profiles/x/chunks/review.md` is not reviewer-mutable.
pub fn is_reviewer_mutable(relative: &Path) -> bool;

/// `corpus-<digest>` after `validate_digest`.
pub fn review_corpus_directory_name(digest: &str) -> Result<String, ArtifactError>;

/// Require the regenerated tree and the installed tree to agree on every immutable byte.
/// Both maps are relative path -> bytes. Refuses a path present on one side only, and any byte
/// difference on a path that is not reviewer-mutable. Reviewer-mutable paths may differ.
pub fn compare_regenerated_corpus(
    regenerated: &BTreeMap<PathBuf, Vec<u8>>,
    installed: &BTreeMap<PathBuf, Vec<u8>>,
) -> Result<(), ArtifactError>;

/// Rules only the four-profile review corpus satisfies, on top of `validate_run_metadata`:
/// `run.sandbox.mode() == SandboxMode::Stable` and the sorted `run.profiles` equal the sorted
/// required ids.
pub fn validate_review_corpus_run(run: &RunMetadata) -> Result<(), ArtifactError>;

/// Cross-file binding on top of `validate_manifest` (which already enforces schema 1, exactly
/// four profiles in required order, each embedded profile against `required_review_profiles()`,
/// and global chunk-id uniqueness): `manifest.operations_sha256 == run.operations_sha256`;
/// `stored_profiles == manifest.profiles` (Vec equality in manifest order); the set of
/// `run.profiles` equals the set of manifest ids.
pub fn validate_review_corpus_manifest(
    manifest: &CorpusManifest,
    run: &RunMetadata,
    stored_profiles: &[ProfileManifest],
) -> Result<(), ArtifactError>;

/// The report format rule shared by `validate_review_files` and `validate_completed_review_claim`:
/// UTF-8, non-empty, exactly one trailing newline.
pub fn validate_review_report(report: &[u8]) -> Result<(), ArtifactError>;

/// Reviewer files. `readme` must equal `REVIEW_GUIDE`. `report` must satisfy
/// `validate_review_report`. `progress` decodes with `decode_review_progress(manifest, ..)`.
/// When `progress.review_sha256` is `Some(x)`, `x` must equal `sha256_hex(report)` even when the
/// review is incomplete: a claim that is present must be true. When `progress.complete`,
/// `validate_completed_review_claim(manifest, &progress, report)` must pass. Returns the progress.
pub fn validate_review_files(
    manifest: &CorpusManifest,
    readme: &[u8],
    progress: &[u8],
    report: &[u8],
) -> Result<ReviewProgress, ArtifactError>;
```

Contracts must pin each refusal message and kill mutants: exact root-level matching for
`is_reviewer_mutable`; digest validation and prefix for the directory name; missing, extra, and
changed immutable paths versus changed mutable paths for the comparison; random mode, missing,
extra, and misnamed profiles for the run rule; digest mismatch, stored-profile inequality (including
a permutation with the same set), and profile-set mismatch for the manifest binding; every reviewer
file rule, including a present false `review_sha256` on an incomplete progress, and a complete
progress whose digest matches a report that is still the unedited template.

## D3. Recorded corpus and the writer (CLI)

Keep from Codex's G4a and G4b: `RecordedRealCorpus`, `StableCorpusTracePair` (trace and sandbox
equality per profile, raw leak facts free to differ), `record_corpus_profile`,
`prepare_corpus_host_with`, `combine_corpus_result_after_close`, `canonical_corpus_artifact_values`,
`ReviewCorpusDocuments`, and the `BundleWriter` split (`create_common`, `prepare_profile`,
`write_profile_trace`, `write_root_documents`, `write_review_corpus`). Changes:

- `RecordedRealCorpus::new` per D1.
- `pub(super) fn record_real_review_corpus_in(namespace: StableSandboxNamespace, operations: &[CorpusOperation], phase: StablePairPhase) -> Result<RecordedRealCorpus, String>`
  iterates `required_corpus_factories()` in order and records each profile with
  `record_corpus_profile` into the same namespace (one per-profile lease each; sequential), then
  `RecordedRealCorpus::new`. A failure in profile N closes that host and returns; earlier hosts are
  already closed. `record_real_review_corpus(operations, phase)` uses
  `StableSandboxNamespace::system()`.
- `pub(super) fn record_stable_review_corpus_pair_in(namespace, operations) -> Result<StableCorpusTracePair, String>`
  records main, then replay from fresh seeds. Recording is sequential everywhere; there is one
  assembly path, and the diagnostic uses it too.
- `ReviewCorpusDocuments::new` uses the plain merge. `run.operation_count` is the shared operation
  count. `run.profiles` stays the sorted sandbox key order (`en-80x24, pseudo-120x12, zh-cn-120x30,
  zh-tw-40x40`) while the manifest keeps required order; the existing test pins that they differ.
- `write_review_corpus` writes the shared objects and views, the four profile trees, then
  `operations.json`, `final-liveness.json`, `run.json`, `manifest.json`, `coverage.json`,
  `review-progress.json` (template progress: manifest digest, `complete:false`,
  `review_sha256:null`, empty maps), `README.md` (= `REVIEW_GUIDE`), `review.md`
  (= `REVIEW_TEMPLATE`), each through `write_same_or_new`. The writer only ever writes into the
  fresh staged tree.

## D4. Byte-only core read-back

Refactor `read_bundle` so the per-profile validation is one function shared by the single-profile
reader and the aggregate reader.

```rust
struct ProfileReadback {
    profile: ProfileManifest,
    profile_sha256: String,      // sha256_hex of the stored profile.json bytes, never re-encoded
    rows: Vec<TimelineRow>,
    objects: BTreeMap<(ObjectKind, String), Value>,
}

/// Everything `read_bundle` does after decoding run, operations, and final liveness, for one
/// profile: `profile.json` decode and `validate_profile_manifest(expected)`, digest binding to
/// run, timeline typed round trip, `validate_timeline_semantics`, `validate_live_locales`, row
/// count, first-row identity, chunks and views, objects, styled-frame viewport, cast, reducer
/// replay, host state, effect chain termination. It does not touch `BundleLayout`; the caller
/// declares the layout.
fn validate_profile_tree(
    files: &BTreeMap<PathBuf, Vec<u8>>,
    expected: ProfileExpectation<'_>,
    run: &RunMetadata,
    operations: &[Value],
    final_liveness: &Value,
) -> Result<ProfileReadback, String>;

struct ValidatedReviewCorpus {
    digest: String,
    run: RunMetadata,
    manifest: CorpusManifest,
    operations_bytes: Vec<u8>,
    profiles: Vec<ProfileReadback>,                           // manifest order
    coverage: CoverageSummary,                                // stored aggregate
    profile_coverage: Vec<(SafeProfileId, CoverageSummary)>,  // reconstructed per profile
    progress: ReviewProgress,
    layout: BundleLayout,
}

/// Byte-only. Depends on nothing but the tree under `root`.
fn read_review_corpus(root: &Path) -> Result<ValidatedReviewCorpus, String>;
```

`read_review_corpus` order: `read_tree`; decode `run.json` with `bundle::decode_run_metadata` and
`validate_review_corpus_run`; decode `operations.json` and `final-liveness.json` and bind count and
digests to run as `read_bundle` does; decode `manifest.json` with `decode_manifest` (never plain
serde: `ChunkDescriptor` has no `deny_unknown_fields`); for each `required_review_profiles()[i]`
build `ProfileExpectation { id, locale, viewport, operations_sha256: run.operations_sha256 }` and
call `validate_profile_tree`; collect the stored profile documents and call
`validate_review_corpus_manifest(manifest, run, stored)`; reconstruct per-profile coverage with
`measured_effect_coverage_from_timeline(rows)` and check `validate_stored_effect_coverage_summary`
against the decoded `coverage.json`; `validate_review_files(manifest, README, progress, review)`;
`BundleLayout::expect_review_corpus(manifest, rows_by_profile)` then `compare(actual)`; digest =
`bundle_digest(four profile_sha256 values, run.source_revision, run.sandbox)`; when the directory
name starts with `corpus-`, it must equal `review_corpus_directory_name(digest)`.

`read_bundle` (single profile) becomes: decode run (one profile), operations, liveness;
`validate_profile_tree`; `expect_bundle` layout and compare; digest; `validate_installed_bundle_leaks`.
Its behavior and its tests do not change.

## D5. Leak validation with the four in-memory facts

### Decision update: deterministic frame text (2026-09-07)

The user approved this change after the leak-oracle design audit. The corpus must be
reproducible. It does not protect a user from their own local content.

Stable frames keep the literal sandbox paths and allocator names that the product draws.
This narrows the former rule that no sandbox root may occur in any artifact to non-frame
artifacts. JSON state, causes, effects, sessions, and host observations still use their
typed projections. Raw file identities and modified values still require recorded pairs.

The host resolves its profile root once at creation. It keeps both the declared spelling
and the resolved spelling in the existing path facts under the profile sentinel. This
covers Windows 8.3 names without changing the declared namespace. Path lookup uses that
one root mapping after files are removed. Frames may show either proven spelling; JSON
still projects both. The oracle does not infer aliases from rendered text.

The oracle checks actual checkout, process working directory, home, and system temporary
paths in every channel. A stable sandbox path may have an ambient temporary path as its
parent. Only that occurrence is allowed in a frame; another ambient path in the same row
still fails. Random sandbox paths still fail in frames. Omitted frame cell symbols use
the same rule as visible frame rows. Frame views and casts remain exact derived bytes.

A readable row can end before the full declared namespace fits. At the physical right
edge, its visible path suffix may equal a prefix of that namespace. The suffix must extend
past the matched ambient parent. This rule uses declared metadata and the frame width;
it needs no recorded renderer grants. It does not apply to JSON or omitted symbols.

The oracle no longer infers allocator names from partial text or requires recorded row,
form, or clipping grants. Non-frame checks use exact registered raw values. Unregistered
user lookalikes remain unchanged. Whole-tree equality between fresh main and replay runs
is the primary reproducibility proof.

The checkpoint sink also runs the same scan before it changes the cast, object map, or
timeline. It uses current memory-only facts after host registration and projection. An
error reports the operation and JSON pointer. The complete installed-file scan remains.
Projection and scanning share one text-boundary predicate in walker-support.

This update supersedes the renderer exception and observed-form rules in Phase4 C3 and
the related test cases below. The remaining transaction and byte-only reader rules apply.

```rust
/// Not byte-only. `facts[i]` belongs to manifest profile i.
fn validate_review_corpus_leaks(
    files: &BTreeMap<PathBuf, Vec<u8>>,
    corpus: &ValidatedReviewCorpus,
    facts: &[&LeakOracleFacts],
) -> Result<(), String>;
```

For each profile i, build `InstalledBundleLeakOracle::new(&corpus.run, facts[i])` (its constructor
already forbids every root of every profile in `run.sandbox.profiles()`) and scan, with the existing
per-file dispatch (`masked_run_value` for `run.json`, which already iterates all profiles;
`scan_styled_frame` for the styled frames referenced by this profile's rows; `scan_ndjson_bytes` for
this profile's timeline; the cast skipped after the core rebuild proof; JSON with `scan_json_bytes`;
other files as text), this profile's file set: `profiles/<id>/**`, every object and view referenced
by its rows, plus every root file. Every root file is scanned by all four oracles because only
per-profile draft and artifact facts differ between oracles. A content-addressed object referenced by
two profiles is scanned by both. The union of scanned paths must equal the layout file set
(`require_exact_scan_coverage`). Facts are never merged: `LeakOracleFacts.artifacts` is keyed by
projected path with per-profile raw spellings.

Extract the per-file dispatch loop of `validate_installed_bundle_leaks` into one shared scanner that
takes an oracle, a file subset, and the renderer rows, and returns the scanned set; the single-profile
validator and the corpus validator both call it, and each checks exact coverage once over its union.

Reviewer-authored content is trusted local content. `review.md` and `review-progress.json` are
leak-scanned only while they equal the template bytes the writer produced; when a reviewer has
edited them (the collision path), they are accounted as scanned without scanning. `README.md` stays
zero tolerance because it must equal `REVIEW_GUIDE`.

## D6. The transaction

```rust
/// Install or re-verify the four-profile review corpus under `parent`.
fn install_review_corpus(
    parent: &Path,
    corpus: &RecordedRealCorpus,
    fingerprint: &mut impl FnMut() -> Result<String, String>,
) -> Result<PathBuf, String>;
```

1. `revision = fingerprint()?`.
2. Stage into `tempfile::Builder::new().prefix(".walker-").tempdir_in(parent)` (mode `0700` on
   Unix); `BundleWriter::create_common`; `write_review_corpus(corpus, &revision)`.
3. `(staged_layout, staged_files) = read_tree(staged_path)`.
4. `staged = read_review_corpus(staged_path)?` (byte-only), then
   `validate_review_corpus_leaks(&staged_files, &staged, &facts)` with the four facts from
   `corpus.profiles[i].leak_oracle_facts`.
5. `fingerprint()? == revision`, else refuse; the staged `TempDir` drops and nothing is left behind.
6. `destination_name = review_corpus_directory_name(&staged.digest)`.
7. Publish with a no-replace rename through the pinned parent: open `parent` with
   `PinnedDirectory::open_ambient_parent`, then call the new
   `PinnedDirectory::rename_child_directory_noreplace(&source_name, &destination_name)` (D6a). The
   source is the staged directory's basename; the rename consumes the path, and the `TempDir` value
   still drops afterwards (its cleanup of a path that no longer exists is ignored by `tempfile`).
   - Success: `read_review_corpus(destination)` and the leak validation again; require the installed
     layout and files to equal the staged ones; return the destination path.
   - Refusal because the destination name is in use (`SandboxFsError::destination_already_exists`),
     then `PinnedDirectory::open_directory_shared(&destination_name)` succeeds (a directory of any
     mode, no-follow; a file, symlink, or reparse point fails here with its own error):
     `read_tree(destination)`;
     `compare_regenerated_corpus(&staged_files, &installed_files)`; then `read_review_corpus(destination)`
     (which validates the existing reviewer files against the manifest) and the leak validation;
     return the destination. The destination is never written.
   - Any other refusal (a file, symlink, reparse point, or a foreign error such as a cross-device
     or permission failure) is an error with its own message; only a typed "destination exists"
     refusal enters the compare path.
   - The parent must be a gitignored location: `source_identity()` hashes every untracked file, so a
     corpus written anywhere else changes the fingerprint between step 1 and step 5. `target/` is the
     only safe home inside the checkout.
   - Any difference, corruption, or invalid reviewer file refuses and leaves the destination
     unchanged; the staged tree is dropped.

D6a. Add to `crates/skit-cli/src/cli/tui_real_sandbox_fs.rs`:

```rust
impl PinnedDirectory {
    /// Rename one private child directory to a sibling name without replacing any destination.
    pub(super) fn rename_child_directory_noreplace(
        &self,
        source: &ChildName,
        destination: &ChildName,
    ) -> Result<PinnedDirectory, SandboxFsError>;
}
```

It opens the source through the existing private `open_directory` (mode `0700`, no-follow), builds
`RenameTicket::new(source.clone(), destination.clone(), identity)`, drops the handle, and calls the
existing `rename_noreplace`. Beside it, `SandboxFsError::destination_already_exists(&self) -> bool`
tells that a no-replace rename refused because the name is in use (it does not tell the kind), and
`PinnedDirectory::open_directory_shared(&self, name: &ChildName)` opens a child directory of any
mode without following links, the same policy `open_ambient_parent` applies to the parent. Contracts in the fs module: an absent destination moves the directory
and the returned handle has the source identity; an existing empty directory, a regular file, and a
symlink (Windows: a junction) at the destination refuse and leave both trees untouched; a source
identity swap between open and rename refuses. The installer needs one more narrow accessor only if
resolving "destination exists as a directory" through the pinned parent is impossible with the
current `pub(super)` surface; prefer reusing `open_directory` semantics over any new path check.

## D7. Owners and tests

Location: `<checkout>/target/ui-walker-review-corpus/` (`checkout_root()`), mirroring the legacy
`target/ui-walker-agent-review`. The final corpus uses `StableSandboxNamespace::system()`.

Owner structure. An `#[ignore]`d test body under `crates/*/src/` is a set of uncovered lines
under the CI coverage command (`scripts/check_coverage.sh` has no exemption for it, and the repo's
own ignored tests in `cli/tests.rs` are spawned by non-ignored parents for that reason). A
17-minute owner cannot use that pattern, so no committed test in `src/` is ignored. Instead:

- `pub(super) fn generate_and_install_review_corpus(namespace: StableSandboxNamespace, parent: &Path, operations: &[CorpusOperation], fingerprint: &mut impl FnMut() -> Result<String, String>) -> Result<(PathBuf, StableCorpusTracePair), String>`
  is the permanent generation owner: it records main, installs, records replay from fresh seeds,
  installs again, and returns the installed path with the pair. Fast tests cover every line of it
  with the small vector. The canonical-vector invocation (with `StableSandboxNamespace::system()`,
  `checkout_root().join("target/ui-walker-review-corpus")`, `source_identity`, then
  `read_review_corpus`, `validate_final_review_corpus`, the per-profile `SaveRunner { name ==
  "new-agent" }` and `Add(RememberRunner("new-agent"))` reachability assertions, and no `.walker-*`
  residue) is a temporary diagnostic test applied on top of the committed tree, run locally, and
  frozen as `refs/codex-review/g4-final-full-diagnostic-<date>` exactly like the G3 precedent. The
  handoff records the ref, the command, and the measured wall time; regeneration re-applies it.
- The completed-review validator is a plain function
  `validate_completed_review_corpus(root: &Path, expected_revision: Option<&str>) -> Result<ValidatedReviewCorpus, String>`
  (`read_review_corpus`, `validate_final_review_corpus`, `validate_completed_review_claim`,
  optional revision equality). Its non-ignored test takes the path from `SKIT_WALKER_REVIEW` when set
  and otherwise from a small corpus it completes itself, written without a branch:
  `env::var_os("SKIT_WALKER_REVIEW").map(PathBuf::from).unwrap_or_else(|| small_completed_corpus(..))`,
  and the expected revision from `SKIT_WALKER_EXPECT_REVISION` the same way. A human validates a real
  review by setting the variable; CI covers every line with the small corpus. This env-driven test
  applies the final policy only when the path came from the environment, which is itself a branch;
  so the fast test covers the shared function with `expected_final: bool` passed as a plain
  parameter, and the env-driven entry passes `env::var_os("SKIT_WALKER_REVIEW").is_some()`.
- Measured 2026-09-04: the repository's coverage measurement (`cargo llvm-cov` with its default
  ignore rules) records no `SF` entry for `*_tests.rs`, `tests.rs`, or `tests/` files (117 files in
  the committed `lcov.info`, none of them); `bundle_tests.rs`, `aggregate_contract_tests.rs`, and
  `cli/tests.rs` have always been outside the gate. So the two checked-in owners live in a new
  `crates/skit-cli/src/cli/tui_walker_corpus_tests.rs` (declared `#[cfg(test)] mod` from
  `tui_walker_bundle.rs` or `cli.rs`), both `#[ignore]`, both thin: the canonical-vector generation
  owner calls `generate_and_install_review_corpus` with `StableSandboxNamespace::system()`,
  `checkout_root().join("target/ui-walker-review-corpus")`, `canonical_corpus_operations()`, and
  `source_identity`, then `read_review_corpus`, `validate_final_review_corpus`, the two per-profile
  reachability assertions, the residue check, and prints the path; the env-driven owner calls
  `validate_completed_review_corpus(SKIT_WALKER_REVIEW, SKIT_WALKER_EXPECT_REVISION, true)`. Every
  function they call is covered by the fast tests in `tui_walker_bundle.rs`; the frozen-diagnostic
  route is no longer needed. Selector:
  `cargo test --locked -p skit-cli-rs --lib cli::tui_walker_corpus_tests::generates_and_reinstalls_the_stable_review_corpus -- --exact --ignored --nocapture`.

The small vector must be viewport-independent so all four profiles complete it with zero
`not_applicable` rows, including `pseudo-120x12`: use `Focus`, `CommandKeyboard`, and `RawKey`
operations only (for example `CommandKeyboard(UiCommand::OpenRun)` then `RawKey(Escape)` so the
timeline has a host row for the coverage-change test, then `Focus { gained: false }`). A prefix of
the canonical vector is wrong (its third operation is a viewport-dependent hit).

Fast tests, all with real hosts. Each multi-scenario test records its own small four-profile corpus
into an explicit `StableSandboxNamespace` under a test-owned `TempDir` (no process-lifetime
`OnceLock`; statics never drop). A small vector is, for example, `[Focus { gained: false }]` or the
short prefix already used by `short_stable_corpus_prefix_replays_the_complete_real_trace`. About
40 seconds per corpus under load; keep the count of recordings to what the scenarios need.

1. Fresh install: exact aggregate layout, `read_review_corpus` succeeds, directory name equals
   `corpus-<digest>`, README equals the guide, review equals the template, progress is the template
   progress, `run.profiles` sorted differs from manifest order, no `.walker-*` residue.
2. Reinstall from a fresh replay recording: same path; installed bytes unchanged.
3. Reviewer writes a valid partial `review-progress.json` (one reviewed chunk, `review_sha256`
   equal to an edited `review.md`): reinstall preserves both byte for byte; read-back accepts.
4. Reviewer completes the review (all chunks, all verdicts, matching digest, edited report):
   reinstall preserves; the shared completed-review body passes; the env-driven owner body is
   called directly with the path.
5. Invalid reviewer files refuse the reinstall and leave the destination unchanged: complete
   progress whose digest matches a report that is still the unedited template; a present
   `review_sha256` that does not match the report on an incomplete progress; a stale chunk digest;
   a report without its trailing newline; an edited README.
6. One immutable byte changed in the destination refuses and leaves it unchanged: an object, a
   timeline row, a `coverage.json` count, `manifest.json` profile order, `profile.json` disagreeing
   with the manifest.
7. Changed-source rollback: the fingerprint differs on its second call; no destination and no
   `.walker-*` residue.
8. Extra file, missing file, FIFO (`#[cfg(unix)]`), and symlink paths in the destination refuse.
9. `validate_review_corpus_run` and `RecordedRealCorpus::new` refusals (keep Codex's G4a test,
   rebased on the small corpus rather than a cloned smoke trace where practical; synthetic
   mutations remain acceptable for pure refusal branches).
10. Coverage: deleting one host row from a profile's rows, or renaming one host request, changes the
    reconstruction so the stored summary no longer matches.
11. `validate_final_review_corpus`: the small corpus refuses (count, digest, vocabulary); the
    canonical bytes with `complete_required_coverage(1)` fixtures pass.
12. `record_real_review_corpus_in` failure in a later profile reports the profile and closes hosts
    (the namespace can be re-acquired afterwards).
13. Leak validation: a raw sandbox root or a raw draft path injected into `review.md` is refused by
    `validate_review_corpus_leaks`, not by the byte-only reader; profile A's draft name injected into
    profile B's frame view is refused by B's oracle; scan coverage is exact over four profiles.
14. D6a contracts in the fs module.
15. Root-document shapes for the three new root files (`manifest.json`, `coverage.json`,
    `review-progress.json`): unknown field, missing field, non-object, noncanonical bytes, wrong
    schema, each refused by the byte-only reader, mirroring `read_back_refuses_root_document_corruption`.
16. Cross-profile object sharing is pinned as a fact in the small corpus (whether any object digest is
    referenced by two profiles), so the shared-object double-scan branch is either exercised or
    known to be dead and removed.

Coverage: zero new executable-line misses in every changed file (cargo-llvm-cov plus
`scripts/check_coverage.sh`). Mutation: every D2 rule has a killing contract; the package-scoped
`cargo mutants --no-config --package skit-tui-walker-support --file crates/skit-tui-walker-support/src/bundle.rs`
run is development evidence only; the real workspace gate (`test_workspace = true`) remains the
deferred issue #47 debt, and the small-corpus recordings add to every mutant's suite time.
Preservation is conditional on replay byte equality: `review-progress.json` binds the manifest
digest, so a stable-replay divergence makes regeneration refuse rather than preserve.
Do not touch `.github/workflows/ui-walker.yml`; `scripts/test_tooling_contracts.sh` pins its text.

## D8. Commit split and review

- G4a `feat(walker): define review corpus regeneration contract`: walker-support D2 only.
- G4b `feat(walker): record and write the four-profile review corpus`: D1, D3, D6a (Codex's
  reviewed G4a trace owner plus the writer split), if the reviewer prefers a smaller unit.
- G4c `feat(walker): install the four-profile review corpus`: D4–D7. G4b and G4c may land as one
  commit when the review scope stays intelligible.

Each unit is frozen as an immutable `refs/codex-review/g4*` ref and reviewed read-only by a
fresh-context reviewer at 0 blocking / 0 major / 0 minor before commit. Codex reviewers are out of
quota until 2026-09-06 20:50; both commits get a Codex review when the quota returns, before any
push.

## Implementer traps recorded by the code map and the advisors

- Use `decode_manifest`, never `serde_json::from_slice`, for `manifest.json`.
- `write_same_or_new` refuses differing existing bytes; it only ever runs inside the fresh staged
  tree.
- `validate_manifest` already enforces exactly four profiles in required order and validates each
  embedded profile; do not duplicate it.
- `InstalledBundleLeakOracle::new` and `masked_run_value` already handle every profile in
  `run.sandbox.profiles()`.
- Every line in a `#[cfg(test)]` module under `crates/*/src/` is coverage-gated; use
  `#[cfg(target_os)]` function pairs, no runtime platform branches, no unreachable `else`.
- `refs/codex-review/g3-final-full-diagnostic-20260902` is a stash commit on `095ef1c`; read its
  test `canonical_corpus_reaches_required_measured_coverage_in_every_profile` as an example and
  build on `record_corpus_profile`.
- `PinnedDirectory::open_ambient_parent` exists; `rename_noreplace` re-opens the destination as a
  private directory and syncs the parent; the installed directory keeps `0700`.
- Linux `RENAME_NOREPLACE` needs a supporting filesystem; `target/` on ext4 and CI ext4 are fine,
  the same dependency the sandbox already has.
- Gate new tests with `#[cfg(any(target_os = "linux", target_os = "windows"))]` like their
  neighbours; FIFO tests with `#[cfg(unix)]`.
