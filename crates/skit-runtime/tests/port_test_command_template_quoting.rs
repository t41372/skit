//! Public-surface ports of Python v0.4 `tests/test_template_context_quoting.py`.
//!
//! The Python-only quote-state/value helpers are accounted separately by the completeness gate.
//! Every whole-template contract below keeps its frozen Python name. POSIX contracts that proved
//! safety by running `/bin/sh -c` still run a real shell here; rendering-only substitutes are not
//! accepted for those tests. Windows contracts compile and run only on Windows, where the product
//! actually uses list2cmdline/cmd.exe quoting.

use std::{collections::BTreeMap, path::PathBuf};

use skit_application::delivery::Assembly;
use skit_domain::{Entry, EntryKind, EntryMeta, EntrySettings, Slug};
use skit_runtime::{
    LaunchError, LaunchPaths, ProgramProbe, build_launch_plan, render_command_template,
};

fn values(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

#[derive(Debug, Default)]
struct FakeProbe {
    programs: BTreeMap<String, PathBuf>,
    dirs: Vec<PathBuf>,
}

impl ProgramProbe for FakeProbe {
    fn find_program(&self, name: &str) -> Option<PathBuf> {
        self.programs.get(name).cloned()
    }

    fn is_file(&self, _path: &std::path::Path) -> bool {
        false
    }

    fn is_dir(&self, path: &std::path::Path) -> bool {
        self.dirs.iter().any(|candidate| candidate == path)
    }

    fn is_executable(&self, _path: &std::path::Path) -> bool {
        false
    }
}

fn command_entry(template: &str, params: &[&str]) -> Entry {
    let mut meta = EntryMeta::minimal("Demo", EntryKind::parse("command").unwrap());
    meta.workdir = "invoke".to_owned();
    let settings = EntrySettings {
        template: template.to_owned(),
        params: params.iter().map(|name| (*name).to_owned()).collect(),
        ..EntrySettings::default()
    };
    settings.write_to_meta(&mut meta);
    Entry {
        slug: Slug::parse("demo").unwrap(),
        meta,
    }
}

fn launch_command(
    template: &str,
    params: &[&str],
    command_values: BTreeMap<String, String>,
    extra_args: &[&str],
) -> skit_runtime::LaunchPlan {
    let entry = command_entry(template, params);
    let assembly = Assembly {
        args: extra_args.iter().map(|value| (*value).to_owned()).collect(),
        masked_args: extra_args.iter().map(|value| (*value).to_owned()).collect(),
        command_values: command_values.clone(),
        masked_command_values: command_values,
        ..Assembly::default()
    };
    let mut programs = BTreeMap::new();
    #[cfg(windows)]
    programs.insert("cmd.exe".to_owned(), PathBuf::from("cmd.exe"));
    #[cfg(not(windows))]
    programs.insert("sh".to_owned(), PathBuf::from("/bin/sh"));
    build_launch_plan(
        &entry,
        &LaunchPaths {
            script: PathBuf::from("unused"),
            entry_dir: PathBuf::from("/data/scripts/demo"),
            invoke_cwd: PathBuf::from("/invoke"),
        },
        &assembly,
        None,
        None,
        &FakeProbe {
            programs,
            dirs: vec![PathBuf::from("/invoke")],
        },
    )
    .unwrap()
}

#[cfg(not(windows))]
fn run_sh(command: &str) -> std::process::Output {
    std::process::Command::new("/bin/sh")
        .args(["-c", command])
        .output()
        .unwrap()
}

#[cfg(not(windows))]
fn assert_sh_stdout(command: &str, expected: &str) {
    let output = run_sh(command);
    assert!(
        output.status.success(),
        "shell failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(String::from_utf8(output.stdout).unwrap(), expected);
}

#[cfg(not(windows))]
#[test]
fn test_double_quoted_placeholder_neutralizes_command_substitution() {
    let command = render_command_template(
        "printf \"%s\\n\" \"{value}\"",
        &values(&[("value", "$(printf unexpected)")]),
    )
    .unwrap();
    assert_eq!(command, "printf \"%s\\n\" \"\\$(printf unexpected)\"");
    assert_sh_stdout(&command, "$(printf unexpected)\n");
}

#[cfg(not(windows))]
#[test]
fn test_single_quoted_placeholder_stays_literal_with_apostrophe_and_substitution() {
    let command = render_command_template("echo '{v}'", &values(&[("v", "a'b $(id)")])).unwrap();
    assert_eq!(command, "echo 'a'\\''b $(id)'");
    assert_sh_stdout(&command, "a'b $(id)\n");
}

#[cfg(not(windows))]
#[test]
fn test_unquoted_placeholder_embedded_in_a_word() {
    let command =
        render_command_template("echo scale={width}:-1", &values(&[("width", "640")])).unwrap();
    assert_eq!(command, "echo scale=640:-1");
    assert_sh_stdout(&command, "scale=640:-1\n");
}

#[cfg(not(windows))]
#[test]
fn test_unquoted_placeholder_hostile_value_cannot_escape_the_word() {
    let command = render_command_template(
        "echo scale={width}:-1",
        &values(&[("width", "640 $(id)")]),
    )
    .unwrap();
    assert_eq!(command, "echo scale='640 $(id)':-1");
    assert_sh_stdout(&command, "scale=640 $(id):-1\n");
}

#[cfg(not(windows))]
#[test]
fn test_unfilled_placeholder_travels_through_unchanged() {
    assert_eq!(
        render_command_template("echo {leftover}", &BTreeMap::new()).unwrap(),
        "echo {leftover}"
    );
}

#[cfg(not(windows))]
#[test]
fn test_brace_escapes_collapse_inside_quotes_without_disturbing_state() {
    let command =
        render_command_template("echo \"{{x}} {v}\"", &values(&[("v", "$X")])).unwrap();
    assert_eq!(command, "echo \"{x} \\$X\"");
    assert_sh_stdout(&command, "{x} $X\n");
}

#[cfg(not(windows))]
#[test]
fn test_substituted_value_containing_double_braces_is_not_rescanned() {
    let command =
        render_command_template("echo \"{v}\"", &values(&[("v", "{{x}}")])).unwrap();
    assert_eq!(command, "echo \"{{x}}\"");
    assert_sh_stdout(&command, "{{x}}\n");
}

#[cfg(not(windows))]
#[test]
fn test_extra_args_are_appended_shell_quoted_after_the_template() {
    let plan = launch_command("echo {v}", &["v"], values(&[("v", "hi")]), &["a b", "$X"]);
    assert_eq!(plan.program, PathBuf::from("/bin/sh"));
    assert_eq!(plan.args, ["-c", "echo hi 'a b' '$X'"]);
    assert_sh_stdout(&plan.args[1], "hi a b $X\n");
}

#[cfg(not(windows))]
#[test]
fn test_quote_state_affects_only_later_placeholders() {
    let command = render_command_template(
        "echo \"{a}\" {b}",
        &values(&[("a", "$A"), ("b", "$B")]),
    )
    .unwrap();
    assert_eq!(command, "echo \"\\$A\" '$B'");
    assert_sh_stdout(&command, "$A $B\n");
}

#[cfg(not(windows))]
#[test]
fn test_describe_command_uses_the_same_context_aware_quoting() {
    let plan = launch_command("echo \"{v}\"", &["v"], values(&[("v", "$(id)")]), &[]);
    assert_eq!(plan.args, ["-c", "echo \"\\$(id)\""]);
    assert_eq!(plan.display, "/bin/sh -c 'echo \"\\$(id)\"'");
}

#[cfg(not(windows))]
#[test]
fn test_dangling_backslash_before_a_placeholder_cannot_eat_the_value_escape() {
    let command = render_command_template(
        "printf \"%s\\n\" \"foo\\{name}\"",
        &values(&[("name", "$(printf pwned)")]),
    )
    .unwrap();
    assert_eq!(command, "printf \"%s\\n\" \"foo\\\\\\$(printf pwned)\"");
    assert_sh_stdout(&command, "foo\\$(printf pwned)\n");
}

#[cfg(not(windows))]
#[test]
fn test_dangling_backslash_in_unquoted_position_is_neutralized_too() {
    let command = render_command_template(
        "printf %s\\\\n foo\\{name}",
        &values(&[("name", "$(printf pwned)")]),
    )
    .unwrap();
    assert_eq!(command, "printf %s\\\\n foo\\\\'$(printf pwned)'");
    assert_sh_stdout(&command, "foo\\$(printf pwned)\n");
}

#[cfg(not(windows))]
#[test]
fn test_even_backslash_run_before_a_placeholder_adds_no_neutralizer() {
    let command = render_command_template(
        "printf \"%s\\n\" \"a\\\\{name}\"",
        &values(&[("name", "$(printf pwned)")]),
    )
    .unwrap();
    assert_eq!(command, "printf \"%s\\n\" \"a\\\\\\$(printf pwned)\"");
    assert_sh_stdout(&command, "a\\$(printf pwned)\n");
}

#[cfg(not(windows))]
#[test]
fn test_dangling_backslash_before_brace_escape_and_unfilled_placeholder_is_absorbed() {
    let command = render_command_template(
        "printf \"%s\\n\" \"\\{{x}} {later}\"",
        &values(&[("later", "$(printf pwned)")]),
    )
    .unwrap();
    assert_eq!(command, "printf \"%s\\n\" \"\\{x} \\$(printf pwned)\"");
    assert_sh_stdout(&command, "\\{x} $(printf pwned)\n");

    assert_eq!(
        render_command_template("\"\\{never} {later}\"", &values(&[("later", "$(x)")]))
            .unwrap(),
        "\"\\{never} \\$(x)\""
    );
}

#[cfg(windows)]
#[test]
fn test_render_win32_uses_list2cmdline_not_posix_quoting() {
    let plan = launch_command("echo {v}", &["v"], values(&[("v", "a b")]), &["c d"]);
    assert_eq!(plan.program, PathBuf::from("cmd.exe"));
    assert_eq!(plan.args, ["/C", "echo \"a b\" \"c d\""]);
}

#[cfg(windows)]
#[test]
fn test_render_win32_repl_handles_brace_escapes_and_unfilled_placeholders() {
    assert_eq!(
        render_command_template(
            "echo {{x}} {filled} {unfilled}",
            &values(&[("filled", "v")]),
        )
        .unwrap(),
        "echo {x} v {unfilled}"
    );
}

#[cfg(not(windows))]
#[test]
fn test_value_survives_a_nested_command_substitution_verbatim() {
    let cases = [
        ("printf \"%s\\n\" \"$(printf %s {v})\"", "safe; printf INJECTED"),
        ("printf \"%s\\n\" \"$(printf %s {v})\"", "a b"),
        ("printf \"%s\\n\" \"$(printf %s \"{v}\")\"", "$(printf PWNED)"),
        ("printf \"%s\\n\" \"$(printf %s \"{v}\")\"", "a b"),
        ("printf \"%s\\n\" \"`printf %s {v}`\"", "safe; printf INJECTED"),
        ("printf \"%s\\n\" \"`printf %s '{v}'`\"", "it's $HOME"),
        (
            "printf \"%s\\n\" \"$(printf %s \"$(printf %s \"{v}\")\")\"",
            "deep 'a b' $X",
        ),
    ];
    for (template, value) in cases {
        let command = render_command_template(template, &values(&[("v", value)])).unwrap();
        assert_sh_stdout(&command, &format!("{value}\n"));
    }
}

#[cfg(not(windows))]
#[test]
fn test_double_quotes_nested_in_backticks_are_refused_not_guessed() {
    let error = render_command_template(
        "printf \"%s\\n\" \"`printf %s \"{v}\"`\"",
        &values(&[("v", "$(printf PWNED)")]),
    )
    .unwrap_err();
    assert!(matches!(&error, LaunchError::UnsafeTemplatePlaceholder { .. }));
    assert_eq!(
        error.to_string(),
        "Can't safely fill in a value inside double quotes nested in a `…` command substitution — the shell strips one layer of escaping there. Rewrite that part of the template with $(…) instead of backticks."
    );
}

#[test]
fn rust_additive_uppercase_placeholder_is_substituted() {
    assert_eq!(
        render_command_template("echo {NAME}", &values(&[("NAME", "x")])).unwrap(),
        "echo x"
    );
}

#[cfg(not(windows))]
#[test]
fn rust_additive_replacement_text_is_not_reparsed_as_another_placeholder() {
    assert_eq!(
        render_command_template("echo {a} {b}", &values(&[("a", "{b}"), ("b", "real")])).unwrap(),
        "echo '{b}' real"
    );
}

#[cfg(not(windows))]
#[test]
fn rust_additive_double_quoted_value_escapes_backslash_quote_dollar_and_backtick() {
    assert_eq!(
        render_command_template(
            "printf \"%s\" \"{v}\"",
            &values(&[("v", "\\\"$x`y`")]),
        )
        .unwrap(),
        "printf \"%s\" \"\\\\\\\"\\$x\\`y\\`\""
    );
}
