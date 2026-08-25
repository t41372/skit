use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

use skit_domain::parameters::{
    ParamDecl, ParameterBinding, ParameterDelivery, ParameterType, ParameterValue,
};
use skit_language::{
    CliSurface, ParseOutcome, detect_candidates, inject_values, managed_params, parse_document,
    source_is_valid, write_managed_params,
};

fn corpus_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/corpus")
}

fn files(directory: &Path, extension: &str) -> Vec<PathBuf> {
    let mut paths = fs::read_dir(directory)
        .unwrap_or_else(|error| panic!("cannot read {}: {error}", directory.display()))
        .map(|entry| entry.expect("corpus directory entry").path())
        .filter(|path| path.extension().and_then(|value| value.to_str()) == Some(extension))
        .collect::<Vec<_>>();
    paths.sort();
    paths
}

fn source(path: &Path) -> String {
    String::from_utf8(
        fs::read(path).unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display())),
    )
    .unwrap_or_else(|error| panic!("{} is not UTF-8: {error}", path.display()))
}

fn sample_values(declarations: &[ParamDecl]) -> BTreeMap<String, String> {
    declarations
        .iter()
        .filter(|declaration| declaration.delivery == ParameterDelivery::Inject)
        .map(|declaration| {
            let value = match declaration.parameter_type {
                ParameterType::Int => "7",
                ParameterType::Float => "1.5",
                ParameterType::Bool => "true",
                ParameterType::Str | ParameterType::Choice | ParameterType::Path => "sample",
            };
            (declaration.name.clone(), value.to_owned())
        })
        .collect()
}

fn assert_comment_block_fidelity(kind: &str, path: &Path, comment: &str) {
    let original = source(path);
    let declarations = detect_candidates(kind, &original);
    let written = write_managed_params(kind, &original, &declarations)
        .unwrap_or_else(|error| panic!("{} metadata write failed: {error}", path.display()));
    assert_eq!(
        managed_params(kind, &written),
        declarations,
        "{}",
        path.display()
    );

    let original_lines = original.split_inclusive(['\n', '\r']).collect::<Vec<_>>();
    for line in written.split_inclusive(['\n', '\r']) {
        if !original_lines.contains(&line) && !line.trim().is_empty() {
            assert!(
                line.trim_start().starts_with(comment),
                "{} added a non-comment line: {line:?}",
                path.display()
            );
        }
    }

    let original_lines = original.lines().collect::<Vec<_>>();
    if original_lines
        .first()
        .is_some_and(|line| line.starts_with("#!"))
    {
        assert_eq!(written.lines().next(), original_lines.first().copied());
    }
    for line in original_lines {
        if !line.trim_start().starts_with(comment) {
            assert!(
                written.contains(line),
                "{} lost source line {line:?}",
                path.display()
            );
        }
    }
}

fn assert_rewrite_contract(kind: &str, path: &Path) {
    let original = source(path);
    let declarations = detect_candidates(kind, &original);
    if !source_is_valid(kind, &original) {
        assert!(
            declarations.is_empty(),
            "{} exposed partial results for invalid source",
            path.display()
        );
        return;
    }
    assert_eq!(
        inject_values(kind, &original, &declarations, &BTreeMap::new())
            .unwrap_or_else(|error| panic!("{} empty rewrite failed: {error}", path.display())),
        original,
        "{}",
        path.display()
    );

    let values = sample_values(&declarations);
    let rewritten = inject_values(kind, &original, &declarations, &values)
        .unwrap_or_else(|error| panic!("{} full rewrite failed: {error}", path.display()));
    assert!(source_is_valid(kind, &rewritten), "{}", path.display());

    let comment = if matches!(kind, "js" | "ts") {
        "//"
    } else {
        "#"
    };
    for line in original.lines() {
        if line.trim_start().starts_with(comment) {
            assert!(
                rewritten.contains(line),
                "{} lost comment line {line:?}",
                path.display()
            );
        }
    }
}

