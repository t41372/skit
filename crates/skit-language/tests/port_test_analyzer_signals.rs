//! Mechanical port of the Python oracle module `tests/test_analyzer_signals.py`
//! (`origin/main@206f9ef`). Each `#[test]` keeps its Python `def test_*` name and the Python
//! "WHY" comment is preserved verbatim above it.
//!
//! Concept mapping:
//! - Python `analyzer.analyze(src)` -> `parsed(src).analysis()` (`SemanticAnalysis`).
//! - Python `candidate.demoted` (bool) -> `candidate.demotion.is_some()`.
//! - Python `candidate.demotion` (string: "accumulator" | "") -> `candidate.demotion`
//!   (`Option<DegradationReason>`): "accumulator" -> `Some(DegradationReason::Accumulator)`,
//!   "" -> `None`. NOTE: `demotion` is a field on the `SemanticCandidate`, not on `.declaration`.
//! - Python `.uses_argv` / `.filename_literals` -> the same-named `SemanticAnalysis` fields.

use skit_language::{
    DegradationReason, ParseOutcome, ParsedDocument, SemanticAnalysis, SemanticCandidate,
    parse_document,
};

fn parsed(source: &str) -> ParsedDocument {
    match parse_document("python", source) {
        ParseOutcome::Parsed(document) => document,
        other => panic!("expected valid Python, got {other:?}"),
    }
}

/// Python `next(c for c in result.candidates if c.name == name)`.
fn candidate<'a>(analysis: &'a SemanticAnalysis, name: &str) -> &'a SemanticCandidate {
    analysis
        .candidates
        .iter()
        .find(|candidate| candidate.declaration.name == name)
        .unwrap_or_else(|| panic!("missing candidate {name}"))
}

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
fn test_accumulator_is_demoted() {
    let analysis = parsed(IMAGE_STITCH).analysis();
    let y = candidate(&analysis, "y_offset");
    assert!(y.demotion.is_some());
    assert_eq!(y.demotion, Some(DegradationReason::Accumulator));
}

#[test]
fn test_clean_constant_is_not_demoted() {
    let analysis = parsed("OUTPUT = 'out.jpg'\nprint(OUTPUT)\n").analysis();
    let out = candidate(&analysis, "OUTPUT");
    assert!(out.demotion.is_none());
    assert_eq!(out.demotion, None);
}

#[test]
fn test_reassignment_inside_while_loop_demotes() {
    let analysis = parsed("count = 0\nwhile go():\n    count = count + 1\n").analysis();
    let c = candidate(&analysis, "count");
    assert!(c.demotion.is_some());
}

#[test]
fn test_augassign_outside_loop_still_demotes() {
    let analysis = parsed("total = 0\ntotal += cost()\n").analysis();
    let c = candidate(&analysis, "total");
    assert!(c.demotion.is_some());
}

#[test]
fn test_uses_argv_detected() {
    assert!(parsed(IMAGE_STITCH).analysis().uses_argv);
    assert!(!parsed("print('no args')\n").analysis().uses_argv);
    assert!(
        parsed("import sys\nn = len(sys.argv)\n")
            .analysis()
            .uses_argv
    );
}

#[test]
fn test_filename_literal_hint_found() {
    assert_eq!(
        parsed(IMAGE_STITCH).analysis().filename_literals,
        ["output_long_image.jpg"]
    );
}

#[test]
fn test_no_hint_for_named_constant_usage() {
    // Once the literal is extracted to a named constant, the call site holds a Name,
    // not a Constant — the hint disappears (the edit→rescan loop from the simulation).
    let text = "OUTPUT = 'output_long_image.jpg'\nsave(OUTPUT)\n";
    assert!(parsed(text).analysis().filename_literals.is_empty());
}

#[test]
fn test_hint_excludes_non_filenames() {
    let text = concat!(
        "new('RGB')\n",                            // no extension
        "log('finished: output.jpg now ready')\n", // sentence, has spaces
        "get('https://example.com/a.zip')\n",      // URL
        "ver('3.14')\n",                           // numeric "extension" is a version
    );
    assert!(parsed(text).analysis().filename_literals.is_empty());
}

#[test]
fn test_hint_dedupes_and_caps_at_three() {
    let text = "f('a.png')\nf('a.png')\nf('b.png')\nf('c.png')\nf('d.png')\n";
    assert_eq!(
        parsed(text).analysis().filename_literals,
        ["a.png", "b.png", "c.png"]
    );
}
