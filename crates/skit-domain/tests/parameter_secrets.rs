use skit_domain::parameters::{
    ParameterBinding, ParameterDelivery, ParameterType, is_secret_name, synthesized_placeholder,
};

fn assert_secret(values: &[&str]) {
    for value in values {
        assert!(is_secret_name(value), "{value:?} should be secret-looking");
    }
}

fn assert_public(values: &[&str]) {
    for value in values {
        assert!(
            !is_secret_name(value),
            "{value:?} should not be secret-looking"
        );
    }
}

#[test]
fn password_and_secret_suffixes_survive_plural_camel_and_jammed_shapes() {
    assert_secret(&[
        "DB_PASSWORD",
        "PASSWORDS",
        "PASSWORD_HINT_TEXT",
        "MYSECRET",
        "clientSecretValue",
        "dbPasswd",
        "awsSecretKey",
    ]);
    assert_public(&["secretary", "passage", "username"]);
}

#[test]
fn key_detection_uses_exact_words_or_the_frozen_jammed_prefix_allowlist() {
    assert_secret(&[
        "api_key",
        "apiKeys",
        "APIkey",
        "stripeKey",
        "publicKey",
        "base64key",
        "sort_key",
        "authkey",
        "accesskey",
        "privatekey",
        "passkey",
        "sshkey",
        "gpgkey",
        "awskey",
        "masterkey",
        "signingkey",
        "licensekey",
        "encryptionkey",
    ]);
    assert_public(&[
        "MONKEY",
        "TURKEY",
        "HOTKEY",
        "WHISKEY",
        "publickey",
        "hostkey",
        "stripekey",
        "gcpkey",
        "azurekey",
        "basekey",
        "decryptionkey",
    ]);
}

#[test]
fn token_names_distinguish_credentials_from_the_exact_count_lexicon() {
    assert_secret(&[
        "github_tokens",
        "session_token",
        "N8N_TOKEN",
        "N8NToken",
        "GITHUB_TOKEN_2",
        "STEP_2_TOKEN",
        "available_tokens",
        "rate_tokens",
    ]);
    assert_public(&[
        "max_tokens",
        "token_limit",
        "n_tokens",
        "maxOutputTokens",
        "nTokens",
        "max64Tokens",
        "2_tokens",
        "tokens",
        "many_tokens",
        "number_tokens",
        "tokens_per",
    ]);
}

#[test]
fn sentence_prompts_only_suppress_an_immediately_count_qualified_token_mention() {
    assert_public(&["How many tokens?", "rate limit 60 tokens/min", "2 tokens"]);
    assert_secret(&[
        "rate tokens",
        "available tokens",
        "Paste your GitHub token (rate limit 60 tokens/min):",
        "step 2 token",
        "Enter session token",
    ]);
}

#[test]
fn empty_and_unrelated_text_never_become_secret_by_substrings_alone() {
    assert_public(&["", "   ", "authentication_mode", "tokenizer", "keynote"]);
}

#[test]
fn synthesized_placeholders_apply_the_same_heuristic_and_frozen_defaults() {
    let secret = synthesized_placeholder("API_TOKEN");
    assert_eq!(secret.name, "API_TOKEN");
    assert_eq!(secret.binding, ParameterBinding::None);
    assert_eq!(secret.delivery, ParameterDelivery::Placeholder);
    assert_eq!(secret.parameter_type, ParameterType::Str);
    assert!(secret.required);
    assert!(secret.secret);
    assert!(secret.env_source.is_empty());

    let public = synthesized_placeholder("host");
    assert!(public.required);
    assert!(!public.secret);
}
