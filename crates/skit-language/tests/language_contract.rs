use std::{collections::BTreeMap, path::Path};

use skit_domain::parameters::{
    ParamDecl, ParameterBinding, ParameterDelivery, ParameterType, ParameterValue,
};
use skit_language::{
    cli_params, detect_candidates, external_dependencies, infer_kind, inject_values,
    managed_params, normalize_shell_default, placeholder_params, read_uv_metadata,
    write_managed_params, write_uv_metadata,
};

#[test]
fn kind_inference_covers_extensions_compound_prompt_names_and_shebangs() {
    let cases = [
        ("tool.py", None, false, Some("python")),
        ("tool.sh", None, false, Some("shell")),
        ("tool.bash", None, false, Some("shell")),
        ("tool.fish", None, false, Some("fish")),
        ("tool.js", None, false, Some("js")),
        ("tool.mjs", None, false, Some("js")),
        ("tool.ts", None, false, Some("ts")),
        ("tool.ps1", None, false, Some("powershell")),
        ("tool.rb", None, false, Some("ruby")),
        ("tool.pl", None, false, Some("perl")),
        ("tool.lua", None, false, Some("lua")),
        ("tool.r", None, false, Some("r")),
        ("review.prompt.md", None, false, Some("prompt")),
        (
            "plain",
            Some("#!/usr/bin/env python3"),
            false,
            Some("python"),
        ),
        ("plain", Some("#!/bin/zsh"), false, Some("shell")),
        ("plain", Some("#!/usr/bin/env node"), false, Some("js")),
        (
            "plain",
            Some("#!/usr/bin/env pwsh"),
            false,
            Some("powershell"),
        ),
        ("plain", None, true, Some("exe")),
        ("plain.txt", None, false, None),
    ];
    for (name, shebang, executable, expected) in cases {
        assert_eq!(
            infer_kind(Path::new(name), shebang, executable),
            expected,
            "name={name}"
        );
    }
}

#[test]
fn managed_block_round_trips_python_shell_and_javascript_comment_dialects() {
    for (kind, source) in [
        ("python", "#!/usr/bin/env python3\nprint('ok')\n"),
        ("shell", "#!/usr/bin/env bash\necho ok\n"),
        ("js", "console.log('ok');\n"),
        ("ts", "console.log('ok');\n"),
    ] {
        let mut secret = ParamDecl::new("API_KEY");
        secret.binding = ParameterBinding::Const;
        secret.delivery = ParameterDelivery::Inject;
        secret.default = Some(ParameterValue::String("old".to_owned()));
        secret.secret = true;
        let mut count = ParamDecl::new("count");
        count.binding = ParameterBinding::Input;
        count.delivery = ParameterDelivery::Inject;
        count.parameter_type = ParameterType::Int;
        count.prompt = "Count: ".to_owned();
        count.order = 0;

        let written = write_managed_params(kind, source, &[secret.clone(), count.clone()]).unwrap();
        let read = managed_params(kind, &written);

        assert_eq!(read, [secret, count], "kind={kind}\n{written}");
        assert!(written.contains("[tool.skit]"));
        assert!(written.contains("[[tool.skit.params]]"));
        let leader = if matches!(kind, "js" | "ts") {
            "//"
        } else {
            "#"
        };
        assert!(written.lines().any(|line| line.starts_with(leader)));
        let removed = write_managed_params(kind, &written, &[]).unwrap();
        assert!(!removed.contains("[tool.skit]"));
        assert!(removed.contains("ok"));
    }
}

#[test]
fn python_cli_reader_covers_argparse_click_and_typer_shapes() {
    let argparse = r#"
import argparse
p = argparse.ArgumentParser()
p.add_argument("input")
p.add_argument("--count", type=int, required=True, default=3, help="How many")
p.add_argument("--mode", choices=["fast", "safe"], default="safe")
p.add_argument("--verbose", action="store_true")
"#;
    let params = cli_params("python", argparse);
    assert_eq!(params.len(), 4);
    assert_eq!(params[0].name, "input");
    assert!(params[0].flag.is_empty());
    assert_eq!(params[1].name, "count");
    assert_eq!(params[1].flag, "--count");
    assert_eq!(params[1].parameter_type, ParameterType::Int);
    assert!(params[1].required);
    assert_eq!(params[1].default, Some(ParameterValue::Integer(3)));
    assert_eq!(params[1].help, "How many");
    assert_eq!(params[2].choices, ["fast", "safe"]);
    assert_eq!(params[3].parameter_type, ParameterType::Bool);
    assert_eq!(params[3].action, "store_true");

    let click = r#"
@click.command()
@click.option("--name", required=True, default="world", help="Who")
def main(name): pass
"#;
    let params = cli_params("python", click);
    assert_eq!(params.len(), 1);
    assert_eq!(params[0].name, "name");
    assert_eq!(
        params[0].default,
        Some(ParameterValue::String("world".to_owned()))
    );
    assert!(params[0].required);

    let typer = r#"
def main(count: int = typer.Option(2, "--count", help="Count")):
    pass
"#;
    let params = cli_params("python", typer);
    assert_eq!(params.len(), 1);
    assert_eq!(params[0].name, "count");
    assert_eq!(params[0].parameter_type, ParameterType::Int);
    assert_eq!(params[0].default, Some(ParameterValue::Integer(2)));
}

