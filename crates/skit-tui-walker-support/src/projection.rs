//! Product-independent helpers for deterministic walker projections.

use std::collections::BTreeMap;

use serde_json::Value;

/// Rewrite every value that matches one JSON pointer pattern.
///
/// An empty pattern matches the root. A nonempty pattern uses RFC 6901 escapes.
/// A raw `*` segment visits every array element or object value.
pub fn rewrite_json_pointer_matches(
    value: &mut Value,
    pattern: &str,
    mut rewrite: impl FnMut(&mut Value) -> Result<(), String>,
) -> Result<(), String> {
    let segments = parse_pointer_pattern(pattern)?;
    rewrite_pointer_matches(value, &segments, &mut rewrite)
}

/// Replace registered text tokens in deterministic longest-first order.
///
/// The acceptance function receives the complete prefix and suffix around each
/// candidate. An empty raw token does not match.
#[must_use]
pub fn replace_longest_text_tokens(
    text: &str,
    replacements: &BTreeMap<String, String>,
    accept: impl Fn(&str, &str) -> bool,
) -> String {
    let mut tokens = replacements
        .iter()
        .filter(|(raw, _)| !raw.is_empty())
        .collect::<Vec<_>>();
    tokens.sort_by(|(left, _), (right, _)| {
        right.len().cmp(&left.len()).then_with(|| left.cmp(right))
    });
    tokens
        .into_iter()
        .fold(text.to_owned(), |rendered, (raw, stable)| {
            replace_text_token(&rendered, raw, stable, &accept)
        })
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum PointerSegment {
    Wildcard,
    Literal(String),
}

fn parse_pointer_pattern(pattern: &str) -> Result<Vec<PointerSegment>, String> {
    if pattern.is_empty() {
        return Ok(Vec::new());
    }
    let Some(pointer) = pattern.strip_prefix('/') else {
        return Err(invalid_pointer_pattern(pattern));
    };
    pointer
        .split('/')
        .map(|raw| parse_pointer_segment(pattern, raw))
        .collect()
}

fn parse_pointer_segment(pattern: &str, raw: &str) -> Result<PointerSegment, String> {
    if raw == "*" {
        return Ok(PointerSegment::Wildcard);
    }
    let mut decoded = String::with_capacity(raw.len());
    let mut characters = raw.chars();
    while let Some(character) = characters.next() {
        if character != '~' {
            decoded.push(character);
            continue;
        }
        match characters.next() {
            Some('0') => decoded.push('~'),
            Some('1') => decoded.push('/'),
            _ => return Err(invalid_pointer_pattern(pattern)),
        }
    }
    Ok(PointerSegment::Literal(decoded))
}

fn invalid_pointer_pattern(pattern: &str) -> String {
    format!("invalid JSON pointer pattern: {pattern}")
}

fn rewrite_pointer_matches(
    value: &mut Value,
    segments: &[PointerSegment],
    rewrite: &mut impl FnMut(&mut Value) -> Result<(), String>,
) -> Result<(), String> {
    let Some((segment, remaining)) = segments.split_first() else {
        return rewrite(value);
    };
    match (segment, value) {
        (PointerSegment::Wildcard, Value::Array(values)) => {
            for value in values {
                rewrite_pointer_matches(value, remaining, rewrite)?;
            }
        }
        (PointerSegment::Wildcard, Value::Object(values)) => {
            let mut keys = values.keys().cloned().collect::<Vec<_>>();
            keys.sort_unstable();
            for key in keys {
                let value = values
                    .get_mut(&key)
                    .expect("a key from the object still exists");
                rewrite_pointer_matches(value, remaining, rewrite)?;
            }
        }
        (PointerSegment::Literal(key), Value::Object(values)) => {
            if let Some(value) = values.get_mut(key) {
                rewrite_pointer_matches(value, remaining, rewrite)?;
            }
        }
        (PointerSegment::Literal(index), Value::Array(values)) => {
            if let Some(value) =
                canonical_array_index(index).and_then(|index| values.get_mut(index))
            {
                rewrite_pointer_matches(value, remaining, rewrite)?;
            }
        }
        (PointerSegment::Wildcard | PointerSegment::Literal(_), _) => {}
    }
    Ok(())
}

fn canonical_array_index(segment: &str) -> Option<usize> {
    if segment == "0" {
        return Some(0);
    }
    if segment.is_empty()
        || segment.starts_with('0')
        || !segment.bytes().all(|byte| byte.is_ascii_digit())
    {
        return None;
    }
    segment.parse().ok()
}

fn replace_text_token(
    text: &str,
    raw: &str,
    stable: &str,
    accept: &impl Fn(&str, &str) -> bool,
) -> String {
    let mut output = String::with_capacity(text.len());
    let mut cursor = 0;
    while let Some(relative) = text[cursor..].find(raw) {
        let start = cursor + relative;
        let end = start + raw.len();
        output.push_str(&text[cursor..start]);
        if accept(&text[..start], &text[end..]) {
            output.push_str(stable);
        } else {
            output.push_str(raw);
        }
        cursor = end;
    }
    output.push_str(&text[cursor..]);
    output
}

#[cfg(test)]
mod tests {
    use std::{
        cell::{Cell, RefCell},
        collections::BTreeMap,
    };

    use serde_json::{Value, json};

    use super::{replace_longest_text_tokens, rewrite_json_pointer_matches};

    fn replacements<const N: usize>(pairs: [(&str, &str); N]) -> BTreeMap<String, String> {
        pairs
            .into_iter()
            .map(|(raw, stable)| (raw.to_owned(), stable.to_owned()))
            .collect()
    }

    fn replace_with_marker(value: &mut Value) -> Result<(), String> {
        *value = json!("<rewritten>");
        Ok(())
    }

    fn counting_marker<'a>(
        calls: &'a Cell<usize>,
    ) -> impl FnMut(&mut Value) -> Result<(), String> + 'a {
        move |value| {
            calls.set(calls.get() + 1);
            replace_with_marker(value)
        }
    }

    #[test]
    fn pointer_empty_pattern_rewrites_the_root_once() {
        let mut value = json!({"nested": [1, 2]});
        let calls = Cell::new(0);

        rewrite_json_pointer_matches(&mut value, "", counting_marker(&calls)).unwrap();

        assert_eq!(value, json!("<rewritten>"));
        assert_eq!(calls.get(), 1);
    }

    #[test]
    fn pointer_matches_literal_empty_and_escaped_object_keys() {
        let mut value = json!({
            "plain": {"": 1},
            "a/b": {"~key": 2},
        });

        rewrite_json_pointer_matches(&mut value, "/plain/", replace_with_marker).unwrap();
        rewrite_json_pointer_matches(&mut value, "/a~1b/~0key", replace_with_marker).unwrap();

        assert_eq!(value["plain"][""], "<rewritten>");
        assert_eq!(value["a/b"]["~key"], "<rewritten>");
    }

    #[test]
    fn pointer_refuses_invalid_syntax_before_any_mutation() {
        for pattern in ["missing-slash", "/invalid~", "/invalid~2escape"] {
            let mut value = json!({"invalid~2escape": 1});
            let before = value.clone();
            let calls = Cell::new(0);

            let error = rewrite_json_pointer_matches(&mut value, pattern, counting_marker(&calls))
                .unwrap_err();

            assert_eq!(error, format!("invalid JSON pointer pattern: {pattern}"));
            assert_eq!(value, before);
            assert_eq!(calls.get(), 0);
        }
    }

    #[test]
    fn pointer_treats_missing_and_container_mismatches_as_zero_matches() {
        let mut value = json!({
            "scalar": 1,
            "array": [10],
            "object": {"key": 20},
        });
        let before = value.clone();
        let calls = Cell::new(0);

        for pattern in [
            "/missing",
            "/scalar/child",
            "/array/key",
            "/object/0",
            "/scalar/*",
        ] {
            rewrite_json_pointer_matches(&mut value, pattern, counting_marker(&calls)).unwrap();
        }

        assert_eq!(value, before);
        assert_eq!(calls.get(), 0);
    }

    #[test]
    fn pointer_uses_only_canonical_in_range_array_indices() {
        let mut value = json!({"items": [10, 20, 30]});
        rewrite_json_pointer_matches(&mut value, "/items/0", |matched| {
            *matched = json!(11);
            Ok(())
        })
        .unwrap();
        rewrite_json_pointer_matches(&mut value, "/items/2", |matched| {
            *matched = json!(31);
            Ok(())
        })
        .unwrap();

        let before = value.clone();
        let calls = Cell::new(0);
        for pattern in [
            "/items/00",
            "/items/01",
            "/items/+1",
            "/items/-",
            "/items/3",
            "/items/999999999999999999999999999999999999999",
        ] {
            rewrite_json_pointer_matches(&mut value, pattern, counting_marker(&calls)).unwrap();
        }

        assert_eq!(value, before);
        assert_eq!(before, json!({"items": [11, 20, 31]}));
        assert_eq!(calls.get(), 0);
    }

    #[test]
    fn pointer_wildcard_visits_arrays_in_ascending_index_order() {
        let mut value = json!({"items": ["zero", "one", "two"]});
        let visited = RefCell::new(Vec::new());

        rewrite_json_pointer_matches(&mut value, "/items/*", |matched| {
            visited
                .borrow_mut()
                .push(matched.as_str().unwrap().to_owned());
            *matched = json!("done");
            Ok(())
        })
        .unwrap();

        assert_eq!(&*visited.borrow(), &["zero", "one", "two"]);
        assert_eq!(value, json!({"items": ["done", "done", "done"]}));
    }

    #[test]
    fn pointer_wildcard_visits_object_keys_in_utf8_order() {
        let mut value = serde_json::from_str::<Value>(
            r#"{"rows":{"中":"chinese","z":"zed","é":"accent","a":"ascii"}}"#,
        )
        .unwrap();
        let visited = RefCell::new(Vec::new());

        rewrite_json_pointer_matches(&mut value, "/rows/*", |matched| {
            visited
                .borrow_mut()
                .push(matched.as_str().unwrap().to_owned());
            Ok(())
        })
        .unwrap();

        assert_eq!(&*visited.borrow(), &["ascii", "zed", "accent", "chinese"]);
    }

    #[test]
    fn pointer_supports_nested_wildcards_and_visits_each_match_once() {
        let mut value = json!({
            "groups": {
                "b": {"members": [{"score": 0}, {"score": 0}]},
                "a": {"members": [{"score": 0}]},
                "empty": {"members": []},
                "scalar": 1,
            },
        });
        let calls = Cell::new(0);

        rewrite_json_pointer_matches(&mut value, "/groups/*/members/*/score", |matched| {
            calls.set(calls.get() + 1);
            *matched = json!(matched.as_u64().unwrap() + 1);
            Ok(())
        })
        .unwrap();

        assert_eq!(calls.get(), 3);
        assert_eq!(value["groups"]["a"]["members"][0]["score"], 1);
        assert_eq!(value["groups"]["b"]["members"][0]["score"], 1);
        assert_eq!(value["groups"]["b"]["members"][1]["score"], 1);
    }

    #[test]
    fn pointer_stops_on_callback_error_and_keeps_prior_mutations() {
        let mut value = json!({"rows": {"c": 3, "b": 2, "a": 1}});
        let visited = RefCell::new(Vec::new());

        let error = rewrite_json_pointer_matches(&mut value, "/rows/*", |matched| {
            let number = matched.as_u64().unwrap();
            visited.borrow_mut().push(number);
            if number == 2 {
                return Err("callback stopped".to_owned());
            }
            *matched = json!(number + 10);
            Ok(())
        })
        .unwrap_err();

        assert_eq!(error, "callback stopped");
        assert_eq!(&*visited.borrow(), &[1, 2]);
        assert_eq!(value, json!({"rows": {"a": 11, "b": 2, "c": 3}}));
    }

    #[test]
    fn pointer_calls_observe_prior_calls_in_caller_order() {
        let mut add_then_multiply = json!({"value": 1});
        rewrite_json_pointer_matches(&mut add_then_multiply, "/value", |matched| {
            *matched = json!(matched.as_u64().unwrap() + 1);
            Ok(())
        })
        .unwrap();
        rewrite_json_pointer_matches(&mut add_then_multiply, "/value", |matched| {
            *matched = json!(matched.as_u64().unwrap() * 2);
            Ok(())
        })
        .unwrap();

        let mut multiply_then_add = json!({"value": 1});
        rewrite_json_pointer_matches(&mut multiply_then_add, "/value", |matched| {
            *matched = json!(matched.as_u64().unwrap() * 2);
            Ok(())
        })
        .unwrap();
        rewrite_json_pointer_matches(&mut multiply_then_add, "/value", |matched| {
            *matched = json!(matched.as_u64().unwrap() + 1);
            Ok(())
        })
        .unwrap();

        assert_eq!(add_then_multiply["value"], 4);
        assert_eq!(multiply_then_add["value"], 3);
    }

    #[test]
    fn text_replacement_handles_empty_inputs_and_ignores_empty_raw_tokens() {
        let none = BTreeMap::new();
        assert_eq!(replace_longest_text_tokens("", &none, |_, _| true), "");
        assert_eq!(
            replace_longest_text_tokens("unchanged", &none, |_, _| true),
            "unchanged"
        );

        let values = replacements([("", "bad"), ("raw", "stable")]);
        assert_eq!(replace_longest_text_tokens("", &values, |_, _| true), "");
        assert_eq!(
            replace_longest_text_tokens("raw", &values, |_, _| true),
            "stable"
        );
    }

    #[test]
    fn text_replacement_rewrites_exact_and_repeated_nonoverlapping_tokens() {
        let values = replacements([("root", "stable")]);

        assert_eq!(
            replace_longest_text_tokens("root", &values, |_, _| true),
            "stable"
        );
        assert_eq!(
            replace_longest_text_tokens("rootroot root", &values, |_, _| true),
            "stablestable stable"
        );
    }

    #[test]
    fn text_replacement_passes_full_prefix_and_suffix_to_custom_policy() {
        let values = replacements([("root", "stable")]);
        let candidates = RefCell::new(Vec::new());

        let output = replace_longest_text_tokens("x root y root z", &values, |prefix, suffix| {
            candidates
                .borrow_mut()
                .push((prefix.to_owned(), suffix.to_owned()));
            prefix.ends_with("x ")
        });

        assert_eq!(output, "x stable y root z");
        assert_eq!(
            &*candidates.borrow(),
            &[
                ("x ".to_owned(), " y root z".to_owned()),
                ("x root y ".to_owned(), " z".to_owned()),
            ]
        );
    }

    #[test]
    fn text_replacement_processes_longer_raw_tokens_first() {
        let values = replacements([("/tmp", "<root>"), ("/tmp/file", "<file>")]);

        assert_eq!(
            replace_longest_text_tokens("/tmp/file /tmp", &values, |_, _| true),
            "<file> <root>"
        );
    }

    #[test]
    fn text_replacement_breaks_equal_length_overlap_ties_by_raw_token() {
        let values = replacements([("ba", "<ba>"), ("ab", "<ab>")]);

        assert_eq!(
            replace_longest_text_tokens("aba", &values, |_, _| true),
            "<ab>a"
        );
    }

    #[test]
    fn text_replacement_sorts_by_utf8_byte_length_and_preserves_unicode() {
        let values = replacements([("aX", "<short>"), ("🧪a", "<long>")]);

        assert_eq!(
            replace_longest_text_tokens("前🧪aX後", &values, |_, _| true),
            "前<long>X後"
        );
    }

    #[test]
    fn text_replacement_supports_chinese_and_pseudo_locale_policies() {
        let values = replacements([("路徑", "<path>")]);
        let locale_boundary = |prefix: &str, suffix: &str| {
            matches!(prefix.chars().next_back(), Some('「' | '【'))
                && matches!(suffix.chars().next(), Some('」' | '】'))
        };

        assert_eq!(
            replace_longest_text_tokens("「路徑」 【路徑】 路徑中", &values, locale_boundary),
            "「<path>」 【<path>】 路徑中"
        );
    }

    #[test]
    fn text_replacement_policy_can_distinguish_a_period_from_an_extension() {
        let values = replacements([("root", "stable")]);
        let sentence_period = |_: &str, suffix: &str| {
            suffix.strip_prefix('.').is_some_and(|after| {
                after.is_empty() || after.chars().next().is_some_and(char::is_whitespace)
            })
        };

        assert_eq!(
            replace_longest_text_tokens("root. root.py", &values, sentence_period),
            "stable. root.py"
        );
    }

    #[test]
    fn text_replacement_preserves_sequential_cascade_semantics() {
        let values = replacements([("cat", "dog"), ("dog", "fox")]);

        assert_eq!(
            replace_longest_text_tokens("cat", &values, |_, _| true),
            "fox"
        );
    }
}
