# Real-host corpus review comparison

The user selected Terra high and Luna max on 2026-09-07. Each reviewer received the same
prompt and a separate copy of the same corpus. Only the assigned paths differed. Neither
reviewer received the other report. No model was added to CI.

## Evidence

- Models: `gpt-5.6-terra`, reasoning `high`; `gpt-5.6-luna`, reasoning `max`.
- Corpus: `corpus-91b4e24d55fd694106b8ae3d1c8802875e0dcb5d3406e14f3205890a2072e38d`.
- Source: `aee95d086eaada51ffd585890b5eb1b4962c4b47+worktree:1373509cdf9972dbb8d88ef43d106b18d809fa7cf3465bf521184a98e2b6002f`.
- Manifest SHA-256: `93629c7f2f9a7d92ae7d7bcd09f62795f84862d552e2e9a32e7203891c9d3d8e`.
- Fresh main and replay runs produced identical immutable bytes. The owner passed in
  1490.69 seconds on Linux. The installed corpus is about 218 MiB.
- `diff -rq` found no difference between the two review copies before review.
- Both completed reports passed `validates_a_completed_review_corpus_from_env` through
  the real Linux byte-only validator: 77.19 seconds for Terra and 77.11 seconds for Luna.

Both reviewers covered 32 chunks, 724 checkpoint rows, and 1,576 unique objects. Each profile
has 181 rows. There are 552 presented rows and 172 causal rows that were not presented.
The object union has 199 reducer, 428 host, 484 session, 385 frame, and 80 geometry objects.
Both reviewers kept invisible intermediate frames separate from visible UI evidence.

The raw reports and their bound progress files are preserved here:

- [Terra report](terra.md), [Terra progress](terra-progress.json).
- [Luna report](luna.md), [Luna progress](luna-progress.json).

The initial prompt incorrectly requested progress JSON without a final newline. Both
reviewers received the same correction before validation. This was an orchestrator error.
The reports remain as returned; their progress files bind their exact bytes.

## Independent assessment

| Result in this corpus | Terra high | Luna max |
| --- | --- | --- |
| Reported findings | 1 | 1 |
| Confirmed product defects | 0 | 1 |
| Shared findings | 0 | 0 |
| Missed independently confirmed defects in this corpus | 2 | 1 |
| Rejected defect claims | 1 | 0 |
| Complete structural attestation | Passed | Passed |

### Stored declarations: confirmed

Luna traced a real data-loss path. An unchanged Entry settings save cleared metadata
flag and environment declarations for a source-owned kind. The later Run form lost those
fields. The four profiles show the same root cause, not four separate defects.

The seed is valid stored data. `skit-form::form_plan` and `with_riders` explicitly support
metadata flag and environment riders on source-owned forms. The existing CLI contract
`params_host_updates_managed_secrets_and_skips_an_invalid_environment_source_without_writes`
also requires preservation of a shell metadata rider.

Version `v0.4.0` resolves to `e044d569947e385d206cfa90f12ef69dfcb6ea5e`.
Its `tui_settings.py` collects metadata declarations only for declared-schema kinds and
calls `store.write_parameters` only for that branch. Source-owned saves leave metadata
riders intact. Thus the current unconditional clear was not a compatibility rule.

The repair keeps stored declarations when the settings controls own a source block.
It does not migrate them into source or add a new validation policy. A regression first
failed on an unchanged Python copy save. It now covers Python, shell, JavaScript,
TypeScript, and fish, in copy and reference modes. It checks unchanged metadata bytes,
an unrelated description edit, flag defaults, an environment secret flag, source bytes,
and the later form declarations.

The post-repair corpus also passed fresh main/replay byte equality in 992.00 seconds:
`corpus-03efa4de5166e16029303ab3fca3d8eed158e426b8431b1a3f0e520d4d277158`.
Its source identity is
`aee95d086eaada51ffd585890b5eb1b4962c4b47+worktree:b41d6f0f67b71ebec05e92080af4faa581452f0808e058735c5131e4b39de9a3`.
All four profiles keep `value:pattern` in the later Run responses at sequences 168 and 172.
The original reports remain bound to the pre-repair evidence.

### The word agent: not a missing translation

Terra accurately identified the displayed word but treated an English technical term as
proof of an untranslated string. The title has complete Chinese catalog entries. Both
Chinese READMEs and many catalog rows intentionally use `AI agent` and `Agent Skill`.
An adjacent use of another term can be a copy-consistency suggestion; it does not establish
the reported P2 localization defect. No product change follows from this claim.

### Footer values: later confirmation, missed by both

Native CI later exposed a second defect that was already visible in this reviewed corpus.
In `pseudo-120x12`, presented sequence 55, operation 27, frame
`90467d389937792ee2217c8c54e1c3190a834b5462d10f79d91bbdb56d4b00a2`, row 11
changes `/tmp/skit-ui-walker-v1/...` to `/tmp/skît-ûî-wàlkér-v1/...`. It also adds a second
pair of pseudo-locale markers around the receipt.

The host inserts path values after translating its template. The footer then translated
the completed receipt again. Commit `13160ba0` keeps composed receipts intact and still
translates complete catalog keys from the reducer. A four-locale regression first failed
on changed path letters, then passed. The real corpus case with `TMPDIR=/tmp` also passed.

The final full corpus passed main/replay byte equality in 1017.28 seconds:
`corpus-b77347af5f56e9b77bcafc601a071daeaae03f1a62f9da93b35eb8591f2aabcb`.
Its source identity is
`aee95d086eaada51ffd585890b5eb1b4962c4b47+worktree:ced3df783d9eb6818704d2de8937976392fe8bffec8fad057fb254e46ce91f41`.
All 429 Rust and Cargo files match pushed commit `28df410e`. All four profiles retain
`value:pattern` at sequences 168 and 172. The pseudo sequence 55 frame preserves the exact
path and has one pair of pseudo markers. This evidence confirms both repairs.

Neither model reported this defect. The table includes this later-confirmed miss for both
models. Windows file-handle and root-alias failures are not charged as corpus-review misses:
the reviewed corpus was recorded on Linux and did not contain those Windows failures.

## Review method and limits

Both reviewers used structured deltas and readable views instead of printing repetitive
cell JSON. Both traced their claim to the renderer or save code. Luna connected an early
settings save to lost metadata and a later Run form. Terra found a visible copy detail but
did not reconcile its claim with the broader terminology convention.

Terra reported about nine minutes. Luna did not report elapsed time. Token and cost data
were not collected. These results describe this one corpus and do not establish a general
model ranking. The reports describe the preserved pre-repair corpus; later fixes do not
silently change its source identity or reviewer evidence.
