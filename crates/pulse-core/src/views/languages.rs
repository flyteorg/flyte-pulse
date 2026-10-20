//! Language breakdowns from GitHub's per-repository language detection.

use std::collections::HashMap;

use serde_json::{Value, json};

use crate::state::Language;

/// Languages below this share, and past the largest few, are grouped as "Other".
const MIN_SHARE: f64 = 0.01;
const MAX_LANGUAGES: usize = 8;

/// Combine repositories' breakdowns. Each repository contributes `weight` split by its
/// languages' byte shares, so a weight of its total bytes sums code size, and a weight
/// of a person's commits there credits their activity. Rows are sorted by share.
pub(crate) fn language_rows<'a>(parts: impl IntoIterator<Item = (&'a [Language], f64)>) -> Vec<Value> {
    let mut scores: HashMap<&str, (f64, &str)> = HashMap::new();
    for (languages, weight) in parts {
        let total: u64 = languages.iter().map(|l| l.bytes).sum();
        if total == 0 || weight <= 0.0 {
            continue;
        }
        for language in languages {
            let entry = scores.entry(&language.name).or_insert((0.0, &language.color));
            entry.0 += weight * language.bytes as f64 / total as f64;
        }
    }
    let sum: f64 = scores.values().map(|(score, _)| score).sum();
    if sum <= 0.0 {
        return Vec::new();
    }
    let mut rows: Vec<(&str, f64, &str)> =
        scores.into_iter().map(|(name, (score, color))| (name, score / sum, color)).collect();
    rows.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(b.0)));
    let share = |value: f64| (value * 10_000.0).round() / 10_000.0;
    let mut out = Vec::new();
    let mut other = 0.0;
    for (index, (name, value, color)) in rows.into_iter().enumerate() {
        if index < MAX_LANGUAGES && value >= MIN_SHARE {
            out.push(json!({"name": name, "color": color, "share": share(value)}));
        } else {
            other += value;
        }
    }
    if other > 0.0 {
        out.push(json!({"name": "Other", "color": "", "share": share(other)}));
    }
    out
}

/// A repository's breakdown weighted by its size.
pub(crate) fn by_size(languages: &[Language]) -> (&[Language], f64) {
    (languages, languages.iter().map(|l| l.bytes).sum::<u64>() as f64)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn language(name: &str, bytes: u64) -> Language {
        Language { name: name.into(), color: format!("#{name}"), bytes }
    }

    #[test]
    fn combines_and_groups_small_languages() {
        let big = vec![language("Go", 600), language("Python", 300), language("Shell", 5)];
        let small = vec![language("Rust", 95)];
        let rows = language_rows([by_size(&big), by_size(&small)]);
        let names: Vec<&str> = rows.iter().map(|r| r["name"].as_str().unwrap()).collect();
        assert_eq!(names, ["Go", "Python", "Rust", "Other"], "Shell is under 1%");
        assert_eq!(rows[0]["share"], 0.6);
        assert_eq!(rows[3]["color"], "");

        // Weighted by activity: equal weights count each repository's shares equally.
        let rows = language_rows([(big.as_slice(), 1.0), (small.as_slice(), 1.0)]);
        assert_eq!(rows[0]["name"], "Rust");
        assert_eq!(rows[0]["share"], 0.5);
        assert!(language_rows([(big.as_slice(), 0.0)]).is_empty());
    }
}
