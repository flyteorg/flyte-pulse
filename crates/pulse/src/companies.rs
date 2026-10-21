//! Company identities, current mappings, and dated affiliations.

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::LazyLock;

use anyhow::{Context, Result, bail};
use pulse_core::snapshot::{Affiliation, Company};
use pulse_core::state::Contributor;
use pulse_core::time::{format_date, format_timestamp, parse_date};
use regex::Regex;
use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::config::hex;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CompaniesFile {
    companies: Vec<CompanyRow>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CompanyRow {
    id: String,
    canonical_name: String,
    website_url: String,
    #[serde(default)]
    aliases: Vec<String>,
    /// Corporate email domains of the company's employees.
    #[serde(default)]
    domains: Vec<String>,
    /// The company maintains the project; activity while affiliated with it is not
    /// community activity.
    #[serde(default)]
    maintainer: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AffiliationsFile {
    affiliations: Vec<AffiliationRow>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AffiliationRow {
    username: Option<String>,
    contributor_id: Option<String>,
    company_id: Option<String>,
    status: Option<String>,
    valid_from: Option<serde_yaml_ng::Value>,
    valid_until: Option<serde_yaml_ng::Value>,
}

static COMPANY_ID: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[a-z][a-z0-9_-]*$").unwrap());
static GITHUB_ID: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^github:[0-9]+$").unwrap());

/// Whole-phrase matching that ignores case and repeated whitespace.
pub fn normalize_company(value: &str) -> String {
    value.to_lowercase().split_whitespace().collect::<Vec<_>>().join(" ")
}

#[derive(Clone, Debug, Default)]
pub struct CompanyCatalog {
    pub companies: Vec<Company>,
    pub by_id: HashMap<String, usize>,
    pub aliases: HashMap<String, String>,
    /// Lowercased email domain to company ID.
    pub domains: HashMap<String, String>,
    /// Dated periods per contributor, sorted by `valid_from`.
    pub periods: HashMap<String, Vec<Affiliation>>,
    /// Undated mappings: the fallback for every date and the current profile affiliation.
    pub current: HashMap<String, Affiliation>,
    pub revision: String,
}

impl CompanyCatalog {
    pub fn company_name(&self, id: &str) -> String {
        self.by_id.get(id).map(|&i| self.companies[i].canonical_name.clone()).unwrap_or_default()
    }

    pub fn website(&self, id: &str) -> String {
        self.by_id.get(id).map(|&i| self.companies[i].website_url.clone()).unwrap_or_default()
    }

    /// The company whose aliases include `@<login>` for a GitHub organization.
    pub fn organization_company(&self, login: &str) -> Option<&str> {
        self.aliases.get(&normalize_company(&format!("@{login}"))).map(String::as_str)
    }

    /// The company for an email domain or one of its subdomains.
    pub fn domain_company(&self, domain: &str) -> Option<&str> {
        let mut candidate = domain;
        loop {
            if let Some(id) = self.domains.get(candidate) {
                return Some(id);
            }
            candidate = candidate.split_once('.')?.1;
            if !candidate.contains('.') {
                return None;
            }
        }
    }

    /// The website host without `www.`, such as `union.ai`.
    pub fn website_host(&self, id: &str) -> String {
        let url = self.website(id);
        let host = url.trim_start_matches("https://").split(['/', '?', '#']).next().unwrap_or_default();
        host.trim_start_matches("www.").to_lowercase()
    }

    /// The dated period covering `day` (`YYYY-MM-DD`), if any.
    pub fn period(&self, contributor_id: &str, day: &str) -> Option<&Affiliation> {
        let rows = self.periods.get(contributor_id)?;
        let index = rows.partition_point(|row| row.valid_from.as_str() <= day);
        let row = rows.get(index.checked_sub(1)?)?;
        (row.valid_until.is_empty() || day < row.valid_until.as_str()).then_some(row)
    }

    /// A dated period first, then the undated mapping for all other days.
    pub fn effective(&self, contributor_id: &str, day: &str) -> Option<&Affiliation> {
        self.period(contributor_id, day).or_else(|| self.current.get(contributor_id))
    }

    /// Company credited for activity on `day`; empty means Independent.
    pub fn historical(&self, contributor_id: &str, day: &str) -> String {
        self.effective(contributor_id, day).map(|row| row.company_id.clone()).unwrap_or_default()
    }

    /// Day ranges (`[start, end)`) in which the contributor's credited company is a
    /// maintainer, merged and in order.
    pub fn maintainer_days(&self, contributor_id: &str) -> Vec<(i32, i32)> {
        let maintainer =
            |row: &Affiliation| self.by_id.get(&row.company_id).is_some_and(|&i| self.companies[i].maintainer);
        let fallback = self.current.get(contributor_id).is_some_and(maintainer);
        let mut ranges: Vec<(i32, i32)> = Vec::new();
        let mut add = |start: i32, end: i32| match ranges.last_mut() {
            _ if start >= end => {}
            Some(last) if last.1 == start => last.1 = end,
            _ => ranges.push((start, end)),
        };
        // Dated periods first; the undated mapping covers the days between them.
        let mut cursor = i32::MIN;
        for row in self.periods.get(contributor_id).into_iter().flatten() {
            let start = parse_date(&row.valid_from).unwrap_or(i32::MIN);
            let end = parse_date(&row.valid_until).unwrap_or(i32::MAX);
            if fallback {
                add(cursor, start);
            }
            if maintainer(row) {
                add(start, end);
            }
            cursor = end;
        }
        if fallback {
            add(cursor, i32::MAX);
        }
        ranges
    }

    /// The contributor's current affiliation as shown on profile pages.
    pub fn current_affiliation(&self, contributor: &Contributor, day: &str) -> Affiliation {
        let profile = contributor.profile_company.trim();
        let matched = self.aliases.get(&normalize_company(profile)).cloned().unwrap_or_default();
        let mut result = Affiliation {
            company_name: if matched.is_empty() { "Independent".to_string() } else { self.company_name(&matched) },
            website_url: self.website(&matched),
            status: if matched.is_empty() { "INDEPENDENT" } else { "AFFILIATED" }.to_string(),
            source: if profile.is_empty() { "" } else { "PROFILE_SELF_REPORTED" }.to_string(),
            observed_at: contributor.profile_observed_at.map(|t| format_timestamp(t.0)).unwrap_or_default(),
            evidence: contributor.profile_url.clone(),
            company_id: matched.clone(),
            ..Affiliation::default()
        };
        if let Some(period) = self.effective(&contributor.id, day) {
            let conflict = !matched.is_empty() && matched != period.company_id;
            result = Affiliation {
                company_name: self.company_name(&period.company_id),
                website_url: self.website(&period.company_id),
                observed_at: result.observed_at,
                conflict,
                ..period.clone()
            };
        }
        result
    }
}

fn yaml_date(value: &serde_yaml_ng::Value, context: &str) -> Result<String> {
    let text = match value {
        serde_yaml_ng::Value::String(text) => text.clone(),
        other => serde_yaml_ng::to_string(other)?.trim().to_string(),
    };
    match parse_date(&text) {
        Some(days) if text.len() == 10 => Ok(format_date(days)),
        _ => bail!("{context} must be a YYYY-MM-DD date"),
    }
}

/// Load companies.yaml and affiliations.yaml, resolving usernames against the
/// collected contributors so affiliations are stored by stable account ID.
pub fn load_company_catalog(config_dir: &Path, contributors: &[Contributor]) -> Result<CompanyCatalog> {
    let company_bytes = std::fs::read(config_dir.join("companies.yaml")).context("cannot read companies.yaml")?;
    let affiliation_bytes =
        std::fs::read(config_dir.join("affiliations.yaml")).context("cannot read affiliations.yaml")?;
    let companies_file: CompaniesFile = serde_yaml_ng::from_slice(&company_bytes).context("invalid companies.yaml")?;
    let affiliations_file: AffiliationsFile =
        serde_yaml_ng::from_slice(&affiliation_bytes).context("invalid affiliations.yaml")?;

    let mut catalog = CompanyCatalog::default();
    for (index, row) in companies_file.companies.into_iter().enumerate() {
        let context = format!("companies[{index}]");
        let id = row.id.trim().to_string();
        if !COMPANY_ID.is_match(&id) || ["unknown", "independent", "unmapped"].contains(&id.as_str()) {
            bail!("{context}: invalid or reserved company ID {id:?}");
        }
        if catalog.by_id.contains_key(&id) {
            bail!("duplicate company ID {id}");
        }
        let name = row.canonical_name.trim().to_string();
        let website = row.website_url.trim().to_string();
        if name.is_empty() {
            bail!("{context}.canonical_name must be a nonempty string");
        }
        let host = website.strip_prefix("https://").unwrap_or_default();
        let authority = host.split(['/', '?', '#']).next().unwrap_or_default();
        if authority.is_empty() || authority.contains('@') {
            bail!("{context}.website_url must be an HTTPS website URL");
        }
        if row.aliases.iter().any(|alias| alias.trim().is_empty()) {
            bail!("{context}.aliases must be a list of nonempty strings");
        }
        for alias in std::iter::once(&name).chain(&row.aliases) {
            let normalized = normalize_company(alias);
            if let Some(existing) = catalog.aliases.get(&normalized)
                && existing != &id
            {
                bail!("alias {alias:?} matches multiple companies");
            }
            catalog.aliases.insert(normalized, id.clone());
        }
        for domain in &row.domains {
            let normalized = domain.trim().to_lowercase();
            if normalized.is_empty() || !normalized.contains('.') || normalized.contains('@') {
                bail!("{context}.domains must be email domains such as example.com");
            }
            if let Some(existing) = catalog.domains.insert(normalized.clone(), id.clone())
                && existing != id
            {
                bail!("domain {normalized:?} belongs to multiple companies");
            }
        }
        catalog.by_id.insert(id.clone(), catalog.companies.len());
        catalog.companies.push(Company {
            id,
            canonical_name: name,
            aliases: row.aliases,
            website_url: website,
            maintainer: row.maintainer,
        });
    }

    let mut ids_by_login: HashMap<String, HashSet<String>> = HashMap::new();
    for contributor in contributors {
        let login = contributor.login.trim().to_lowercase();
        if !login.is_empty() && GITHUB_ID.is_match(&contributor.id) {
            ids_by_login.entry(login).or_default().insert(contributor.id.clone());
        }
    }

    for (index, row) in affiliations_file.affiliations.into_iter().enumerate() {
        let context = format!("affiliations[{index}]");
        let contributor_id = match (&row.username, &row.contributor_id) {
            (Some(username), None) => {
                let username = username.trim().trim_start_matches('@');
                let matches = ids_by_login.get(&username.to_lowercase()).cloned().unwrap_or_default();
                match matches.len() {
                    0 => bail!(
                        "{context}: username {username:?} was not found in the collected contributors; \
                         check the spelling, sync its profile, or use contributor_id: github:<numeric ID>"
                    ),
                    1 => matches.into_iter().next().unwrap_or_default(),
                    _ => bail!(
                        "{context}: username {username:?} matches multiple GitHub account IDs; \
                         use contributor_id to select the account"
                    ),
                }
            }
            (None, Some(id)) => {
                let id = id.trim().to_string();
                if !GITHUB_ID.is_match(&id) {
                    bail!("{context}: contributor_id must be github:<numeric ID>");
                }
                id
            }
            _ => bail!("{context}: specify exactly one of username or contributor_id"),
        };
        let status = row.status.clone().unwrap_or_else(|| "AFFILIATED".to_string());
        let company_id = row.company_id.clone().unwrap_or_default();
        if status != "AFFILIATED" && status != "INDEPENDENT" {
            bail!("{context}: status must be AFFILIATED or INDEPENDENT; omit unknown affiliations");
        }
        if (status == "AFFILIATED" && !catalog.by_id.contains_key(&company_id))
            || (status != "AFFILIATED" && !company_id.is_empty())
        {
            bail!("{context}: invalid company for affiliation status");
        }
        let start = row.valid_from.as_ref().map(|v| yaml_date(v, &format!("{context}.valid_from"))).transpose()?;
        let end = match &row.valid_until {
            Some(serde_yaml_ng::Value::Null) | None => None,
            Some(value) => Some(yaml_date(value, &format!("{context}.valid_until"))?),
        };
        let start = start.unwrap_or_default();
        let end = end.unwrap_or_default();
        if !end.is_empty() && start.is_empty() {
            bail!("{context}: valid_until requires valid_from");
        }
        if !end.is_empty() && start >= end {
            bail!("{context}: valid_until must follow valid_from");
        }
        let affiliation = Affiliation {
            contributor_id: contributor_id.clone(),
            company_id,
            status,
            valid_from: start.clone(),
            valid_until: end,
            source: "CURATED".to_string(),
            // Evidence lives in the reviewing PR; merging it establishes review.
            reviewed: true,
            ..Affiliation::default()
        };
        if start.is_empty() {
            if catalog.current.contains_key(&contributor_id) {
                bail!("{context}: duplicate undated affiliation for {contributor_id}");
            }
            catalog.current.insert(contributor_id, affiliation);
        } else {
            catalog.periods.entry(contributor_id).or_default().push(affiliation);
        }
    }
    for (contributor_id, rows) in catalog.periods.iter_mut() {
        rows.sort_by(|a, b| a.valid_from.cmp(&b.valid_from));
        for pair in rows.windows(2) {
            if pair[0].valid_until.is_empty() || pair[0].valid_until > pair[1].valid_from {
                bail!("overlapping affiliation periods for {contributor_id}");
            }
        }
    }
    let mut digest = Sha256::new();
    digest.update(&company_bytes);
    digest.update([0u8]);
    digest.update(&affiliation_bytes);
    catalog.revision = hex(&digest.finalize());
    Ok(catalog)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn contributor(id: &str, login: &str) -> Contributor {
        Contributor { id: id.into(), login: login.into(), ..Contributor::default() }
    }

    #[test]
    fn periods_take_precedence() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("companies.yaml"),
            "companies:\n- id: union\n  canonical_name: Union\n  website_url: https://union.ai\n  aliases: ['@unionai']\n- id: lyft\n  canonical_name: Lyft\n  website_url: https://lyft.com\n",
        )
        .unwrap();
        std::fs::write(
            dir.path().join("affiliations.yaml"),
            "affiliations:\n- username: Alice\n  company_id: union\n- username: alice\n  company_id: lyft\n  valid_from: '2020-01-01'\n  valid_until: '2021-01-01'\n",
        )
        .unwrap();
        let catalog = load_company_catalog(dir.path(), &[contributor("github:1", "alice")]).unwrap();
        assert_eq!(catalog.historical("github:1", "2020-06-01"), "lyft");
        assert_eq!(catalog.historical("github:1", "2021-01-01"), "union");
        assert_eq!(catalog.historical("github:2", "2021-01-01"), "");
        let mut alice = contributor("github:1", "alice");
        alice.profile_company = "@UnionAI".into();
        let current = catalog.current_affiliation(&alice, "2026-01-01");
        assert_eq!(current.company_id, "union");
        assert!(!current.conflict);
    }

    #[test]
    fn maintainer_days_follow_dated_periods() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("companies.yaml"),
            "companies:\n- id: union\n  canonical_name: Union\n  website_url: https://union.ai\n  maintainer: true\n- id: lyft\n  canonical_name: Lyft\n  website_url: https://lyft.com\n",
        )
        .unwrap();
        std::fs::write(
            dir.path().join("affiliations.yaml"),
            "affiliations:\n- username: alice\n  company_id: union\n- username: alice\n  company_id: lyft\n  valid_from: '2020-01-01'\n  valid_until: '2021-01-01'\n- username: bob\n  company_id: union\n  valid_from: '2022-01-01'\n  valid_until: '2023-01-01'\n- username: bob\n  company_id: lyft\n  valid_from: '2023-01-01'\n",
        )
        .unwrap();
        let people = [contributor("github:1", "alice"), contributor("github:2", "bob")];
        let catalog = load_company_catalog(dir.path(), &people).unwrap();
        let day = |value: &str| parse_date(value).unwrap();
        // Undated at Union except for a year at Lyft.
        assert_eq!(catalog.maintainer_days("github:1"), [(i32::MIN, day("2020-01-01")), (day("2021-01-01"), i32::MAX)]);
        assert_eq!(catalog.maintainer_days("github:2"), [(day("2022-01-01"), day("2023-01-01"))]);
        assert!(catalog.maintainer_days("github:3").is_empty());
    }

    #[test]
    fn loads_repository_config() {
        let config = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../config");
        let bytes = std::fs::read(config.join("affiliations.yaml")).unwrap();
        let text = String::from_utf8(bytes).unwrap();
        // Resolve every configured username (once, ignoring case) to a synthetic account.
        let mut logins: Vec<String> = text
            .lines()
            .filter_map(|line| line.trim().strip_prefix("- username:"))
            .map(|login| login.trim().trim_matches('\'').trim_start_matches('@').to_lowercase())
            .collect();
        logins.sort();
        logins.dedup();
        let contributors: Vec<Contributor> =
            logins.iter().enumerate().map(|(i, login)| contributor(&format!("github:{i}"), login)).collect();
        let catalog = load_company_catalog(&config, &contributors).unwrap();
        assert!(catalog.companies.len() > 50);
        assert!(!catalog.current.is_empty());
    }
}
