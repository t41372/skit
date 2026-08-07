use std::collections::{BTreeMap, BTreeSet};

use serde_json::Value;

use super::{ParamDecl, ParameterDelivery, synthesized_placeholder};

/// Decode declared metadata rows, dropping nameless rows while preserving order and duplicates.
#[must_use]
pub fn declared_from_meta(
    parameters: Option<&[BTreeMap<String, Value>]>,
) -> Vec<ParamDecl> {
    parameters
        .unwrap_or_default()
        .iter()
        .map(ParamDecl::from_meta_map)
        .filter(|declaration| !declaration.name.is_empty())
        .collect()
}

/// Merge declared metadata with command-template placeholders.
///
/// Placeholder order, case, and multiplicity come from the template. A same-name declaration is
/// used only when it has placeholder delivery; otherwise the historical implicit schema is
/// synthesized. Environment riders follow the placeholder fields. Duplicate declared names match
/// Python dict semantics: the last schema wins without moving the first insertion position.
#[must_use]
pub fn declared_for_template(
    parameters: Option<&[BTreeMap<String, Value>]>,
    placeholders: &[String],
) -> Vec<ParamDecl> {
    let mut unique = Vec::<ParamDecl>::new();
    let mut indices = BTreeMap::<String, usize>::new();
    for declaration in declared_from_meta(parameters) {
        if let Some(index) = indices.get(&declaration.name).copied() {
            unique[index] = declaration;
        } else {
            indices.insert(declaration.name.clone(), unique.len());
            unique.push(declaration);
        }
    }

    let placeholder_names = placeholders.iter().cloned().collect::<BTreeSet<_>>();
    let mut output = Vec::with_capacity(placeholders.len() + unique.len());
    for name in placeholders {
        let declaration = indices
            .get(name)
            .and_then(|index| unique.get(*index))
            .filter(|declaration| declaration.delivery == ParameterDelivery::Placeholder)
            .cloned()
            .unwrap_or_else(|| synthesized_placeholder(name));
        output.push(declaration);
    }

    output.extend(unique.into_iter().filter(|declaration| {
        declaration.delivery == ParameterDelivery::Env
            && !placeholder_names.contains(&declaration.name)
    }));
    output
}
