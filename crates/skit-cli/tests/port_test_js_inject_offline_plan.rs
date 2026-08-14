#![cfg(unix)]
use std::fs;
use skit_application::EntryRepository as _;
use skit_domain::EntrySettings;
use skit_form::{FormSource, form_plan};
#[path = "support/js_inject.rs"] mod support;
use support::Sandbox;

#[test]
fn test_execute_runs_a_js_entry_offline_plan() {
    let sandbox = Sandbox::new();
    sandbox.create_managed_entry(
        "offline", "js", "offline.js", "const WIDTH = 800;\nconsole.log(WIDTH);\n", "node",
    );
    let entry = sandbox.store().resolve("offline").unwrap();
    let source = fs::read_to_string(sandbox.payload_path("offline")).unwrap();
    let plan = form_plan("js", &source, &EntrySettings::from_meta(&entry.meta));
    assert_eq!(plan.source, FormSource::Inject);
    assert_eq!(
        plan.fields.iter().map(|field| field.declaration.name.as_str()).collect::<Vec<_>>(),
        ["WIDTH"]
    );
}
