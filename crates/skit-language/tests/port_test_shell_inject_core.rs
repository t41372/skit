//! Exact public-surface ports from Python v0.4 `tests/test_shell_inject.py` at `main@206f9ef`.
//!
//! Runtime claims execute the real rewritten source under the requested POSIX shell.  These tests
//! intentionally keep Python's exact behavioral oracle even when the current Rust implementation
//! disagrees; a parity mismatch is allowed to stay red on this test-only branch.

use std::{
    collections::BTreeMap,
    fs,
    io::Write as _,
    process::{Command, Output, Stdio},
};

use skit_domain::parameters::{ParamDecl, ParameterBinding, ParameterDelivery, ParameterType};
use skit_language::{
    LanguageError, ParseOutcome, ShellInputError, inject_values, inject_values_for_interpreter,
    parse_document,
};
use tempfile::TempDir;

fn declarations(source: &str) -> Vec<ParamDecl> {
    let ParseOutcome::Parsed(document) = parse_document("shell", source) else {
        panic!("expected valid shell source");
    };
    document
        .analysis()
        .candidates
        .into_iter()
        .map(|candidate| candidate.declaration)
        .collect()
}

fn values(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn inject(source: &str, pairs: &[(&str, &str)]) -> Result<String, LanguageError> {
    inject_values("shell", source, &declarations(source), &values(pairs))
}

fn inject_for(
    interpreter: &str,
    source: &str,
    pairs: &[(&str, &str)],
) -> Result<String, LanguageError> {
    inject_values_for_interpreter(
        "shell",
        source,
        &declarations(source),
        &values(pairs),
        Some(interpreter),
    )
}

fn const_decl(name: &str, parameter_type: ParameterType) -> ParamDecl {
    let mut declaration = ParamDecl::new(name);
    declaration.binding = ParameterBinding::Const;
    declaration.delivery = ParameterDelivery::Inject;
    declaration.parameter_type = parameter_type;
    declaration
}

fn input_decl(name: &str, order: i64, prompt: &str, secret: bool) -> ParamDecl {
    let mut declaration = ParamDecl::new(name);
    declaration.binding = ParameterBinding::Input;
    declaration.delivery = ParameterDelivery::Inject;
    declaration.order = order;
    declaration.prompt = prompt.to_owned();
    declaration.secret = secret;
    declaration
}

#[cfg(unix)]
fn shell_available(program: &str) -> bool {
    Command::new(program)
        .args(["-c", "exit 0"])
        .status()
        .is_ok_and(|status| status.success())
}

#[cfg(unix)]
fn run_shell(program: &str, source: &str, stdin: &str) -> (TempDir, Output) {
    let root = TempDir::new().unwrap();
    let path = root.path().join("injected.sh");
    fs::write(&path, source).unwrap();
    let mut child = Command::new(program)
        .arg(&path)
        .current_dir(root.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    if let Some(mut pipe) = child.stdin.take() {
        pipe.write_all(stdin.as_bytes()).unwrap();
    }
    let output = child.wait_with_output().unwrap();
    (root, output)
}

#[cfg(unix)]
#[test]
fn test_const_injection_runs_with_the_new_value() {
    if !shell_available("bash") { return; }
    let source = "#!/usr/bin/env bash\nWIDTH=800\necho \"w=$WIDTH\"\n";
    let rewritten = inject(source, &[("WIDTH", "1200")]).unwrap();
    let (_, output) = run_shell("bash", &rewritten, "");
    assert_eq!(output.status.code(), Some(0), "{}", String::from_utf8_lossy(&output.stderr));
    assert_eq!(String::from_utf8(output.stdout).unwrap(), "w=1200\n");
}

#[test]
fn test_const_str_is_single_quoted_and_int_is_bare() {
    let source = "#!/usr/bin/env bash\nWIDTH=800\nCITY=Taipei\n";
    let rewritten = inject(source, &[("WIDTH", "1200"), ("CITY", "New York")]).unwrap();
    assert!(rewritten.contains("WIDTH=1200"), "{rewritten}");
    assert!(rewritten.contains("CITY='New York'"), "{rewritten}");
}

#[cfg(unix)]
#[test]
fn test_const_rewrites_every_same_name_occurrence() {
    if !shell_available("bash") { return; }
    let source = "#!/usr/bin/env bash\nMODE=fast\nMODE=slow\necho \"$MODE\"\n";
    let rewritten = inject(source, &[("MODE", "turbo")]).unwrap();
    assert_eq!(rewritten.matches("MODE='turbo'").count(), 2, "{rewritten}");
    let (_, output) = run_shell("bash", &rewritten, "");
    assert_eq!(String::from_utf8(output.stdout).unwrap(), "turbo\n");
}

#[test]
fn test_const_quoting_is_normalized_not_preserved() {
    let source = "#!/usr/bin/env bash\nA=bare\nB='raw'\nC=\"double\"\n";
    let rewritten = inject(source, &[("A", "x y"), ("B", "x y"), ("C", "x y")]).unwrap();
    assert_eq!(rewritten.matches("='x y'").count(), 3, "{rewritten}");
}

#[test]
fn test_bad_int_value_raises_the_value_error_not_drift() {
    let error = inject("#!/usr/bin/env bash\nWIDTH=800\n", &[("WIDTH", "not-a-number")]).unwrap_err();
    assert_eq!(
        error,
        LanguageError::InvalidValue {
            name: "WIDTH".to_owned(),
            value: "not-a-number".to_owned(),
            parameter_type: ParameterType::Int,
        }
    );
}

#[test]
fn test_bad_float_and_non_finite_values_are_refused() {
    for bad in ["abc", "inf", "-inf", "nan"] {
        assert_eq!(
            inject("#!/usr/bin/env bash\nRATE=0.5\n", &[("RATE", bad)]).unwrap_err(),
            LanguageError::InvalidValue {
                name: "RATE".to_owned(),
                value: bad.to_owned(),
                parameter_type: ParameterType::Float,
            }
        );
    }
}

#[cfg(unix)]
#[test]
fn test_float_const_injects_a_bare_number() {
    if !shell_available("bash") { return; }
    let source = "#!/usr/bin/env bash\nRATE=0.5\necho \"r=$RATE\"\n";
    let rewritten = inject(source, &[("RATE", "2.75")]).unwrap();
    assert!(rewritten.contains("RATE=2.75"), "{rewritten}");
    let (_, output) = run_shell("bash", &rewritten, "");
    assert_eq!(String::from_utf8(output.stdout).unwrap(), "r=2.75\n");
}

#[test]
fn test_missing_const_target_is_drift() {
    let declaration = const_decl("GONE", ParameterType::Str);
    let error = inject_values(
        "shell",
        "#!/usr/bin/env bash\nWIDTH=800\n",
        &[declaration],
        &values(&[("GONE", "x")]),
    )
    .unwrap_err();
    assert_eq!(error, LanguageError::BindingNotFound { name: "GONE".to_owned() });
}

#[test]
fn test_readonly_const_is_never_a_target() {
    let declaration = const_decl("MAX", ParameterType::Int);
    let error = inject_values(
        "shell",
        "#!/usr/bin/env bash\nreadonly MAX=100\n",
        &[declaration],
        &values(&[("MAX", "5")]),
    )
    .unwrap_err();
    assert_eq!(error, LanguageError::BindingNotFound { name: "MAX".to_owned() });
}

#[cfg(unix)]
#[test]
fn test_const_targets_skip_array_and_valueless_assignments() {
    if !shell_available("bash") { return; }
    let source = "#!/usr/bin/env bash\nARR[0]=1\nEMPTY=\nWIDTH=800\necho \"$WIDTH${ARR[0]}[$EMPTY]\"\n";
    let rewritten = inject(source, &[("WIDTH", "1200")]).unwrap();
    assert!(rewritten.contains("ARR[0]=1"), "{rewritten}");
    assert!(rewritten.contains("EMPTY=\n"), "{rewritten}");
    let (_, output) = run_shell("bash", &rewritten, "");
    assert_eq!(String::from_utf8(output.stdout).unwrap(), "12001[]\n");
}

#[cfg(unix)]
#[test]
fn test_read_interception_echoes_prompt_and_value() {
    if !shell_available("bash") { return; }
    let source = "#!/usr/bin/env bash\nread -p \"Name: \" who\necho \"hi $who\"\n";
    let rewritten = inject(source, &[("input-1", "Ada")]).unwrap();
    let (_, output) = run_shell("bash", &rewritten, "");
    assert_eq!(String::from_utf8(output.stdout).unwrap(), "Name: Ada\nhi Ada\n");
}

#[test]
fn test_read_rewrite_keeps_every_flag_and_varname() {
    let source = "#!/usr/bin/env bash\nread -r -p \"Name: \" who\n";
    let rewritten = inject(source, &[("input-1", "Ada")]).unwrap();
    assert!(
        rewritten.contains("_skit_read 0 'Ada' 0 'Name: ' -r -p \"Name: \" who"),
        "{rewritten}"
    );
}

#[cfg(unix)]
#[test]
fn test_secret_read_masks_the_echo_but_delivers_the_value() {
    if !shell_available("bash") { return; }
    let source = "#!/usr/bin/env bash\nread -s -p \"Password: \" PW\necho \"len=${#PW}\"\n";
    let rewritten = inject(source, &[("input-1", "hunter2")]).unwrap();
    let (_, output) = run_shell("bash", &rewritten, "");
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert_eq!(stdout, "Password: ***\nlen=7\n");
    assert!(!stdout.contains("hunter2"));
}

#[cfg(unix)]
#[test]
fn test_read_in_a_loop_takes_the_value_once_then_reads_real_stdin() {
    if !shell_available("bash") { return; }
    let source = "#!/usr/bin/env bash\nfor i in 1 2 3; do\n  read -p \"Item: \" it\n  echo \"item=$it\"\ndone\n";
    let rewritten = inject(source, &[("input-1", "first")]).unwrap();
    let (_, output) = run_shell("bash", &rewritten, "second\nthird\n");
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        "Item: first\nitem=first\nitem=second\nitem=third\n"
    );
}

#[cfg(unix)]
#[test]
fn test_function_read_defined_above_invoked_after_keeps_its_value() {
    if !shell_available("bash") { return; }
    let source = concat!(
        "#!/usr/bin/env bash\n",
        "ask_secret() {\n",
        "  read -s -p \"Password: \" PW\n",
        "}\n",
        "read -p \"Name: \" NAME\n",
        "ask_secret\n",
        "echo \"name=$NAME pw=$PW\"\n",
    );
    let rewritten = inject(source, &[("input-1", "SUPERSECRET"), ("input-2", "alice")]).unwrap();
    let (_, output) = run_shell("bash", &rewritten, "");
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("name=alice pw=SUPERSECRET"), "{stdout}");
    assert!(stdout.contains("Password: ***"), "{stdout}");
    assert!(stdout.contains("Name: alice"), "{stdout}");
}

