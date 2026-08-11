//! Mechanical port of the Python oracle module `tests/test_agent_skill.py`
//! (`origin/main@206f9ef`): "The bundled Agent Skill: spec compliance, packaging, and
//! anti-drift." Each `#[test]` keeps its Python `def test_*` name so it traces back to its
//! origin, and each Python "WHY" comment is preserved above it.
//!
//! What the oracle exercises: the bundled `skills/skit/SKILL.md` (the Agent Skill body) and
//! the typer `cli.app` command tree it must stay synchronized with. In Rust the skill is a
//! single file, embedded in the `skit` binary at compile time with `include_bytes!`
//! (`crates/skit-cli/src/cli.rs`), and the command tree is the private `Cli` clap type.
//!
//! Concept mapping used throughout:
//! - Python `ROOT_SKILL` (repo-root `skills/skit/SKILL.md`, what `npx skills add …` discovers)
//!   -> the same repo-root file, read at runtime.
//! - Python `PACKAGED_SKILL` (`src/skit/skills/skit/SKILL.md`, what `skit agent install` writes)
//!   -> the bytes the `skit` binary embeds. Rust keeps ONE source of truth (no second on-disk
//!   copy), so the "root == packaged" invariant is checked against the real installer output:
//!   `skit agent install --to <tmp>` then a byte compare with the repo root.
//! - Python `resources.files("skit").joinpath("skills","skit","SKILL.md")` -> `BUNDLED_SKILL`
//!   (`include_str!` of the repo-root file — the same embedding mechanism the CLI uses).
//! - Python `_frontmatter(text)` -> the local `frontmatter` helper (plain `key: value` lines).
//! - Python `NAME_RE` (`^[a-z0-9]+(-[a-z0-9]+)*$`) -> the local `matches_name_re` helper.
//! - Python `_skill_command_lines()` -> the local `skill_command_lines` helper.
//! - Python `shlex.split(line, comments=True)` -> strip a trailing " #" comment, then
//!   `shlex::split` (the exact shape the crate's own internal test uses).
//! - Python typer/click tree walk (`_click_root`/`_resolve`, `command.params[*].opts`) ->
//!   the private `Cli` clap tree — see the two cross-tier stubs below.
//!
//! Buckets:
//! - Bucket 1 (real asserting tests): packaging/spec/text invariants (root==packaged via the real
//!   installer, ships-inside-package, frontmatter, disclosure budget, placeholder-delivery copy,
//!   empty-value pin spellings).
//! - Bucket 2 (cross-tier, `#[ignore]` compiling stubs): the two command-tree tests
//!   (`test_every_command_the_skill_teaches_exists`,
//!   `test_the_skill_never_mentions_json_free_surfaces_wrongly`). Resolving each `skit …` line and
//!   its flags against the command tree needs skit-cli's PRIVATE `Cli` type (`Cli::try_parse_from`
//!   / `Cli::command()`), which `lib.rs` does not export — unreachable from an integration test
//!   without pub-exposing it. The behavior is already owned by the internal unit test
//!   `every_agent_skill_command_example_matches_the_real_cli_tree`
//!   (`crates/skit-cli/src/cli/tests.rs`), which drives the real tree via `Cli::try_parse_from`.

use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

use tempfile::TempDir;

/// The Agent Skill the `skit` binary embeds — the Rust twin of
/// `resources.files("skit").joinpath("skills","skit","SKILL.md")`. `include_str!` is the same
/// embedding mechanism the CLI uses.
const BUNDLED_SKILL: &str = include_str!("../../../skills/skit/SKILL.md");

/// The repo-root `skills/skit/SKILL.md` — what `npx skills add t41372/skit` discovers.
fn root_skill_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("skills/skit/SKILL.md")
}