fn expected_names(kind: &str, file: &str) -> &'static [&'static str] {
    match (kind, file) {
        ("python", "01_simple_const.py") => &["CITY", "RETRIES"],
        ("python", "02_pep723_block.py") => &["URL", "TIMEOUT"],
        ("python", "03_shebang_coding.py") => &["GREETING"],
        ("python", "04_docstring_module.py") => &["LEVEL"],
        ("python", "05_future_imports.py") => &["NAME", "input-1"],
        ("python", "06_main_guard.py") => &["COUNT"],
        ("python", "07_negative_numbers.py") => &["OFFSET", "FACTOR"],
        ("python", "08_annotated_assign.py") => &["LIMIT", "RATE", "LABEL"],
        ("python", "09_bool_flags.py") => &["VERBOSE", "DRY_RUN"],
        ("python", "10_multiple_inputs.py") => &["input-1", "input-2", "input-3"],
        ("python", "11_input_in_loop.py")
        | ("python", "12_input_cast.py")
        | ("python", "13_input_in_function.py")
        | ("python", "28_input_comprehension.py") => &["input-1"],
        ("python", "14_secret_names.py") => &["API_KEY", "input-1"],
        ("python", "15_multiline_paren_const.py") => &["MSG"],
        ("python", "16_trailing_comments.py") => &["HOST", "PORT"],
        ("python", "17_unicode_cjk.py") => &["標題", "CITY"],
        ("python", "18_windows_paths.py") => &["SRC", "DST"],
        ("python", "19_string_mentions_input.py") => &["HELP"],
        ("python", "20_decorated_toplevel.py") => &["RETRY"],
        ("python", "21_class_based.py") => &["THRESHOLD"],
        ("python", "22_argparse_framework.py") => &["DEFAULT_N"],
        ("python", "23_walrus_and_fstring.py") => &["BASE"],
        ("python", "24_global_statement.py") => &["MODE"],
        ("python", "25_try_except_top.py") => &["PATH"],
        ("python", "26_while_input_sentinel.py") => &["total", "input-1"],
        ("python", "27_dict_config_untouched.py") => &["NAME"],
        ("python", "29_main_guard_reversed.py") => &["SEED"],
        ("python", "30_pep723_tool_skit.py") => &["CITY"],
        ("python", "31_async_script.py") => &["DELAY"],
        ("python", "32_no_trailing_newline.py") => &["N"],
        ("python", "33_crlf_endings.py") => &["CITY", "input-1"],
        ("python", "34_tab_indent.py") => &["LIMIT"],
        ("shell", "01_plain_consts.sh") => &["CITY", "RETRIES", "THRESHOLD", "GREETING", "TITLE"],
        ("shell", "02_declarations.sh") => &["API_HOST", "PORT", "COUNT", "FLAVOR"],
        ("shell", "03_envdefaults.sh") => &["GREETING", "TIMEOUT", "LEVEL", "RETRIES"],
        ("shell", "04_suppression.sh") => &["PORT", "HOST", "MODE"],
        ("shell", "05_self_idiom.sh") => &["PORT", "NAME"],
        ("shell", "06_reads.sh") => &["input-1", "input-2", "input-3", "input-4"],
        ("shell", "07_read_clustered.sh") => &["input-1", "input-2"],
        ("shell", "08_data_reads.sh")
        | ("shell", "11_hints_argv.sh")
        | ("shell", "12_hints_selfloc.sh")
        | ("shell", "19_zsh_dialect.sh") => &[],
        ("shell", "09_heredoc.sh") => &["TEMPLATE_NAME"],
        ("shell", "10_demotions.sh") => &["COUNT", "TOTAL", "STEP", "SUM"],
        ("shell", "13_function_bodies.sh") => &["GLOBAL_LABEL"],
        ("shell", "14_retry_prompts.sh") => &["input-1", "input-2"],
        ("shell", "15_dynamic_prompt.sh") => &["input-1", "input-2"],
        ("shell", "16_cjk_emoji.sh") => &["CITY", "GREETING", "EMOJI", "input-1"],
        ("shell", "17_crlf.sh") => &["WIDTH", "HEIGHT", "LABEL"],
        ("shell", "18_no_trailing_newline.sh") => &["VERSION"],
        ("shell", "20_secret_names.sh") => &["API_KEY", "SECRET_TOKEN", "input-1"],
        ("shell", "21_mixed_all.sh") => &["OUTPUT_DIR", "MAX_RETRIES", "LOG_LEVEL", "input-1"],
        ("shell", "22_function_read_order.sh") => &["input-1", "input-2"],
        ("shell", "23_selfloc_const.sh") => &["OUTPUT_DIR", "RETRIES"],
        ("shell", "24_quoting_payloads.sh") => &["BARE", "RAW", "DOUBLE", "NUMBER", "input-1"],
        ("js", "01_const_numbers.mjs") => &["WIDTH", "RATIO"],
        ("js", "02_const_string_bool.mjs") => &["CITY", "VERBOSE", "DRY_RUN"],
        ("js", "03_template_excluded.mjs") => &["NAME"],
        ("js", "04_object_array_excluded.mjs")
        | ("js", "09_parseargs_inline.mjs")
        | ("js", "10_parseargs_identifier.mjs")
        | ("js", "11_parseargs_spread.mjs") => &[],
        ("js", "05_let_var_demoted.mjs") => &["counter", "total", "STABLE"],
        ("js", "06_destructuring_excluded.mjs") => &["REAL"],
        ("js", "07_json_escaping.mjs") => &["MESSAGE"],
        ("js", "08_cjk_emoji.mjs") => &["CITY", "BANNER"],
        ("js", "12_shebang_block.mjs") => &["PORT"],
        ("js", "13_crlf.mjs") => &["WIDTH", "HEIGHT"],
        ("js", "14_no_trailing_newline.mjs") => &["VERSION"],
        ("ts", "01_typed_consts.ts") => &["MAX_RETRIES", "ENDPOINT", "ENABLED", "TIMEOUT"],
        ("ts", "02_parseargs.ts") => &[],
        ("ts", "03_ts_features.ts") => &["DEFAULT_WIDTH", "mode"],
        ("fish", "01_env_idioms.fish") => &["PORT", "RATE", "REGION", "LOG_DIR"],
        ("fish", "02_argparse.fish") | ("fish", "05_reads_and_consts.fish") => {
            if file == "05_reads_and_consts.fish" {
                &["RETRIES"]
            } else {
                &[]
            }
        }
        ("fish", "03_quoting.fish") => &["GREETING", "PROMPT", "PATTERN"],
        ("fish", "04_block_nesting.fish") => &["TOP", "ALSO_TOP"],
        ("fish", "06_cjk.fish") => &["問候", "EMOJI", "CITY"],
        _ => panic!("missing corpus expectation for {kind}:{file}"),
    }
}