#[test]
fn test_two_specs_claiming_one_read_site_is_drift() {
    let source = "#!/usr/bin/env bash\nread -p \"Go? \" a\n";
    let declarations = [
        input_decl("input-1", 0, "Go? ", false),
        input_decl("input-2", 0, "Go? ", false),
    ];
    let error = inject_values(
        "shell",
        source,
        &declarations,
        &values(&[("input-1", "AAA"), ("input-2", "BBB")]),
    )
    .unwrap_err();
    assert!(error.to_string().contains("input-2"), "{error}");
}

#[test]
fn test_vanished_read_site_is_drift() {
    let declaration = input_decl("input-3", 2, "Gone? ", false);
    assert!(
        inject_values(
            "shell",
            "#!/usr/bin/env bash\nread -p \"Go? \" a\n",
            &[declaration],
            &values(&[("input-3", "x")]),
        )
        .is_err()
    );
}

#[cfg(unix)]
#[test]
fn test_value_follows_its_prompt_not_its_position() {
    if !shell_available("bash") { return; }
    let declaration = input_decl("input-1", 0, "Password: ", true);
    let edited = concat!(
        "#!/usr/bin/env bash\n",
        "read -p \"Name: \" NAME\n",
        "read -s -p \"Password: \" PW\n",
        "echo \"pw=$PW name=[$NAME]\"\n",
    );
    let rewritten = inject_values(
        "shell",
        edited,
        &[declaration],
        &values(&[("input-1", "hunter2")]),
    )
    .unwrap();
    let (_, output) = run_shell("bash", &rewritten, "typed\n");
    assert!(
        String::from_utf8(output.stdout).unwrap().contains("pw=hunter2 name=[typed]")
    );
}

