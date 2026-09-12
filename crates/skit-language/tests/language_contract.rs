use std::{collections::BTreeMap, path::Path};

use skit_domain::parameters::{
    ParamDecl, ParameterBinding, ParameterDelivery, ParameterType, ParameterValue,
};
use skit_language::{
    cli_params, detect_candidates, external_dependencies, infer_kind, inject_values,
    managed_params, normalize_shell_default, placeholder_params, read_uv_metadata,
    render_prompt_body, write_managed_params, write_uv_metadata,
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
fn prompt_render_is_one_pass_over_the_original_text() {
    let values = BTreeMap::from([
        ("a".to_owned(), "{{b}}".to_owned()),
        ("b".to_owned(), "changed".to_owned()),
    ]);

    assert_eq!(
        render_prompt_body("A={{a}} B={{b}}", &values, true),
        "A={{b}} B=changed"
    );
    assert_eq!(
        render_prompt_body("{{{a}}} {{missing}}", &values, true),
        "{{{a}}} {{missing}}"
    );
    assert_eq!(render_prompt_body("A={{a}}", &values, false), "A={{a}}");
}

#[test]
fn prompt_render_keeps_reserved_and_brace_adjacent_tokens_byte_exact() {
    let values = BTreeMap::from([
        ("prompt".to_owned(), "must-not-land".to_owned()),
        ("raw".to_owned(), "must-not-land".to_owned()),
        ("y".to_owned(), "must-not-land".to_owned()),
        ("real".to_owned(), "R".to_owned()),
    ]);
    let body = "{{{raw}}} and {{y}}} keep {{prompt}}; replace {{real}} and {{outer {{real}}";

    assert_eq!(
        render_prompt_body(body, &values, true),
        "{{{raw}}} and {{y}}} keep {{prompt}}; replace R and {{outer R"
    );
    assert_eq!(
        placeholder_params("prompt", "{{outer {{real}}")
            .into_iter()
            .map(|parameter| parameter.name)
            .collect::<Vec<_>>(),
        ["real"]
    );
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
import click
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
import typer
def main(count: int = typer.Option(2, "--count", help="Count")):
    pass
typer.run(main)
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
    assert_eq!(candidates[0].env_var(), "COLOR");
    assert_eq!(candidates[1].binding, ParameterBinding::Input);

    let js = "const PORT = 3000;\n";
    let candidates = detect_candidates("js", js);
    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0].name, "PORT");
}

#[test]
fn python_builtin_input_detection_uses_stable_call_site_names_and_scope_rules() {
    let source = r#"
def ask(input):
    return input("not builtin")

def nested():
    return input("Inner: ")

value = int(input("Outer: "))
"#;
    let inputs = detect_candidates("python", source)
        .into_iter()
        .filter(|declaration| declaration.binding == ParameterBinding::Input)
        .collect::<Vec<_>>();
    assert_eq!(
        inputs
            .iter()
            .map(|declaration| declaration.name.as_str())
            .collect::<Vec<_>>(),
        ["input-1", "input-2"]
    );
    assert_eq!(inputs[0].prompt, "Inner: ");
    assert_eq!(inputs[1].prompt, "Outer: ");
    assert_eq!(inputs[0].order, 0);
    assert_eq!(inputs[1].order, 1);

    for shadowed in [
        "input = str\nvalue = input('x')\n",
        "def input(prompt=''):\n    return prompt\nvalue = input('x')\n",
        "from provider import input\nvalue = input('x')\n",
        "import input\nvalue = input('x')\n",
    ] {
        assert!(
            detect_candidates("python", shadowed)
                .iter()
                .all(|declaration| declaration.binding != ParameterBinding::Input),
            "{shadowed}"
        );
    }
}

#[test]
fn python_duplicate_constants_keep_one_schema_row_but_rewrite_each_binding() {
    let source = "CITY = 'first'\nOTHER = 1\nCITY = 'last'\n";
    let declarations = detect_candidates("python", source);
    assert_eq!(
        declarations
            .iter()
            .map(|declaration| declaration.name.as_str())
            .collect::<Vec<_>>(),
        ["CITY", "OTHER"]
    );
    assert_eq!(
        declarations[0].default,
        Some(ParameterValue::String("last".to_owned()))
    );
    let rewritten = inject_values(
        "python",
        source,
        &declarations,
        &BTreeMap::from([("CITY".to_owned(), "Paris".to_owned())]),
    )
    .unwrap();
    assert_eq!(rewritten, "CITY = 'Paris'\nOTHER = 1\nCITY = 'Paris'\n");
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
fn prompt_identifiers_use_unicode_xid_while_command_identifiers_stay_ascii() {
    let decomposed = "e\u{301}";
    let prompt = placeholder_params(
        "prompt",
        &format!("{{{{任务}}}} {{{{café}}}} {{{{{decomposed}}}}} {{{{9bad}}}} {{{{💥}}}}"),
    );
    assert_eq!(
        prompt
            .iter()
            .map(|field| field.name.as_str())
            .collect::<Vec<_>>(),
        ["任务", "café", decomposed]
    );
    assert!(placeholder_params("command", "{任务} {café} {e\u{301}}").is_empty());

    let rendered = render_prompt_body(
        &format!("任务={{{{任务}}}} name={{{{{decomposed}}}}}"),
        &BTreeMap::from([
            ("任务".to_owned(), "完成".to_owned()),
            (decomposed.to_owned(), "accent".to_owned()),
        ]),
        true,
    );
    assert_eq!(rendered, "任务=完成 name=accent");
}

#[test]
fn injection_rewrites_only_selected_python_shell_and_javascript_bindings() {
    let mut python_const = ParamDecl::new("WIDTH");
    python_const.binding = ParameterBinding::Const;
    python_const.delivery = ParameterDelivery::Inject;
    python_const.parameter_type = ParameterType::Int;
    let mut python_input = ParamDecl::new("input-1");
    python_input.binding = ParameterBinding::Input;
    python_input.delivery = ParameterDelivery::Inject;
    python_input.order = 0;
    let python = "WIDTH = 800\nname = input(\"Name: \")\nprint(WIDTH, name)\n";
    let rewritten = inject_values(
        "python",
        python,
        &[python_const, python_input],
        &BTreeMap::from([
            ("WIDTH".to_owned(), "1024".to_owned()),
            ("input-1".to_owned(), "Ada".to_owned()),
        ]),
    )
    .unwrap();
    assert!(rewritten.contains("WIDTH = 1024"));
    assert!(rewritten.contains("name = _skit_i[0](\"Name: \")"));
    assert!(rewritten.contains("'Ada'"));
    assert!(rewritten.contains("# skit:shim"));
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
    js_const.parameter_type = ParameterType::Int;
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
        ["react", "@scope/pkg", "chalk"]
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
        "#!/bin/sh\r\nNAME=\"${NAME:-world}\"\r\nOTHER=value\r\necho \"$NAME\"\r\n"
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
