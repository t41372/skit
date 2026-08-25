//! Deterministic source workloads for parser benchmarks.

use thiserror::Error;

use crate::python_random::PythonRandom;

/// Languages in the stable analyzer grid.
pub const LANGUAGES: &[&str] = &["python", "shell", "js", "ts"];
const WORDS: &[&str] = &[
    "alpha", "bravo", "delta", "gamma", "kilo", "lima", "omega", "sigma",
];
const SEED: u64 = 20_260_720;

/// Source generation failed.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum SourceError {
    /// The language is outside the benchmark grid.
    #[error("unknown language {0:?} (expected python, shell, js, or ts)")]
    UnknownLanguage(String),
    /// The requested source cannot hold the fixed scaffold.
    #[error("source workloads need at least 8 lines")]
    TooShort,
}

/// Return the fixture extension for one benchmark language.
#[must_use]
pub fn extension(language: &str) -> &'static str {
    match language {
        "python" => "py",
        "shell" => "sh",
        "js" => "js",
        "ts" => "ts",
        _ => "",
    }
}

/// Generate one valid parser workload with an exact line count and final newline.
pub fn generate(language: &str, lines: usize) -> Result<String, SourceError> {
    if lines < 8 {
        return Err(SourceError::TooShort);
    }
    let mut rng = PythonRandom::seeded(&format!("{SEED}:{language}:{lines}"));
    let mut body = match language {
        "python" => python(lines, &mut rng),
        "shell" => shell(lines, &mut rng),
        "js" => javascript(lines, &mut rng, false),
        "ts" => javascript(lines, &mut rng, true),
        _ => return Err(SourceError::UnknownLanguage(language.to_owned())),
    };
    let comment = if matches!(language, "js" | "ts") {
        "//"
    } else {
        "#"
    };
    while body.len() < lines {
        body.push(format!("{comment} filler line {}", body.len() + 1));
    }
    debug_assert_eq!(body.len(), lines);
    Ok(format!("{}\n", body.join("\n")))
}

/// Generate the same workload with a half-written final construct.
pub fn generate_broken(language: &str, lines: usize) -> Result<String, SourceError> {
    let truncation = match language {
        "python" => "def fn_half_written(x: int,",
        "shell" => "if [ \"${ALPHA}\" = ",
        "js" => "function fnHalfWritten(x) {",
        "ts" => "function fnHalfWritten(x: number): number {",
        _ => return Err(SourceError::UnknownLanguage(language.to_owned())),
    };
    let source = generate(language, lines)?;
    let mut body = source.lines().map(str::to_owned).collect::<Vec<_>>();
    *body.last_mut().expect("minimum line count") = truncation.to_owned();
    Ok(format!("{}\n", body.join("\n")))
}

fn python(lines: usize, rng: &mut PythonRandom) -> Vec<String> {
    let word = random_word(rng);
    let default = rng.range(0, 9);
    let mut body = vec![
        "import argparse".to_owned(),
        String::new(),
        "parser = argparse.ArgumentParser(description='generated bench source')".to_owned(),
        format!("parser.add_argument('--{word}', type=int, default={default})"),
        "parser.add_argument('--verbose', action='store_true')".to_owned(),
        "args = parser.parse_args()".to_owned(),
    ];
    while body.len() < lines - 2 {
        let length = random_word(rng).len();
        body.extend([
            format!("def fn_{}(x: int) -> int:", body.len()),
            format!("    return x + {length}"),
        ]);
    }
    body
}

fn shell(lines: usize, rng: &mut PythonRandom) -> Vec<String> {
    let mut body = vec![
        "#!/usr/bin/env bash".to_owned(),
        "set -euo pipefail".to_owned(),
    ];
    let variables = rng.range(3, 6);
    for word in &WORDS[..variables] {
        body.push(format!(
            "{}=\"${{{}:-{}}}\"",
            word.to_ascii_uppercase(),
            word.to_ascii_uppercase(),
            rng.range(0, 99)
        ));
    }
    while body.len() < lines - 1 {
        let variable = WORDS[body.len() % WORDS.len()].to_ascii_uppercase();
        body.push(format!("echo \"step {}: ${variable}\"", body.len()));
    }
    body
}

fn javascript(lines: usize, rng: &mut PythonRandom, typed: bool) -> Vec<String> {
    let word = random_word(rng);
    let default = rng.range(0, 9);
    let mut body = if typed {
        vec![
            "const args: string[] = process.argv.slice(2);".to_owned(),
            format!("let {word}: number = {default};"),
        ]
    } else {
        vec![
            "const args = process.argv.slice(2);".to_owned(),
            format!("let {word} = {default};"),
        ]
    };
    while body.len() + 3 <= lines {
        let index = body.len();
        if typed {
            body.extend([
                format!("function fn{index}(x: number): number {{"),
                format!("  return x + {index};"),
                "}".to_owned(),
            ]);
        } else {
            body.extend([
                format!("function fn{index}(x) {{"),
                format!("  return x + {index};"),
                "}".to_owned(),
            ]);
        }
    }
    body
}

fn random_word(rng: &mut PythonRandom) -> &'static str {
    rng.choice(WORDS)
}
