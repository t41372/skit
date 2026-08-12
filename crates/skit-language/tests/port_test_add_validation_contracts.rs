//! Public language-validator ports from Python `tests/test_add_validation_contracts.py` at
//! `main@206f9ef`.

use skit_i18n::{Locale, Localize as _};
use skit_language::{validate_pep440_specifiers, validate_pep508_requirement};

#[test]
fn test_requires_python_error_is_none_for_valid_constraints() {
    assert!(validate_pep440_specifiers(">=3.11").is_ok());
    assert!(validate_pep440_specifiers(">=3.12,<3.13").is_ok());
}

#[test]
fn test_requires_python_error_localizes_a_message_for_an_invalid_constraint() {
    let error = validate_pep440_specifiers("not-a-version").unwrap_err();
    let message = error.message().localize(Locale::En);
    assert!(
        message.starts_with("not-a-version isn't a Python version constraint"),
        "{message}"
    );
}

#[test]
fn test_requires_python_error_rejects_a_bare_version_without_operator() {
    assert!(validate_pep440_specifiers("3.11").is_err());
}

#[test]
fn test_requirement_error_is_none_for_valid_requirements() {
    for value in ["requests", "rich>=13,<16", "demo[bold]"] {
        assert!(
            validate_pep508_requirement(value).is_ok(),
            "valid PEP 508 requirement was refused: {value}"
        );
    }
}

#[test]
fn test_requirement_error_localizes_a_message_for_an_invalid_requirement() {
    let error = validate_pep508_requirement("@@@").unwrap_err();
    let message = error.message().localize(Locale::En);
    assert!(
        message.starts_with("@@@ isn't a package requirement"),
        "{message}"
    );
}
