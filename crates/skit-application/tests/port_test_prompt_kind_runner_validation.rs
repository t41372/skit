use skit_application::runner_management::validate_runner_argv;

fn argv(values:&[&str])->Vec<String>{values.iter().map(|value|(*value).to_owned()).collect()}

#[test]
fn test_fill_runner_argv_rejects_missing_duplicate_or_binary_slot(){
 assert!(validate_runner_argv(&argv(&["agent"])).is_err());
 assert!(validate_runner_argv(&argv(&["agent","{{prompt}}","--again={{prompt}}"])).is_err());
 assert!(validate_runner_argv(&argv(&["{{prompt}}","--x"])).is_err());
}

#[test]
fn test_fill_runner_argv_rejects_foreign_double_brace_holes(){
 for values in [vec!["agent","{{prompt}}","{{other}}"],vec!["agent","{{other}}"]]{assert!(validate_runner_argv(&values.into_iter().map(str::to_owned).collect::<Vec<_>>()).is_err());}
 assert!(validate_runner_argv(&argv(&["agent","{{prompt}}","literal-{other}"])).is_ok(),"single-brace text is literal runner argv, not a template hole");
}