#[cfg(unix)]
#[test]
fn test_multi_variable_read_joins_its_values_on_one_line() {
    if !shell_available("bash") { return; }
    let source = "#!/usr/bin/env bash\nread -p \"First and last: \" FIRST LAST\necho \"[$FIRST][$LAST]\"\n";
    let rewritten = inject(source, &[("input-1", "Ada"), ("input-2", "Lovelace")]).unwrap();
    let (_, output) = run_shell("bash", &rewritten, "");
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        "First and last: Ada Lovelace\n[Ada][Lovelace]\n"
    );
}

#[cfg(unix)]
#[test]
fn test_multi_variable_read_accepts_a_short_prefix() {
    if !shell_available("bash") { return; }
    let source = "#!/usr/bin/env bash\nread -p \"First and last: \" FIRST LAST\necho \"[$FIRST][$LAST]\"\n";
    let rewritten = inject(source, &[("input-1", "Ada")]).unwrap();
    let (_, output) = run_shell("bash", &rewritten, "");
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        "First and last: Ada\n[Ada][]\n"
    );
}

#[test]
fn test_multi_variable_read_refuses_a_positional_gap() {
    let source = "#!/usr/bin/env bash\nread -p \"First and last: \" FIRST LAST\n";
    let error = inject(source, &[("input-2", "Lovelace")]).unwrap_err();
    assert_eq!(
        error,
        LanguageError::ShellInput(ShellInputError::Gap {
            empty: "input-1".to_owned(),
            filled: "input-2".to_owned(),
        })
    );
}

