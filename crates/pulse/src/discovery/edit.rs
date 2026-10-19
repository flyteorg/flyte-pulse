//! Line-based edits of companies.yaml and affiliations.yaml. Editing lines instead
//! of re-serializing keeps the reviewed files' comments and layout intact.

use std::path::Path;

use anyhow::{Context, Result};
use pulse_core::state::Contributor;

use super::{Change, Discovery};
use crate::companies::load_company_catalog;

/// Apply the discovery's edits to the config files after validating the result.
pub fn apply(discovery: &Discovery, config_dir: &Path, contributors: &[Contributor], today: &str) -> Result<()> {
    let companies_path = config_dir.join("companies.yaml");
    let affiliations_path = config_dir.join("affiliations.yaml");
    let mut companies = Lines::read(&companies_path)?;
    let mut affiliations = Lines::read(&affiliations_path)?;

    for mapping in &discovery.aliases {
        companies.add_list_item(&mapping.company_id, "aliases", &quote(&mapping.alias));
    }
    for mapping in &discovery.domains {
        companies.add_list_item(&mapping.company_id, "domains", &mapping.domain);
    }
    if !discovery.companies.is_empty() {
        companies.append(&[format!("# Proposed by company discovery on {today} from GitHub organization profiles.")]);
        for company in &discovery.companies {
            companies.append(&[
                format!("- id: {}", company.id),
                format!("  canonical_name: {}", quote(&company.canonical_name)),
                format!("  website_url: {}", company.website_url),
                "  aliases:".to_string(),
                format!("  - {}", quote(&company.alias)),
            ]);
            if !company.domains.is_empty() {
                companies.append(&["  domains:".to_string()]);
                companies.append(&company.domains.iter().map(|d| format!("  - {d}")).collect::<Vec<_>>());
            }
        }
    }

    let mut appended: Vec<Vec<String>> = Vec::new();
    for change in &discovery.changes {
        affiliations.apply_change(change, &mut appended);
    }
    for affiliation in &discovery.affiliations {
        appended.push(undated(&affiliation.login, &affiliation.company_id));
    }
    for period in &discovery.periods {
        appended.push(dated(&period.login, &period.company_id, &period.valid_from, &period.valid_until));
    }
    if !appended.is_empty() {
        affiliations
            .append(&[format!("# Proposed by company discovery on {today}; merging the pull request approves them.")]);
        for entry in appended {
            affiliations.append(&entry);
        }
    }

    // Validate with the same loader the build uses before touching the files.
    let staging = tempfile::tempdir()?;
    std::fs::write(staging.path().join("companies.yaml"), companies.text())?;
    std::fs::write(staging.path().join("affiliations.yaml"), affiliations.text())?;
    load_company_catalog(staging.path(), contributors).context("the proposed config edits are invalid")?;
    std::fs::write(&companies_path, companies.text())?;
    std::fs::write(&affiliations_path, affiliations.text())?;
    Ok(())
}

fn quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

fn undated(login: &str, company_id: &str) -> Vec<String> {
    vec![format!("- username: {login}"), format!("  company_id: {company_id}")]
}

fn dated(login: &str, company_id: &str, valid_from: &str, valid_until: &str) -> Vec<String> {
    vec![
        format!("- username: {login}"),
        format!("  company_id: {company_id}"),
        format!("  valid_from: '{valid_from}'"),
        format!("  valid_until: '{valid_until}'"),
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

    /// Blocks for one contributor, matched by username or contributor ID.
    fn contributor_blocks(&self, login: &str, contributor_id: &str) -> Vec<Block> {
        self.blocks()
            .into_iter()
            .filter(|b| {
                self.field(*b, "username").is_some_and(|u| u.trim_start_matches('@').eq_ignore_ascii_case(login))
                    || self.field(*b, "contributor_id").as_deref() == Some(contributor_id)
            })
            .collect()
    }

    /// End the current affiliation on the change date and start the new one.
    fn apply_change(&mut self, change: &Change, appended: &mut Vec<Vec<String>>) {
        let blocks = self.contributor_blocks(&change.login, &change.contributor_id);
        let has_undated = blocks.iter().any(|b| self.field(*b, "valid_from").is_none());
        let open_period = blocks.iter().copied().find(|b| {
            self.field(*b, "valid_from").is_some_and(|from| from <= change.date)
                && self.field(*b, "valid_until").is_none()
        });
        // The old company's dated period starts after any earlier period ends.
        let earlier_end = blocks
            .iter()
            .filter_map(|b| self.field(*b, "valid_until"))
            .filter(|until| *until <= change.date)
            .max()
            .unwrap_or_default();

        if let Some(block) = open_period {
            let line = self.field_line(block, "valid_from").unwrap_or(block.end - 1);
            self.lines.insert(line + 1, format!("  valid_until: '{}'", change.date));
        } else if !change.from.is_empty() && has_undated {
            let start = std::cmp::max(change.first_activity.clone(), earlier_end);
            if !start.is_empty() && start < change.date {
                appended.push(dated(&change.login, &change.from, &start, &change.date));
            }
        }

        // Re-find the undated block: the insert above may have shifted lines.
        let blocks = self.contributor_blocks(&change.login, &change.contributor_id);
        let current = blocks.iter().copied().find(|b| self.field(*b, "valid_from").is_none());
        match (current, change.to.is_empty()) {
            (Some(block), false) => {
                let key = self.lines[block.start].clone();
                self.lines.splice(block.start..block.end, [key, format!("  company_id: {}", change.to)]);
            }
            (Some(block), true) => {
                self.lines.drain(block.start..block.end);
            }
            (None, false) => appended.push(undated(&change.login, &change.to)),
            (None, true) => {}
        }
    }
}
