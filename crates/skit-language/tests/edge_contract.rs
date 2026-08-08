use std::{collections::BTreeMap, fs, path::Path};

use skit_domain::parameters::{
    ParamDecl, ParameterBinding, ParameterDelivery, ParameterType, ParameterValue,
};
use skit_language::{
    cli_params, detect_candidates, external_dependencies, external_dependencies_at, infer_kind,
    inject_values, managed_params, normalize_shell_default, placeholder_params, python_version_pin,
    read_uv_metadata, shebang_program, source_is_valid, validate_pep440_specifiers,
    validate_pep508_requirement, write_managed_params, write_uv_metadata,
};
use tempfile::TempDir;

#[test]
fn inference_and_parser_gates_cover_every_supported_shebang_and_compound_extension() {
    for (program, kind) in [
        ("fish", "fish"),
        ("ruby", "ruby"),
        ("perl", "perl"),
        ("luajit", "lua"),
        ("Rscript", "r"),
        ("powershell.exe", "powershell"),
        ("deno", "js"),
        ("bun", "js"),
        ("dash", "shell"),
    ] {
        let shebang = format!("#! /usr/bin/env -S {program} --flag");
        assert_eq!(
            infer_kind(Path::new("tool"), Some(&shebang), false),
            Some(kind)
        );
    }
    for (name, kind) in [
        ("tool.mjs", "js"),
        ("tool.cjs", "js"),
        ("tool.mts", "ts"),
        ("tool.cts", "ts"),
        ("tool.zsh", "shell"),
    ] {
        assert_eq!(infer_kind(Path::new(name), None, false), Some(kind));
    }
    assert_eq!(
        infer_kind(Path::new("tool"), Some("#! /bin/unknown"), false),
        None
    );
    assert_eq!(
        infer_kind(Path::new("tool"), Some("not a shebang"), false),
        None
    );
    assert!(source_is_valid("future", "any bytes \0 are accepted"));
    assert!(!source_is_valid("python", "def broken(:\n"));
}

#[test]
fn versioned_python_shebangs_publish_kind_and_constraint_signals() {
    for (program, expected) in [
        ("python3.12", ">=3.12,<3.13"),
        ("python3.12.1", ">=3.12.1,<3.13"),
        ("python3.12.1.7", ">=3.12.1.7,<3.13"),
    ] {
        let line = format!("#!/usr/bin/env -S {program} -I");
        assert_eq!(shebang_program(&line), Some(program));
        assert_eq!(
            infer_kind(Path::new("tool"), Some(&line), false),
            Some("python")
        );
        assert_eq!(python_version_pin(program).as_deref(), Some(expected));
    }

    for program in ["python", "python3", "python2.7", "python3.", "python3.x"] {
        assert_eq!(python_version_pin(program), None, "program={program}");
    }
    assert_eq!(python_version_pin("Python3.12"), None);
    assert_eq!(shebang_program("print('not a shebang')"), None);
    assert_eq!(shebang_program(" #!/usr/bin/python3.12"), None);
    assert_eq!(shebang_program("#!/usr/bin/Env python3.12"), Some("Env"));
}

#[test]
fn python_metadata_validation_uses_pep_508_and_pep_440_grammars() {
    for requirement in [
        "requests>=2,<3",
        "httpx[http2]>=0.28; python_version >= '3.12'",
        "demo @ https://example.com/demo-1.0.tar.gz",
    ] {
        validate_pep508_requirement(requirement).unwrap();
    }
    for requirement in ["", "@@@", "requests=>2", "not a name"] {
        assert!(
            validate_pep508_requirement(requirement).is_err(),
            "requirement={requirement}"
        );
    }

    for constraint in ["", ">=3.12", ">=3.12,<3.13", "~=3.12"] {
        validate_pep440_specifiers(constraint).unwrap();
    }
    for constraint in ["3.12", ">=banana", "^3.12"] {
        assert!(
            validate_pep440_specifiers(constraint).is_err(),
            "constraint={constraint}"
        );
    }
}