/// Python `_frontmatter`: the skill's YAML frontmatter as plain `key: value` lines (skit's own
/// skill deliberately keeps it flat, so a line parser is enough — no YAML dependency).
fn frontmatter(text: &str) -> BTreeMap<String, String> {
    assert!(
        text.starts_with("---\n"),
        "SKILL.md must open with a frontmatter fence"
    );
    // Python `text.split("---\n", 2)[1]` — the block between the first two fences. Python caps the
    // split at 2 only to keep the tail intact; taking the second piece is identical either way.
    let block = text
        .split("---\n")
        .nth(1)
        .expect("frontmatter block present");
    let mut out = BTreeMap::new();
    for line in block.lines() {
        let (key, value) = line
            .split_once(':')
            .unwrap_or_else(|| panic!("frontmatter line without a colon: {line:?}"));
        out.insert(key.trim().to_owned(), value.trim().to_owned());
    }
    out
}

/// Python `NAME_RE = re.compile(r"^[a-z0-9]+(-[a-z0-9]+)*$")` (agentskills.io: lowercase
/// alphanumerics joined by single hyphens). `split('-')` yields an empty segment for a leading,
/// trailing, or doubled hyphen, so the per-segment check covers every boundary rule.
fn matches_name_re(name: &str) -> bool {
    !name.is_empty()
        && name.split('-').all(|segment| {
            !segment.is_empty()
                && segment
                    .bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
        })
}

/// Python `_skill_command_lines()`: every fenced-code line that starts with `skit ` (the fence
/// toggles on any ``` line, so both the bash and python blocks are "inside a block").
fn skill_command_lines(text: &str) -> Vec<String> {
    let mut lines = Vec::new();
    let mut in_block = false;
    for raw in text.lines() {
        let trimmed = raw.trim();
        if trimmed.starts_with("```") {
            in_block = !in_block;
            continue;
        }
        if in_block && trimmed.starts_with("skit ") {
            lines.push(trimmed.to_owned());
        }
    }
    lines
}

/// Python `shlex.split(line, comments=True)`: drop a trailing " #" comment, then tokenize. This is
/// the exact two-step shape the crate's own internal skill test uses.
fn split_command_line(line: &str) -> Vec<String> {
    let command = line.split_once(" #").map_or(line, |(command, _)| command);
    shlex::split(command).unwrap_or_else(|| panic!("invalid shell line: {line}"))
}

#[test]
fn test_root_and_packaged_copies_are_identical() {
    // To update the skill: edit skills/skit/SKILL.md, then rebuild — the CLI re-embeds it.
    //
    // The oracle asserts the repo-root copy (what npx discovers) is byte-identical to the packaged
    // copy (what `skit agent install` writes). Rust keeps ONE source of truth, embedded in the
    // binary, so the falsifiable port drives the REAL installer: a stale/renamed embed path would
    // ship bytes that differ from the repo root, and this catches exactly that.
    let data = TempDir::new().unwrap();
    let state = TempDir::new().unwrap();
    let config = TempDir::new().unwrap();
    let home = TempDir::new().unwrap();
    let target = TempDir::new().unwrap();

    let mut command = assert_cmd::cargo::cargo_bin_cmd!("skit");
    command
        .env("SKIT_DATA_DIR", data.path())
        .env("SKIT_STATE_DIR", state.path())
        .env("SKIT_CONFIG_DIR", config.path())
        .env("SKIT_LANG", "en")
        .env("HOME", home.path())
        .env("USERPROFILE", home.path())
        .args(["agent", "install", "--to", target.path().to_str().unwrap()])
        .assert()
        .success();

    let installed = target.path().join("skit/SKILL.md");
    assert!(installed.is_file(), "installer wrote no SKILL.md");
    assert_eq!(
        fs::read(&installed).unwrap(),
        fs::read(root_skill_path()).unwrap(),
        "the installed (packaged) skill must be byte-identical to the repo-root copy",
    );
}

#[test]
fn test_skill_ships_inside_the_package() {
    // The oracle proves the skill is reachable as a package resource and matches the repo root.
    // The Rust binary embeds it with `include_str!`, so the embedded bytes ARE the shipped skill:
    // non-empty, carrying the frontmatter marker, and equal to the repo-root file.
    assert!(!BUNDLED_SKILL.is_empty());
    assert!(BUNDLED_SKILL.starts_with("---\nname: skit\n"));
    assert_eq!(
        BUNDLED_SKILL,
        fs::read_to_string(root_skill_path()).unwrap(),
        "the embedded skill must equal the repo-root copy",
    );
}

