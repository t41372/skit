//! Public-API ports of Python v0.4 add-time analyzer signals.
//!
//! These facts drive add-review warnings and default selection but do not execute user code:
//! accumulator demotion, raw argv use, and filename-shaped literal hints.

use skit_form::onboarding_plan;
use skit_language::DegradationReason;

const IMAGE_STITCH: &str = r#"
from PIL import Image
import sys

images = [Image.open(x) for x in sys.argv[1:]]

y_offset = 0
for im in images:
    im.paste(im, (0, y_offset))
    y_offset += im.size[1]

im.save('output_long_image.jpg')
print("done")
"#;

#[test]
fn test_python_accumulator_is_demoted_and_not_selected_by_default() {
    let plan = onboarding_plan("python", IMAGE_STITCH);
    let y = plan
        .candidates
        .iter()
        .find(|candidate| candidate.declaration.name == "y_offset")
        .unwrap();
    assert_eq!(y.demotion, Some(DegradationReason::Accumulator));
    assert!(!y.selected_by_default());
}

#[test]
fn test_clean_python_constant_is_not_demoted() {
    let plan = onboarding_plan("python", "OUTPUT = 'out.jpg'\nprint(OUTPUT)\n");
    let output = plan
        .candidates
        .iter()
        .find(|candidate| candidate.declaration.name == "OUTPUT")
        .unwrap();
    assert_eq!(output.demotion, None);
    assert!(output.selected_by_default());
}

#[test]
fn test_python_reassignment_inside_while_loop_demotes() {
    let plan = onboarding_plan("python", "count = 0\nwhile go():\n    count = count + 1\n");
    let count = plan
        .candidates
        .iter()
        .find(|candidate| candidate.declaration.name == "count")
        .unwrap();
    assert_eq!(count.demotion, Some(DegradationReason::Accumulator));
}

#[test]
fn test_python_augassign_outside_loop_still_demotes() {
    let plan = onboarding_plan("python", "total = 0\ntotal += cost()\n");
    let total = plan
        .candidates
        .iter()
        .find(|candidate| candidate.declaration.name == "total")
        .unwrap();
    assert_eq!(total.demotion, Some(DegradationReason::Accumulator));
}

#[test]
fn test_python_annotated_reassignment_inside_loop_demotes() {
    let plan = onboarding_plan(
        "python",
        "COUNT = 0\nfor i in range(3):\n    COUNT: int = i\n",
    );
    let count = plan
        .candidates
        .iter()
        .find(|candidate| candidate.declaration.name == "COUNT")
        .unwrap();
    assert_eq!(count.demotion, Some(DegradationReason::Accumulator));
    assert!(!count.selected_by_default());
}

#[test]
fn test_python_uses_argv_detected_without_false_positive() {
    assert!(onboarding_plan("python", IMAGE_STITCH).uses_argv);
    assert!(!onboarding_plan("python", "print('no args')\n").uses_argv);
    assert!(onboarding_plan("python", "import sys\nn = len(sys.argv)\n").uses_argv);
}

#[test]
fn test_python_filename_literal_hint_is_found() {
    assert_eq!(
        onboarding_plan("python", IMAGE_STITCH).filename_literals,
        ["output_long_image.jpg"]
    );
}

#[test]
fn test_filename_hint_scans_past_non_string_call_arguments() {
    assert_eq!(
        onboarding_plan("python", "f(1, 'notes.txt')\n").filename_literals,
        ["notes.txt"]
    );
}

#[test]
fn test_filename_hint_disappears_when_call_uses_a_named_constant() {
    let source = "OUTPUT = 'output_long_image.jpg'\nsave(OUTPUT)\n";
    assert!(
        onboarding_plan("python", source)
            .filename_literals
            .is_empty()
    );
}

#[test]
fn test_filename_hints_exclude_sentences_urls_versions_and_non_extensions() {
    let source = concat!(
        "new('RGB')\n",
        "log('finished: output.jpg now ready')\n",
        "get('https://example.com/a.zip')\n",
        "ver('3.14')\n",
    );
    assert!(
        onboarding_plan("python", source)
            .filename_literals
            .is_empty()
    );
}

#[test]
fn test_filename_hints_dedupe_and_cap_at_three() {
    let source = "f('a.png')\nf('a.png')\nf('b.png')\nf('c.png')\nf('d.png')\n";
    assert_eq!(
        onboarding_plan("python", source).filename_literals,
        ["a.png", "b.png", "c.png"]
    );
}

#[test]
fn test_shadowed_input_disables_input_candidates_without_aborting_const_analysis() {
    let source = concat!(
        "def input(p=''):\n",
        "    return 'x'\n",
        "CITY = 'Taipei'\n",
        "name = input('Name: ')\n",
    );
    let plan = onboarding_plan("python", source);
    assert!(plan.candidates.iter().all(|candidate| {
        candidate.declaration.binding != skit_domain::parameters::ParameterBinding::Input
    }));
    assert_eq!(
        plan.candidates
            .iter()
            .filter(|candidate| {
                candidate.declaration.binding == skit_domain::parameters::ParameterBinding::Const
            })
            .map(|candidate| candidate.declaration.name.as_str())
            .collect::<Vec<_>>(),
        ["CITY"]
    );

    let control = onboarding_plan("python", "name = input('Name: ')\n");
    assert_eq!(
        control
            .candidates
            .iter()
            .filter(|candidate| {
                candidate.declaration.binding == skit_domain::parameters::ParameterBinding::Input
            })
            .map(|candidate| candidate.declaration.name.as_str())
            .collect::<Vec<_>>(),
        ["input-1"]
    );
}
