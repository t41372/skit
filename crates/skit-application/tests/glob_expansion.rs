use std::{collections::BTreeMap, fmt};

use skit_application::{
    delivery::PreparedValue,
    glob_expansion::{GlobExpander, expand_multi_values, prepare_extra_args},
    tokens::{TokenContext, TokenError},
};
use skit_domain::parameters::ParamDecl;

#[derive(Default)]
struct FakeGlob {
    matches: BTreeMap<String, Vec<String>>,
}

impl fmt::Debug for FakeGlob {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_struct("FakeGlob").finish_non_exhaustive()
    }
}

impl GlobExpander for FakeGlob {
    fn expand_piece(&self, piece: &str) -> Vec<String> {
        self.matches
            .get(piece)
            .cloned()
            .unwrap_or_else(|| vec![piece.to_owned()])
    }
}

fn context() -> TokenContext {
    TokenContext {
        cwd: "/work".to_owned(),
        home: Some("/home/user".to_owned()),
        env: BTreeMap::from([("PATTERN".to_owned(), "*.txt".to_owned())]),
        today: "2026-08-07".to_owned(),
        now: "12-00-00".to_owned(),
    }
}

#[test]
fn only_multi_value_fields_expand_each_already_split_piece() {
    let scalar = ParamDecl::new("scalar");
    let mut multiple = ParamDecl::new("files");
    multiple.multiple = true;
    let mut glob = FakeGlob::default();
    glob.matches.insert(
        "*.txt".to_owned(),
        vec!["a.txt".to_owned(), "b.txt".to_owned()],
    );
    let prepared = BTreeMap::from([
        (
            "scalar".to_owned(),
            PreparedValue::Scalar("*.txt".to_owned()),
        ),
        (
            "files".to_owned(),
            PreparedValue::Multiple(vec!["*.txt".to_owned(), "literal.md".to_owned()]),
        ),
    ]);

    let expanded = expand_multi_values(&[scalar, multiple], &prepared, &glob);

    assert_eq!(
        expanded["scalar"],
        PreparedValue::Scalar("*.txt".to_owned())
    );
    assert_eq!(
        expanded["files"],
        PreparedValue::Multiple(vec![
            "a.txt".to_owned(),
            "b.txt".to_owned(),
            "literal.md".to_owned(),
        ])
    );
}

#[test]
fn missing_or_shape_mismatched_entries_pass_through_without_guessing() {
    let mut multiple = ParamDecl::new("files");
    multiple.multiple = true;
    let scalar = ParamDecl::new("name");
    let prepared = BTreeMap::from([
        (
            "files".to_owned(),
            PreparedValue::Scalar("*.txt".to_owned()),
        ),
        (
            "unknown".to_owned(),
            PreparedValue::Multiple(vec!["*.rs".to_owned()]),
        ),
    ]);

    assert_eq!(
        expand_multi_values(&[multiple, scalar], &prepared, &FakeGlob::default()),
        prepared
    );
}

#[test]
fn raw_extra_tail_expands_tokens_then_globs_without_shell_splitting() {
    let mut glob = FakeGlob::default();
    glob.matches.insert(
        "*.txt".to_owned(),
        vec!["a.txt".to_owned(), "b.txt".to_owned()],
    );
    glob.matches.insert(
        "/work/out_2026-08-07.log".to_owned(),
        vec!["/work/out_2026-08-07.log".to_owned()],
    );

    let expanded = prepare_extra_args(
        &[
            "{env:PATTERN}".to_owned(),
            "{cwd}/out_{today}.log".to_owned(),
            "two words".to_owned(),
        ],
        &context(),
        true,
        &glob,
    )
    .unwrap();

    assert_eq!(
        expanded,
        ["a.txt", "b.txt", "/work/out_2026-08-07.log", "two words",]
    );
}

#[test]
fn literal_replay_tail_bypasses_both_token_and_glob_expansion() {
    let mut glob = FakeGlob::default();
    glob.matches
        .insert("*.txt".to_owned(), vec!["expanded.txt".to_owned()]);
    let original = vec!["{today}".to_owned(), "*.txt".to_owned()];

    assert_eq!(
        prepare_extra_args(&original, &context(), false, &glob).unwrap(),
        original
    );
}

#[test]
fn extra_tail_token_failures_keep_the_exact_token_error() {
    let error = prepare_extra_args(
        &["{env:MISSING}".to_owned()],
        &context(),
        true,
        &FakeGlob::default(),
    )
    .unwrap_err();

    assert_eq!(
        error,
        TokenError::MissingEnvironment {
            name: "MISSING".to_owned(),
            token: "{env:MISSING}".to_owned(),
        }
    );
}
