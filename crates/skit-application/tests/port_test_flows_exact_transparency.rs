//! Exact transparency ports from Python v0.4 `tests/test_flows.py`.

use std::collections::BTreeMap;

use skit_application::delivery::{PreparedValue, assemble, transparency_messages};
use skit_domain::parameters::{ParamDecl, ParameterDelivery};
use skit_i18n::Locale;

fn scalar(values: &[(&str, &str)]) -> BTreeMap<String, PreparedValue> {
    values
        .iter()
        .map(|(name, value)| ((*name).to_owned(), PreparedValue::Scalar((*value).to_owned())))
        .collect()
}

#[test]
fn test_transparency_lines_inject_source_shows_masked_and_temp_note() {
    let mut output = ParamDecl::new("OUTPUT");
    output.delivery = ParameterDelivery::Inject;
    let mut width = ParamDecl::new("WIDTH");
    width.delivery = ParameterDelivery::Inject;
    let mut key = ParamDecl::new("API_KEY");
    key.delivery = ParameterDelivery::Inject;
    key.secret = true;

    let assembly = assemble(
        &[output, width, key],
        &scalar(&[("OUTPUT", "new.jpg"), ("WIDTH", "900"), ("API_KEY", "sekret")]),
        &[],
    )
    .unwrap();
    let lines = transparency_messages(&assembly, "python /data/script.py")
        .into_iter()
        .map(|message| message.localize(Locale::En))
        .collect::<Vec<_>>();
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
    let lines = transparency_messages(&assembly, "python script.py --output o.png")
        .into_iter()
        .map(|message| message.localize(Locale::En))
        .collect::<Vec<_>>();

    assert_eq!(lines.len(), 1);
    assert!(lines[0].starts_with("→ "), "{:?}", lines);
}
