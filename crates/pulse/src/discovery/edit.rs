//! Line-based edits of companies.yaml and affiliations.yaml. Editing lines instead
//! of re-serializing keeps the reviewed files' comments and layout intact.

use std::path::Path;

use anyhow::{Context, Result};
use pulse_core::state::Contributor;

use super::{Change, Discovery};
use crate::companies::load_company_catalog;
use crate::config_format::{sort_affiliations, sort_companies};

/// Apply the discovery's edits to the config files after validating the result.
pub fn apply(discovery: &Discovery, config_dir: &Path, contributors: &[Contributor]) -> Result<()> {
    let companies_path = config_dir.join("companies.yaml");
    let affiliations_path = config_dir.join("affiliations.yaml");
    let mut companies = Lines::read(&companies_path)?;
    let mut affiliations = Lines::read(&affiliations_path)?;

    for mapping in &discovery.aliases {
        companies.add_list_item(&mapping.company_id, "aliases", &scalar(&mapping.alias));
    }
    for mapping in &discovery.domains {
        companies.add_list_item(&mapping.company_id, "domains", &mapping.domain);
    }
    for company in &discovery.companies {
        companies.append(&[
            format!("- id: {}", company.id),
            format!("  canonical_name: {}", scalar(&company.canonical_name)),
            format!("  website_url: {}", company.website_url),
            "  aliases:".to_string(),
            format!("  - {}", scalar(&company.alias)),
        ]);
        if !company.domains.is_empty() {
            companies.append(&["  domains:".to_string()]);
            companies.append(&company.domains.iter().map(|d| format!("  - {d}")).collect::<Vec<_>>());
        }
    }

    // Periods to add, by username; they join the person's entry or start a new one.
    let mut appended: Vec<(String, Vec<String>)> = Vec::new();
    for change in &discovery.changes {
        affiliations.apply_change(change, &mut appended);
    }
    for start in &discovery.current_starts {
        affiliations.set_start(&start.login, &start.contributor_id, &start.company_id, &start.valid_from);
    }
    for affiliation in &discovery.affiliations {
        let period = match &affiliation.valid_from {
            Some(valid_from) => open_period(&affiliation.company_id, valid_from),
            None => undated(&affiliation.company_id),
        };
        appended.push((affiliation.login.clone(), period));
    }
    for period in &discovery.periods {
        appended.push((period.login.clone(), dated(&period.company_id, &period.valid_from, &period.valid_until)));
    }
    for (login, period) in appended {
        affiliations.append_period(&login, &period);
    }
    // New entries land in sorted position; the pull request describes them.
    let companies = sort_companies(&companies.text());
    let affiliations = sort_affiliations(&affiliations.text());

    // Validate with the same loader the build uses before touching the files.
    let staging = tempfile::tempdir()?;
    std::fs::write(staging.path().join("companies.yaml"), &companies)?;
    std::fs::write(staging.path().join("affiliations.yaml"), &affiliations)?;
    load_company_catalog(staging.path(), contributors).context("the proposed config edits are invalid")?;
    std::fs::write(&companies_path, companies)?;
    std::fs::write(&affiliations_path, affiliations)?;
    Ok(())
}

/// A string as a YAML scalar: plain, like hand-written entries, unless YAML would
/// read it differently (as a boolean, null, or number, or because of indicator
/// characters or surrounding spaces); then single-quoted.
fn scalar(value: &str) -> String {
    let plain = !value.is_empty()
        && value.trim() == value
        && !value.starts_with(|c: char| c.is_ascii_digit() || "-?:,[]{}#&*!|>'\"%@`+.~".contains(c))
        && !value.ends_with(':')
        && !value.contains(": ")
        && !value.contains(" #")
        && !value.chars().any(char::is_control)
        && !matches!(
            value.to_ascii_lowercase().as_str(),
            "true" | "false" | "yes" | "no" | "on" | "off" | "y" | "n" | "null"
        )
        && value.parse::<f64>().is_err();
    if plain { value.to_string() } else { format!("'{}'", value.replace('\'', "''")) }
}

fn undated(company_id: &str) -> Vec<String> {
    vec![format!("  - company: {company_id}")]
}

fn open_period(company_id: &str, valid_from: &str) -> Vec<String> {
    vec![format!("  - company: {company_id}"), format!("    from: '{valid_from}'")]
}

fn dated(company_id: &str, valid_from: &str, valid_until: &str) -> Vec<String> {
    vec![
        format!("  - company: {company_id}"),
        format!("    from: '{valid_from}'"),
        format!("    until: '{valid_until}'"),
    ]
}

fn unquote(value: &str) -> String {
    value.trim().trim_matches(|c| c == '\'' || c == '"').to_string()
}

struct Lines {
    lines: Vec<String>,
}

