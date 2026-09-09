+# UI walker review

## Verdict

One P1 finding survives. The finding has the same root cause in all four profiles.

### P1 — An untouched Entry settings save deletes stored parameter declarations

Trigger: open Entry settings for the shell entry, then press `Ctrl+S` without changing a field.
This is operation 12. The reducer submits `values={}`. The save returns `Settings saved`.

Evidence in `en-80x24`:

- Before the save, sequence 3 presents Run with `Preset`, `pattern`, and `Extra arguments` in styled frame `75809f55b0a0e891da58c8c336fb93dc2e382865e9dde16d0d55f333e4ba0297`. The initial host tree in `d4c7cb67fe1a8602ad578e2ec592a4e628b3488ece55ef00e39dff2182cece47` contains `meta.toml` with `params = ["pattern"]` and a `[[parameters]]` row for `pattern`.
- Sequence 21 is the causal reducer row. Its reducer is `cf3d7cbf89a899cda156c2a8f9daf948dda2ed49e025e4906d89b5295a111260`; its action is `settings.save`, and its emitted submit has an empty values object. This row is `presentation=not_presented` and is treated only as causal evidence.
- Sequence 22 is the presented host completion. Host object `b0ba49c681b610ebe281270530d86ea2fd4608ca67c2aa16932f419fa519b4a2` reports `parameters: []`. Its `meta.toml` keeps the stale `params = ["pattern"]` scalar but has lost the entire `[[parameters]]` declaration. The presented frame is `3ba94700a5d2bac717308ed80c732eb2e2ded861bb0b01d8f9eda6aaeb9ff022`; its detail pane no longer shows `Parameters pattern=*`.
- Sequence 168 later opens Run. Host object `549d81fecfb5dbb44cd1b3517cf5813b4cf6cce323cc79bc81e383a21f206016` presents one field, `_skit_args`; the earlier `value:pattern` field is gone. Styled frame `af27f7005abec881bdbb017832f980d4091023d24724b8b01f0b4916be24cc96` shows only `Extra arguments`.

The same sequence transition drops `parameters` in `zh-cn-120x30` (`7c7527ffc78800dc342f8573375fd6e7ce672ec4940a241110b4daebbaef69cc`), `zh-tw-40x40` (`5afec7195740809d68c71b6ac49f58f884a9dcc10f71bd484b8a446f8bcf3757`), and `pseudo-120x12` (`7e2c906b55981a2c0ca0db83fb321aac4da16f21a884ca77acb3b2253f343686`). Their later sequence 168 Run responses also contain only `_skit_args`.

The control flow explains the loss. `source_owned_schema` classifies shell as source-owned (`crates/skit-cli/src/cli.rs:8123-8127`). The settings screen therefore exposes only the source resync control for this schema. On save, `tui_submit_settings_at` starts from stored settings, keeps untouched axes by key presence, and then unconditionally assigns `settings.parameters = Vec::new()` for source-owned kinds (`crates/skit-cli/src/cli.rs:10995-11000`, `11095-11118`, `11161-11178`). With no source-management request, the source bytes stay unchanged, but the metadata update still removes the existing declaration block. `tui_complete_at` then rebuilds the surface and the next run form from the damaged metadata.

Impact: a user can lose a parameter definition by saving an unchanged settings screen. The metadata becomes internally inconsistent, and later runs omit the parameter input and its flag. The user receives a successful save message and no warning. Reproduction needs one copied shell entry with a legacy or declared `pattern` parameter, followed by the trigger above. The corpus reproduces it in every locale and viewport profile.

I did not count the nested pseudo markers on host status strings as a product finding. `x-pseudo` is explicitly a test aid and is absent from the normal locale picker. I also did not count the 24x6 footer and long-name clipping: the footer exposes a scroll indicator and the source has explicit narrow-footer scroll contracts; the entry remains keyboard and mouse selectable.

## Review method

I read the manifest in order: `en-80x24`, `zh-cn-120x30`, `zh-tw-40x40`, and `pseudo-120x12`. I reviewed all 32 chunks and all 181 checkpoint rows in each profile: 724 rows total, with 552 presented rows and 172 `not_presented` rows. The four initial rows and four final-liveness rows are included in the presented count. The shared vector has 100 operations; each profile contains 179 operation rows because some operations produce multiple checkpoints.

I parsed every referenced reducer, host, session, geometry, and styled-frame object when it first appeared. The complete union was 80 geometry objects, 428 host objects, 199 reducer objects, 484 session objects, and 385 styled-frame objects: 1,576 unique objects. I checked chunk and object SHA-256 values, reconstructed every presented frame from its cells, inspected first-appearance state and hit maps, and followed each keyboard, mouse, paste, resize, and focus transition. I treated `presentation=not_presented` rows as causal state only. I traced the surviving finding through the real reducer, TUI host, CLI settings save, metadata write, surface projection, and later Run form.

The review used only the assigned corpus and the matching source worktree. It made no source edits. It did not run a live terminal session; conclusions are limited to the declared operation vector, four viewports, and the captured real-host state.
