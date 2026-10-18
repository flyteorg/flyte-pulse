//! Contributor identities: email hashing, GitHub noreply addresses, and profile merging.

use std::collections::HashMap;

use pulse_core::state::{CompanyObservation, Contributor};
use pulse_core::time::Ts;
use sha2::{Digest, Sha256};

use crate::config::{classify_github_account, hex};

/// Emails are never stored; commits keep this truncated SHA-256 of the lowercased address.
pub fn email_hash(email: &str) -> String {
    let normalized = email.trim().to_lowercase();
    if normalized.is_empty() {
        return String::new();
    }
    hex(&Sha256::digest(normalized.as_bytes()))[..32].to_string()
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Noreply {
    /// `<id>+<login>@users.noreply.github.com`
    WithId { id: String, login: String },
    /// `<login>@users.noreply.github.com` (older accounts)
    LoginOnly { login: String },
}

pub fn parse_noreply(email: &str) -> Option<Noreply> {
    let email = email.trim();
    let (local, domain) = email.rsplit_once('@')?;
    if !domain.eq_ignore_ascii_case("users.noreply.github.com") || local.is_empty() {
        return None;
    }
    if let Some((id, login)) = local.split_once('+')
        && !id.is_empty()
        && id.bytes().all(|b| b.is_ascii_digit())
        && !login.is_empty()
    {
        return Some(Noreply::WithId { id: id.to_string(), login: login.to_string() });
    }
    Some(Noreply::LoginOnly { login: local.to_string() })
}

/// Account details from a GitHub payload, before merging into the stored row.
#[derive(Clone, Debug, Default)]
pub struct Account {
    pub database_id: i64,
    pub login: String,
    /// `User`, `Bot`, ...; empty when unknown.
    pub account_type: String,
    pub name: Option<String>,
    pub company: Option<String>,
    pub url: Option<String>,
}

impl Account {
    pub fn contributor_id(&self) -> String {
        format!("github:{}", self.database_id)
    }

    /// A contributor row as seen in an activity payload (pull request author,
    /// commit author lookup, or noreply address).
    pub fn to_contributor(&self, observed_at: Ts, identity_rules: &HashMap<String, String>) -> Contributor {
        let (identity_kind, identity_kind_source) =
            classify_github_account(&self.login, &self.account_type, identity_rules);
        let name = self.name.as_deref().map(str::trim).unwrap_or("");
        let is_bot = identity_kind == "BOT"
            || self.account_type.eq_ignore_ascii_case("bot")
            || self.login.to_lowercase().ends_with("[bot]");
        Contributor {
            id: self.contributor_id(),
            provider: "GITHUB".into(),
            provider_user_id: self.database_id.to_string(),
            login: self.login.clone(),
            display_name: if name.is_empty() { self.login.clone() } else { name.to_string() },
            profile_url: self
                .url
                .clone()
                .filter(|u| !u.is_empty())
                .unwrap_or_else(|| format!("https://github.com/{}", self.login)),
            is_bot,
            profile_company: self.company.as_deref().unwrap_or("").trim().to_string(),
            profile_observed_at: Some(observed_at),
            identity_kind,
            identity_kind_source,
            profile_name_checked: !name.is_empty(),
            profile_company_history: Vec::new(),
        }
    }
}

/// Merge an activity payload into the stored row: an enriched display name, the
/// company, and its observation history only change through a full profile refresh.
pub fn merge_contributor(old: Option<&Contributor>, update: Contributor) -> Contributor {
    let Some(old) = old else {
        return update;
    };
    let mut row = old.clone();
    let login =
        if update.login.trim().is_empty() { old.login.trim().to_string() } else { update.login.trim().to_string() };
    let display_name = update.display_name.trim();
    row.display_name = if display_name.is_empty() || display_name.eq_ignore_ascii_case(&login) {
        let old_display = old.display_name.trim();
        if !old_display.is_empty() && !old.login.is_empty() && !old_display.eq_ignore_ascii_case(old.login.trim()) {
            old_display.to_string()
        } else if !login.is_empty() {
            login.clone()
        } else {
            old_display.to_string()
        }
    } else {
        display_name.to_string()
    };
    row.login = login;
    if !update.profile_url.is_empty() {
        row.profile_url = update.profile_url;
    }
    if !update.identity_kind.is_empty() && update.identity_kind != "UNKNOWN" {
        row.identity_kind = update.identity_kind;
        row.identity_kind_source = update.identity_kind_source;
        row.is_bot = update.is_bot || row.identity_kind == "BOT";
    }
    if row.provider.is_empty() {
        row.provider = update.provider;
    }
    if row.provider_user_id.is_empty() {
        row.provider_user_id = update.provider_user_id;
    }
    row
}

/// Insert or merge an activity payload into `contributors`.
pub fn upsert_contributor(
    contributors: &mut Vec<Contributor>,
    index: &mut HashMap<String, usize>,
    update: Contributor,
) {
    match index.get(&update.id) {
        Some(&position) => {
            let merged = merge_contributor(Some(&contributors[position]), update);
            contributors[position] = merged;
        }
        None => {
            index.insert(update.id.clone(), contributors.len());
            contributors.push(update);
        }
    }
}

/// Apply a full public profile lookup.
pub fn apply_profile(row: &mut Contributor, profile: &Account, observed_at: Ts) {
    let name = profile.name.as_deref().unwrap_or("").trim().to_string();
    if !profile.login.is_empty() {
        row.login = profile.login.clone();
    }
    row.display_name = if name.is_empty() { row.login.clone() } else { name };
    if let Some(url) = profile.url.as_deref().filter(|u| !u.is_empty()) {
        row.profile_url = url.to_string();
    }
    // A successful lookup always reports the company; null means explicitly cleared.
    if row.profile_company_history.is_empty()
        && row.profile_name_checked
        && let Some(previous) = row.profile_observed_at
    {
        row.profile_company_history
            .push(CompanyObservation { company: row.profile_company.clone(), observed_at: Some(previous) });
    }
    let company = profile.company.as_deref().unwrap_or("").trim().to_string();
    row.profile_company_history.push(CompanyObservation { company: company.clone(), observed_at: Some(observed_at) });
    row.profile_company = company;
    row.profile_observed_at = Some(observed_at);
    row.profile_name_checked = true;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hashes_normalized_emails() {
        assert_eq!(email_hash(" Ada@Example.COM "), email_hash("ada@example.com"));
        assert_eq!(email_hash("ada@example.com").len(), 32);
        assert_eq!(email_hash("  "), "");
    }

    #[test]
    fn parses_noreply_addresses() {
        assert_eq!(
            parse_noreply("49699333+dependabot[bot]@users.noreply.github.com"),
            Some(Noreply::WithId { id: "49699333".into(), login: "dependabot[bot]".into() })
        );
        assert_eq!(
            parse_noreply("octocat@users.noreply.github.com"),
            Some(Noreply::LoginOnly { login: "octocat".into() })
        );
        assert_eq!(parse_noreply("noreply@github.com"), None);
        assert_eq!(parse_noreply("ada@example.com"), None);
    }

    fn row(login: &str, display: &str, company: &str) -> Contributor {
        Contributor {
            id: "github:1".into(),
            login: login.into(),
            display_name: display.into(),
            profile_company: company.into(),
            identity_kind: "HUMAN".into(),
            identity_kind_source: "GITHUB_ACCOUNT_TYPE".into(),
            profile_name_checked: true,
            ..Contributor::default()
        }
    }

    #[test]
    fn merge_keeps_enriched_name_and_company() {
        let old = row("ada", "Ada Lovelace", "Analytical Engines");
        let update = Contributor { profile_name_checked: false, ..row("ada", "ada", "") };
        let merged = merge_contributor(Some(&old), update);
        assert_eq!(merged.display_name, "Ada Lovelace");
        assert_eq!(merged.profile_company, "Analytical Engines");
        assert!(merged.profile_name_checked);

        let renamed = merge_contributor(Some(&old), row("ada2", "Ada L.", "Other"));
        assert_eq!(renamed.login, "ada2");
        assert_eq!(renamed.display_name, "Ada L.");
        assert_eq!(renamed.profile_company, "Analytical Engines");
    }

    #[test]
    fn profile_refresh_records_company_history() {
        let mut contributor = row("ada", "ada", "Old Co");
        contributor.profile_observed_at = Some(Ts(100));
        let profile = Account {
            database_id: 1,
            login: "ada".into(),
            account_type: "User".into(),
            name: Some("Ada".into()),
            company: None,
            url: Some("https://github.com/ada".into()),
        };
        apply_profile(&mut contributor, &profile, Ts(200));
        assert_eq!(contributor.display_name, "Ada");
        assert_eq!(contributor.profile_company, "");
        let history: Vec<_> =
            contributor.profile_company_history.iter().map(|o| (o.company.as_str(), o.observed_at)).collect();
        assert_eq!(history, vec![("Old Co", Some(Ts(100))), ("", Some(Ts(200)))]);
    }
}