#[test]
fn test_multi_variable_read_refuses_whitespace_in_a_non_last_field() {
    let source = "#!/usr/bin/env bash\nread -p \"First and last: \" FIRST LAST\n";
    let error = inject(source, &[("input-1", "John Paul"), ("input-2", "Doe")]).unwrap_err();
    assert_eq!(
        error,
        LanguageError::ShellInput(ShellInputError::FieldSplit { name: "input-1".to_owned() })
    );
}

#[test]
fn test_read_refuses_a_newline_in_any_field_including_a_single_variable() {
    let single = "#!/usr/bin/env bash\nread -p \"Name: \" NAME\n";
    assert_eq!(
        inject(single, &[("input-1", "a\nb")]).unwrap_err(),
        LanguageError::ShellInput(ShellInputError::LineBreak { name: "input-1".to_owned() })
    );
    let multi = "#!/usr/bin/env bash\nread -p \"A B: \" A B\n";
    assert_eq!(
        inject(multi, &[("input-1", "x"), ("input-2", "a\nb")]).unwrap_err(),
        LanguageError::ShellInput(ShellInputError::LineBreak { name: "input-2".to_owned() })
    );
}

#[test]
fn test_read_refuses_edge_whitespace_that_the_shell_would_strip() {
    let source = "#!/usr/bin/env bash\nread -p \"Name: \" NAME\n";
    for edge in [" lead", "trail ", "\ttab-lead"] {
        assert_eq!(
            inject(source, &[("input-1", edge)]).unwrap_err(),
            LanguageError::ShellInput(ShellInputError::EdgeSpace { name: "input-1".to_owned() })
        );
    }
    assert!(inject(source, &[("input-1", "de Lovelace")]).is_ok());
}

