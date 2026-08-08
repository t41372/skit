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
/// Command and prompt entries use the managed names stored in `params`.
#[must_use]
pub fn form_params(kind: &str, text: &str, settings: &EntrySettings) -> Vec<ParamDecl> {
    if kind == "prompt" && !settings.interpolate {
        return Vec::new();
    }
    if matches!(kind, "command" | "prompt") {
        return template_params(&settings.params, &settings.parameters);
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
        .filter(|item| {
            matches!(
                item.delivery,
                ParameterDelivery::Flag | ParameterDelivery::Env
            )
        })
        .cloned()
        .collect()
}

fn with_riders(mut fields: Vec<ParamDecl>, declared: &[ParamDecl]) -> Vec<ParamDecl> {
    let mut taken = fields
        .iter()
        .map(|item| item.name.as_str())
        .collect::<BTreeSet<_>>();
    for item in declared {
        if matches!(
            item.delivery,
            ParameterDelivery::Flag | ParameterDelivery::Env
        ) && taken.insert(item.name.as_str())
        {
            fields.push(item.clone());
        }
    }
    fields
}

fn template_params(managed: &[String], declared: &[ParamDecl]) -> Vec<ParamDecl> {
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

    let managed_set = managed.iter().cloned().collect::<BTreeSet<_>>();
    let mut output = Vec::new();
    for name in managed {
        let item = indices
            .get(name)
            .and_then(|index| unique.get(*index))
            .filter(|item| item.delivery == ParameterDelivery::Placeholder)
            .cloned()
            .unwrap_or_else(|| synthesized_placeholder(name));
        output.push(item);
    }
    output.extend(unique.into_iter().filter(|item| {
        item.delivery == ParameterDelivery::Env && !managed_set.contains(&item.name)
    }));
    output
}