#[test]
fn corpus_candidate_names_match_the_v040_analyzers() {
    let root = corpus_root();
    for (kind, directory, extension) in [
        ("python", root.clone(), "py"),
        ("shell", root.join("shell"), "sh"),
        ("js", root.join("js"), "mjs"),
        ("ts", root.join("ts"), "ts"),
        ("fish", root.join("fish"), "fish"),
    ] {
        for path in files(&directory, extension) {
            let file = path.file_name().and_then(|value| value.to_str()).unwrap();
            let actual = detect_candidates(kind, &source(&path))
                .into_iter()
                .map(|declaration| declaration.name)
                .collect::<Vec<_>>();
            assert_eq!(actual, expected_names(kind, file), "{kind}:{file}");
        }
    }
}

#[test]
fn every_analyzer_corpus_is_projected_from_one_parser_owned_document() {
    let root = corpus_root();
    for (kind, directory, extension) in [
        ("python", root.clone(), "py"),
        ("shell", root.join("shell"), "sh"),
        ("js", root.join("js"), "mjs"),
        ("ts", root.join("ts"), "ts"),
        ("fish", root.join("fish"), "fish"),
    ] {
        for path in files(&directory, extension) {
            let file = path.file_name().and_then(|value| value.to_str()).unwrap();
            let actual = match parse_document(kind, &source(&path)) {
                ParseOutcome::Parsed(document) => document
                    .analysis()
                    .candidates
                    .into_iter()
                    .map(|candidate| candidate.declaration.name)
                    .collect::<Vec<_>>(),
                ParseOutcome::SyntaxError(_) if expected_names(kind, file).is_empty() => Vec::new(),
                ParseOutcome::SyntaxError(_) => {
                    panic!("{kind}:{file} must parse because it has semantic candidates")
                }
                ParseOutcome::ParserUnavailable(_) => {
                    panic!("{kind}:{file} must have a parser adapter")
                }
            };
            assert_eq!(actual, expected_names(kind, file), "{kind}:{file}");
        }
    }
}

#[test]
fn every_static_cli_corpus_is_projected_from_the_same_document() {
    let root = corpus_root();
    for (kind, relative, expected_framework, expected_names) in [
        ("shell", "shell/11_hints_argv.sh", "getopts", vec!["n", "v"]),
        (
            "js",
            "js/09_parseargs_inline.mjs",
            "parseArgs",
            vec!["name", "verbose", "tag", "dry-run"],
        ),
        (
            "ts",
            "ts/02_parseargs.ts",
            "parseArgs",
            vec!["output", "force"],
        ),
        (
            "fish",
            "fish/02_argparse.fish",
            "argparse",
            vec!["help", "city", "retries", "file", "dry-run", "verbose"],
        ),
    ] {
        let path = root.join(relative);
        let ParseOutcome::Parsed(document) = parse_document(kind, &source(&path)) else {
            panic!("{kind}:{relative} must have a parser-owned document");
        };
        let CliSurface::Static(surface) = document.cli_surface() else {
            panic!("{kind}:{relative} must have a static CLI surface");
        };
        assert_eq!(surface.framework, expected_framework);
        assert_eq!(
            surface
                .fields
                .into_iter()
                .map(|field| field.declaration.name)
                .collect::<Vec<_>>(),
            expected_names
        );
    }

    for (kind, relative, expected_framework) in [
        ("js", "js/10_parseargs_identifier.mjs", "parseArgs"),
        ("js", "js/11_parseargs_spread.mjs", "parseArgs"),
    ] {
        let path = root.join(relative);
        let ParseOutcome::Parsed(document) = parse_document(kind, &source(&path)) else {
            panic!("{kind}:{relative} must have a parser-owned document");
        };
        let CliSurface::Dynamic(surface) = document.cli_surface() else {
            panic!("{kind}:{relative} must have a dynamic CLI surface");
        };
        assert_eq!(surface.framework, expected_framework);
    }
}