#[test]
fn shell_js_fish_and_powershell_cli_readers_produce_flag_fields() {
    let shell = r#"
while getopts "vf:o:" opt; do
  case "$opt" in
    v) verbose=1 ;;
    f) file="$OPTARG" ;;
    o) output="$OPTARG" ;;
  esac
done
"#;
    let params = cli_params("shell", shell);
    assert_eq!(
        params.iter().map(|p| p.flag.as_str()).collect::<Vec<_>>(),
        ["-v", "-f", "-o"]
    );
    assert_eq!(params[0].parameter_type, ParameterType::Bool);

    let js = r#"
const { values } = parseArgs({ options: {
  count: { type: 'string', short: 'c' },
  verbose: { type: 'boolean', short: 'v' }
}});
"#;
    let params = cli_params("js", js);
    assert_eq!(params.len(), 2);
    assert_eq!(params[0].flag, "--count");
    assert_eq!(params[1].parameter_type, ParameterType::Bool);

    let fish = "argparse 'v/verbose' 'o/output=' -- $argv\n";
    let params = cli_params("fish", fish);
    assert_eq!(params.len(), 2);
    assert_eq!(params[0].flag, "--verbose");
    assert_eq!(params[0].parameter_type, ParameterType::Bool);
    assert_eq!(params[1].flag, "--output");

    let powershell = r#"
param(
  [Parameter(Mandatory=$true)][string]$Name,
  [int]$Count = 2,
  [switch]$Force
)
"#;
    let params = cli_params("powershell", powershell);
    assert_eq!(params.len(), 3);
    assert_eq!(params[0].flag, "-Name");
    assert!(params[0].required);
    assert_eq!(params[1].parameter_type, ParameterType::Int);
    assert_eq!(params[2].parameter_type, ParameterType::Bool);
}

#[test]
fn candidate_detection_covers_rewrite_and_environment_idioms() {
    let python = "WIDTH = 800\nname = input(\"Name: \")\n";
    let candidates = detect_candidates("python", python);
    assert_eq!(candidates.len(), 2);
    assert_eq!(candidates[0].name, "WIDTH");
    assert_eq!(candidates[0].binding, ParameterBinding::Const);
    assert_eq!(candidates[0].default, Some(ParameterValue::Integer(800)));
    assert_eq!(candidates[1].binding, ParameterBinding::Input);
    assert_eq!(candidates[1].prompt, "Name: ");

    let shell = "COLOR=${COLOR:-blue}\nread -p 'Name: ' NAME\n";
    let candidates = detect_candidates("shell", shell);
    assert_eq!(candidates.len(), 2);
    assert_eq!(candidates[0].binding, ParameterBinding::EnvDefault);
    assert_eq!(candidates[0].delivery, ParameterDelivery::Env);
    assert_eq!(candidates[0].env_target, "COLOR");
    assert_eq!(candidates[1].binding, ParameterBinding::Input);

    let js = "const PORT = 3000;\n";
    let candidates = detect_candidates("js", js);
    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0].name, "PORT");
}

#[test]
fn placeholder_detection_preserves_order_and_secret_heuristics() {
    let command = placeholder_params("command", "tool {name} {name} {API_TOKEN}");
    assert_eq!(
        command.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(),
        ["name", "API_TOKEN"]
    );
    assert_eq!(command[0].delivery, ParameterDelivery::Placeholder);
    assert!(command[1].secret);

    let prompt = placeholder_params(
        "prompt",
        "Review {{path}} then use {{API_KEY}} and {{path}}.",
    );
    assert_eq!(
        prompt.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(),
        ["path", "API_KEY"]
    );
    assert!(prompt[1].secret);
}