#[test]
fn metadata_writers_cover_creation_removal_crlf_and_invalid_input() {
    assert!(write_managed_params("ruby", "puts 1\n", &[]).is_err());
    assert!(managed_params("ruby", "puts 1\n").is_empty());
    assert!(write_uv_metadata("# /// script\n# invalid = [\n# ///\n", &[], "").is_err());

    let created = write_uv_metadata("#!/usr/bin/python", &["rich".to_owned()], ">=3.13").unwrap();
    assert!(created.starts_with("#!/usr/bin/python\n# /// script\n"));
    let removed = write_uv_metadata(&created, &[], "").unwrap();
    let metadata = read_uv_metadata(&removed).unwrap();
    assert!(metadata.dependencies.is_empty());
    assert!(metadata.requires_python.is_empty());

    let mut declaration = ParamDecl::new("VALUE");
    declaration.binding = ParameterBinding::Const;
    declaration.delivery = ParameterDelivery::Inject;
    declaration.prompt = "quote \" slash \\ newline\n tab\t control \u{85}".to_owned();
    declaration.choices = vec!["one".to_owned(), "two".to_owned()];
    let written = write_managed_params("shell", "echo ok\r\n", &[declaration]).unwrap();
    assert!(written.contains("\\n"));
    assert!(written.contains("\\t"));
    assert!(written.contains("\\u0085"));
    assert_eq!(managed_params("shell", &written).len(), 1);

    let nested = concat!(
        "# /// script\n",
        "# [tool.skit]\n",
        "# schema = 1\n",
        "# [[tool.skit.params]]\n",
        "# name = \"value\"\n",
        "# when = 2026-08-08T00:00:00Z\n",
        "# array = [1, 2]\n",
        "# table = { key = true }\n",
        "# ///\n",
    );
    assert_eq!(managed_params("python", nested)[0].name, "value");

    let without_shebang = write_uv_metadata("print(1)\n", &[], ">=3.13").unwrap();
    assert!(without_shebang.starts_with("# /// script\n"));
}

#[test]
fn managed_metadata_accepts_legacy_whitespace_and_preserves_unknown_fields() {
    let source = concat!(
        "# /// script   \n",
        "# dependencies = [\"requests\"]\n",
        "# [tool.skit]\n",
        "# schema = 1\n",
        "# future = { enabled = true }\n",
        "# [[tool.skit.params]]\n",
        "# name = \"WIDTH\"\n",
        "# kind = \"const\"\n",
        "# type = \"int\"\n",
        "# default = 800\n",
        "# future_row = 2026-08-08T00:00:00Z\n",
        "# ///   \n",
        "WIDTH = 800\n",
    );
    let mut declaration = managed_params("python", source).remove(0);
    declaration.prompt = "Width".to_owned();

    let written = write_managed_params("python", source, &[declaration]).unwrap();

    assert_eq!(managed_params("python", &written)[0].prompt, "Width");
    assert!(written.contains("dependencies = [\"requests\"]"));
    assert!(written.contains("future"));
    assert!(written.contains("enabled = true"));
    assert!(written.contains("future_row = 2026-08-08T00:00:00Z"));
    assert_eq!(written.matches("# /// script").count(), 1);
}

#[test]
fn cli_readers_cover_optional_shapes_and_duplicate_replacement() {
    let python = r#"
p.add_argument('--level', type=float, default=1.5)
p.add_argument('--level', type=str, default='high')
@click.option('-q', '--quiet', type=bool)
def main(path: str = typer.Argument(...), mode: str = typer.Option('safe', '-m')): pass
"#;
    let fields = cli_params("python", python);
    assert_eq!(
        fields.iter().filter(|field| field.name == "level").count(),
        1
    );
    assert_eq!(
        fields
            .iter()
            .find(|field| field.name == "level")
            .unwrap()
            .parameter_type,
        ParameterType::Str
    );
    assert!(
        fields
            .iter()
            .any(|field| field.name == "quiet" && field.parameter_type == ParameterType::Bool)
    );
    assert!(
        fields
            .iter()
            .any(|field| field.name == "path" && field.flag.is_empty())
    );

    let shell = cli_params("shell", "while getopts ':ab:' value; do :; done\n");
    assert_eq!(
        shell
            .iter()
            .map(|field| field.name.as_str())
            .collect::<Vec<_>>(),
        ["a", "b"]
    );
    assert!(cli_params("shell", "echo ok\n").is_empty());
    let powershell = cli_params(
        "powershell",
        "param([double]$Ratio = 1.5, [bool]$Enabled = $false, [string]$Raw = bare)\n",
    );
    assert_eq!(powershell[0].parameter_type, ParameterType::Float);
    assert_eq!(powershell[1].parameter_type, ParameterType::Bool);
    assert_eq!(
        powershell[2].default,
        Some(ParameterValue::String("bare".to_owned()))
    );

    assert!(cli_params("ruby", "puts 1").is_empty());
    assert!(cli_params("python", "p.add_argument").is_empty());
    assert!(cli_params("python", "text = \"value: str = typer.Option(\"\n",).is_empty());
}