#[cfg(unix)]
#[test]
fn test_read_accepts_a_carriage_return_which_the_shell_delivers_intact() {
    if !shell_available("bash") { return; }
    let source = "#!/usr/bin/env bash\nread -p \"V: \" V\nprintf \"<%s>\" \"$V\"\n";
    let rewritten = inject(source, &[("input-1", "a\rb")]).unwrap();
    let (_, output) = run_shell("bash", &rewritten, "");
    assert!(output.stdout.windows(5).any(|window| window == b"<a\rb>"), "{:?}", output.stdout);
}

#[test]
fn test_multi_variable_read_refuses_whitespace_when_a_trailing_var_is_unmanaged() {
    let source = "#!/usr/bin/env bash\nread -p \"First and last: \" FIRST LAST\n";
    let error = inject(source, &[("input-1", "John Paul")]).unwrap_err();
    assert_eq!(
        error,
        LanguageError::ShellInput(ShellInputError::FieldSplit { name: "input-1".to_owned() })
    );
}

#[test]
fn test_multi_variable_read_refuses_a_newline_in_a_non_last_field() {
    let source = "#!/usr/bin/env bash\nread -p \"First and last: \" FIRST LAST\n";
    assert_eq!(
        inject(source, &[("input-1", "a\nb"), ("input-2", "KEEP")]).unwrap_err(),
        LanguageError::ShellInput(ShellInputError::LineBreak { name: "input-1".to_owned() })
    );
}

#[cfg(unix)]
#[test]
fn test_multi_variable_read_allows_whitespace_in_the_last_field() {
    if !shell_available("bash") { return; }
    let source = "#!/usr/bin/env bash\nread -p \"First and last: \" FIRST LAST\necho \"[$FIRST][$LAST]\"\n";
    let rewritten = inject(source, &[("input-1", "Ada"), ("input-2", "de Lovelace")]).unwrap();
    let (_, output) = run_shell("bash", &rewritten, "");
    assert!(String::from_utf8(output.stdout).unwrap().ends_with("[Ada][de Lovelace]\n"));
}

#[cfg(unix)]
#[test]
fn test_builtin_read_spelling_is_rewritten_whole() {
    if !shell_available("bash") { return; }
    let source = "#!/usr/bin/env bash\nbuiltin read -p \"Name: \" who\necho \"hi $who\"\n";
    let rewritten = inject(source, &[("input-1", "Ada")]).unwrap();
    assert!(!rewritten.contains("builtin _skit_read"), "{rewritten}");
    let (_, output) = run_shell("bash", &rewritten, "");
    assert_eq!(String::from_utf8(output.stdout).unwrap(), "Name: Ada\nhi Ada\n");
}

#[cfg(unix)]
#[test]
fn test_unmanaged_read_still_reads_real_stdin() {
    if !shell_available("bash") { return; }
    let source = "#!/usr/bin/env bash\nread -p \"One: \" a\nread -p \"Two: \" b\necho \"[$a][$b]\"\n";
    let rewritten = inject(source, &[("input-1", "injected")]).unwrap();
    let (_, output) = run_shell("bash", &rewritten, "typed\n");
    assert!(String::from_utf8(output.stdout).unwrap().ends_with("[injected][typed]\n"));
}

#[cfg(unix)]
#[test]
fn test_the_preamble_runs_on_every_supported_dialect() {
    let source = "#!/bin/sh\nNAME=x\nread who\necho \"hi $who / $NAME\"\nread it\necho \"it=$it\"\n";
    for shell in ["bash", "sh", "zsh", "dash"] {
        if !shell_available(shell) { continue; }
        let rewritten = inject_for(shell, source, &[("NAME", "y"), ("input-1", "Ada")]).unwrap();
        let (_, output) = run_shell(shell, &rewritten, "typed\n");
        assert_eq!(output.status.code(), Some(0), "shell={shell}: {}", String::from_utf8_lossy(&output.stderr));
        assert_eq!(String::from_utf8(output.stdout).unwrap(), "Ada\nhi Ada / y\nit=typed\n", "shell={shell}");
    }
}