#[test]
fn injection_rewrites_only_selected_python_shell_and_javascript_bindings() {
    let mut python_const = ParamDecl::new("WIDTH");
    python_const.binding = ParameterBinding::Const;
    python_const.delivery = ParameterDelivery::Inject;
    let mut python_input = ParamDecl::new("name");
    python_input.binding = ParameterBinding::Input;
    python_input.delivery = ParameterDelivery::Inject;
    let python = "WIDTH = 800\nname = input(\"Name: \")\nprint(WIDTH, name)\n";
    let rewritten = inject_values(
        "python",
        python,
        &[python_const, python_input],
        &BTreeMap::from([
            ("WIDTH".to_owned(), "1024".to_owned()),
            ("name".to_owned(), "Ada".to_owned()),
        ]),
    )
    .unwrap();
    assert!(rewritten.contains("WIDTH = 1024"));
    assert!(rewritten.contains("name = 'Ada'"));
    assert!(rewritten.contains("print(WIDTH, name)"));

    let mut shell_const = ParamDecl::new("COLOR");
    shell_const.binding = ParameterBinding::Const;
    shell_const.delivery = ParameterDelivery::Inject;
    let rewritten = inject_values(
        "shell",
        "COLOR=blue\necho \"$COLOR\"\n",
        &[shell_const],
        &BTreeMap::from([("COLOR".to_owned(), "light blue".to_owned())]),
    )
    .unwrap();
    assert!(rewritten.starts_with("COLOR='light blue'\n"));

    let mut js_const = ParamDecl::new("PORT");
    js_const.binding = ParameterBinding::Const;
    js_const.delivery = ParameterDelivery::Inject;
    let rewritten = inject_values(
        "js",
        "const PORT = 3000;\nconsole.log(PORT);\n",
        &[js_const],
        &BTreeMap::from([("PORT".to_owned(), "4000".to_owned())]),
    )
    .unwrap();
    assert!(rewritten.starts_with("const PORT = 4000;\n"));
}

#[test]
fn dependency_scanner_finds_python_pep723_and_js_package_imports() {
    let python = r#"
# /// script
# dependencies = ["httpx>=0.28", "rich"]
# requires-python = ">=3.13"
# ///
print('ok')
"#;
    assert_eq!(
        external_dependencies("python", python),
        ["httpx>=0.28", "rich"]
    );
    assert_eq!(
        external_dependencies(
            "python",
            "import requests\nimport os, json\nfrom rich.console import Console\nfrom .local import value\n",
        ),
        ["requests", "rich"]
    );

    let js = r#"
import React from "react";
import x from "@scope/pkg/subpath";
import local from "./local.js";
const fs = require("node:fs");
const chalk = require("chalk");
"#;
    assert_eq!(
        external_dependencies("js", js),
        ["@scope/pkg", "chalk", "react"]
    );
}

#[test]
fn parser_backed_languages_degrade_together_for_invalid_source() {
    for (kind, source) in [
        ("shell", "if then\nNAME=value\n"),
        ("js", "const value = {;\nimport x from 'pkg';\n"),
        ("ts", "interface X { value: ; }\nimport x from 'pkg';\n"),
    ] {
        assert!(cli_params(kind, source).is_empty(), "kind={kind}");
        assert!(detect_candidates(kind, source).is_empty(), "kind={kind}");
        assert!(
            external_dependencies(kind, source).is_empty(),
            "kind={kind}"
        );
    }
}

#[test]
fn shell_normalization_changes_one_bare_constant_and_preserves_crlf() {
    let source = "#!/bin/sh\r\nNAME=world\r\nOTHER=value\r\necho \"$NAME\"\r\n";
    let rewritten = normalize_shell_default(source, "NAME").unwrap();
    assert_eq!(
        rewritten,
        "#!/bin/sh\r\nNAME=${NAME:-world}\r\nOTHER=value\r\necho \"$NAME\"\r\n"
    );
    assert!(normalize_shell_default(&rewritten, "NAME").is_err());
}

#[test]
fn uv_metadata_updates_preserve_managed_rows_and_source_bytes() {
    let source = r#"#!/usr/bin/env python3
# /// script
# requires-python = ">=3.11"
# future = "keep"
#
# [tool.skit]
# schema = 1
#
# [[tool.skit.params]]
# name = "WIDTH"
# kind = "const"
# type = "int"
# ///
WIDTH = 1
"#;
    let rewritten = write_uv_metadata(
        source,
        &["requests>=2,<3".to_owned(), "rich".to_owned()],
        ">=3.12",
    )
    .unwrap();
    let metadata = read_uv_metadata(&rewritten).unwrap();
    assert_eq!(metadata.dependencies, ["requests>=2,<3", "rich"]);
    assert_eq!(metadata.requires_python, ">=3.12");
    assert!(rewritten.contains("future = \"keep\""));
    assert!(rewritten.contains("[[tool.skit.params]]"));
    assert!(rewritten.ends_with("WIDTH = 1\n"));
}