#[test]
fn injection_errors_and_literal_preservation_are_explicit() {
    let mut missing = ParamDecl::new("MISSING");
    missing.binding = ParameterBinding::Const;
    missing.delivery = ParameterDelivery::Inject;
    for kind in ["python", "shell", "js"] {
        assert!(
            inject_values(
                kind,
                if kind == "shell" {
                    "echo ok\n"
                } else {
                    "print('ok')\n"
                },
                std::slice::from_ref(&missing),
                &BTreeMap::from([("MISSING".to_owned(), "value".to_owned())]),
            )
            .is_err()
        );
    }
    assert!(
        inject_values(
            "ruby",
            "puts 1\n",
            std::slice::from_ref(&missing),
            &BTreeMap::from([("MISSING".to_owned(), "value".to_owned())]),
        )
        .is_err()
    );
    assert!(inject_values("python", "broken(:\n", &[missing], &BTreeMap::new(),).is_err());

    let mut value = ParamDecl::new("VALUE");
    value.binding = ParameterBinding::Const;
    value.delivery = ParameterDelivery::Inject;
    for (parameter_type, source, replacement, expected) in [
        (ParameterType::Float, "VALUE = 1.5\n", "2", "VALUE = 2.0\n"),
        (
            ParameterType::Bool,
            "VALUE = True\n",
            "yes",
            "VALUE = True\n",
        ),
        (ParameterType::Int, "VALUE = 1\n", "01", "VALUE = 1\n"),
        (
            ParameterType::Str,
            "VALUE = 'old'\n",
            "a'b\\c\nnext\rline",
            "VALUE = 'a\\'b\\\\c\\nnext\\rline'\n",
        ),
    ] {
        value.parameter_type = parameter_type;
        assert_eq!(
            inject_values(
                "python",
                source,
                std::slice::from_ref(&value),
                &BTreeMap::from([("VALUE".to_owned(), replacement.to_owned())]),
            )
            .unwrap(),
            expected,
        );
    }
}

#[test]
fn dependency_and_placeholder_scanners_reject_local_and_malformed_specifiers() {
    assert!(placeholder_params("python", "{name}").is_empty());
    assert!(
        placeholder_params("prompt", "{{not-valid!}} {{ok}}")
            .iter()
            .any(|field| field.name == "ok")
    );
    let source = r#"
import "side-effect";
export { x } from "exported";
const a = require("@scope/pkg/deep");
const b = require("../local");
const c = require("https://example.test/mod.js");
"#;
    assert_eq!(
        external_dependencies("ts", source),
        ["@scope/pkg", "exported", "side-effect"]
    );
    assert!(external_dependencies("shell", "echo ok").is_empty());
    assert!(normalize_shell_default("if then\n", "NAME").is_err());
}

