use std::collections::{BTreeMap, BTreeSet};

use crate::{Binding, Delivery, ParamDecl, ParamDefault, ParamType};

/// One pure declared-schema edit request. Persistence and UI wording stay outside this
/// model; warnings are stable symbolic `code:name` tokens.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeclaredEdits {
    pub add: Vec<String>,
    pub remove: Vec<String>,
    pub types: BTreeMap<String, ParamType>,
    pub defaults: BTreeMap<String, String>,
    pub choices: BTreeMap<String, Vec<String>>,
    pub deliveries: BTreeMap<String, Delivery>,
    pub flags: BTreeMap<String, String>,
    pub required: BTreeSet<String>,
    pub optional: BTreeSet<String>,
    pub help: BTreeMap<String, String>,
    pub prompts: BTreeMap<String, String>,
    pub secret: BTreeSet<String>,
    pub no_secret: BTreeSet<String>,
    pub env_sources: BTreeMap<String, String>,
    pub allowed_deliveries: Vec<Delivery>,
    pub placeholder_names: BTreeSet<String>,
}

impl Default for DeclaredEdits {
    fn default() -> Self {
        Self {
            add: Vec::new(),
            remove: Vec::new(),
            types: BTreeMap::new(),
            defaults: BTreeMap::new(),
            choices: BTreeMap::new(),
            deliveries: BTreeMap::new(),
            flags: BTreeMap::new(),
            required: BTreeSet::new(),
            optional: BTreeSet::new(),
            help: BTreeMap::new(),
            prompts: BTreeMap::new(),
            secret: BTreeSet::new(),
            no_secret: BTreeSet::new(),
            env_sources: BTreeMap::new(),
            allowed_deliveries: vec![Delivery::Flag, Delivery::Env],
            placeholder_names: BTreeSet::new(),
        }
    }
}

/// Result of one pure edit operation.
#[derive(Debug, Clone, PartialEq)]
pub struct DeclaredEditResult {
    pub decls: Vec<ParamDecl>,
    pub warnings: Vec<String>,
}

/// Apply declared-schema edits without mutating the caller's declarations.
#[must_use]
pub fn edit_declared(initial: &[ParamDecl], edits: &DeclaredEdits) -> DeclaredEditResult {
    let mut by_name = initial
        .iter()
        .map(|decl| (decl.name.clone(), decl.clone()))
        .collect::<BTreeMap<_, _>>();
    let mut order = initial
        .iter()
        .map(|decl| decl.name.clone())
        .collect::<Vec<_>>();
    let mut warnings = Vec::new();

    for name in &edits.remove {
        if by_name.remove(name).is_some() {
            order.retain(|existing| existing != name);
        } else {
            warnings.push(format!("not-declared:{name}"));
        }
    }

    for name in &edits.add {
        if by_name.contains_key(name) {
            warnings.push(format!("already-declared:{name}"));
            continue;
        }
        let decl = if edits.placeholder_names.contains(name) {
            ParamDecl {
                name: name.clone(),
                delivery: Delivery::Placeholder,
                required: true,
                ..ParamDecl::default()
            }
        } else {
            ParamDecl {
                name: name.clone(),
                delivery: edits
                    .allowed_deliveries
                    .first()
                    .copied()
                    .unwrap_or(Delivery::Flag),
                ..ParamDecl::default()
            }
        };
        by_name.insert(name.clone(), decl);
        order.push(name.clone());
    }

    let tweak_names = tweak_names(edits);
    for name in tweak_names {
        let Some(current) = by_name.get(&name).cloned() else {
            warnings.push(format!("not-declared:{name}"));
            continue;
        };
        let mut decl = current.clone();

        if let Some(delivery) = edits.deliveries.get(&name).copied() {
            if edits.allowed_deliveries.contains(&delivery) {
                decl.delivery = delivery;
            } else {
                warnings.push(format!("invalid-delivery:{name}"));
            }
        }
        if let Some(param_type) = edits.types.get(&name).copied() {
            decl.param_type = param_type;
        }
        if let Some(choices) = edits.choices.get(&name) {
            decl.choices = choices.clone();
        }
        if let Some(value) = edits.defaults.get(&name) {
            if let Some(default) = coerce_default(value, decl.param_type) {
                decl.default = Some(default);
            } else {
                warnings.push(format!("invalid-default:{name}"));
            }
        }
        if let Some(flag) = edits.flags.get(&name) {
            decl.flag = flag.clone();
        }
        if edits.required.contains(&name) {
            decl.required = true;
        }
        if edits.optional.contains(&name) {
            decl.required = false;
        }
        if let Some(help) = edits.help.get(&name) {
            decl.help = help.clone();
        }
        if let Some(prompt) = edits.prompts.get(&name) {
            decl.prompt = prompt.clone();
        }
        if edits.secret.contains(&name) {
            decl.secret = true;
        }
        if edits.no_secret.contains(&name) {
            decl.secret = false;
            decl.env_source.clear();
        }
        if let Some(env_source) = edits.env_sources.get(&name) {
            if decl.secret {
                decl.env_source = env_source.clone();
            } else {
                warnings.push(format!("env-source-not-secret:{name}"));
            }
        }

        normalize_binding_delivery(&mut decl);
        if let Some(code) = apply_bool_flag_action(&mut decl) {
            warnings.push(format!("{code}:{name}"));
            by_name.insert(name, current);
            continue;
        }
        if decl.param_type == ParamType::Choice && decl.choices.is_empty() {
            warnings.push(format!("choice-without-choices:{name}"));
            by_name.insert(name, current);
            continue;
        }
        by_name.insert(name, decl);
    }

    DeclaredEditResult {
        decls: order
            .into_iter()
            .filter_map(|name| by_name.remove(&name))
            .collect(),
        warnings,
    }
}