#[cfg(unix)]
#[test]
fn test_set_u_and_set_e_survive_the_preamble() {
    if !shell_available("bash") { return; }
    let source = concat!(
        "#!/usr/bin/env bash\n",
        "set -euo pipefail\n",
        "OUT=/tmp/out\n",
        "read -p \"Deploy? \" confirm\n",
        "echo \"$OUT $confirm\"\n",
    );
    let rewritten = inject(source, &[("OUT", "/tmp/x"), ("input-1", "yes")]).unwrap();
    let (_, output) = run_shell("bash", &rewritten, "");
    assert_eq!(output.status.code(), Some(0), "{}", String::from_utf8_lossy(&output.stderr));
    assert!(String::from_utf8(output.stdout).unwrap().ends_with("/tmp/x yes\n"));
}

#[cfg(unix)]
#[test]
fn test_const_payload_is_inert() {
    if !shell_available("bash") { return; }
    let source = "#!/usr/bin/env bash\nTITLE=hello\necho \"[$TITLE]\"\n";
    for payload in ["'; touch pwned; echo '", "$(touch pwned)", "`touch pwned`", "$(id) && touch pwned"] {
        let rewritten = inject(source, &[("TITLE", payload)]).unwrap();
        let (root, output) = run_shell("bash", &rewritten, "");
        assert_eq!(output.status.code(), Some(0), "payload={payload:?}: {}", String::from_utf8_lossy(&output.stderr));
        assert_eq!(String::from_utf8(output.stdout).unwrap(), format!("[{payload}]\n"));
        assert!(!root.path().join("pwned").exists(), "payload executed: {payload:?}");
    }
}

#[cfg(unix)]
#[test]
fn test_read_payload_is_inert() {
    if !shell_available("bash") { return; }
    let source = "#!/usr/bin/env bash\nread -p \"Name: \" who\necho \"[$who]\"\n";
    for payload in ["'; touch pwned; echo '", "$(touch pwned)", "`touch pwned`", "$(id) && touch pwned"] {
        let rewritten = inject(source, &[("input-1", payload)]).unwrap();
        let (root, output) = run_shell("bash", &rewritten, "");
        assert_eq!(String::from_utf8(output.stdout).unwrap(), format!("Name: {payload}\n[{payload}]\n"));
        assert!(!root.path().join("pwned").exists(), "payload executed: {payload:?}");
    }
}

#[cfg(unix)]
#[test]
fn test_quote_in_a_read_prompt_survives() {
    if !shell_available("bash") { return; }
    let source = "#!/usr/bin/env bash\nread -p \"It's here: \" who\necho \"[$who]\"\n";
    let rewritten = inject(source, &[("input-1", "x")]).unwrap();
    let (_, output) = run_shell("bash", &rewritten, "");
    assert_eq!(String::from_utf8(output.stdout).unwrap(), "It's here: x\n[x]\n");
}

#[cfg(unix)]
#[test]
fn test_cjk_emoji_const_and_prompt_round_trip() {
    if !shell_available("bash") { return; }
    let source = "#!/usr/bin/env bash\nCITY=台北\nread -p \"请输入名字 🙂: \" NAME\necho \"$CITY|$NAME\"\n";
    let rewritten = inject(source, &[("CITY", "高雄 🚀"), ("input-1", "愛達")]).unwrap();
    assert!(matches!(parse_document("shell", &rewritten), ParseOutcome::Parsed(_)));
    let (_, output) = run_shell("bash", &rewritten, "");
    assert_eq!(String::from_utf8(output.stdout).unwrap(), "请输入名字 🙂: 愛達\n高雄 🚀|愛達\n");
}

