use std::collections::BTreeSet;

use super::{ParamDecl, ParameterDelivery};

const SECRET_SUFFIXES: [&str; 3] = ["SECRET", "PASSWORD", "PASSWD"];

#[derive(Clone, Copy, Debug)]
struct SegmentVerdict {
    secret: bool,
    token: bool,
    token_plural: bool,
    internal_count: bool,
    county: bool,
    numeric: bool,
}

/// Conservatively detect credential-looking parameter names and prompts.
///
/// The detector deliberately recognizes separator, camelCase, jammed, plural, and digit-split
/// spellings. `TOKEN` receives additional count-context handling so controls such as
/// `max_tokens`, `nTokens`, and `60 tokens/min` remain public while credential prompts stay secret.
#[must_use]
pub fn is_secret_name(text: &str) -> bool {
    let raw_segments = text
        .split(|character: char| !character.is_ascii_alphanumeric())
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>();
    let verdicts = raw_segments
        .iter()
        .map(|segment| judge_segment(segment))
        .collect::<Vec<_>>();

    if verdicts.iter().any(|verdict| verdict.secret) {
        return true;
    }

    let sentence = text.trim().chars().any(char::is_whitespace);
    if !verdicts
        .iter()
        .any(|verdict| token_hit(verdict, sentence))
    {
        return false;
    }

    if sentence {
        return verdicts.iter().enumerate().any(|(index, verdict)| {
            token_hit(verdict, sentence) && !qualified_token_hit(&verdicts, index)
        });
    }

    if verdicts.iter().any(|verdict| verdict.county) {
        return false;
    }
    if verdicts
        .iter()
        .enumerate()
        .filter(|(_, verdict)| token_hit(verdict, sentence))
        .all(|(index, _)| qualified_token_hit(&verdicts, index))
    {
        return false;
    }

    !(raw_segments.len() == 1 && raw_segments[0].eq_ignore_ascii_case("TOKENS"))
}

/// Synthesize the implicit declaration used by command-template placeholders.
#[must_use]
pub fn synthesized_placeholder(name: &str) -> ParamDecl {
    ParamDecl {
        delivery: ParameterDelivery::Placeholder,
        required: true,
        secret: is_secret_name(name),
        ..ParamDecl::new(name)
    }
}

fn judge_segment(raw: &str) -> SegmentVerdict {
    let jam = raw.to_ascii_uppercase();
    let camel = camel_words(raw);
    let digit_parts = digit_split(&jam);
    let mut all_forms = BTreeSet::new();
    for word in std::iter::once(&jam)
        .chain(camel.iter())
        .chain(digit_parts.iter())
    {
        all_forms.extend(word_forms(word));
    }

    let secret = all_forms.iter().any(|form| {
        SECRET_SUFFIXES
            .iter()
            .any(|suffix| form.ends_with(suffix))
    }) || all_forms.contains("KEY")
        || all_forms.iter().any(|form| {
            form.strip_suffix("KEY")
                .is_some_and(is_credential_key_prefix)
        });
    let county = is_count_word(&jam) || is_count_word(fold_plural(&jam));
    let numeric = jam.bytes().all(|byte| byte.is_ascii_digit());
    let internal_count = camel.iter().any(|word| {
        word_forms(word).into_iter().any(|variant| {
            is_count_word(&variant) && (variant.len() != 1 || word == &variant)
        })
    }) || digit_parts.iter().any(|word| {
        word_forms(word)
            .into_iter()
            .any(|variant| is_count_word(&variant) && variant.len() != 1)
    });
    let token = all_forms.iter().any(|form| token_form(form)) && !county && !numeric;
    let token_plural = token
        && std::iter::once(jam.as_str())
            .chain(camel.iter().map(String::as_str))
            .chain(digit_parts.iter().map(String::as_str))
            .any(|word| word.ends_with("TOKENS"));

    SegmentVerdict {
        secret,
        token,
        token_plural,
        internal_count,
        county,
        numeric,
    }
}

fn token_hit(verdict: &SegmentVerdict, sentence: bool) -> bool {
    verdict.token && !(verdict.internal_count && !sentence)
}

fn qualified_token_hit(verdicts: &[SegmentVerdict], index: usize) -> bool {
    if index == 0 {
        return false;
    }
    let prior = verdicts[index - 1];
    prior.county || (prior.numeric && verdicts[index].token_plural)
}

fn word_forms(word: &str) -> BTreeSet<String> {
    let stripped = word
        .chars()
        .filter(|character| !character.is_ascii_digit())
        .collect::<String>();
    let mut forms = BTreeSet::from([word.to_owned(), fold_plural(word).to_owned()]);
    if !stripped.is_empty() {
        forms.insert(stripped.clone());
        forms.insert(fold_plural(&stripped).to_owned());
    }
    forms
}

fn fold_plural(word: &str) -> &str {
    word.strip_suffix('S').unwrap_or(word)
}

fn digit_split(jam: &str) -> Vec<String> {
    jam.split(|character: char| character.is_ascii_digit())
        .filter(|part| !part.is_empty())
        .map(str::to_owned)
        .collect()
}

fn camel_words(raw: &str) -> Vec<String> {
    let bytes = raw.as_bytes();
    let mut words = Vec::new();
    let mut start = 0;
    for index in 1..bytes.len() {
        let previous = bytes[index - 1];
        let current = bytes[index];
        let next_is_lower = bytes
            .get(index + 1)
            .is_some_and(|next| next.is_ascii_lowercase());
        let boundary = (previous.is_ascii_lowercase() && current.is_ascii_uppercase())
            || (previous.is_ascii_uppercase() && current.is_ascii_uppercase() && next_is_lower);
        if boundary {
            words.push(raw[start..index].to_ascii_uppercase());
            start = index;
        }
    }
    words.push(raw[start..].to_ascii_uppercase());
    words
}

fn token_form(form: &str) -> bool {
    form.strip_suffix("TOKEN")
        .is_some_and(|prefix| !is_count_word(prefix))
}

fn is_credential_key_prefix(prefix: &str) -> bool {
    matches!(
        prefix,
        "API"
            | "AUTH"
            | "ACCESS"
            | "PRIVATE"
            | "PUBLIC"
            | "SSH"
            | "PGP"
            | "SIGNING"
            | "ENCRYPTION"
            | "DECRYPTION"
            | "STRIPE"
            | "AWS"
            | "GCP"
            | "AZURE"
            | "BASE"
    )
}

fn is_count_word(word: &str) -> bool {
    matches!(
        word,
        "MAX"
            | "MIN"
            | "NUM"
            | "NUMBER"
            | "COUNT"
            | "LIMIT"
            | "LENGTH"
            | "SIZE"
            | "TOTAL"
            | "AVAILABLE"
            | "REMAINING"
            | "USED"
            | "USAGE"
            | "BUDGET"
            | "INPUT"
            | "OUTPUT"
            | "PROMPT"
            | "COMPLETION"
            | "RATE"
            | "PER"
            | "SECONDS"
            | "SECOND"
            | "MINUTES"
            | "MINUTE"
            | "HOUR"
            | "HOURS"
            | "DAY"
            | "DAYS"
            | "MONTH"
            | "MONTHS"
            | "YEAR"
            | "YEARS"
            | "N"
    )
}
