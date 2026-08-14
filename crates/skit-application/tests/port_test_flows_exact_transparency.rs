//! Exact transparency ports from Python v0.4 `tests/test_flows.py`.

use std::collections::BTreeMap;

use skit_application::delivery::{Assembly, PreparedValue, assemble, transparency_messages};
use skit_domain::parameters::{ParamDecl, ParameterDelivery};
use skit_i18n::Locale;

fn scalar(values: &[(&str, &str)]) -> BTreeMap<String, PreparedValue> {
    values
        .iter()
        .map(|(name, value)| ((*name).to_owned(), PreparedValue::Scalar((*value).to_owned())))
        .collect()
}

fn localized_lines(assembly: &Assembly, command: &str) -> Vec<String> {
    transparency_messages(assembly, command)
        .into_iter()
        .map(|message| message.localize(Locale::En))
        .collect()
}

fn inject_assembly(secret: &str) -> Assembly {
    let mut output = ParamDecl::new("OUTPUT");
    output.delivery = ParameterDelivery::Inject;
    let mut width = ParamDecl::new("WIDTH");
    width.delivery = ParameterDelivery::Inject;
    let mut key = ParamDecl::new("API_KEY");
    key.delivery = ParameterDelivery::Inject;
    key.secret = true;
    assemble(
        &[output, width, key],
        &scalar(&[("OUTPUT", "new.jpg"), ("WIDTH", "900"), ("API_KEY", secret)]),
        &[],
    )
    .unwrap()
}

#[test]
fn test_transparency_lines_inject_source_shows_masked_and_temp_note() {
    let assembly = inject_assembly("sekret");
    let lines = localized_lines(&assembly, "python /data/script.py");
    let joined = lines.join("\n");

    assert!(joined.contains("→ inject:"), "{joined}");
    assert!(joined.contains("OUTPUT = new.jpg"), "{joined}");
    assert!(joined.contains("temporary copy"), "{joined}");
    assert!(!joined.contains("sekret"), "secret leaked into transparency: {joined}");
    assert!(joined.contains("•••"), "{joined}");
}

#[test]
fn test_assemble_display_lists_only_inject_delivered_values() {
    let mut out = ParamDecl::new("OUT");
    out.delivery = ParameterDelivery::Inject;
    let mut city = ParamDecl::new("CITY");
    city.delivery = ParameterDelivery::Env;
    let mut name = ParamDecl::new("name");
    name.delivery = ParameterDelivery::Flag;
    name.flag = "--name".to_owned();

    let assembly = assemble(
        &[out, city, name],
        &scalar(&[("OUT", "out.jpg"), ("CITY", "Taipei"), ("name", "ada")]),
        &[],
    )
    .unwrap();

    assert_eq!(assembly.display, [("OUT".to_owned(), "out.jpg".to_owned())]);
    assert_eq!(assembly.masked_env, BTreeMap::from([("CITY".to_owned(), "Taipei".to_owned())]));
    assert!(assembly.args.iter().any(|arg| arg == "--name"));
    assert!(assembly.args.iter().any(|arg| arg == "ada"));
}

#[test]
fn test_transparency_lines_flag_source_is_single_command_line() {
    let mut output = ParamDecl::new("output");
    output.delivery = ParameterDelivery::Flag;
    output.flag = "--output".to_owned();
    let assembly = assemble(&[output], &scalar(&[("output", "o.png")]), &[]).unwrap();
    let lines = localized_lines(&assembly, "python script.py --output o.png");

    assert_eq!(lines.len(), 1);
    assert!(lines[0].starts_with("→ "), "{:?}", lines);
}

#[test]
fn test_transparency_inject_lines_are_exact() {
    let assembly = inject_assembly("s");
    let lines = localized_lines(&assembly, "python /tmp/.injected-abc.py");
    assert_eq!(
        lines[0],
        "→ inject: OUTPUT = new.jpg, WIDTH = 900, API_KEY = •••"
    );
    assert!(
        lines[1].starts_with("  (written to a temporary copy"),
        "{:?}",
        lines
    );
}

#[test]
fn test_transparency_shows_the_injected_temp_path() {
    let assembly = inject_assembly("s");
    let lines = localized_lines(&assembly, "python /tmp/.injected-abc.py");
    assert!(
        lines.last().is_some_and(|line| line.contains(".injected-abc.py")),
        "the actual staged path disappeared from transparency: {:?}",
        lines
    );
}

#[test]
fn test_transparency_flag_source_masks_secret_in_command() {
    let mut api_key = ParamDecl::new("api_key");
    api_key.delivery = ParameterDelivery::Flag;
    api_key.flag = "--api-key".to_owned();
    api_key.secret = true;
    let mut name = ParamDecl::new("name");
    name.delivery = ParameterDelivery::Flag;
    name.flag = "--name".to_owned();
    let assembly = assemble(
        &[api_key, name],
        &scalar(&[("api_key", "sk-SECRET"), ("name", "ada")]),
        &[],
    )
    .unwrap();
    assert_eq!(
        assembly.masked_args,
        ["--api-key", "•••", "--name", "ada"]
    );
    let command = format!("python script.py {}", assembly.masked_args.join(" "));
    let lines = localized_lines(&assembly, &command);
    let line = lines.last().expect("flag transparency has a command line");
    assert!(!line.contains("sk-SECRET"), "secret leaked into transparency: {line}");
    assert!(line.contains("•••"), "masked value disappeared: {line}");
    assert!(line.contains("--name ada"), "non-secret flags disappeared: {line}");
}
