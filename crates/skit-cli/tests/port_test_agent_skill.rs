//! Behavioral ports of Python `tests/test_agent_skill.py` at `main@206f9ef`.
//!
//! Rust has no second wheel-copy of the skill: `skit-cli` embeds the repo-root skill into the
//! binary. The first two Python anti-drift contracts therefore cross the real `skit agent install`
//! boundary and demand that the shipped bytes installed by the binary are byte-identical to the
//! repo-root source, even when the process runs outside the checkout.

use std::{collections::BTreeMap, fs, path::Path};

use assert_cmd::Command;
use regex::Regex;
use tempfile::TempDir;

const ROOT_SKILL: &str = include_str!("../../../skills/skit/SKILL.md");

fn hermetic_cli(home: &Path) -> Command {
    let mut command = assert_cmd::cargo::cargo_bin_cmd!("skit");
    command
        .env("SKIT_LANG", "en")
        .env("SKIT_DATA_DIR", home.join("data"))
        .env("SKIT_STATE_DIR", home.join("state"))
        .env("SKIT_CONFIG_DIR", home.join("config"))
        .env("HOME", home)
        .env("USERPROFILE", home)
        .env("XDG_CONFIG_HOME", home.join("xdg-config"))
        .env("XDG_DATA_HOME", home.join("xdg-data"))
        .env("XDG_STATE_HOME", home.join("xdg-state"))
        .current_dir(home);
    command
}

fn install_from_binary(home: &Path, destination: &Path) -> Vec<u8> {
    let output = hermetic_cli(home)
        .args(["agent", "install", "--to"])
        .arg(destination)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "agent install failed: {}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    fs::read(destination.join("skit/SKILL.md")).unwrap()
}

fn frontmatter(text: &str) -> BTreeMap<String, String> {
    assert!(text.starts_with("---\n"));
    let block = text
        .strip_prefix("---\n")
        .and_then(|tail| tail.split_once("---\n").map(|(block, _)| block))
        .expect("skill has closing frontmatter marker");
    block
        .lines()
        .map(|line| {
            let (key, value) = line
                .split_once(':')
                .unwrap_or_else(|| panic!("frontmatter line without a colon: {line:?}"));
            (key.trim().to_owned(), value.trim().to_owned())
        })
        .collect()
}

fn strip_shell_comment(line: &str) -> &str {
    let bytes = line.as_bytes();
    let mut quote = None;
    let mut escaped = false;
    for (index, byte) in bytes.iter().copied().enumerate() {
        if escaped {
            escaped = false;
            continue;
        }
        if byte == b'\\' && quote != Some(b'\'') {
            escaped = true;
            continue;
        }
        if matches!(byte, b'\'' | b'"') {
            if quote == Some(byte) {
                quote = None;
            } else if quote.is_none() {
                quote = Some(byte);
            }
            continue;
        }
        if byte == b'#' && quote.is_none() {
            return line[..index].trim_end();
        }
    }
    line
}

fn skill_command_lines() -> Vec<String> {
    let mut lines = Vec::new();
    let mut in_block = false;
    for line in ROOT_SKILL.lines() {
        if line.trim_start().starts_with("```") {
            in_block = !in_block;
            continue;
        }
        let stripped = line.trim();
        if in_block && stripped.starts_with("skit ") {
            lines.push(stripped.to_owned());
        }
    }
    lines
}

fn shell_tokens(line: &str) -> Vec<String> {
    shlex::split(strip_shell_comment(line))
        .unwrap_or_else(|| panic!("SKILL.md command has invalid shell quoting: {line}"))
}

fn command_path(tokens: &[String]) -> Vec<String> {
    assert_eq!(tokens.first().map(String::as_str), Some("skit"));
    let top = tokens.get(1).expect("skit command has a subcommand").clone();
    let mut path = vec![top.clone()];
    if matches!(top.as_str(), "runner" | "preset" | "agent") {
        let nested = tokens
            .get(2)
            .filter(|token| !token.starts_with('-'))
            .unwrap_or_else(|| panic!("nested command has no operation: {tokens:?}"));
        path.push(nested.clone());
    }
    path
}

