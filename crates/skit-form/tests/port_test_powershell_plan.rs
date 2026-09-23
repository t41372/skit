//! Form-plan ports from Python `tests/test_powershell.py` at `main@206f9ef`.

use skit_domain::EntrySettings;
use skit_form::{FormSource, form_plan};

#[test]
fn test_plan_reads_powershell_param_block() {
    let plan = form_plan(
        "powershell",
        "param([string]$City = 'Taipei')\nWrite-Host $City\n",
        &EntrySettings::default(),
    );
    assert_eq!(plan.source, FormSource::Reader);
    assert_eq!(plan.source.as_str(), "argparse");
    assert_eq!(
        plan.fields
            .iter()
            .map(|field| field.declaration.name.as_str())
            .collect::<Vec<_>>(),
        ["City"]
    );
    assert_eq!(plan.fields[0].declaration.flag, "-City");
}

#[test]
fn test_plan_none_when_reader_finds_no_surface() {
    let plan = form_plan("powershell", "Write-Host hi\n", &EntrySettings::default());
    assert_eq!(plan.source, FormSource::None);
    assert_eq!(plan.source.as_str(), "none");
    assert!(plan.fields.is_empty());
}
