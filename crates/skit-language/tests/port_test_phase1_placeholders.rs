//! Placeholder-extraction contract from Python `tests/test_phase1.py`.

use skit_language::placeholder_params;

#[test]
fn test_extract_placeholders() {
    let params = placeholder_params(
        "command",
        "ffmpeg -i {input} -vf scale={width}:{height} {output}",
    );
    let names = params
        .iter()
        .map(|parameter| parameter.name.as_str())
        .collect::<Vec<_>>();

    assert_eq!(names, ["input", "width", "height", "output"]);
}
