use skit_core::{Binding, ParamDefault, ParamType, analyze_python_managed};

fn names(source: &str) -> Vec<String> {
    analyze_python_managed(source)
        .candidates
        .into_iter()
        .map(|candidate| candidate.decl.name)
        .collect()
}

#[test]
fn module_literals_keep_python_types_and_values() {
    let analysis = analyze_python_managed(
        "CITY = 'Taipei'\nRETRIES = 3\nHEX = 0x10\nTHRESHOLD = -0.5\nVERBOSE = True\n_INTERNAL = 'skip'\nderived = RETRIES * 2\n",
    );
    assert!(!analysis.syntax_error);
    let by_name = analysis
        .candidates
        .iter()
        .map(|candidate| (candidate.decl.name.as_str(), &candidate.decl))
        .collect::<std::collections::BTreeMap<_, _>>();
    assert_eq!(by_name.len(), 5);
    assert_eq!(by_name["CITY"].param_type, ParamType::String);
    assert_eq!(
        by_name["CITY"].default,
        Some(ParamDefault::String("Taipei".to_owned()))
    );
    assert_eq!(by_name["RETRIES"].default, Some(ParamDefault::Integer(3)));
    assert_eq!(by_name["HEX"].default, Some(ParamDefault::Integer(16)));
    assert_eq!(
        by_name["THRESHOLD"].default,
        Some(ParamDefault::Float(-0.5))
    );
    assert_eq!(
        by_name["VERBOSE"].default,
        Some(ParamDefault::Boolean(true))
    );
}

#[test]
fn annotated_assignments_and_string_escapes_are_literals() {
    let analysis = analyze_python_managed(
        "count: int = 10\nflag: bool = False\nTEXT = \"a\\nb\\u0021\"\nRAW = r\"a\\nb\"\n",
    );
    let by_name = analysis
        .candidates
        .iter()
        .map(|candidate| (candidate.decl.name.as_str(), &candidate.decl))
        .collect::<std::collections::BTreeMap<_, _>>();
    assert_eq!(by_name["count"].default, Some(ParamDefault::Integer(10)));
    assert_eq!(by_name["flag"].default, Some(ParamDefault::Boolean(false)));
    assert_eq!(
        by_name["TEXT"].default,
        Some(ParamDefault::String("a\nb!".to_owned()))
    );
    assert_eq!(
        by_name["RAW"].default,
        Some(ParamDefault::String("a\\nb".to_owned()))
    );
}

#[test]
fn main_guard_candidates_are_scanned_and_module_binding_wins() {
    let analysis = analyze_python_managed(
        "TOP = 1\nif __name__ == \"__main__\":\n    INNER = 'yes'\n    TOP = 99\nif '__main__' == __name__:\n    SECOND = 2\n",
    );
    let by_name = analysis
        .candidates
        .iter()
        .map(|candidate| (candidate.decl.name.as_str(), &candidate.decl))
        .collect::<std::collections::BTreeMap<_, _>>();
    assert_eq!(by_name["TOP"].default, Some(ParamDefault::Integer(1)));
    assert_eq!(
        by_name["INNER"].default,
        Some(ParamDefault::String("yes".to_owned()))
    );
    // Only the first top-level main guard is part of the frozen C4 contract.
    assert!(!by_name.contains_key("SECOND"));
}

#[test]
fn reversed_main_guard_is_recognized() {
    assert_eq!(names("if \"__main__\" == __name__:\n    X = 5\n"), ["X"]);
}

#[test]
fn duplicate_const_keeps_first_slot_but_last_runtime_value() {
    let analysis = analyze_python_managed("X = 1\nY = 5\nX = 2\n");
    assert_eq!(
        analysis
            .candidates
            .iter()
            .map(|candidate| candidate.decl.name.as_str())
            .collect::<Vec<_>>(),
        ["X", "Y"]
    );
    assert_eq!(
        analysis.candidates[0].decl.default,
        Some(ParamDefault::Integer(2))
    );
    assert_eq!(analysis.candidates[0].line, 1);
}

