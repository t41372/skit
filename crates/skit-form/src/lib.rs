//! Compose one parameter schema for all skit frontends.

#![forbid(unsafe_code)]

use std::collections::{BTreeMap, BTreeSet};

use skit_domain::{
    EntrySettings,
    parameters::{ParamDecl, ParameterDelivery, synthesized_placeholder},
};
use skit_language::{cli_params, managed_params};

/// Build the fields that one entry exposes to all frontends.
///
/// Managed source fields take priority over static CLI fields.
/// Metadata flag and environment fields can extend either source form.
/// Command and prompt placeholders control their own order.
#[must_use]
pub fn form_params(kind: &str, text: &str, settings: &EntrySettings) -> Vec<ParamDecl> {
    if matches!(kind, "command" | "prompt") {
        return template_params(kind, text, &settings.parameters);
    }

    let managed = managed_params(kind, text);
    if !managed.is_empty() {
        return with_riders(managed, &settings.parameters);
    }

    let reflected = cli_params(kind, text);
    if !reflected.is_empty() {
        return with_riders(reflected, &settings.parameters);
    }

    settings
        .parameters
        .iter()
        .filter(|item| matches!(item.delivery, ParameterDelivery::Flag | ParameterDelivery::Env))
        .cloned()
        .collect()
}

fn with_riders(mut fields: Vec<ParamDecl>, declared: &[ParamDecl]) -> Vec<ParamDecl> {
    let mut taken = fields
        .iter()
        .map(|item| item.name.as_str())
        .collect::<BTreeSet<_>>();
    for item in declared {
        if matches!(item.delivery, ParameterDelivery::Flag | ParameterDelivery::Env)
            && taken.insert(item.name.as_str())
        {
            fields.push(item.clone());
        }
    }
    fields
}

fn template_params(kind: &str, text: &str, declared: &[ParamDecl]) -> Vec<ParamDecl> {
    let placeholders = placeholder_names(kind, text);
    let mut unique = Vec::<ParamDecl>::new();
    let mut indices = BTreeMap::<String, usize>::new();
    for item in declared {
        if let Some(index) = indices.get(&item.name).copied() {
            unique[index] = item.clone();
        } else {
            indices.insert(item.name.clone(), unique.len());
            unique.push(item.clone());
        }
    }

    let placeholder_set = placeholders.iter().cloned().collect::<BTreeSet<_>>();
    let mut output = Vec::new();
    for name in placeholders {
        let item = indices
            .get(&name)
            .and_then(|index| unique.get(*index))
            .filter(|item| item.delivery == ParameterDelivery::Placeholder)
            .cloned()
            .unwrap_or_else(|| synthesized_placeholder(&name));
        output.push(item);
    }
    output.extend(unique.into_iter().filter(|item| {
        item.delivery == ParameterDelivery::Env && !placeholder_set.contains(&item.name)
    }));
    output
}

fn placeholder_names(kind: &str, text: &str) -> Vec<String> {
    let doubled = kind == "prompt";
    let bytes = text.as_bytes();
    let mut output = Vec::new();
    let mut seen = BTreeSet::new();
    let mut index = 0;
    while index < bytes.len() {
        let open: &[u8] = if doubled { b"{{" } else { b"{" };
        let close: &[u8] = if doubled { b"}}" } else { b"}" };
        if !bytes[index..].starts_with(open) {
            index += 1;
            continue;
        }
        if !doubled && bytes[index..].starts_with(b"{{") {
            index += 2;
            continue;
        }
        let start = index + open.len();
        let Some(relative_end) = bytes[start..]
            .windows(close.len())
            .position(|window| window == close)
        else {
            break;
        };
        let end = start + relative_end;
        let name = &text[start..end];
        if valid_identifier(name) && seen.insert(name.to_owned()) {
            output.push(name.to_owned());
        }
        index = end + close.len();
    }
    output
}

fn valid_identifier(value: &str) -> bool {
    let mut chars = value.chars();
    chars
        .next()
        .is_some_and(|first| first.is_ascii_alphabetic() || first == '_')
        && chars.all(|character| character.is_ascii_alphanumeric() || character == '_')
}
