use std::{fs, path::Path, process::Command};

use sha2::{Digest as _, Sha256};
use skit_benchmarks::{
    dataset::RUNOVER_PYTHON,
    sources::{LANGUAGES, extension, generate, generate_broken},
};
use skit_language::{ParseOutcome, parse_document};
use skit_runtime::{ProgramProbe as _, SystemProbe};
use tempfile::TempDir;

fn sha256(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

#[test]
fn source_workloads_are_deterministic_exact_and_language_shaped() {
    for language in LANGUAGES {
        let source = generate(language, 200).unwrap();
        assert_eq!(source.lines().count(), 200);
        assert!(source.ends_with('\n'));
        assert_eq!(source, generate(language, 200).unwrap());
        assert!(!extension(language).is_empty());

        let broken = generate_broken(language, 2_000).unwrap();
        assert_eq!(broken.lines().count(), 2_000);
        assert_ne!(broken, generate(language, 2_000).unwrap());
    }
}

#[test]
fn source_workloads_refuse_unknown_or_too_small_inputs() {
    assert!(generate("unknown", 20).is_err());
    assert!(generate_broken("unknown", 20).is_err());
    assert!(generate("python", 7).is_err());
    assert_eq!(extension("unknown"), "");
}

#[test]
fn source_workloads_keep_the_latest_python_main_bytes() {
    let expected = [
        (
            "python",
            20,
            "331e49e5afdc220ec7072bce1c36bcadfbf4ef27a272fbcfea6c58232048c5eb",
        ),
        (
            "python",
            200,
            "bddbff14521b972353a786a23d26878241ebd201bb3abddbf41816f5b6a0e30c",
        ),
        (
            "python",
            2_000,
            "d3930447d045e075d42e9de5af26d7fc82b00aadddfd22ab5e3280c71611c2fd",
        ),
        (
            "shell",
            20,
            "18313d3c90cf19940c4d7929f98175c78e581a6a8ae2beb3298e11e92dd6e71f",
        ),
        (
            "shell",
            200,
            "1580868ff9403e7994f6154dcf140ef662296cffe4b8270323e6f7f27a46bdd9",
        ),
        (
            "shell",
            2_000,
            "f06766fb3268b15d1ef41004302894cab8dcc0f60e7cd8e40481ff07b8883076",
        ),
        (
            "js",
            20,
            "a71c521298245d452903ff3c72ecb46587eaa0a2d94207d1752a3a50c2c66c0b",
        ),
        (
            "js",
            200,
            "3c5dce15dd809b1943dfd33347a27d9f5bf63b7100f5c4570032450b58d3f78c",
        ),
        (
            "js",
            2_000,
            "d6fcca3167e919aa5745cea2789e995ce312d9cda6e8486ae2bc08321298de4d",
        ),
        (
            "ts",
            20,
            "53d5404ce5ee784b79841b1897a8c63c81e064076d62cfe56404f761ca8c0e1e",
        ),
        (
            "ts",
            200,
            "9dfa7b5ba9138fd6a9e23166eead6e9a73fae74eb9aa39e178a4d8ee927c18f4",
        ),
        (
            "ts",
            2_000,
            "4fa53b7268b29fccdac92e0c751ba48d48980f6d4ad75c3fdb45cb88ecd8f558",
        ),
    ];
    for (language, lines, digest) in expected {
        let source = generate(language, lines).unwrap();
        assert_eq!(sha256(source.as_bytes()), digest);
    }
}

#[test]
fn test_broken_workloads_are_byte_stable_and_actually_broken() {
    let expected = [
        (
            "python",
            "def fn_half_written(x: int,",
            "af266a68d9826a6d003dc4e4b1609c15ff74ac4a8dd76a21217505fcbe2fdb22",
        ),
        (
            "shell",
            "if [ \"${ALPHA}\" = ",
            "cd792b3d0663223351bb25f585e0da85d220b7dbada1720dc3185504076b0b2b",
        ),
        (
            "js",
            "function fnHalfWritten(x) {",
            "deaf98beca99466bcf3f651f757adbb1ddb2f94733087156b1b6a68514437ff2",
        ),
        (
            "ts",
            "function fnHalfWritten(x: number): number {",
            "8e64a7453ec0fbf7d32dc257f1c44c13d4927c8571055e59c34e87985176de21",
        ),
    ];

    for (language, final_line, digest) in expected {
        let valid = generate(language, 2_000).unwrap();
        let broken = generate_broken(language, 2_000).unwrap();
        assert_eq!(
            sha256(broken.as_bytes()),
            digest,
            "{language} broken workload bytes drifted from the frozen oracle"
        );
        assert!(broken.ends_with(&format!("{final_line}\n")));

        let valid_lines = valid.lines().collect::<Vec<_>>();
        let broken_lines = broken.lines().collect::<Vec<_>>();
        assert_eq!(valid_lines.len(), 2_000);
        assert_eq!(broken_lines.len(), 2_000);
        assert_eq!(
            broken_lines[..1_999],
            valid_lines[..1_999],
            "{language} broken workload changed bytes outside its frozen damaged line"
        );
        assert_eq!(broken_lines[1_999], final_line);
        assert_ne!(broken_lines[1_999], valid_lines[1_999]);

        assert!(
            matches!(
                parse_document(language, &broken),
                ParseOutcome::SyntaxError(_)
            ),
            "{language} frozen broken workload is no longer rejected by its real analyzer"
        );
        assert!(
            matches!(parse_document(language, &valid), ParseOutcome::Parsed(_)),
            "{language} valid twin is no longer accepted by its real analyzer"
        );
    }
}

fn collect_python_subjects(directory: &Path, output: &mut Vec<(String, Vec<u8>)>) {
    for entry in fs::read_dir(directory).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            collect_python_subjects(&path, output);
        } else if path.extension().is_some_and(|extension| extension == "py") {
            output.push((
                path.strip_prefix(directory)
                    .unwrap_or(&path)
                    .display()
                    .to_string(),
                fs::read(&path).unwrap(),
            ));
        }
    }
}

