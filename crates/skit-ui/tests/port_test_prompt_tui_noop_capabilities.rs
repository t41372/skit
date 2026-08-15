use std::collections::BTreeMap;

use skit_domain::parameters::ParamDecl;
use skit_ui::{RUNNER_KEY, RunFormView, SettingsInputs, SettingsView};

#[test]
fn test_form_ctrl_n_is_a_noop_without_a_picker() {
    let form = RunFormView::from_declarations(
        "plaincmd",
        "plaincmd",
        &[ParamDecl::new("x")],
        &BTreeMap::new(),
        &[],
        "",
        &BTreeMap::new(),
        "",
    );
    assert!(form.has_parameters());
    assert!(!form.has_runner_picker());
    assert!(
        form.fields().iter().all(|field| !field.capabilities.new_runner),
        "a non-prompt form advertises the New agent action despite having no runner picker"
    );
}

#[test]
fn test_settings_ctrl_n_is_a_noop_on_non_prompt_entries() {
    let view = SettingsView::from_inputs(&SettingsInputs {
        selector: "plainpy".to_owned(),
        kind: "python".to_owned(),
        name: "plainpy".to_owned(),
        source: "/work/plainpy.py".to_owned(),
        workdir: "invoke".to_owned(),
        has_original_file: true,
        has_stored_name: true,
        supports_modes: true,
        declared_schema: true,
        ..SettingsInputs::default()
    });
    assert!(view.field(RUNNER_KEY).is_none(), "non-prompt settings unexpectedly created a runner picker");
    assert!(
        view.fields().all(|field| !field.capabilities.new_runner),
        "non-prompt settings advertise New agent despite having no runner lane"
    );
}
