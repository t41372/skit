use skit_core::{normalize_python_dependency, normalize_requires_python};

#[test]
fn valid_pep508_requirements_are_trimmed_but_otherwise_preserved()
-> Result<(), Box<dyn std::error::Error>> {
    assert_eq!(
        normalize_python_dependency("  requests[security]>=2.32; python_version >= '3.10'  ")?,
        Some("requests[security]>=2.32; python_version >= '3.10'".to_owned())
    );
    assert_eq!(
        normalize_python_dependency("demo @ https://example.com/demo.whl")?,
        Some("demo @ https://example.com/demo.whl".to_owned())
    );
    Ok(())
}

#[test]
fn empty_dependency_flags_are_dropped() -> Result<(), Box<dyn std::error::Error>> {
    assert_eq!(normalize_python_dependency("")?, None);
    assert_eq!(normalize_python_dependency("   \t")?, None);
    Ok(())
}

#[test]
fn malformed_requirement_is_rejected_before_storage() {
    let error = normalize_python_dependency("requests => 2").err();
    assert!(error.is_some());
}

#[test]
fn valid_requires_python_is_trimmed_and_automatic_tokens_normalize_empty()
-> Result<(), Box<dyn std::error::Error>> {
    assert_eq!(normalize_requires_python("  >=3.12,<3.13  ")?, ">=3.12,<3.13");
    assert_eq!(normalize_requires_python("")?, "");
    assert_eq!(normalize_requires_python("-")?, "");
    assert_eq!(normalize_requires_python("NONE")?, "");
    Ok(())
}

#[test]
fn bare_python_version_is_not_a_requires_python_specifier_set() {
    let error = normalize_requires_python("3.12").err();
    assert!(error.is_some());
}