#[test]
fn test_frontmatter_satisfies_the_agent_skills_spec() {
    let fm = frontmatter(BUNDLED_SKILL);
    assert_eq!(fm["name"], "skit");
    // spec: name must match the directory that holds SKILL.md
    assert_eq!(
        fm["name"],
        root_skill_path()
            .parent()
            .unwrap()
            .file_name()
            .unwrap()
            .to_str()
            .unwrap()
    );
    assert!(matches_name_re(&fm["name"]));
    assert!(fm["name"].chars().count() <= 64);
    assert!((1..=1024).contains(&fm["description"].chars().count()));
    if let Some(compatibility) = fm.get("compatibility") {
        assert!((1..=500).contains(&compatibility.chars().count()));
    }
    assert_eq!(fm["license"], "MIT");
}

#[test]
fn test_skill_stays_within_the_progressive_disclosure_budget() {
    // The spec recommends keeping SKILL.md under 500 lines; agents load the whole body
    // on activation, so bloat here is a per-use context tax.
    assert!(BUNDLED_SKILL.lines().count() < 500);
}

#[test]
#[ignore = "CROSS-TIER (private clap tree): resolving each `skit …` line and its flags against the \
command tree needs skit-cli's PRIVATE `Cli` type (`Cli::try_parse_from` / `Cli::command()`), which \
`lib.rs` does not export — unreachable from an integration test without pub-exposing it. The \
behavior is owned by the internal unit test `every_agent_skill_command_example_matches_the_real_cli_tree` \
(crates/skit-cli/src/cli/tests.rs), which drives the real tree via `Cli::try_parse_from`. The \
observable `>= 15` half is asserted in the body below."]
fn test_every_command_the_skill_teaches_exists() {
    // Python: assert len(lines) >= 15 (the skill actually teaches the surface, not a stub), then
    // resolve each line against `_click_root()` and assert every `--flag`/`-x` is in that
    // command's declared opts. The resolution/flag half needs the private clap tree (see the
    // attribute); only the line-count half is observable from here.
    let lines = skill_command_lines(BUNDLED_SKILL);
    assert!(
        lines.len() >= 15,
        "the skill teaches only {} commands",
        lines.len()
    );
}

#[test]
#[ignore = "CROSS-TIER (private clap tree): verifying every documented `--json` is actually offered \
by that exact command needs skit-cli's PRIVATE `Cli` tree, unreachable from an integration test. \
`Cli::try_parse_from` in the internal unit test \
`every_agent_skill_command_example_matches_the_real_cli_tree` (crates/skit-cli/src/cli/tests.rs) \
rejects a `--json` on a command that lacks it, so it owns this guarantee."]
fn test_the_skill_never_mentions_json_free_surfaces_wrongly() {
    // Every `--json` the skill shows must be real: a command that silently ignores an unknown flag
    // does not exist in click/typer (it errors); the cheap guarantee is that we never document
    // --json on a command that lacks it. Python iterates each `--json` line, resolves it against
    // `_click_root()`, and asserts `--json` is in that command's opts — the resolve/opts step needs
    // the private clap tree named in the attribute.
}

#[test]
fn test_skill_describes_placeholder_delivery_for_both_real_entry_kinds() {
    assert!(BUNDLED_SKILL.contains("registered command template or prompt body"));
    assert!(BUNDLED_SKILL.contains("placeholder` (command templates and prompt bodies)"));
}

#[test]
fn test_skill_teaches_executable_empty_value_spellings_for_clearing_pins() {
    let commands: Vec<Vec<String>> = skill_command_lines(BUNDLED_SKILL)
        .iter()
        .map(|line| split_command_line(line))
        .collect();
    let runner_clear: Vec<String> = ["skit", "params", "<name>", "--runner", ""]
        .into_iter()
        .map(str::to_owned)
        .collect();
    let interpreter_clear: Vec<String> = ["skit", "params", "<name>", "--interpreter", ""]
        .into_iter()
        .map(str::to_owned)
        .collect();
    assert!(
        commands.contains(&runner_clear),
        "skill must teach clearing the runner pin with an empty value"
    );
    assert!(
        commands.contains(&interpreter_clear),
        "skill must teach clearing the interpreter pin with an empty value"
    );
}