#[test]
fn dependency_scanners_use_syntax_mappings_and_the_source_directory() {
    let directory = TempDir::new().unwrap();
    fs::write(directory.path().join("helpers.py"), "VALUE = 1\n").unwrap();
    fs::create_dir(directory.path().join("package")).unwrap();
    fs::write(
        directory.path().join("package").join("__init__.py"),
        "VALUE = 2\n",
    )
    .unwrap();
    let python = r#"
"""import fake_from_string"""
# import fake_from_comment
import PIL, yaml, helpers
from package.module import VALUE
from cv2 import imread
"#;
    assert_eq!(
        external_dependencies_at("python", python, Some(directory.path())),
        ["Pillow", "PyYAML", "opencv-python"]
    );

    let javascript = r##"
// import fake from "comment-only";
const text = "require('string-only')";
import value from "package-one/deep";
export { other } from "@scope/package-two/deep";
const dynamic = import("package-three/feature");
const common = require("package-four/subpath");
const builtin = require("fs");
import "node:path";
import "npm:package-five";
import "jsr:@scope/package-six";
import "data:text/javascript,export default 1";
import "file:///tmp/local.js";
import "bun:test";
import "#internal";
"##;
    assert_eq!(
        external_dependencies("js", javascript),
        [
            "@scope/package-two",
            "package-four",
            "package-one",
            "package-three",
        ]
    );
}

#[test]
fn analyzer_and_injector_edges_keep_only_actionable_bindings() {
    assert!(detect_candidates("ruby", "VALUE = 1\n").is_empty());

    let python = "_PRIVATE = 1\nmodule.VALUE = 2\nCALL = make()\nVALUE = ((1))\n";
    let python_candidates = detect_candidates("python", python);
    assert_eq!(python_candidates.len(), 1);
    assert_eq!(python_candidates[0].name, "VALUE");
    assert_eq!(
        python_candidates[0].default,
        Some(ParameterValue::Integer(1))
    );

    let shell = concat!(
        "_HIDDEN=${_HIDDEN:-one}\n",
        "NAME=${NAME:-one}\n",
        "NAME=${NAME:-two}\n",
        "COLOR=red\n",
        "COLOR=blue\n",
    );
    let shell_candidates = detect_candidates("shell", shell);
    assert_eq!(
        shell_candidates
            .iter()
            .find(|candidate| candidate.name == "COLOR")
            .and_then(|candidate| candidate.default.clone()),
        Some(ParameterValue::String("blue".to_owned()))
    );

    let fish = concat!(
        "set -q\n",
        "or echo missing\n",
        "set -q _HIDDEN\n",
        "or set -g _HIDDEN one\n",
        "set PORT 9\n",
        "set -q PORT\n",
        "or set PORT 10\n",
        "set -q VALUE\n",
        "or set -g VALUE one\n",
        "set -q VALUE\n",
        "or set -g VALUE two\n",
        "set -q BAD\n",
        "or echo no\n",
        "set -q LEFT\n",
        "or set RIGHT no\n",
        "set -q EMPTY\n",
        "or set EMPTY\n",
        "\"unterminated\n",
    );
    let fish_candidates = detect_candidates("fish", fish);
    assert_eq!(fish_candidates.len(), 1);
    assert_eq!(fish_candidates[0].name, "VALUE");

    assert!(placeholder_params("command", "{{escaped}} {unclosed").is_empty());

    let mut ignored = ParamDecl::new("VALUE");
    ignored.delivery = ParameterDelivery::Flag;
    assert_eq!(
        inject_values(
            "js",
            "const VALUE = 1;\n",
            &[ignored],
            &BTreeMap::from([("VALUE".to_owned(), "2".to_owned())]),
        )
        .unwrap(),
        "const VALUE = 1;\n"
    );

    let mut first = ParamDecl::new("first");
    first.binding = ParameterBinding::Input;
    first.delivery = ParameterDelivery::Inject;
    first.order = 0;
    let mut second = ParamDecl::new("second");
    second.binding = ParameterBinding::Input;
    second.delivery = ParameterDelivery::Inject;
    second.order = 1;
    assert!(
        inject_values(
            "shell",
            "read -p 'Values: ' FIRST SECOND\n",
            std::slice::from_ref(&first),
            &BTreeMap::from([("first".to_owned(), "one".to_owned())]),
        )
        .is_err()
    );
    assert_eq!(
        inject_values(
            "shell",
            "read -p 'Values: ' FIRST SECOND\n",
            &[first, second],
            &BTreeMap::from([
                ("first".to_owned(), "one".to_owned()),
                ("second".to_owned(), "two words".to_owned()),
            ]),
        )
        .unwrap(),
        "FIRST=one; SECOND='two words'\n"
    );
}