fn candidate(kind: &str, relative: &str, name: &str) -> ParamDecl {
    let path = corpus_root().join(relative);
    detect_candidates(kind, &source(&path))
        .into_iter()
        .find(|declaration| declaration.name == name)
        .unwrap_or_else(|| panic!("{kind}:{relative} did not detect {name}"))
}

#[test]
fn corpus_candidate_details_match_the_v040_contract() {
    let negative = candidate("python", "07_negative_numbers.py", "OFFSET");
    assert_eq!(negative.parameter_type, ParameterType::Int);
    assert_eq!(negative.default, Some(ParameterValue::Integer(-3)));

    let python_input = candidate("python", "14_secret_names.py", "input-1");
    assert_eq!(python_input.binding, ParameterBinding::Input);
    assert_eq!(python_input.order, 0);
    assert_eq!(python_input.prompt, "Token: ");
    assert!(python_input.secret);

    let shell_float = candidate("shell", "shell/01_plain_consts.sh", "THRESHOLD");
    assert_eq!(shell_float.parameter_type, ParameterType::Float);
    assert_eq!(shell_float.default, Some(ParameterValue::Float(-0.5)));
    let shell_input = candidate("shell", "shell/06_reads.sh", "input-4");
    assert_eq!(shell_input.order, 3);
    assert_eq!(shell_input.prompt, "First and last: ");
    let shell_default = candidate("shell", "shell/03_envdefaults.sh", "TIMEOUT");
    assert_eq!(shell_default.binding, ParameterBinding::EnvDefault);
    assert_eq!(shell_default.parameter_type, ParameterType::Int);
    assert_eq!(shell_default.default, Some(ParameterValue::Integer(30)));

    let javascript = candidate("js", "js/02_const_string_bool.mjs", "VERBOSE");
    assert_eq!(javascript.parameter_type, ParameterType::Bool);
    assert_eq!(javascript.default, Some(ParameterValue::Bool(true)));
    let typescript = candidate("ts", "ts/01_typed_consts.ts", "TIMEOUT");
    assert_eq!(typescript.parameter_type, ParameterType::Float);
    assert_eq!(typescript.default, Some(ParameterValue::Float(30.5)));

    let fish = candidate("fish", "fish/03_quoting.fish", "GREETING");
    assert_eq!(fish.binding, ParameterBinding::EnvDefault);
    assert_eq!(
        fish.default,
        Some(ParameterValue::String("hello; world".to_owned()))
    );
    let fish_integer = candidate("fish", "fish/01_env_idioms.fish", "PORT");
    assert_eq!(fish_integer.parameter_type, ParameterType::Int);
    assert_eq!(fish_integer.default, Some(ParameterValue::Integer(8080)));
}

#[test]
fn python_and_shell_corpus_preserves_metadata_and_source_bytes() {
    let root = corpus_root();
    for path in files(&root, "py") {
        assert_comment_block_fidelity("python", &path, "#");
    }
    for path in files(&root.join("shell"), "sh") {
        assert_comment_block_fidelity("shell", &path, "#");
    }
}

#[test]
fn javascript_and_typescript_corpus_preserves_metadata_and_source_bytes() {
    let root = corpus_root();
    for path in files(&root.join("js"), "mjs") {
        assert_comment_block_fidelity("js", &path, "//");
    }
    for path in files(&root.join("ts"), "ts") {
        assert_comment_block_fidelity("ts", &path, "//");
    }
}

#[test]
fn every_rewritten_corpus_file_remains_valid_source() {
    let root = corpus_root();
    for (kind, directory, extension) in [
        ("python", root.clone(), "py"),
        ("shell", root.join("shell"), "sh"),
        ("js", root.join("js"), "mjs"),
        ("ts", root.join("ts"), "ts"),
    ] {
        for path in files(&directory, extension) {
            assert_rewrite_contract(kind, &path);
        }
    }
}