#[test]
fn augmented_or_loop_reassignment_demotes_accumulator_candidates() {
    let analysis = analyze_python_managed(
        "TOTAL = 0\nTOTAL += 1\nCOUNT = 0\nfor item in range(3):\n    COUNT = COUNT + item\nCLEAN = 1\n",
    );
    let by_name = analysis
        .candidates
        .iter()
        .map(|candidate| (candidate.decl.name.as_str(), candidate))
        .collect::<std::collections::BTreeMap<_, _>>();
    assert!(by_name["TOTAL"].demoted);
    assert_eq!(by_name["TOTAL"].demotion, "accumulator");
    assert!(by_name["COUNT"].demoted);
    assert!(!by_name["CLEAN"].demoted);
}

#[test]
fn builtin_input_calls_keep_source_order_prompt_and_secret_hint() {
    let analysis = analyze_python_managed(
        "name = input(\"Name: \")\ndef f():\n    return input(\"Password: \")\nage = input()\n",
    );
    let inputs = analysis
        .candidates
        .iter()
        .filter(|candidate| candidate.decl.binding == Binding::Input)
        .collect::<Vec<_>>();
    assert_eq!(inputs.len(), 3);
    assert_eq!(inputs[0].decl.name, "input-1");
    assert_eq!(inputs[0].decl.order, 0);
    assert_eq!(inputs[0].decl.prompt, "Name: ");
    assert!(!inputs[0].decl.secret);
    assert_eq!(inputs[1].decl.prompt, "Password: ");
    assert!(inputs[1].decl.secret);
    assert_eq!(inputs[2].decl.prompt, "");
}

#[test]
fn input_shadowing_is_scope_aware() {
    assert!(
        analyze_python_managed("def input(prompt=''):\n    return 'x'\nname = input('Name: ')\n",)
            .candidates
            .iter()
            .all(|candidate| candidate.decl.binding != Binding::Input)
    );

    let analysis = analyze_python_managed(
        "def f(input):\n    return input('inner')\nname = input('Name: ')\n",
    );
    let lines = analysis
        .candidates
        .iter()
        .filter(|candidate| candidate.decl.binding == Binding::Input)
        .map(|candidate| candidate.line)
        .collect::<Vec<_>>();
    assert_eq!(lines, [3]);

    let analysis = analyze_python_managed(
        "def f():\n    input = str\n    return input('inner')\nname = input('Name: ')\n",
    );
    let lines = analysis
        .candidates
        .iter()
        .filter(|candidate| candidate.decl.binding == Binding::Input)
        .map(|candidate| candidate.line)
        .collect::<Vec<_>>();
    assert_eq!(lines, [4]);

    assert!(
        analyze_python_managed("input = str\ndef f():\n    return input('inner')\n")
            .candidates
            .iter()
            .all(|candidate| candidate.decl.binding != Binding::Input)
    );
}

#[test]
fn comprehension_and_lambda_bindings_do_not_shadow_module_input() {
    let analysis = analyze_python_managed(
        "xs = [input for input in range(3)]\ng = lambda input: input\nname = input('Name: ')\n",
    );
    assert_eq!(
        analysis
            .candidates
            .iter()
            .filter(|candidate| candidate.decl.binding == Binding::Input)
            .count(),
        1
    );
}

#[test]
fn framework_detection_preserves_first_source_occurrence() {
    let analysis = analyze_python_managed(
        "import typer, click\nimport os\nfrom argparse import ArgumentParser\n",
    );
    assert_eq!(analysis.frameworks, ["typer", "click", "argparse"]);
}

#[test]
fn syntax_error_returns_empty_analysis() {
    let analysis = analyze_python_managed("def broken(:\nCITY = 'x'\n");
    assert!(analysis.syntax_error);
    assert!(analysis.candidates.is_empty());
    assert!(analysis.frameworks.is_empty());
}

#[test]
fn secret_const_names_are_marked() {
    let analysis = analyze_python_managed("API_KEY = 'x'\nCITY = 'Taipei'\n");
    let key = analysis
        .candidates
        .iter()
        .find(|candidate| candidate.decl.name == "API_KEY");
    let city = analysis
        .candidates
        .iter()
        .find(|candidate| candidate.decl.name == "CITY");
    assert!(key.is_some_and(|candidate| candidate.decl.secret));
    assert!(city.is_some_and(|candidate| !candidate.decl.secret));
}
