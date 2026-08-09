use sha2::{Digest as _, Sha256};
use skit_benchmarks::sources::{LANGUAGES, extension, generate, generate_broken};

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
        assert_eq!(hex::encode(Sha256::digest(source.as_bytes())), digest);
    }
}
