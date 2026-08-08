use std::collections::{BTreeMap, BTreeSet};

/// Bind stored ordered/prompted reads to their current source call sites.
///
/// Literal prompt text wins over bare position so inserting or deleting an earlier
/// call cannot silently move a value onto a different question. Position is only a
/// fallback; when a stored prompt existed, that fallback is marked ambiguous. Exact
/// prompt claims are resolved first and are strictly one-to-one, including duplicate
/// prompt multisets.
#[must_use]
pub fn match_calls(
    stored: &[(i64, String)],
    current: &[(i64, String)],
) -> BTreeMap<i64, (i64, bool)> {
    let current_by_order = current.iter().cloned().collect::<BTreeMap<_, _>>();
    let mut by_prompt = BTreeMap::<String, Vec<i64>>::new();
    for (order, prompt) in current {
        if !prompt.is_empty() {
            by_prompt.entry(prompt.clone()).or_default().push(*order);
        }
    }

    let mut exact = BTreeMap::<i64, i64>::new();
    let mut claimed = BTreeSet::<i64>::new();
    match_prompt_multisets(stored, &by_prompt, &mut exact, &mut claimed);

    for (order, prompt) in stored {
        if exact.contains_key(order) || prompt.is_empty() {
            continue;
        }
        let Some(candidates) = by_prompt.get(prompt) else {
            continue;
        };
        if candidates.len() == 1 && !claimed.contains(&candidates[0]) {
            exact.insert(*order, candidates[0]);
            claimed.insert(candidates[0]);
        }
    }

    let mut output = BTreeMap::new();
    for (order, prompt) in stored {
        if let Some(current_order) = exact.get(order) {
            output.insert(*order, (*current_order, false));
            continue;
        }
        if current_by_order.contains_key(order) && !claimed.contains(order) {
            output.insert(*order, (*order, !prompt.is_empty()));
        }
    }
    output
}

fn match_prompt_multisets(
    stored: &[(i64, String)],
    by_prompt: &BTreeMap<String, Vec<i64>>,
    exact: &mut BTreeMap<i64, i64>,
    claimed: &mut BTreeSet<i64>,
) {
    let mut stored_by_prompt = BTreeMap::<String, Vec<i64>>::new();
    for (order, prompt) in stored {
        if !prompt.is_empty() {
            stored_by_prompt
                .entry(prompt.clone())
                .or_default()
                .push(*order);
        }
    }
    for (prompt, stored_orders) in stored_by_prompt {
        let Some(current_orders) = by_prompt.get(&prompt) else {
            continue;
        };
        if stored_orders.len() <= 1 || current_orders.len() != stored_orders.len() {
            continue;
        }
        let mut stored_orders = stored_orders;
        let mut current_orders = current_orders.clone();
        stored_orders.sort_unstable();
        current_orders.sort_unstable();
        for (stored_order, current_order) in stored_orders.into_iter().zip(current_orders) {
            exact.insert(stored_order, current_order);
            claimed.insert(current_order);
        }
    }
}
