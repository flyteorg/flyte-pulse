//! Keep `companies.yaml` sorted by company ID and `affiliations.yaml` by username,
//! with a contributor's undated company before the dated periods. Lines inside an
//! entry, including comments, move with it. Comments between entries only marked
//! groups (their history is in Git) and are dropped by sorting; the file header stays.

use std::path::Path;

use anyhow::{Context, Result};

/// Sort the entries of the top-level `list_key:` list by `sort_key`.
fn sort_list<K: Ord>(text: &str, list_key: &str, sort_key: impl Fn(&[&str]) -> K) -> String {
    let lines: Vec<&str> = text.lines().collect();
    let Some(list_line) = lines.iter().position(|line| line.trim_end() == format!("{list_key}:")) else {
        return text.to_string();
    };
    let mut blocks: Vec<Vec<&str>> = Vec::new();
    for line in &lines[list_line + 1..] {
        if line.starts_with("- ") {
            blocks.push(vec![line]);
        } else if line.starts_with("  ")
            && let Some(block) = blocks.last_mut()
        {
            block.push(line);
        }
    }
    blocks.sort_by_cached_key(|block| sort_key(block));
    let mut out: Vec<&str> = lines[..=list_line].to_vec();
    out.extend(blocks.into_iter().flatten());
    let mut text = out.join("\n");
    text.push('\n');
    text
}

/// The value of `key` in an entry (`- key: value` or `  key: value`), unquoted.
fn field(block: &[&str], key: &str) -> String {
    block
        .iter()
        .find_map(|line| {
            let line = line.strip_prefix("- ").or_else(|| line.strip_prefix("  "))?;
            line.strip_prefix(&format!("{key}:"))
        })
        .map(|value| value.trim().trim_matches(|c| c == '\'' || c == '"').to_string())
        .unwrap_or_default()
}

pub fn sort_companies(text: &str) -> String {
    sort_list(text, "companies", |block| field(block, "id").to_lowercase())
}

pub fn sort_affiliations(text: &str) -> String {
    let sorted = sort_list(text, "affiliations", |block| {
        let who = field(block, "username");
        let who = if who.is_empty() { field(block, "contributor_id") } else { who };
        who.trim_start_matches('@').to_lowercase()
    });
    sort_periods(&sorted)
}

/// The value of `key` in a company period (`  - key: value` or `    key: value`).
fn period_field(block: &[&str], key: &str) -> String {
    block
        .iter()
        .find_map(|line| {
            let line = line.strip_prefix("  - ").or_else(|| line.strip_prefix("    "))?;
            line.strip_prefix(&format!("{key}:"))
        })
        .map(|value| value.trim().trim_matches(|c| c == '\'' || c == '"').to_string())
        .unwrap_or_default()
}

/// Sort each entry's `companies` list: the undated company first, then by start date.
fn sort_periods(text: &str) -> String {
    let lines: Vec<&str> = text.lines().collect();
    let mut out: Vec<&str> = Vec::with_capacity(lines.len());
    let mut index = 0;
    while index < lines.len() {
        out.push(lines[index]);
        index += 1;
        if lines[index - 1].trim_end() != "  companies:" {
            continue;
        }
        let mut periods: Vec<Vec<&str>> = Vec::new();
        while index < lines.len() {
            let line = lines[index];
            if line.starts_with("  - ") {
                periods.push(vec![line]);
            } else if line.starts_with("    ")
                && let Some(period) = periods.last_mut()
            {
                period.push(line);
            } else {
                break;
            }
            index += 1;
        }
        periods.sort_by_cached_key(|period| (period_field(period, "from"), period_field(period, "until")));
        out.extend(periods.into_iter().flatten());
    }
    let mut text = out.join("\n");
    text.push('\n');
    text
}

/// Sort both files in `config_dir`. With `check`, only report whether they are
/// sorted. Returns the files that are (or were) out of order.
pub fn format_config(config_dir: &Path, check: bool) -> Result<Vec<String>> {
    let mut unsorted = Vec::new();
    for (name, sort) in
        [("companies.yaml", sort_companies as fn(&str) -> String), ("affiliations.yaml", sort_affiliations)]
    {
        let path = config_dir.join(name);
        let text = std::fs::read_to_string(&path).with_context(|| format!("cannot read {}", path.display()))?;
        let sorted = sort(&text);
        if sorted != text {
            unsorted.push(name.to_string());
            if !check {
                std::fs::write(&path, sorted)?;
            }
        }
    }
    Ok(unsorted)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sorts_entries_and_keeps_their_comments() {
        let text = "\
# Header comment.
affiliations:
- username: zed
  companies:
  - company: union
# A group comment between entries.
- username: Amy
  companies:
  - company: lyft
    # Attribution note for the dated period.
    from: '2021-01-01'
    until: '2022-01-01'
  - company: union
    from: '2022-01-01'
  - company: acme
";
        assert_eq!(
            sort_affiliations(text),
            "\
# Header comment.
affiliations:
- username: Amy
  companies:
  - company: acme
  - company: lyft
    # Attribution note for the dated period.
    from: '2021-01-01'
    until: '2022-01-01'
  - company: union
    from: '2022-01-01'
- username: zed
  companies:
  - company: union
"
        );
        let companies =
            "companies:\n- id: union\n  canonical_name: Union\n- id: anthropic\n  aliases:\n  - '@anthropics'\n";
        assert_eq!(
            sort_companies(companies),
            "companies:\n- id: anthropic\n  aliases:\n  - '@anthropics'\n- id: union\n  canonical_name: Union\n"
        );
    }

    #[test]
    fn repository_config_is_sorted() {
        let config = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../config");
        let unsorted = format_config(&config, true).unwrap();
        assert!(unsorted.is_empty(), "run `cargo run -p pulse -- format-config` to sort {unsorted:?}");
    }
}