fn help_for(home: &Path, path: &[String]) -> String {
    let mut command = hermetic_cli(home);
    command.args(path).arg("--help");
    let output = command.output().unwrap();
    assert!(
        output.status.success(),
        "documented command path is not accepted: skit {}\n{}{}",
        path.join(" "),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

fn documented_flags(tokens: &[String]) -> Vec<&str> {
    let mut flags = Vec::new();
    for token in tokens.iter().skip(1) {
        if token == "--" {
            break;
        }
        if token.starts_with("--") {
            flags.push(token.split_once('=').map_or(token.as_str(), |(flag, _)| flag));
        } else if token.len() == 2 && token.starts_with('-') {
            flags.push(token.as_str());
        }
    }
    flags
}

#[test]
fn test_root_and_packaged_copies_are_identical() {
    let root = TempDir::new().unwrap();
    let destination = root.path().join("installed");

    let installed = install_from_binary(root.path(), &destination);

    assert_eq!(installed, ROOT_SKILL.as_bytes());
}

#[test]
fn test_skill_ships_inside_the_package() {
    let root = TempDir::new().unwrap();
    let empty_runtime_cwd = root.path().join("outside-checkout");
    fs::create_dir_all(&empty_runtime_cwd).unwrap();
    assert!(!empty_runtime_cwd.join("skills/skit/SKILL.md").exists());
    let destination = empty_runtime_cwd.join("agent-skills");

    let installed = install_from_binary(&empty_runtime_cwd, &destination);

    assert_eq!(installed, ROOT_SKILL.as_bytes());
}

#[test]
fn test_frontmatter_satisfies_the_agent_skills_spec() {
    let fm = frontmatter(ROOT_SKILL);
    let name = fm.get("name").expect("name frontmatter");
    let description = fm.get("description").expect("description frontmatter");
    let name_re = Regex::new(r"^[a-z0-9]+(-[a-z0-9]+)*$").unwrap();

    assert_eq!(name, "skit");
    assert_eq!(name, "skit", "frontmatter name must match skills/skit directory");
    assert!(name_re.is_match(name));
    assert!(name.len() <= 64);
    assert!((1..=1024).contains(&description.len()));
    if let Some(compatibility) = fm.get("compatibility") {
        assert!((1..=500).contains(&compatibility.len()));
    }
    assert_eq!(fm.get("license").map(String::as_str), Some("MIT"));
}

#[test]
fn test_skill_stays_within_the_progressive_disclosure_budget() {
    assert!(ROOT_SKILL.lines().count() < 500);
}

#[test]
fn test_every_command_the_skill_teaches_exists() {
    let root = TempDir::new().unwrap();
    let lines = skill_command_lines();
    assert!(
        lines.len() >= 15,
        "SKILL.md stopped teaching the command surface and became a stub"
    );

    for line in &lines {
        let tokens = shell_tokens(line);
        let path = command_path(&tokens);
        let help = help_for(root.path(), &path);
        for flag in documented_flags(&tokens) {
            assert!(
                help.contains(flag),
                "SKILL.md uses unknown flag {flag:?} in {line:?}; help for `skit {}` was:\n{help}",
                path.join(" ")
            );
        }
    }
}

#[test]
fn test_the_skill_never_mentions_json_free_surfaces_wrongly() {
    let root = TempDir::new().unwrap();
    for line in skill_command_lines() {
        if !line.contains("--json") {
            continue;
        }
        let tokens = shell_tokens(&line);
        let path = command_path(&tokens);
        let help = help_for(root.path(), &path);
        assert!(
            help.contains("--json"),
            "--json documented but not offered by `skit {}`: {line}",
            path.join(" ")
        );
    }
}

#[test]
fn test_skill_describes_placeholder_delivery_for_both_real_entry_kinds() {
    assert!(ROOT_SKILL.contains("registered command template or prompt body"));
    assert!(ROOT_SKILL.contains("placeholder` (command templates and prompt bodies)"));
}

#[test]
fn test_skill_teaches_executable_empty_value_spellings_for_clearing_pins() {
    let commands = skill_command_lines()
        .iter()
        .map(|line| shell_tokens(line))
        .collect::<Vec<_>>();

    assert!(commands.contains(&vec![
        "skit".to_owned(),
        "params".to_owned(),
        "<name>".to_owned(),
        "--runner".to_owned(),
        String::new(),
    ]));
    assert!(commands.contains(&vec![
        "skit".to_owned(),
        "params".to_owned(),
        "<name>".to_owned(),
        "--interpreter".to_owned(),
        String::new(),
    ]));
}