#[test]
#[ignore = "HOST TOOL GATE: requires a real CPython 3.13 on PATH. The three-platform CI matrix installs 3.13 and runs this exact ignored owner explicitly."]
fn test_python_compiles() {
    let probe = SystemProbe;
    let python = ["python3.13", "python3", "python"]
        .into_iter()
        .filter_map(|name| probe.find_program(name))
        .find(|program| {
            let output = Command::new(program).arg("--version").output().unwrap();
            let version = format!(
                "{}{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            output.status.success() && version.trim().starts_with("Python 3.13.")
        })
        .expect("CPython 3.13 is required; install it on PATH and rerun this ignored tool gate");

    let repository = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut subjects = Vec::new();
    collect_python_subjects(&repository.join("benchmarks/fixtures"), &mut subjects);
    assert!(
        subjects
            .iter()
            .any(|(_, bytes)| bytes.as_slice() == RUNOVER_PYTHON.as_bytes()),
        "the shipped run-over Python subject was not discovered"
    );
    for lines in [20, 200, 2_000] {
        subjects.push((
            format!("analyzer-{lines}.py"),
            generate("python", lines).unwrap().into_bytes(),
        ));
    }

    let compiled = TempDir::new().unwrap();
    let mut paths = Vec::new();
    for (index, (name, bytes)) in subjects.into_iter().enumerate() {
        let path = compiled.path().join(format!("{index}-{name}"));
        fs::write(&path, bytes).unwrap();
        paths.push(path);
    }
    let output = Command::new(&python)
        .args(["-m", "py_compile"])
        .args(&paths)
        .current_dir(&repository)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{} failed to compile every shipped Python benchmark subject:\n{}",
        python.display(),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stdout.is_empty());
    assert_eq!(paths.len(), 4);
}
