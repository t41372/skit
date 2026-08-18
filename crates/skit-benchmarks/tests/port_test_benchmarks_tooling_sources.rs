//! Frozen source-workload contracts from `tests/test_benchmarks_tooling.py`.

use skit_benchmarks::sources::{LANGUAGES, generate};

#[test]
fn test_exact_line_counts() {
    for language in LANGUAGES {
        for lines in [20, 200] {
            let text = generate(language, lines).unwrap();
            assert_eq!(
                text.lines().count(),
                lines,
                "{language} generator changed the requested cost axis"
            );
            assert!(text.ends_with('\n'), "{language} source lost its final newline");
        }
    }
}

#[test]
fn test_rejects_bad_inputs() {
    let unknown = generate("cobol", 20).unwrap_err().to_string();
    assert!(unknown.contains("unknown language"));
    let short = generate("shell", 4).unwrap_err().to_string();
    assert!(short.contains("at least 8"));
}

#[test]
fn test_analyzer_constructs_present() {
    assert!(generate("python", 20).unwrap().contains("argparse"));
    assert!(generate("shell", 20).unwrap().contains(":-"));
    assert!(generate("js", 20).unwrap().contains("process.argv"));
    assert!(generate("ts", 20).unwrap().contains(": number"));
}
