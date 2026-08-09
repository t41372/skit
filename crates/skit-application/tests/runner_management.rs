use skit_application::runner_management::{
    EditableArgvDialect, RunnerArgvError, RunnerCommandError, join_editable_argv,
    split_editable_argv, validate_runner_argv,
};

#[test]
fn posix_editable_command_round_trips_without_a_shell() {
    let argv = vec![
        "agent".to_owned(),
        "--message".to_owned(),
        "a prompt with spaces".to_owned(),
        "{{prompt}}".to_owned(),
    ];
    let text = join_editable_argv(&argv, EditableArgvDialect::Posix);
    assert_eq!(
        split_editable_argv(&text, EditableArgvDialect::Posix).unwrap(),
        argv
    );
    assert_eq!(
        split_editable_argv("agent '{{prompt}}", EditableArgvDialect::Posix),
        Err(RunnerCommandError::UnbalancedQuotes)
    );
}

#[test]
fn windows_parser_preserves_paths_and_uses_vc_runtime_quoting() {
    let cases = [
        Vec::<String>::new(),
        vec![String::new()],
        vec![
            r"C:\Program Files\Agent\agent.exe".to_owned(),
            "--message".to_owned(),
            "{{prompt}}".to_owned(),
        ],
        vec![
            "say \"hello\"".to_owned(),
            "space and trailing slash\\".to_owned(),
        ],
        vec!["one\\\"quote".to_owned(), "two\\\\\"quote".to_owned()],
        vec![
            String::new(),
            "plain".to_owned(),
            "\t".to_owned(),
            "{{prompt}}".to_owned(),
        ],
    ];
    for argv in cases {
        let text = join_editable_argv(&argv, EditableArgvDialect::Windows);
        assert_eq!(
            split_editable_argv(&text, EditableArgvDialect::Windows).unwrap(),
            argv,
            "command={text:?}"
        );
    }
    assert_eq!(
        split_editable_argv(
            r#"C:\tools\agent.exe "{{prompt}}""#,
            EditableArgvDialect::Windows,
        )
        .unwrap(),
        [r"C:\tools\agent.exe", "{{prompt}}"]
    );
    assert_eq!(
        split_editable_argv(
            r#""C:\Program Files\Agent\agent.exe"#,
            EditableArgvDialect::Windows,
        ),
        Err(RunnerCommandError::UnbalancedQuotes)
    );
}

#[test]
fn runner_validation_allows_only_one_prompt_slot_after_the_program() {
    let valid = |argv: &[&str]| {
        validate_runner_argv(
            &argv
                .iter()
                .map(|value| (*value).to_owned())
                .collect::<Vec<_>>(),
        )
    };
    assert_eq!(valid(&[]), Err(RunnerArgvError::EmptyCommand));
    assert_eq!(valid(&["agent"]), Err(RunnerArgvError::PromptSlotCount));
    assert_eq!(
        valid(&["{{prompt}}", "x"]),
        Err(RunnerArgvError::PromptInProgram)
    );
    assert_eq!(
        valid(&["agent", "{{prompt}}", "{{model}}"]),
        Err(RunnerArgvError::UnsupportedHole)
    );
    assert_eq!(valid(&["agent", "{literal}", "{{prompt}}"]), Ok(()));
    assert_eq!(valid(&["agent", "--message={{prompt}}"]), Ok(()));
}