fn tweak_names(edits: &DeclaredEdits) -> BTreeSet<String> {
    let mut names = BTreeSet::new();
    names.extend(edits.deliveries.keys().cloned());
    names.extend(edits.types.keys().cloned());
    names.extend(edits.choices.keys().cloned());
    names.extend(edits.defaults.keys().cloned());
    names.extend(edits.flags.keys().cloned());
    names.extend(edits.help.keys().cloned());
    names.extend(edits.prompts.keys().cloned());
    names.extend(edits.env_sources.keys().cloned());
    names.extend(edits.required.iter().cloned());
    names.extend(edits.optional.iter().cloned());
    names.extend(edits.secret.iter().cloned());
    names.extend(edits.no_secret.iter().cloned());
    names
}

fn normalize_binding_delivery(decl: &mut ParamDecl) {
    decl.delivery = match decl.binding {
        Binding::Const | Binding::Input => Delivery::Inject,
        Binding::EnvDefault => Delivery::Env,
        Binding::None => decl.delivery,
    };
}

fn apply_bool_flag_action(decl: &mut ParamDecl) -> Option<&'static str> {
    if decl.param_type == ParamType::Boolean
        && decl.delivery == Delivery::Flag
        && !decl.flag.is_empty()
        && decl.action.is_empty()
    {
        if default_truthy(decl.default.as_ref()) {
            return Some("bool-flag-on-by-default");
        }
        decl.action = "store_true".to_owned();
    }
    if decl.param_type != ParamType::Boolean {
        decl.action.clear();
    }
    None
}

fn coerce_default(value: &str, param_type: ParamType) -> Option<ParamDefault> {
    match param_type {
        ParamType::Integer => value.parse::<i64>().ok().map(ParamDefault::Integer),
        ParamType::Float => value.parse::<f64>().ok().and_then(|number| {
            number
                .is_finite()
                .then_some(ParamDefault::Float(number))
        }),
        ParamType::Boolean => parse_bool(value).map(ParamDefault::Boolean),
        ParamType::String | ParamType::Choice | ParamType::Path => {
            Some(ParamDefault::String(value.to_owned()))
        }
    }
}

fn parse_bool(value: &str) -> Option<bool> {
    match value.trim().to_ascii_lowercase().as_str() {
        "true" | "1" | "yes" | "y" | "on" => Some(true),
        "false" | "0" | "no" | "n" | "off" => Some(false),
        _ => None,
    }
}

fn default_truthy(default: Option<&ParamDefault>) -> bool {
    match default {
        Some(ParamDefault::Boolean(value)) => *value,
        Some(ParamDefault::Integer(value)) => *value != 0,
        Some(ParamDefault::Float(value)) => *value != 0.0,
        Some(ParamDefault::String(value)) => parse_bool(value).unwrap_or(false),
        None => false,
    }
}