/// A YAML list item: its first line starts with `- ` and continuation lines are indented.
#[derive(Clone, Copy)]
struct Block {
    start: usize,
    end: usize,
}

impl Lines {
    fn read(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path).with_context(|| format!("cannot read {}", path.display()))?;
        Ok(Self { lines: text.lines().map(str::to_string).collect() })
    }

    fn text(&self) -> String {
        let mut text = self.lines.join("\n");
        text.push('\n');
        text
    }

    fn append(&mut self, lines: &[String]) {
        while self.lines.last().is_some_and(|l| l.trim().is_empty()) {
            self.lines.pop();
        }
        self.lines.extend(lines.iter().cloned());
    }

    fn blocks(&self) -> Vec<Block> {
        let mut blocks = Vec::new();
        let mut index = 0;
        while index < self.lines.len() {
            if self.lines[index].starts_with("- ") {
                let start = index;
                index += 1;
                while index < self.lines.len() && self.lines[index].starts_with("  ") {
                    index += 1;
                }
                blocks.push(Block { start, end: index });
            } else {
                index += 1;
            }
        }
        blocks
    }

    /// The value of `key` in a block (`- key: value` or `  key: value`).
    fn field(&self, block: Block, key: &str) -> Option<String> {
        self.lines[block.start..block.end].iter().find_map(|line| {
            let line = line.strip_prefix("- ").or_else(|| line.strip_prefix("  "))?;
            line.strip_prefix(&format!("{key}:")).map(unquote)
        })
    }

    fn field_line(&self, block: Block, key: &str) -> Option<usize> {
        (block.start..block.end).find(|&i| {
            let line = &self.lines[i];
            let line = line.strip_prefix("- ").or_else(|| line.strip_prefix("  ")).unwrap_or(line);
            line.starts_with(&format!("{key}:"))
        })
    }

    /// Add `value` to the `key` list of a company (`aliases` or `domains`).
    fn add_list_item(&mut self, company_id: &str, key: &str, value: &str) {
        let Some(block) = self.blocks().into_iter().find(|b| self.field(*b, "id").as_deref() == Some(company_id))
        else {
            return;
        };
        match self.field_line(block, key) {
            Some(line) if self.lines[line].trim_end().ends_with("[]") => {
                self.lines[line] = format!("  {key}:");
                self.lines.insert(line + 1, format!("  - {value}"));
            }
            Some(line) => {
                let mut insert = line + 1;
                while insert < block.end && self.lines[insert].starts_with("  - ") {
                    insert += 1;
                }
                self.lines.insert(insert, format!("  - {value}"));
            }
            None => {
                self.lines.insert(block.end, format!("  {key}:"));
                self.lines.insert(block.end + 1, format!("  - {value}"));
            }
        }
    }

    /// The entry of one contributor, matched by username or contributor ID.
    fn contributor_block(&self, login: &str, contributor_id: &str) -> Option<Block> {
        self.blocks().into_iter().find(|b| {
            self.field(*b, "username").is_some_and(|u| u.trim_start_matches('@').eq_ignore_ascii_case(login))
                || self.field(*b, "contributor_id").as_deref() == Some(contributor_id)
        })
    }

    /// The company periods of an entry: items of its `companies` list.
    fn periods(&self, block: Block) -> Vec<Block> {
        let mut periods = Vec::new();
        let mut index = block.start;
        while index < block.end {
            if self.lines[index].starts_with("  - ") {
                let start = index;
                index += 1;
                while index < block.end && self.lines[index].starts_with("    ") {
                    index += 1;
                }
                periods.push(Block { start, end: index });
            } else {
                index += 1;
            }
        }
        periods
    }

    /// The value of `key` in a period (`  - key: value` or `    key: value`).
    fn period_field(&self, period: Block, key: &str) -> Option<String> {
        self.lines[period.start..period.end].iter().find_map(|line| {
            let line = line.strip_prefix("  - ").or_else(|| line.strip_prefix("    "))?;
            line.strip_prefix(&format!("{key}:")).map(unquote)
        })
    }

    fn period_field_line(&self, period: Block, key: &str) -> Option<usize> {
        (period.start..period.end).find(|&i| {
            let line = &self.lines[i];
            let line = line.strip_prefix("  - ").or_else(|| line.strip_prefix("    ")).unwrap_or(line);
            line.starts_with(&format!("{key}:"))
        })
    }

    /// Add a period to a contributor's entry, creating the entry when it is missing.
    fn append_period(&mut self, login: &str, period: &[String]) {
        match self.contributor_block(login, "") {
            Some(block) => {
                let end = self.periods(block).last().map_or(block.end, |p| p.end);
                if self.field_line(block, "companies").is_none() {
                    self.lines.insert(end, "  companies:".to_string());
                    self.lines.splice(end + 1..end + 1, period.iter().cloned());
                } else {
                    self.lines.splice(end..end, period.iter().cloned());
                }
            }
            None => {
                let mut entry = vec![format!("- username: {login}"), "  companies:".to_string()];
                entry.extend(period.iter().cloned());
                self.append(&entry);
            }
        }
    }

    /// Give a contributor's undated period for `company_id` a start date.
    fn set_start(&mut self, login: &str, contributor_id: &str, company_id: &str, valid_from: &str) {
        let Some(block) = self.contributor_block(login, contributor_id) else { return };
        let period = self.periods(block).into_iter().find(|p| {
            self.period_field(*p, "from").is_none() && self.period_field(*p, "company").as_deref() == Some(company_id)
        });
        if let Some(period) = period {
            self.lines.insert(period.end, format!("    from: '{valid_from}'"));
        }
    }

    /// End the current affiliation on the change date and start the new one.
    fn apply_change(&mut self, change: &Change, appended: &mut Vec<(String, Vec<String>)>) {
        let Some(block) = self.contributor_block(&change.login, &change.contributor_id) else {
            if !change.to.is_empty() {
                appended.push((change.login.clone(), open_period(&change.to, &change.date)));
            }
            return;
        };
        let periods = self.periods(block);
        let has_undated = periods.iter().any(|p| self.period_field(*p, "from").is_none());
        let open = periods.iter().copied().find(|p| {
            self.period_field(*p, "from").is_some_and(|from| from <= change.date)
                && self.period_field(*p, "until").is_none()
        });
        // The old company's dated period starts after any earlier period ends.
        let earlier_end = periods
            .iter()
            .filter_map(|p| self.period_field(*p, "until"))
            .filter(|until| *until <= change.date)
            .max()
            .unwrap_or_default();

        if let Some(period) = open {
            let line = self.period_field_line(period, "from").unwrap_or(period.end - 1);
            self.lines.insert(line + 1, format!("    until: '{}'", change.date));
        } else if !change.from.is_empty() && has_undated {
            let start = std::cmp::max(change.first_activity.clone(), earlier_end);
            if !start.is_empty() && start < change.date {
                appended.push((change.login.clone(), dated(&change.from, &start, &change.date)));
            }
        }

        // The undated period for the old company becomes the new company's open-ended
        // period. Re-find it: the insert above may have shifted lines.
        let Some(block) = self.contributor_block(&change.login, &change.contributor_id) else { return };
        let periods = self.periods(block);
        let current = periods.iter().copied().find(|p| {
            self.period_field(*p, "from").is_none()
                && self.period_field(*p, "company").unwrap_or_default() == change.from
        });
        match (current, change.to.is_empty()) {
            (Some(period), false) => {
                let line = self.period_field_line(period, "company").unwrap_or(period.start);
                self.lines[line] = format!("  - company: {}", change.to);
                self.lines.insert(period.end, format!("    from: '{}'", change.date));
            }
            (Some(period), true) => {
                if periods.len() == 1 {
                    // An entry needs at least one company; drop the whole entry.
                    self.lines.drain(block.start..block.end);
                } else {
                    self.lines.drain(period.start..period.end);
                }
            }
            (None, false) => appended.push((change.login.clone(), open_period(&change.to, &change.date))),
            (None, true) => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::scalar;

    #[test]
    fn scalars_are_plain_unless_yaml_needs_quotes() {
        let plain = ["OpenAI", "Acme Labs", "neptune.ai", "Nextdoor.com, Inc.", "AT&T", "O'Reilly", "C# Labs"];
        for value in plain {
            assert_eq!(scalar(value), value);
        }
        let quoted = [
            ("@acme", "'@acme'"),
            ("'quoted'", "'''quoted'''"),
            ("true", "'true'"),
            ("No", "'No'"),
            ("null", "'null'"),
            ("~", "'~'"),
            ("2024", "'2024'"),
            ("1e3", "'1e3'"),
            ("NaN", "'NaN'"),
            (".inf", "'.inf'"),
            ("3M", "'3M'"),
            (" Acme", "' Acme'"),
            ("Acme ", "'Acme '"),
            ("Acme: Labs", "'Acme: Labs'"),
            ("Acme #1", "'Acme #1'"),
            ("Acme:", "'Acme:'"),
            ("[Acme]", "'[Acme]'"),
            ("- Acme", "'- Acme'"),
            ("", "''"),
        ];
        for (value, expected) in quoted {
            assert_eq!(scalar(value), expected);
        }
        // Every form reads back as the same string.
        for value in plain.into_iter().chain(quoted.map(|(value, _)| value)) {
            let parsed: serde_yaml_ng::Value = serde_yaml_ng::from_str(&format!("key: {}", scalar(value))).unwrap();
            assert_eq!(parsed["key"], serde_yaml_ng::Value::String(value.to_string()), "{value:?}");
        }
    }
}