#[cfg(unix)]
#[test]
fn test_crlf_script_injects_and_runs() {
    if !shell_available("bash") { return; }
    let source = "#!/usr/bin/env bash\r\nWIDTH=800\r\nHEIGHT=600\r\necho \"$WIDTH\"\r\n";
    let rewritten = inject(source, &[("WIDTH", "1200")]).unwrap();
    assert!(rewritten.contains("WIDTH=1200\r\n"), "{rewritten:?}");
    assert!(matches!(parse_document("shell", &rewritten), ParseOutcome::Parsed(_)));
    let (_, output) = run_shell("bash", &rewritten, "");
    assert_eq!(output.status.code(), Some(0), "{}", String::from_utf8_lossy(&output.stderr));
    assert!(String::from_utf8(output.stdout).unwrap().starts_with("1200"));
}

#[test]
fn test_no_trailing_newline_script_injects() {
    let rewritten = inject("#!/usr/bin/env bash\nVERSION=1.2.0", &[("VERSION", "2.0.0")]).unwrap();
    assert!(rewritten.ends_with("VERSION='2.0.0'"), "{rewritten:?}");
}

#[cfg(unix)]
#[test]
fn test_no_shebang_puts_the_preamble_at_the_very_top() {
    if !shell_available("bash") { return; }
    let source = "read -p \"Name: \" who\necho \"hi $who\"\n";
    let rewritten = inject(source, &[("input-1", "Ada")]).unwrap();
    assert!(rewritten.starts_with("_skit_read() {"), "{rewritten}");
    let (_, output) = run_shell("bash", &rewritten, "");
    assert_eq!(String::from_utf8(output.stdout).unwrap(), "Name: Ada\nhi Ada\n");
}

#[test]
fn test_preamble_lands_after_the_shebang() {
    let source = "#!/usr/bin/env bash\nread -p \"Name: \" who\n";
    let rewritten = inject(source, &[("input-1", "Ada")]).unwrap();
    let mut lines = rewritten.lines();
    assert_eq!(lines.next(), Some("#!/usr/bin/env bash"));
    assert_eq!(lines.next(), Some("_skit_read() {"));
}

#[cfg(unix)]
#[test]
fn test_backslash_values_arrive_byte_identical_raw_or_not() {
    if !shell_available("bash") { return; }
    let raw = "#!/usr/bin/env bash\nread -r -p \"P: \" a\necho \"[$a]\"\n";
    let cooked = "#!/usr/bin/env bash\nread -p \"P: \" a\necho \"[$a]\"\n";
    let value = "a\\b";
    let (_, raw_out) = run_shell("bash", &inject(raw, &[("input-1", value)]).unwrap(), "");
    assert!(String::from_utf8(raw_out.stdout).unwrap().ends_with("[a\\b]\n"));
    let (_, cooked_out) = run_shell("bash", &inject(cooked, &[("input-1", value)]).unwrap(), "");
    assert!(String::from_utf8(cooked_out.stdout).unwrap().ends_with("[a\\b]\n"));

    let two = "#!/usr/bin/env bash\nread -p \"P: \" A B\necho \"[$A][$B]\"\n";
    let (_, two_out) = run_shell(
        "bash",
        &inject(two, &[("input-1", "C:\\"), ("input-2", "Doe")]).unwrap(),
        "",
    );
    assert!(String::from_utf8(two_out.stdout).unwrap().ends_with("[C:\\][Doe]\n"));

    let konst = "#!/usr/bin/env bash\nP=x\necho \"[$P]\"\n";
    let (_, const_out) = run_shell("bash", &inject(konst, &[("P", value)]).unwrap(), "");
    assert_eq!(String::from_utf8(const_out.stdout).unwrap(), "[a\\b]\n");
}

#[test]
fn test_reframing_and_custom_ifs_reads_are_never_offered() {
    for source in [
        "read -n 3 X\n",
        "read -N 5 X\n",
        "read -d : X\n",
        "IFS=: read A B\n",
        "IFS= read -r LINE\n",
        "read -a ARR\n",
    ] {
        assert!(declarations(source).is_empty(), "{source:?}");
    }
    assert_eq!(
        declarations("read -p \"p: \" A B\n")
            .iter()
            .map(|decl| decl.name.as_str())
            .collect::<Vec<_>>(),
        ["input-1", "input-2"]
    );
}
